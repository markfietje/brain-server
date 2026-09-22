//! The account record — the StewardOS account lane's storage core.
//!
//! Accounts are `workflow_runs` rows with kind [`ACCOUNT_KIND`]: the loop's
//! own tenant-scoped store, so domain scoping, audit tenant lookup, and the
//! revision column come free, and the record layer adds no table and no
//! column. The loop itself never claims an account run — the GDL loader
//! refuses non-`troubleshoot` kinds and no driver claims the kind, which the
//! law test `account_run_is_inert_to_the_loop` pins end to end (a real
//! driver, refusal before any provider work, row untouched).
//!
//! The deliberately-not-a-CRM boundary: the record holds identifiers (name,
//! owner label, status, timestamps) — never request bodies, which stay in
//! the existing gated run rows. Every write lands its own audited row; the
//! audit detail carries ids and counts, never the name text.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::workflow::tx::WorkflowTx;

/// The workflow_runs kind this module owns. NEVER `troubleshoot`: the GDL
/// loader's kind check is what keeps account runs inert to the loop.
pub(crate) const ACCOUNT_KIND: &str = "account";

/// The additive session-log kind carrying the request→account linkage,
/// written under the ACCOUNT's run id.
pub(crate) const LINK_ROW_KIND: &str = "account:link";

/// The audit targets this module writes (one per account operation).
pub(crate) const AUDIT_ACCOUNT: &str = "account";
pub(crate) const AUDIT_ACCOUNT_LINK: &str = "account_link";
pub(crate) const AUDIT_ACCOUNT_ARCHIVE: &str = "account_archive";

/// Name + identifier bound: the `decision_ref` screening shape.
pub(crate) const MAX_NAME_LEN: usize = 256;

/// The record's status vocabulary (closed; the run COLUMN keeps the
/// existing run vocabulary — account code never writes it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AccountStatus {
    Active,
    Archived,
}

impl AccountStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            AccountStatus::Active => "active",
            AccountStatus::Archived => "archived",
        }
    }
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "active" => Some(AccountStatus::Active),
            "archived" => Some(AccountStatus::Archived),
            _ => None,
        }
    }
}

/// The account record as it lives in the run row's `state_json`. The field
/// set is closed: identifiers only, no request bodies.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct AccountRecord {
    pub account_id: String,
    pub name: String,
    pub owner_principal: String,
    pub status: AccountStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The name screening both the creator and the parser ride: trimmed,
/// bounded 1..=256, control/invisible-refused. Named refusals in the
/// `account: <field> <problem>` vocabulary; never panics.
pub(crate) fn screen_account_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("account: name required".into());
    }
    if trimmed.len() > MAX_NAME_LEN {
        return Err(format!("account: name over {MAX_NAME_LEN} chars"));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
    {
        return Err("account: name carries control or invisible characters".into());
    }
    Ok(trimmed.to_string())
}

/// The total parser every stored account record round-trips through: any
/// input yields the record or a named `account: <field> <problem>` refusal.
/// No I/O, no clock, no panic.
pub(crate) fn parse_account_record(value: &serde_json::Value) -> Result<AccountRecord, String> {
    let Some(obj) = value.as_object() else {
        return Err("account: record must be a JSON object".into());
    };
    let account_id = text_field(obj, "account_id")?;
    let name = text_field(obj, "name")?;
    let owner_principal = text_field(obj, "owner_principal")?;
    let status_raw = text_field(obj, "status")?;
    let status = AccountStatus::parse(&status_raw)
        .ok_or_else(|| "account: status must be active | archived".to_string())?;
    let created_at = int_field(obj, "created_at")?;
    let updated_at = int_field(obj, "updated_at")?;
    Ok(AccountRecord {
        account_id,
        name,
        owner_principal,
        status,
        created_at,
        updated_at,
    })
}

fn text_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<String, String> {
    let raw = obj
        .get(field)
        .ok_or_else(|| format!("account: {field} absent"))?;
    let Some(s) = raw.as_str() else {
        return Err(format!("account: {field} must be a string"));
    };
    if s.is_empty() {
        return Err(format!("account: {field} required"));
    }
    if s.len() > MAX_NAME_LEN {
        return Err(format!("account: {field} over {MAX_NAME_LEN} chars"));
    }
    Ok(s.to_string())
}

