//! GPSR recall mode: traceability is the entitlement registry's serial and
//! batch spine. A recall campaign is a BLAST PROPOSAL — human-triggered,
//! DPO-visible, never an autonomous send — carrying the Safety Gate
//! reference fields (Reg. 2023/988) and the affected-unit set computed
//! deterministically from `memory_kind='entitlement'` knowledge rows.

use rusqlite::Connection;

use crate::workflow::entitlement::{EntitlementRecord, region_allows};

/// Read cap on the registry scan — bounds law: every read surface is capped.
pub const RECALL_QUERY_LIMIT: usize = 1_000;

/// Deterministic traceability query over the entitlement registry: every
/// governed row of the given product whose serial OR batch matches, inside
/// this server's region stamp (cross-region rows are foreign and fail
/// closed). Malformed rows deny loudly — a registry row that cannot be read
/// never silently drops out of a recall set.
pub fn traceability_query(
    conn: &Connection,
    product: &str,
    serial_or_batch: &str,
    stamp: &str,
) -> Result<Vec<EntitlementRecord>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT content FROM knowledge
             WHERE knowledge_type = 'entitlement' AND flagged = 0
             ORDER BY id ASC LIMIT ?1",
        )
        .map_err(|e| format!("recall_registry_unreadable: {e}"))?;
    let mut rows = Vec::new();
    let result = stmt.query_map([RECALL_QUERY_LIMIT as i64], |r| r.get::<_, String>(0));
    let mut iter = result.map_err(|e| format!("recall_registry_unreadable: {e}"))?;
    for row in (&mut iter).filter_map(Result::ok) {
        let record = EntitlementRecord::parse(&row)
            .map_err(|e| format!("recall_registry_row_denied: {e}"))?;
        if record.product != product || !region_allows(&record.region, stamp) {
            continue;
        }
        let matches = !serial_or_batch.is_empty()
            && (record.serial == serial_or_batch
                || record.batch.as_deref() == Some(serial_or_batch));
        if matches {
            rows.push(record);
        }
    }
    Ok(rows)
}

/// The Safety Gate reference fields (GPSR Reg. 2023/988 posture): the
/// notification id and member state MUST be present before any campaign
/// packet exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyGateRefs {
    pub notification_id: String,
    pub member_state: String,
    pub hazard_class: String,
    pub corrective_action: String,
}

