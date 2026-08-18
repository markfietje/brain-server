//! the breach-notification workflow.
//!
//! The one genuinely-new primitive in this release: PH DPA (+ most client laws)
//! require breach notification within a bounded window (PH: 72h to the NPC +
//! affected). A breach is **human-opened by the DPO role** (detection is a v2.x
//! monitoring concern) and lives as an append-only incident row with a
//! notification/knowledge event log. Every event is additionally hash-chained
//! into the existing audit (`handlers/breaches.rs` records each with
//! [`crate::audit::AuditKind::Breach`]) — this table is the DPO's readable
//! ledger, the audit chain is the tamper-evident record.
//!
//! `breaches` + `breach_events` are created by the shared migration (per-DB,
//! the `legal_holds` precedent); the handler operates on the `global` pool —
//! an incident is operator data, not domain-scoped memory.

use rusqlite::{Connection, Transaction};

use crate::handlers::HandlerError;
use crate::ph::notification_deadlines;

/// Max description length (a human narrative, bounded like `MAX_HOLD_REASON`).
pub(crate) const MAX_BREACH_DESC: usize = 4000;
/// Max event-body length (a notification log line).
pub(crate) const MAX_BREACH_EVENT: usize = 2000;
/// Max `scope` length (a short in-scope label).
pub(crate) const MAX_BREACH_SCOPE: usize = 200;
/// Max affected jurisdictions listed on one breach (each becomes a deadline row).
pub(crate) const MAX_BREACH_JURISDICTIONS: usize = 8;
/// Page bound for `GET /breaches` (the `MAX_LIMIT`-style clamp).
pub(crate) const MAX_BREACH_LIMIT: i64 = 200;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct BreachRow {
    pub id: i64,
    pub scope: String,
    pub description: String,
    pub severity: String,
    pub discovered_at: i64,
    pub affected_estimate: Option<i64>,
    pub jurisdictions: Vec<String>,
    pub status: String,
    pub opened_by: String,
    pub opened_at: i64,
    pub closed_by: Option<String>,
    pub closed_at: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct BreachEventRow {
    pub id: i64,
    pub event_type: String,
    pub jurisdiction: Option<String>,
    pub body: String,
    pub noted_by: String,
    pub created_at: i64,
}

/// A breach with its append-only event history + the computed per-jurisdiction
/// notification deadlines (the Security-panel countdown source).
#[derive(Debug, serde::Serialize)]
pub(crate) struct BreachView {
    #[serde(flatten)]
    pub breach: BreachRow,
    pub events: Vec<BreachEventRow>,
    pub deadlines: Vec<crate::ph::NotifyDeadline>,
}

/// Validate the open-payload fields before any write. Shared by the handler.
pub(crate) fn validate_open(
    scope: &str,
    description: &str,
    severity: &str,
    jurisdictions: &[String],
) -> Result<(), HandlerError> {
    let scope = scope.trim();
    if scope.is_empty() || scope.len() > MAX_BREACH_SCOPE {
        return Err(HandlerError::bad_request(
            "breach_scope_invalid",
            format!("scope is required and ≤ {MAX_BREACH_SCOPE} characters"),
        ));
    }
    let description = description.trim();
    if description.is_empty() || description.len() > MAX_BREACH_DESC {
        return Err(HandlerError::bad_request(
            "breach_description_invalid",
            format!("description is required and ≤ {MAX_BREACH_DESC} characters"),
        ));
    }
    if !crate::ph::is_severity(severity) {
        return Err(HandlerError::bad_request(
            "breach_severity_invalid",
            format!("severity must be one of {:?}", crate::ph::SEVERITIES),
        ));
    }
    if jurisdictions.len() > MAX_BREACH_JURISDICTIONS {
        return Err(HandlerError::bad_request(
            "breach_too_many_jurisdictions",
            format!("at most {MAX_BREACH_JURISDICTIONS} affected jurisdictions"),
        ));
    }
    Ok(())
}

/// Open a breach incident. Runs inside the caller's transaction; returns the id.
/// (The two-style-args allow matches the repo's private-fn precedent — a struct
/// here is ceremony for one call site.)
#[allow(clippy::too_many_arguments)]
pub(crate) fn open(
    tx: &Transaction,
    scope: &str,
    description: &str,
    severity: &str,
    discovered_at: i64,
    affected_estimate: Option<i64>,
    jurisdictions: &[String],
    opened_by: &str,
    now: i64,
) -> Result<i64, HandlerError> {
    let jur = serde_json::to_string(jurisdictions)
        .map_err(|e| HandlerError::internal(format!("jurisdiction encode: {e}")))?;
    tx.execute(
        "INSERT INTO breaches(
            scope, description, severity, discovered_at, affected_estimate, jurisdictions,
            status, opened_by, opened_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, ?8)",
        rusqlite::params![
            scope.trim(),
            description.trim(),
            severity.trim().to_ascii_lowercase(),
            discovered_at,
            affected_estimate,
            jur,
            opened_by,
            now
        ],
    )
    .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(tx.last_insert_rowid())
}