fn int_field(obj: &serde_json::Map<String, serde_json::Value>, field: &str) -> Result<i64, String> {
    let raw = obj
        .get(field)
        .ok_or_else(|| format!("account: {field} absent"))?;
    raw.as_i64()
        .filter(|t| *t >= 0)
        .ok_or_else(|| format!("account: {field} must be a non-negative integer"))
}

fn record_value(record: &AccountRecord) -> Result<String, String> {
    serde_json::to_string(record).map_err(|e| format!("account: record serialize failed: {e}"))
}

/// The receipt for a create: the run id (the account id) and the stored
/// record as written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AccountCreated {
    pub run_id: i64,
    pub record: AccountRecord,
}

/// Create an account: ONE run row (kind `account`, column status `active`,
/// revision 0) carrying the record, plus ONE audited row. The caller's
/// `WorkflowTx` makes record + audit atomic. The client supplies nothing
/// but the name — id, owner label, status, and clock are server-derived.
pub(crate) fn create_account(
    tx: &mut WorkflowTx<'_>,
    domain: &str,
    name: &str,
    owner: &str,
    now: i64,
) -> Result<AccountCreated, String> {
    let screened = screen_account_name(name)?;
    let run_id = super::state::open_run(tx.tx(), domain, ACCOUNT_KIND, "{}", now)
        .map_err(|e| format!("account: run open failed: {e}"))?;
    let record = AccountRecord {
        account_id: run_id.to_string(),
        name: screened,
        owner_principal: owner.to_string(),
        status: AccountStatus::Active,
        created_at: now,
        updated_at: now,
    };
    let payload = record_value(&record)?;
    let updated = tx
        .tx()
        .execute(
            "UPDATE workflow_runs SET state_json = ?2, updated_at = ?3
              WHERE id = ?1 AND state_revision = 0",
            params![run_id, payload, now],
        )
        .map_err(|e| format!("account: record write failed: {e}"))?;
    if updated != 1 {
        return Err("account: record write missed the fresh row".into());
    }
    // Audit detail: ids and lengths only — never the name text.
    super::audit_write(
        tx.tx(),
        run_id,
        AUDIT_ACCOUNT,
        crate::audit::AuditStatus::Ok,
        &format!(
            "account created in {domain} (name len {})",
            record.name.len()
        ),
    );
    Ok(AccountCreated { run_id, record })
}

/// Load one account record. `None` when the id is absent OR the run is not
/// an account (the two are the same answer — probe-blind at the surface).
/// A present-but-unreadable record is a real fault, not a silent 404.
pub(crate) fn load_account(
    conn: &Connection,
    account_id: i64,
) -> rusqlite::Result<Option<AccountRecord>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT state_json, kind FROM workflow_runs WHERE id = ?1",
            params![account_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((state_json, kind)) = row else {
        return Ok(None);
    };
    if kind != ACCOUNT_KIND {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(&state_json).map_err(|e| {
        rusqlite::Error::InvalidParameterName(format!("account: stored record unreadable: {e}"))
    })?;
    parse_account_record(&value)
        .map(Some)
        .map_err(rusqlite::Error::InvalidParameterName)
}

/// Archive an account: the record-level `active → archived` transition, the
/// machine-refusal law verbatim — no decision reference, no archive. The
/// run COLUMN status stays `active` (the reader vocabulary is untouched);
/// only the record flips.
pub(crate) fn archive_account(
    tx: &mut WorkflowTx<'_>,
    account_id: i64,
    decision_ref: &str,
    now: i64,
) -> Result<AccountRecord, String> {
    if decision_ref.trim().is_empty() {
        return Err("account_archived_requires_decision_ref".into());
    }
    let conn = tx.tx();
    let record = load_account(conn, account_id)
        .map_err(|e| format!("account: stored record unreadable: {e}"))?
        .ok_or("account_not_found")?;
    match record.status {
        AccountStatus::Archived => {
            return Err("account: status already archived".into());
        }
        AccountStatus::Active => {}
    }
    let revision: i64 = conn
        .query_row(
            "SELECT state_revision FROM workflow_runs WHERE id = ?1",
            params![account_id],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| format!("account: revision read failed: {e}"))?;
    let mut archived = record.clone();
    archived.status = AccountStatus::Archived;
    archived.updated_at = now;
    let payload = record_value(&archived)?;
    let updated = conn
        .execute(
            "UPDATE workflow_runs SET state_json = ?2, updated_at = ?3
              WHERE id = ?1 AND state_revision = ?4",
            params![account_id, payload, now, revision],
        )
        .map_err(|e| format!("account: archive write failed: {e}"))?;
    if updated != 1 {
        return Err("account: state stale".into());
    }
    // Presence only — the reference itself never lands in the audit detail.
    super::audit_write(
        conn,
        account_id,
        AUDIT_ACCOUNT_ARCHIVE,
        crate::audit::AuditStatus::Ok,
        "archived under decision_ref",
    );
    Ok(archived)
}

/// The account run's domain, or None when the id is absent OR the run is
/// not an account — the two are the same answer, so the surface's probe
/// never learns whether the id exists as a different kind of run.
pub(crate) fn account_domain_of(
    conn: &Connection,
    account_id: i64,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT domain FROM workflow_runs WHERE id = ?1 AND kind = ?2",
        params![account_id, ACCOUNT_KIND],
        |r| r.get(0),
    )
    .optional()
}

