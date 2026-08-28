//! legal hold: freeze a knowledge id against every
//! erasure path (decay, `/purge`, DSAR) until every hold on it is explicitly
//! released. The WORM-lite posture regulated buyers need (finance/government/
//! litigation), layered on the existing supersede-not-delete + append-only
//! audit foundation — no background worker, no auto-release.
//!
//! The `legal_holds` table lives in every domain DB (migration runs per-file),
//! so an enforcement check is local to the same pool + transaction as the
//! purge it gates. Multiple concurrent holds on one id are allowed (litigation
//! + retention audit); an id is erasable only when ALL its holds are released.
//!
//! Decay interaction: a held id is absent from the `/decayed` registry — the
//! operator never sees a held id as "safe to purge". Erasure interaction:
//! `/purge` refuses with `409 legal_hold_active` listing the reasons; a DSAR
//! defers held ids and lists them (+ reasons) on the certificate — the
//! legal-defensibility artifact that tells the subject *why* erasure is
//! deferred. A held id is frozen against erasure until full release.

use std::collections::{HashMap, HashSet};

use crate::handlers::HandlerError;

/// Max ids per `POST /legal-hold` (the `MAX_PURGE_IDS` bound — holds gate
/// purges, so they accept the same blast radius).
pub(crate) const MAX_HOLD_IDS: usize = 1000;
/// Max reason length (a human citation: case number, ticket, regulation).
pub(crate) const MAX_HOLD_REASON: usize = 500;

/// One `legal_holds` row. `released_at: None` = the hold is active.
#[derive(Debug, serde::Serialize)]
pub(crate) struct LegalHoldRow {
    pub id: i64,
    pub knowledge_id: i64,
    pub reason: String,
    pub held_by: Option<String>,
    pub held_at: i64,
    pub released_at: Option<i64>,
}

/// Validate ids + reason before any write. Shared by the handler seam.
pub(crate) fn validate(ids: &[i64], reason: &str) -> Result<(), HandlerError> {
    if ids.is_empty() {
        return Err(HandlerError::bad_request(
            "no_ids",
            "legal-hold requires a non-empty ids list",
        ));
    }
    if ids.len() > MAX_HOLD_IDS {
        return Err(HandlerError::bad_request(
            "too_many_ids",
            format!("legal-hold accepts at most {MAX_HOLD_IDS} ids"),
        ));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(HandlerError::bad_request(
            "reason_empty",
            "legal-hold requires a reason (case number, ticket, regulation)",
        ));
    }
    if reason.len() > MAX_HOLD_REASON {
        return Err(HandlerError::bad_request(
            "reason_too_long",
            format!("reason exceeds {MAX_HOLD_REASON} characters"),
        ));
    }
    Ok(())
}

/// Insert one hold per id (caller validates first). Runs inside the caller's
/// transaction. Returns the created hold ids.
pub(crate) fn insert_holds(
    tx: &rusqlite::Transaction,
    ids: &[i64],
    reason: &str,
    held_by: Option<&str>,
    now: i64,
) -> Result<Vec<i64>, HandlerError> {
    let mut created = Vec::with_capacity(ids.len());
    for id in ids {
        tx.execute(
            "INSERT INTO legal_holds(knowledge_id, reason, held_by, held_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, reason, held_by, now],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
        created.push(tx.last_insert_rowid());
    }
    Ok(created)
}

/// The active-hold reasons for each of `ids` that is currently held (held ids
/// missing from the map are the free set). The 409/certificate payload
/// builder. Pulls the active holds (a tiny, partial index-served set) and
/// filters in Rust — no IN-list param cap, no batching. Fails with the bare
/// rusqlite error (Quarry convention: storage helpers return storage errors;
/// the handler boundary maps them onto the route vocabulary).
pub(crate) fn active_reasons(
    conn: &rusqlite::Connection,
    ids: &[i64],
) -> Result<HashMap<i64, Vec<String>>, rusqlite::Error> {
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let want: HashSet<i64> = ids.iter().copied().collect();
    let mut stmt =
        conn.prepare("SELECT knowledge_id, reason FROM legal_holds WHERE released_at IS NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for (kid, reason) in rows.flatten() {
        if want.contains(&kid) {
            out.entry(kid).or_default().push(reason);
        }
    }
    Ok(out)
}

/// Every actively-held knowledge id (the `/decayed` exclusion set).
pub(crate) fn active_hold_ids(conn: &rusqlite::Connection) -> Result<HashSet<i64>, HandlerError> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT knowledge_id FROM legal_holds WHERE released_at IS NULL")
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, i64>(0))
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(rows.flatten().collect())
}