/// Build the recall blast-proposal payload. Fail-closed: no Safety Gate
/// reference, or an empty affected set, is not a campaign. The payload is a
/// PROPOSAL — approval, customer notification, and execution stay human.
pub fn build_recall_campaign(
    refs: &SafetyGateRefs,
    affected: &[EntitlementRecord],
    remediation_disposition: &str,
) -> Result<serde_json::Value, String> {
    if refs.notification_id.trim().is_empty() || refs.member_state.trim().is_empty() {
        return Err("recall_safety_gate_refs_required".to_string());
    }
    if affected.is_empty() {
        return Err("recall_blast_without_affected_units".to_string());
    }
    let units: Vec<serde_json::Value> = affected
        .iter()
        .map(|r| {
            serde_json::json!({
                "product": r.product,
                "serial": r.serial,
                "batch": r.batch,
                "remediation_disposition": remediation_disposition,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "proposal_kind": "recall_blast",
        "human_triggered": true,
        "dpo_visible": true,
        "legal_basis": "GPSR-2023/988",
        "safety_gate": {
            "notification_id": refs.notification_id,
            "member_state": refs.member_state,
            "hazard_class": refs.hazard_class,
            "corrective_action": refs.corrective_action,
        },
        "affected_units": units,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_conn() -> (Connection, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tmpdir");
        let conn = Connection::open(dir.path().join("recall-test.db")).expect("open");
        conn.execute_batch(
            "CREATE TABLE knowledge(
                id INTEGER PRIMARY KEY,
                content TEXT NOT NULL,
                knowledge_type TEXT,
                flagged INTEGER NOT NULL DEFAULT 0);",
        )
        .expect("create knowledge");
        (conn, dir)
    }

    fn insert(conn: &Connection, json: &str) {
        conn.execute(
            "INSERT INTO knowledge(content, knowledge_type) VALUES (?1, 'entitlement')",
            [json],
        )
        .expect("insert");
    }

    #[test]
    fn serial_batch_query_backs_traceability() {
        let (conn, _dir) = file_conn();
        insert(
            &conn,
            r#"{"product":"kettle-k2","serial":"SN-100","batch":"B-2026-03","purchase_date":"2026-01-10","region":"eu"}"#,
        );
        insert(
            &conn,
            r#"{"product":"kettle-k2","serial":"SN-200","purchase_date":"2026-02-01","region":"eu"}"#,
        );
        // Foreign-region row: exists but must NEVER answer to this stamp.
        insert(
            &conn,
            r#"{"product":"kettle-k2","serial":"SN-300","batch":"B-2026-03","purchase_date":"2026-01-11","region":"us"}"#,
        );
        // Wrong product.
        insert(
            &conn,
            r#"{"product":"toaster-t1","serial":"SN-100","purchase_date":"2026-01-12","region":"eu"}"#,
        );

        // An unstamped registry answers only to an unstamped site; these
        // rows are stamped eu, so the site stamps itself eu too.
        let by_serial = traceability_query(&conn, "kettle-k2", "SN-100", "eu").expect("query");
        assert_eq!(by_serial.len(), 1);
        assert_eq!(by_serial[0].serial, "SN-100");

        // Batch matching gathers every unit of the batch — the recall spine.
        let by_batch = traceability_query(&conn, "kettle-k2", "B-2026-03", "eu").expect("query");
        assert_eq!(
            by_batch.len(),
            1,
            "foreign-region row is excluded from the batch set"
        );
        assert_eq!(by_batch[0].serial, "SN-100");

        // A stamped site sees only its own region's rows.
        let stamped = traceability_query(&conn, "kettle-k2", "B-2026-03", "eu").expect("query");
        assert_eq!(stamped.len(), 1);
        assert!(
            traceability_query(&conn, "kettle-k2", "", "eu")
                .expect("empty needle")
                .is_empty()
        );

        // A malformed registry row denies loudly — recalls cannot run over
        // a registry that half-reads.
        insert(&conn, "{not json");
        assert!(traceability_query(&conn, "kettle-k2", "SN-100", "").is_err());
    }

    #[test]
    fn recall_campaign_is_a_blast_proposal_with_safety_gate_refs() {
        let refs = SafetyGateRefs {
            notification_id: "SG-2026-0442".to_string(),
            member_state: "DE".to_string(),
            hazard_class: "burn".to_string(),
            corrective_action: "repair".to_string(),
        };
        let unit = EntitlementRecord::parse(
            r#"{"product":"kettle-k2","serial":"SN-100","purchase_date":"2026-01-10"}"#,
        )
        .expect("row");
        let payload = build_recall_campaign(&refs, std::slice::from_ref(&unit), "replace_unit")
            .expect("campaign");
        assert_eq!(payload["proposal_kind"], "recall_blast");
        assert_eq!(
            payload["human_triggered"], true,
            "a recall is always human-triggered"
        );
        assert_eq!(payload["dpo_visible"], true);
        assert_eq!(payload["legal_basis"], "GPSR-2023/988");
        assert_eq!(payload["safety_gate"]["notification_id"], "SG-2026-0442");
        assert_eq!(payload["affected_units"][0]["serial"], "SN-100");
        assert_eq!(
            payload["affected_units"][0]["remediation_disposition"],
            "replace_unit"
        );

        // Fail-closed: no Safety Gate refs → no campaign; no affected units
        // → not a campaign either.
        let empty_refs = SafetyGateRefs {
            notification_id: String::new(),
            ..refs.clone()
        };
        assert!(build_recall_campaign(&empty_refs, std::slice::from_ref(&unit), "r").is_err());
        assert!(build_recall_campaign(&refs, &[], "r").is_err());
    }
}