/// Link one request run to one account: the additive `account:link` row
/// under the ACCOUNT's run id plus its audited row, atomic in the caller's
/// tx. Re-links append (rows are never mutated); a retry of a rolled-back
/// write recomputes the same key and replays as the exactly-once no-op.
pub(crate) fn link_request_to_account(
    tx: &mut WorkflowTx<'_>,
    account_id: i64,
    run_id: i64,
    now: i64,
) -> Result<(bool, i64), String> {
    let record = load_account(tx.tx(), account_id)
        .map_err(|e| format!("account: stored record unreadable: {e}"))?
        .ok_or("link_account_absent")?;
    if record.status == AccountStatus::Archived {
        return Err("account_archived".into());
    }
    let exists: bool = tx
        .tx()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_runs WHERE id = ?1)",
            params![run_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("account: run read failed: {e}"))?;
    if !exists {
        return Err("link_run_absent".into());
    }
    let payload = serde_json::json!({ "run_id": run_id, "linked_at": now }).to_string();
    let n: i64 = tx
        .tx()
        .query_row(
            "SELECT COUNT(*) FROM agent_session_events WHERE run_id = ?1 AND kind = ?2",
            params![account_id, LINK_ROW_KIND],
            |r| r.get(0),
        )
        .map_err(|e| format!("account: link count read failed: {e}"))?;
    let key = format!("account{account_id}:link:{}", n + 1);
    let (created, seq) =
        super::session_log::append(tx.tx(), account_id, LINK_ROW_KIND, &payload, &key, now)
            .map_err(|e| format!("account: link write failed: {e}"))?;
    if created {
        super::audit_write(
            tx.tx(),
            account_id,
            AUDIT_ACCOUNT_LINK,
            crate::audit::AuditStatus::Ok,
            &format!("request {run_id} linked at seq {seq}"),
        );
    }
    Ok((created, seq))
}

/// One decision row of a linked run, rendered for the history join.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecisionRow {
    pub seq: i64,
    pub payload: serde_json::Value,
}

/// One linked request as the per-account history serves it: the link (its
/// seq + `linked_at`), the request run's headline, and the run's recorded
/// decision rows (`handoff_lifecycle`) — the pure decision join over
/// existing rows, no new table shape.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AccountRequestEntry {
    pub run_id: i64,
    pub linked_at: i64,
    pub link_seq: i64,
    pub run_kind: Option<String>,
    pub run_status: Option<String>,
    pub run_updated_at: Option<i64>,
    pub decisions: Vec<DecisionRow>,
}

