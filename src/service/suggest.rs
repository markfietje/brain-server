//! The suggest-feedback aggregate: the last-wins feedback ledger and its
//! outcome metrics.
//!
//! OWNS the `suggest_feedback` storage story: the chunk-existence fence (a
//! never-existed id is refused so the metric isn't poisoned by typos; a
//! DELETED chunk's id still counts — the feedback was real when given), the
//! last-wins upsert (conflict key `(chunk_id, COALESCE(session, ''))`;
//! session + tenant_id are identity and never updated), and the grouped
//! outcome counts behind the false-positive metric.
//!
//! NO audit_events row — by contract: the feedback table IS the audit
//! surface (append-only, hash-of-reason, tenant-scoped). Do not "fix" that
//! during a move.
//!
//! Read seam + metric arithmetic stay handler-side: this module returns the
//! raw (feedback, count) pairs; the wire response is shaped at the handler.

use std::fmt;

use rusqlite::{Connection, params};

/// One grouped outcome count: (feedback, n).
pub(crate) type FeedbackCount = (String, i64);

/// A storage failure. `Database`'s Display carries the exact pre-move
/// message; the handler wraps it in `HandlerError::internal` unchanged.
/// `NoSuchChunk` maps to the route's frozen probe-blind 404.
#[derive(Debug)]
pub(crate) enum FeedbackError {
    Database(String),
    NoSuchChunk(i64),
}

impl fmt::Display for FeedbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedbackError::Database(m) => f.write_str(m),
            FeedbackError::NoSuchChunk(id) => write!(f, "no chunk with id {id}"),
        }
    }
}

impl From<rusqlite::Error> for FeedbackError {
    fn from(e: rusqlite::Error) -> Self {
        FeedbackError::Database(e.to_string())
    }
}

/// The suggest-feedback last-wins upsert, shared by the `/suggest/feedback`
/// handler and the `/ump/feedback` binding. The optional `ump_outcome`
/// carries the granular UMP outcome (`followed`/`overridden`/`ignored`/
/// `contradicted`) in its own column; the suggest path passes `None`
/// (column stays NULL).
///
/// The existence check's fail-open read posture is preserved verbatim: a
/// QUERY ERROR on the EXISTS probe reads as "no chunk" (a 404), not a 500 —
/// the same `.unwrap_or(false)` the handler had.
#[allow(clippy::too_many_arguments)] // 8 positional params, 2 call sites; a struct would be ceremony for a private fn
pub(crate) fn record_feedback(
    conn: &Connection,
    chunk_id: i64,
    feedback: &str,
    reason_hash: Option<String>,
    ts: i64,
    session: Option<String>,
    tenant: &str,
    ump_outcome: Option<&str>,
) -> Result<(), FeedbackError> {
    // chunk_id validity: refuse feedback on a non-existent chunk so the
    // metric isn't poisoned by typos. (A deleted chunk's id still counts —
    // the feedback was real when given; only never-existed ids are rejected.)
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM knowledge WHERE id = ?1)",
            params![chunk_id],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n != 0)
        .unwrap_or(false);
    if !exists {
        return Err(FeedbackError::NoSuchChunk(chunk_id));
    }
    conn.execute(
        "INSERT INTO suggest_feedback(chunk_id, feedback, reason_hash, ts, session, tenant_id, ump_outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(chunk_id, COALESCE(session, '')) DO UPDATE SET
           feedback = excluded.feedback,
           reason_hash = excluded.reason_hash,
           ts = excluded.ts,
           ump_outcome = excluded.ump_outcome",
        params![chunk_id, feedback, reason_hash, ts, session, tenant, ump_outcome],
    )
    .map_err(|e| FeedbackError::Database(format!("feedback insert failed: {e}")))?;
    Ok(())
}

/// The ledger's grouped outcome counts for one tenant, optionally narrowed
/// by a `since` timestamp and a session label — the (tenant_id, ts) index
/// keeps it cheap. Raw pairs; the handler shapes the rates.
pub(crate) fn feedback_counts(
    conn: &Connection,
    tenant: &str,
    since: Option<&str>,
    session: Option<&str>,
) -> Result<Vec<FeedbackCount>, FeedbackError> {
    let mut sql = String::from(
        "SELECT feedback, COUNT(*) FROM suggest_feedback \
         WHERE tenant_id = ?1",
    );
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(tenant.to_string())];
    if let Some(since) = since {
        sql.push_str(" AND ts >= ?");
        params_vec.push(Box::new(since.to_string()));
    }
    if let Some(sess) = session {
        sql.push_str(" AND session = ?");
        params_vec.push(Box::new(sess.to_string()));
    }
    sql.push_str(" GROUP BY feedback");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| FeedbackError::Database(format!("metrics prepare failed: {e}")))?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| FeedbackError::Database(format!("metrics query failed: {e}")))?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}