/// refuse an erasure when ANY target id has an
/// active hold. Runs inside the caller's write transaction (before the first
/// DELETE), so every erasure path — `/purge`, DSAR, `DELETE /memory/{id}`, the
/// source sweeps, quarantine delete, domain delete — is frozen by the same
/// guard. Emits the exact `409 legal_hold_active` shape `/purge` already
/// produces, so clients see one envelope.
pub(crate) fn refuse_if_held(
    tx: &rusqlite::Transaction<'_>,
    ids: &[i64],
) -> Result<(), HandlerError> {
    let held = active_reasons(tx, ids).map_err(|e| HandlerError::internal(e.to_string()))?;
    if !held.is_empty() {
        return Err(HandlerError::conflict_with(
            "legal_hold_active",
            "one or more ids are under legal hold",
            serde_json::json!({ "held": held }),
        ));
    }
    Ok(())
}

/// Release hold `id` (explicit action, never auto). Returns the hold row when
/// it exists (with its pre-release active state), else None → 404.
pub(crate) fn release(
    tx: &rusqlite::Transaction,
    id: i64,
    now: i64,
) -> Result<Option<LegalHoldRow>, HandlerError> {
    let row: Option<LegalHoldRow> = tx
        .query_row(
            "SELECT id, knowledge_id, reason, held_by, held_at, released_at
               FROM legal_holds WHERE id = ?1",
            rusqlite::params![id],
            |r| {
                Ok(LegalHoldRow {
                    id: r.get(0)?,
                    knowledge_id: r.get(1)?,
                    reason: r.get(2)?,
                    held_by: r.get(3)?,
                    held_at: r.get(4)?,
                    released_at: r.get(5)?,
                })
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let Some(hold) = row else {
        return Ok(None);
    };
    if hold.released_at.is_none() {
        tx.execute(
            "UPDATE legal_holds SET released_at = ?1 WHERE id = ?2 AND released_at IS NULL",
            rusqlite::params![now, id],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    }
    Ok(Some(hold))
}

/// The `GET /legal-holds` registry, newest-first, bounded. `id` filters by
/// knowledge id, `reason` by substring (both optional).
pub(crate) fn list_holds(
    conn: &rusqlite::Connection,
    id: Option<i64>,
    reason: Option<&str>,
    limit: i64,
) -> Result<(Vec<LegalHoldRow>, i64), HandlerError> {
    let mut sql = String::from(
        "SELECT id, knowledge_id, reason, held_by, held_at, released_at FROM legal_holds",
    );
    let mut clauses: Vec<&'static str> = Vec::new();
    if id.is_some() {
        clauses.push("knowledge_id = ?");
    }
    if reason.is_some() {
        clauses.push("reason LIKE ?");
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(k) = id {
        params.push(Box::new(k));
    }
    if let Some(needle) = reason {
        params.push(Box::new(format!("%{needle}%")));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM ({sql}) t"),
            refs.as_slice(),
            |r| r.get(0),
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    sql.push_str(" ORDER BY id DESC LIMIT ?");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    params.push(Box::new(limit));
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(refs.as_slice(), |r| {
            Ok(LegalHoldRow {
                id: r.get(0)?,
                knowledge_id: r.get(1)?,
                reason: r.get(2)?,
                held_by: r.get(3)?,
                held_at: r.get(4)?,
                released_at: r.get(5)?,
            })
        })
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok((rows.flatten().collect(), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE legal_holds(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                knowledge_id INTEGER NOT NULL,
                reason TEXT NOT NULL,
                held_by TEXT,
                held_at INTEGER NOT NULL,
                released_at INTEGER
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn validate_rejects_empty_and_oversized() {
        assert!(validate(&[], "litigation").is_err());
        assert!(validate(&[1], "  ").is_err());
        assert!(validate(&[1], &"x".repeat(501)).is_err());
        assert!(validate(&(1..=1001).collect::<Vec<_>>(), "ok").is_err());
        assert!(validate(&[1, 2], "litigation 2026-118").is_ok());
    }

    #[test]
    fn concurrent_holds_all_must_release() {
        // Verification 2 core: two holds on one id; releasing one leaves the
        // id frozen, releasing both frees it.
        let mut conn = db();
        let tx = conn.transaction().unwrap();
        insert_holds(&tx, &[7], "litigation", Some("dpo"), 100).unwrap();
        insert_holds(&tx, &[7], "sox audit", Some("counsel"), 101).unwrap();
        tx.commit().unwrap();

        let reasons = active_reasons(&conn, &[7]).unwrap();
        assert_eq!(reasons.len(), 1, "one held id");
        assert_eq!(
            reasons[&7].len(),
            2,
            "both concurrent holds are active: {:?}",
            reasons[&7]
        );

        // Release hold #1 → the id is still held by #2.
        let tx = conn.transaction().unwrap();
        let held = release(&tx, 1, 200).unwrap().unwrap();
        tx.commit().unwrap();
        assert!(held.released_at.is_none(), "row shows pre-release state");
        let reasons = active_reasons(&conn, &[7]).unwrap();
        assert_eq!(reasons[&7], vec!["sox audit".to_string()], "still frozen");
        assert!(active_hold_ids(&conn).unwrap().contains(&7));

        // Release hold #2 → erasable.
        let tx = conn.transaction().unwrap();
        assert!(release(&tx, 2, 201).unwrap().is_some());
        tx.commit().unwrap();
        assert!(active_reasons(&conn, &[7]).unwrap().is_empty());
        assert!(active_hold_ids(&conn).unwrap().is_empty());
    }

    #[test]
    fn release_is_idempotent_and_404s_on_unknown() {
        let mut conn = db();
        let tx = conn.transaction().unwrap();
        insert_holds(&tx, &[1], "audit", None, 10).unwrap();
        tx.commit().unwrap();
        let tx = conn.transaction().unwrap();
        let first = release(&tx, 1, 20).unwrap().unwrap();
        tx.commit().unwrap();
        let tx = conn.transaction().unwrap();
        let second = release(&tx, 1, 30).unwrap().unwrap();
        tx.commit().unwrap();
        assert_eq!(first.id, second.id);
        // Idempotent: the second release is a no-op — it returns the row whose
        // `released_at` already holds the first (winning) release timestamp.
        assert_eq!(second.released_at, Some(20), "no re-release");
        let released_at: Option<i64> = conn
            .query_row(
                "SELECT released_at FROM legal_holds WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(released_at, Some(20), "first release wins; no re-release");
        let tx = conn.transaction().unwrap();
        assert!(release(&tx, 99, 30).unwrap().is_none());
        tx.commit().unwrap();
    }

    #[test]
    fn list_filters_by_knowledge_id_and_reason() {
        let mut conn = db();
        let tx = conn.transaction().unwrap();
        insert_holds(&tx, &[1], "litigation 2026-118", Some("dpo"), 100).unwrap();
        insert_holds(&tx, &[2], "sox audit", None, 101).unwrap();
        insert_holds(&tx, &[3], "litigation 2027-001", None, 102).unwrap();
        tx.commit().unwrap();
        let (all, total) = list_holds(&conn, None, None, 10).unwrap();
        assert_eq!((all.len(), total), (3, 3));
        assert_eq!(all[0].id, 3, "newest-first");
        let (by_id, _) = list_holds(&conn, Some(2), None, 10).unwrap();
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0].reason, "sox audit");
        let (lit, _) = list_holds(&conn, None, Some("litigation"), 10).unwrap();
        assert_eq!(lit.len(), 2);
    }

    #[test]
    fn active_reasons_covers_many_ids() {
        // 5 ids through the pull-and-filter path: every hold is found, none invented.
        let mut conn = db();
        let tx = conn.transaction().unwrap();
        insert_holds(&tx, &[1, 2, 3, 4, 5], "audit", None, 1).unwrap();
        tx.commit().unwrap();
        let reasons = active_reasons(&conn, &[1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(reasons.len(), 5, "id 6 is free");
    }
}