/// The bounded per-account history: the account's `account:link` rows in
/// row order, each joined to its request run's headline + decision rows.
pub(crate) fn account_requests(
    conn: &Connection,
    account_id: i64,
    limit: usize,
) -> rusqlite::Result<Vec<AccountRequestEntry>> {
    let mut stmt = conn.prepare(
        "SELECT seq, payload_json FROM agent_session_events
          WHERE run_id = ?1 AND kind = ?2 ORDER BY seq LIMIT ?3",
    )?;
    let links = stmt.query_map(params![account_id, LINK_ROW_KIND, limit as i64], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for link in links {
        let (link_seq, payload_json) = link?;
        let payload: serde_json::Value = serde_json::from_str(&payload_json).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("account: stored link unreadable: {e}"))
        })?;
        let run_id = payload
            .get("run_id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("account: stored link missing run_id".into())
            })?;
        let linked_at = payload
            .get("linked_at")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let headline: Option<(String, String, i64)> = conn
            .query_row(
                "SELECT kind, status, updated_at FROM workflow_runs WHERE id = ?1",
                params![run_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let headline = headline.map(|(k, s, u)| (Some(k), Some(s), Some(u)));
        let (run_kind, run_status, run_updated_at) = headline.unwrap_or((None, None, None));
        let mut dstmt = conn.prepare(
            "SELECT seq, payload_json FROM agent_session_events
              WHERE run_id = ?1 AND kind = 'handoff_lifecycle' ORDER BY seq",
        )?;
        let decisions = dstmt
            .query_map(params![run_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let decisions = decisions
            .into_iter()
            .map(|(seq, payload_json)| {
                serde_json::from_str(&payload_json)
                    .map(|payload| DecisionRow { seq, payload })
                    .map_err(|e| {
                        rusqlite::Error::InvalidParameterName(format!(
                            "account: stored decision unreadable: {e}"
                        ))
                    })
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        out.push(AccountRequestEntry {
            run_id,
            linked_at,
            link_seq,
            run_kind,
            run_status,
            run_updated_at,
            decisions,
        });
    }
    Ok(out)
}

/// One listed account (the DPO-gated listing row).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ListedAccount {
    pub account_id: i64,
    pub name: String,
    pub status: AccountStatus,
    pub domain: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The bounded listing of account rows, newest first. The caller owns the
/// DPO dual gate + the audited read.
pub(crate) fn account_listing(
    conn: &Connection,
    limit: usize,
) -> rusqlite::Result<Vec<ListedAccount>> {
    let mut stmt = conn.prepare(
        "SELECT id, domain, state_json, created_at, updated_at FROM workflow_runs
          WHERE kind = ?1 ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![ACCOUNT_KIND, limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (account_id, domain, state_json, created_at, updated_at) = row?;
        let value: serde_json::Value = serde_json::from_str(&state_json).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("account: stored record unreadable: {e}"))
        })?;
        let record = parse_account_record(&value).map_err(rusqlite::Error::InvalidParameterName)?;
        out.push(ListedAccount {
            account_id,
            name: record.name,
            status: record.status,
            domain,
            created_at,
            updated_at,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::migration::run_migration;
    use crate::pool::SqliteConnectionManager;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use rusqlite::Connection;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    fn tx(conn: &mut Connection) -> WorkflowTx<'_> {
        WorkflowTx::begin(conn).unwrap()
    }

    fn open(conn: &mut Connection, domain: &str, name: &str) -> AccountCreated {
        let mut wtx = tx(conn);
        let created = create_account(&mut wtx, domain, name, "hash:op", 100).unwrap();
        wtx.commit().unwrap();
        created
    }

    #[test]
    fn account_record_round_trips() {
        let record = AccountRecord {
            account_id: "7".into(),
            name: "Acme Limited".into(),
            owner_principal: "hash:ab12".into(),
            status: AccountStatus::Active,
            created_at: 1758451200,
            updated_at: 1758451201,
        };
        let value: serde_json::Value =
            serde_json::from_str(&record_value(&record).unwrap()).unwrap();
        assert_eq!(parse_account_record(&value).unwrap(), record);
        // The archived side round-trips too.
        let mut archived = record.clone();
        archived.status = AccountStatus::Archived;
        let value: serde_json::Value =
            serde_json::from_str(&record_value(&archived).unwrap()).unwrap();
        assert_eq!(parse_account_record(&value).unwrap(), archived);
    }

    #[test]
    fn account_parser_total_on_malformed() {
        for bad in [
            serde_json::json!(null),
            serde_json::json!("text"),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!({"account_id": ""}),
            serde_json::json!({"account_id": "1"}),
            serde_json::json!({"account_id": "1", "name": "a"}),
            serde_json::json!({"account_id": "1", "name": "a", "owner_principal": "o"}),
            serde_json::json!({"account_id": "1", "name": "a", "owner_principal": "o", "status": "closed"}),
            serde_json::json!({"account_id": "1", "name": "a", "owner_principal": "o", "status": "active"}),
            serde_json::json!({"account_id": "1", "name": "a", "owner_principal": "o", "status": "active", "created_at": -1}),
            serde_json::json!({"account_id": 1, "name": "a", "owner_principal": "o", "status": "active", "created_at": 1, "updated_at": 1}),
        ] {
            let err = parse_account_record(&bad).unwrap_err();
            assert!(err.starts_with("account: "), "unnamed refusal: {err}");
        }
    }

    #[test]
    fn account_name_screening_refuses_control_invisible() {
        assert_eq!(screen_account_name("  Acme ").unwrap(), "Acme");
        assert_eq!(
            screen_account_name("   ").unwrap_err(),
            "account: name required"
        );
        let over = "x".repeat(MAX_NAME_LEN + 1);
        assert_eq!(
            screen_account_name(&over).unwrap_err(),
            format!("account: name over {MAX_NAME_LEN} chars")
        );
        assert_eq!(
            screen_account_name("Acme\u{200B}").unwrap_err(),
            "account: name carries control or invisible characters"
        );
        assert_eq!(
            screen_account_name("Acme\nLtd").unwrap_err(),
            "account: name carries control or invisible characters"
        );
    }

    #[test]
    fn account_pending_row_is_audited() {
        let mut conn = db();
        let created = open(&mut conn, "acme", "Acme Limited");
        // The create landed exactly one account audit row, ids only.
        {
            let mut wtx = tx(&mut conn);
            link_request_to_account(&mut wtx, created.run_id, 0, 0).unwrap_err();
        }
        // A pending (absent-request) link refuses BEFORE any link row lands
        // — and the refusal leaves no audit row claiming it happened.
        let links: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind = 'account:link'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(links, 0, "a refused link writes no link row");
        // The stored record parses back to exactly what was written.
        let stored = load_account(&conn, created.run_id)
            .unwrap()
            .expect("the account row loads");
        assert_eq!(stored, created.record);
    }

    #[test]
    fn account_archived_transition_requires_decision_ref() {
        let mut conn = db();
        let created = open(&mut conn, "acme", "Acme Limited");
        // No decision reference: the machine never archives.
        {
            let mut wtx = tx(&mut conn);
            let err = archive_account(&mut wtx, created.run_id, "   ", 200).unwrap_err();
            assert_eq!(err, "account_archived_requires_decision_ref");
        }
        // Nothing moved: the record is still active.
        assert_eq!(
            load_account(&conn, created.run_id).unwrap().unwrap().status,
            AccountStatus::Active
        );
        // With the reference the archive lands, audited, and the column
        // status stays in the existing run vocabulary.
        {
            let mut wtx = tx(&mut conn);
            let archived =
                archive_account(&mut wtx, created.run_id, "op-2026-09-22#1", 200).unwrap();
            wtx.commit().unwrap();
            assert_eq!(archived.status, AccountStatus::Archived);
        }
        let column: String = conn
            .query_row(
                "SELECT status FROM workflow_runs WHERE id = ?1",
                params![created.run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(column, "active", "the run column keeps its vocabulary");
        // Archive refuses links afterwards.
        {
            conn.execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('acme', 'interview', '{}', 0, 'active', 1, 1)",
                [],
            )
            .unwrap();
            let mut wtx = tx(&mut conn);
            let err = link_request_to_account(&mut wtx, created.run_id, 1, 300).unwrap_err();
            assert_eq!(err, "account_archived");
        }
        // And a second archive names the state it is already in.
        {
            let mut wtx = tx(&mut conn);
            let err =
                archive_account(&mut wtx, created.run_id, "op-2026-09-22#2", 400).unwrap_err();
            assert_eq!(err, "account: status already archived");
        }
        // The archive + the refused second attempt left exactly one
        // account_archive audit row.
        let archives: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow' AND target_hash = ?1",
                params![crate::audit::hash(AUDIT_ACCOUNT_ARCHIVE)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(archives, 1, "exactly one archive audit row");
    }

    #[test]
    fn account_run_is_inert_to_the_loop() {
        use crate::agentloop::provider::LoopbackProvider;
        use crate::agentloop::run_loop::LoopConfig;
        use crate::workflow::gdl::GdlDriver;
        use crate::workflow::host::SqliteWorkflowHost;
        use brain_engine_sdk::env::{DenyAll, ExecutionEnv};
        use tokio_util::sync::CancellationToken;

        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = SqliteConnectionManager::file(tmp.path());
        let pool: crate::Pool = r2d2::Pool::builder().max_size(4).build(mgr).unwrap();
        run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB).unwrap();
        // The account row, written by the real core.
        let mut conn = pool.get().unwrap();
        let created = open(&mut conn, "acme", "Acme Limited");
        drop(conn);
        let before = {
            let conn = pool.get().unwrap();
            conn.query_row(
                "SELECT state_json, state_revision FROM workflow_runs WHERE id = ?1",
                params![created.run_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .unwrap()
        };
        // A real driver claims the run: the GDL loader refuses the
        // non-troubleshoot kind BEFORE any provider work.
        let host = std::sync::Arc::new(SqliteWorkflowHost::new(pool.clone()));
        let provider = LoopbackProvider::new("loopback", vec![]);
        let driver = GdlDriver::new(
            pool.clone(),
            host,
            provider.clone(),
            vec![],
            ExecutionEnv {
                fs: std::sync::Arc::new(DenyAll),
                read_only: true,
                allow_process: false,
                root: "/".into(),
                allowed_commands: vec![],
            },
            LoopConfig::default(),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result =
            runtime.block_on(driver.run_case(created.run_id, "t", &CancellationToken::new()));
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("corrupt state or ambiguous legacy GDL run"),
            "the loader refuses the account kind: {err}"
        );
        assert!(
            provider.requests().is_empty(),
            "no model work against an account run"
        );
        let after = {
            let conn = pool.get().unwrap();
            conn.query_row(
                "SELECT state_json, state_revision FROM workflow_runs WHERE id = ?1",
                params![created.run_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .unwrap()
        };
        assert_eq!(before, after, "the account row is untouched by the loop");
    }

    #[test]
    fn request_link_attaches_one_account() {
        let mut conn = db();
        let created = open(&mut conn, "acme", "Acme Limited");
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let mut wtx = tx(&mut conn);
        let (created_link, seq) =
            link_request_to_account(&mut wtx, created.run_id, 1, 150).unwrap();
        wtx.commit().unwrap();
        assert!(created_link);
        // The link lives under the ACCOUNT's run id.
        let key = format!("account{account}:link:1", account = created.run_id);
        let rows: (i64, String) = conn
            .query_row(
                "SELECT run_id, kind FROM agent_session_events WHERE idempotency_key = ?1",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rows.0, created.run_id);
        assert_eq!(rows.1, LINK_ROW_KIND);
        // The replay of a ROLLED-BACK write recomputes the same key and
        // lands as the exactly-once first append.
        {
            let mut wtx = tx(&mut conn);
            let (rolled_created, _) =
                link_request_to_account(&mut wtx, created.run_id, 1, 200).unwrap();
            assert!(rolled_created, "the attempt creates inside its own tx");
            // wtx drops without commit: the write rolls back, the count
            // does not move.
        }
        let mut wtx = tx(&mut conn);
        let (retried, retry_seq) =
            link_request_to_account(&mut wtx, created.run_id, 1, 200).unwrap();
        wtx.commit().unwrap();
        assert!(
            retried,
            "the retry lands fresh — nothing leaked from the rollback"
        );
        assert_eq!(
            retry_seq,
            seq + 1,
            "the next seq draws after the committed append"
        );
        // A later re-link of the same pair appends a NEW audited row —
        // rows are never mutated.
        let mut wtx = tx(&mut conn);
        let (relinked, seq2) = link_request_to_account(&mut wtx, created.run_id, 1, 990).unwrap();
        wtx.commit().unwrap();
        assert!(relinked);
        assert_ne!(seq2, seq);
    }

    #[test]
    fn request_link_absent_ids_refuse() {
        let mut conn = db();
        let created = open(&mut conn, "acme", "Acme Limited");
        let mut wtx = tx(&mut conn);
        assert_eq!(
            link_request_to_account(&mut wtx, 99_999, 1, 150).unwrap_err(),
            "link_account_absent"
        );
        wtx.commit().unwrap();
        // A non-account id is an absent account (probe-blind equality).
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let interview_id: i64 = conn.last_insert_rowid();
        let mut wtx = tx(&mut conn);
        assert_eq!(
            link_request_to_account(&mut wtx, interview_id, 1, 150).unwrap_err(),
            "link_account_absent"
        );
        assert_eq!(
            link_request_to_account(&mut wtx, created.run_id, 99_999, 150).unwrap_err(),
            "link_run_absent"
        );
    }

    #[test]
    fn request_relink_writes_new_audited_row() {
        let mut conn = db();
        let a = open(&mut conn, "acme", "Acme Limited");
        let b = {
            let mut wtx = tx(&mut conn);
            let created = create_account(&mut wtx, "acme", "Beta Works", "hash:op", 110).unwrap();
            wtx.commit().unwrap();
            created
        };
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let run_id = 1;
        // Link to A, re-link to B: a NEW audited row under B; A's never mutates.
        let mut wtx = tx(&mut conn);
        link_request_to_account(&mut wtx, a.run_id, run_id, 150).unwrap();
        wtx.commit().unwrap();
        let mut wtx = tx(&mut conn);
        let (recreated, _) = link_request_to_account(&mut wtx, b.run_id, run_id, 160).unwrap();
        wtx.commit().unwrap();
        assert!(recreated, "a re-link to another account appends a new row");
        let link_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind = 'account:link'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(link_rows, 2);
        let link_audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow' AND target_hash = ?1",
                params![crate::audit::hash(AUDIT_ACCOUNT_LINK)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(link_audits, 2, "each append carries its audited row");
    }

    #[test]
    fn per_account_history_is_decision_join() {
        let mut conn = db();
        let created = open(&mut conn, "acme", "Acme Limited");
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let run_a: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        let run_b: i64 = conn.last_insert_rowid();
        let mut wtx = tx(&mut conn);
        link_request_to_account(&mut wtx, created.run_id, run_a, 150).unwrap();
        link_request_to_account(&mut wtx, created.run_id, run_b, 160).unwrap();
        wtx.commit().unwrap();
        // Decision rows on run A only — the join must carry them, nothing else.
        conn.execute(
            "INSERT INTO agent_session_events(run_id, seq, idempotency_key, kind, payload_json, created_at)
             VALUES (?1, 1, 'seed-hl', 'handoff_lifecycle', '{\"transition\":\"delivered\"}', 170)",
            params![run_a],
        )
        .unwrap();
        let page = account_requests(&conn, created.run_id, 500).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].run_id, run_a);
        assert_eq!(page[0].linked_at, 150);
        assert_eq!(page[0].run_kind.as_deref(), Some("troubleshoot"));
        assert_eq!(page[0].decisions.len(), 1, "run A's decision row joins");
        assert_eq!(page[0].decisions[0].payload["transition"], "delivered");
        assert_eq!(page[1].run_id, run_b);
        assert!(page[1].decisions.is_empty(), "run B carries no decisions");
        // The page bounds.
        assert_eq!(account_requests(&conn, created.run_id, 1).unwrap().len(), 1);
        assert!(account_requests(&conn, 99_999, 500).unwrap().is_empty());
    }

    #[test]
    fn account_listing_is_bounded_and_audited_ready() {
        let mut conn = db();
        open(&mut conn, "acme", "Acme Limited");
        open(&mut conn, "acme", "Beta Works");
        open(&mut conn, "acme", "Gamma Co");
        // The listing reads only account rows, newest first, bounded.
        let page = account_listing(&conn, 2).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].name, "Gamma Co");
        assert_eq!(page[0].status, AccountStatus::Active);
        assert_eq!(page[0].domain, "acme");
        // Non-account rows never leak into the listing.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        assert_eq!(account_listing(&conn, 500).unwrap().len(), 3);
        // The create audit rows are on the chain (the listing route adds
        // its own per-call audit at the surface).
        let creates: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow' AND target_hash = ?1",
                params![crate::audit::hash(AUDIT_ACCOUNT)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(creates, 3);
        // The chain still verifies with every account row on it.
        assert!(crate::audit::verify_chain(&conn));
    }

    #[test]
    fn fuzz_corpus_replays_account_parser() {
        let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        dir.push("crates/brain-fuzz/corpus/accounts");
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("corpus dir: {e}"));
        let mut count = 0;
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let bytes = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            let text = String::from_utf8_lossy(&bytes);
            let v: serde_json::Value =
                serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            match parse_account_record(&v) {
                Ok(record) => {
                    assert!(
                        !record.name.is_empty(),
                        "a parsed record carries a screened name"
                    );
                }
                Err(e) => assert!(e.starts_with("account: "), "unnamed error: {e}"),
            }
            count += 1;
        }
        assert!(count >= 10, "the account corpus must stay populated");
    }
}