/// Append one assessment/notification/note event. Runs in the caller's tx.
/// Callers audit each event ([`AuditKind::Breach`]) so the chain mirrors the log.
pub(crate) fn add_event(
    tx: &Transaction,
    breach_id: i64,
    event_type: &str,
    jurisdiction: Option<&str>,
    body: &str,
    noted_by: &str,
    now: i64,
) -> Result<bool, HandlerError> {
    // Refuse events on a closed breach (no post-mortem churn on the incident).
    let status: Option<String> = tx
        .query_row(
            "SELECT status FROM breaches WHERE id = ?1",
            rusqlite::params![breach_id],
            |r| r.get(0),
        )
        .ok();
    if status.as_deref() == Some("closed") {
        return Err(HandlerError::conflict(
            "breach_closed: cannot append an event to a closed breach",
        ));
    }
    tx.execute(
        "INSERT INTO breach_events(breach_id, event_type, jurisdiction, body, noted_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            breach_id,
            event_type.trim(),
            jurisdiction.map(str::trim).filter(|s| !s.is_empty()),
            body.trim(),
            noted_by,
            now
        ],
    )
    .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(true)
}

/// Close a breach (explicit, never auto). Returns the row when it exists,
/// else `None` → 404. Closing an already-closed breach is a no-op.
pub(crate) fn close(
    tx: &Transaction,
    id: i64,
    now: i64,
    closed_by: &str,
) -> Result<Option<BreachRow>, HandlerError> {
    let Some(mut row) = breach_row(tx, id)? else {
        return Ok(None);
    };
    if row.status != "closed" {
        tx.execute(
            "UPDATE breaches SET status = 'closed', closed_by = ?1, closed_at = ?2
              WHERE id = ?3",
            rusqlite::params![closed_by, now, id],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
        row.status = "closed".to_string();
        row.closed_by = Some(closed_by.to_string());
        row.closed_at = Some(now);
    }
    Ok(Some(row))
}

fn breach_row(conn: &Connection, id: i64) -> Result<Option<BreachRow>, HandlerError> {
    let row: Option<BreachRow> = conn
        .query_row(
            "SELECT id, scope, description, severity, discovered_at, affected_estimate,
                    jurisdictions, status, opened_by, opened_at, closed_by, closed_at
               FROM breaches WHERE id = ?1",
            rusqlite::params![id],
            row_from,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(row)
}

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<BreachRow> {
    let jurisdictions: String = r.get(6)?;
    let jurisdictions = serde_json::from_str(&jurisdictions).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(BreachRow {
        id: r.get(0)?,
        scope: r.get(1)?,
        description: r.get(2)?,
        severity: r.get(3)?,
        discovered_at: r.get(4)?,
        affected_estimate: r.get(5)?,
        jurisdictions,
        status: r.get(7)?,
        opened_by: r.get(8)?,
        opened_at: r.get(9)?,
        closed_by: r.get(10)?,
        closed_at: r.get(11)?,
    })
}

/// `GET /breaches` — newest-first incident registry, bounded.
pub(crate) fn list(conn: &Connection, limit: i64) -> Result<Vec<BreachRow>, HandlerError> {
    let limit = limit.clamp(1, MAX_BREACH_LIMIT);
    let mut stmt = conn
        .prepare(
            "SELECT id, scope, description, severity, discovered_at, affected_estimate,
                    jurisdictions, status, opened_by, opened_at, closed_by, closed_at
               FROM breaches ORDER BY id DESC LIMIT ?1",
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![limit], row_from)
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(rows.flatten().collect())
}

/// `GET /breaches/{id}` — one incident + its append-only events + computed
/// notification deadlines (from the affected jurisdictions + `discovered_at`).
pub(crate) fn get(conn: &Connection, id: i64) -> Result<Option<BreachView>, HandlerError> {
    let Some(breach) = breach_row(conn, id)? else {
        return Ok(None);
    };
    let mut stmt = conn
        .prepare(
            "SELECT id, event_type, jurisdiction, body, noted_by, created_at
               FROM breach_events WHERE breach_id = ?1 ORDER BY id ASC",
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![id], |r| {
            Ok(BreachEventRow {
                id: r.get(0)?,
                event_type: r.get(1)?,
                jurisdiction: r.get(2)?,
                body: r.get(3)?,
                noted_by: r.get(4)?,
                created_at: r.get(5)?,
            })
        })
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let events: Vec<BreachEventRow> = rows.flatten().collect();
    let deadlines = notification_deadlines(&breach.jurisdictions, breach.discovered_at);
    Ok(Some(BreachView {
        breach,
        events,
        deadlines,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE breaches(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scope TEXT NOT NULL,
                description TEXT NOT NULL,
                severity TEXT NOT NULL,
                discovered_at INTEGER NOT NULL,
                affected_estimate INTEGER,
                jurisdictions TEXT NOT NULL,
                status TEXT NOT NULL,
                opened_by TEXT NOT NULL,
                opened_at INTEGER NOT NULL,
                closed_by TEXT,
                closed_at INTEGER);
             CREATE TABLE breach_events(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                breach_id INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                jurisdiction TEXT,
                body TEXT NOT NULL,
                noted_by TEXT NOT NULL,
                created_at INTEGER NOT NULL);",
        )
        .unwrap();
        conn
    }

    fn open_one(conn: &mut Connection) -> i64 {
        let tx = conn.transaction().unwrap();
        let id = open(
            &tx,
            "esb staging",
            "Unencrypted backup exposed via S3",
            "high",
            1000,
            Some(250),
            &["ph".to_string(), "eu".to_string()],
            "dpo",
            1100,
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }

    #[test]
    fn breach_lifecycle_open_event_close() {
        let mut conn = db();
        let id = open_one(&mut conn);

        // Append-only notification log entry.
        let tx = conn.transaction().unwrap();
        add_event(
            &tx,
            id,
            "notification",
            Some("ph"),
            "NPC notified",
            "dpo",
            1200,
        )
        .unwrap();
        tx.commit().unwrap();

        // View shows the log + the EU/PH deadlines (the countdown the client
        // Security panel renders).
        let view = get(&conn, id).unwrap().unwrap();
        assert_eq!(view.events.len(), 1);
        assert_eq!(view.events[0].event_type, "notification");
        assert_eq!(
            view.deadlines.len(),
            4,
            "ph NPC/subjects + eu authority/subjects"
        );

        // Close is explicit; event append after close refuses.
        let tx = conn.transaction().unwrap();
        close(&tx, id, 2000, "supervisor").unwrap();
        tx.commit().unwrap();
        let closed = get(&conn, id).unwrap().unwrap();
        assert_eq!(closed.breach.status, "closed");

        let tx = conn.transaction().unwrap();
        let err = add_event(&tx, id, "note", None, "late", "dpo", 2100).unwrap_err();
        tx.commit().unwrap();
        assert_eq!(err.inner.code, "conflict");
        assert!(err.inner.message.starts_with("breach_closed"));

        // 404 on an unknown id.
        assert!(get(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn validate_open_bounds_fields() {
        assert!(validate_open("scope", "desc", "high", &["ph".to_string()]).is_ok());
        assert!(validate_open(" ", "desc", "high", &[]).is_err());
        assert!(validate_open("scope", "", "high", &[]).is_err());
        assert!(validate_open("scope", "desc", "grave", &[]).is_err());
        let many = vec!["a".to_string(); MAX_BREACH_JURISDICTIONS + 1];
        assert!(validate_open("scope", "desc", "high", &many).is_err());
        assert!(validate_open("scope", &"x".repeat(MAX_BREACH_DESC + 1), "high", &[]).is_err());
    }

    #[test]
    fn list_is_newest_first_and_bounded() {
        let mut conn = db();
        let a = open_one(&mut conn);
        let tx = conn.transaction().unwrap();
        let b = open(&tx, "b", "b", "low", 1, None, &[], "dpo", 2).unwrap();
        tx.commit().unwrap();
        let rows = list(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, b, "newest-first");
        assert_eq!(rows[1].id, a);
        // Unknown-id close → None.
        let tx = conn.transaction().unwrap();
        assert!(close(&tx, 999, 3, "dpo").unwrap().is_none());
        tx.commit().unwrap();
    }

    #[test]
    fn countdown_via_notification_deadlines_covers_ph_and_eu() {
        // Grafted onto the deadline rule so the persistence view's countdown
        // source is pinned at the same level as the pure fn it wraps.
        let deadlines = notification_deadlines(&["ph".to_string(), "eu".to_string()], 1000);
        assert_eq!(deadlines.len(), 4);
        assert!(deadlines
            .iter()
            .all(|d| d.deadline == 1000 + d.hours * 3600));
    }

    /// D-1 "never certify silence": a corrupt `jurisdictions` JSON cell must
    /// fail the row decode, not silently deserialize to an empty list (the
    /// pre-v1.27.24 `unwrap_or_default` behaviour hid the corruption).
    #[test]
    fn row_decode_fails_closed_on_corrupt_jurisdictions() {
        let conn = db();
        conn.execute(
            "INSERT INTO breaches(scope, description, severity, discovered_at, \
             affected_estimate, jurisdictions, status, opened_by, opened_at) \
             VALUES ('s','d','high',1,2,'NOT-JSON','open','dpo',1)",
            [],
        )
        .unwrap();
        let mut stmt = conn.prepare("SELECT * FROM breaches").unwrap();
        let rows: Vec<_> = stmt
            .query_map([], |r| row_from(r).map(|_| ()))
            .unwrap()
            .collect();
        assert!(
            rows[0].is_err(),
            "corrupt jurisdictions must surface as a decode error"
        );
        // A well-formed cell still decodes.
        conn.execute("UPDATE breaches SET jurisdictions = '[\"ph\"]'", [])
            .unwrap();
        let mut stmt = conn.prepare("SELECT * FROM breaches").unwrap();
        let ok = stmt.query_map([], row_from).unwrap().next();
        let row = ok.unwrap().unwrap();
        assert_eq!(row.jurisdictions, vec!["ph".to_string()]);
    }
}
