//! The webhook synchronous-path storage: the kb-feedback finding ingest and
//! the inbound-Signal command writes/reads.
//!
//! OWNS the webhook side-door's storage story:
//! - the kb-feedback flood bound's read (findings, trailing hour), the
//!   finding write (with its audit row on the SAME connection — the
//!   evidence is one autocommit after the write, the documented posture),
//!   and the per-slug hot-count read behind the rising-repeat alert;
//! - the Signal flood bound's read (webhook_seen trailing hour — the
//!   replay-claim table every verified delivery already touches);
//! - the draft-approve arm's reads/write: the proposal (content, status)
//!   fetch inside the caller's Immediate tx and the digest-gated approve
//!   UPDATE (the rows-affected count IS the concurrent-decision signal;
//!   `WHERE ... AND status='pending'` closes the TOCTOU against a
//!   concurrent gate approve). The digest CHECK, the mismatch audit, and
//!   the commit stay handler-side — transport orchestration.
//!
//! The run-domain lookup reuses `workflow::state::run_domain_of` and the
//! steering arm reuses `workflow::outbox::enqueue_steering_tx` — one
//! definition each, dup-guard enforced.
//!
//! Flood bounds are compared at the call site against the config constants
//! (the 503 mapping is wire vocabulary); these reads are the counts those
//! bounds consume.

use rusqlite::{Connection, OptionalExtension, params};

/// The kb-feedback trailing-hour ingest count (the synchronous path
/// bypasses the queue cap, so it enforces its own bound).
pub(crate) fn kb_feedback_flood_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM findings
         WHERE claim = 'kb_feedback' AND ts > strftime('%s','now') - 3600",
        [],
        |r| r.get(0),
    )
}

/// One kb-feedback finding, with its audit row on the same connection
/// (best-effort evidence in the documented autocommit-after-write shape).
pub(crate) fn record_kb_feedback_finding(
    conn: &Connection,
    slug: &str,
    source: &str,
) -> rusqlite::Result<usize> {
    let n = conn.execute(
        "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
         VALUES (0, 'kb_feedback', ?1, ?2, 1.0, strftime('%s','now'))",
        params![slug, source],
    )?;
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Webhook,
        "kb-feedback",
        slug,
        crate::audit::AuditStatus::Ok,
        "feedback recorded",
    );
    Ok(n)
}

/// The slug's total feedback count — the rising-repeat signal behind the
/// hot-topic alert (the handler owns the threshold comparison).
pub(crate) fn kb_feedback_slug_count(conn: &Connection, slug: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM findings WHERE claim = 'kb_feedback' AND evidence = ?1",
        params![slug],
        |r| r.get(0),
    )
}

/// The Signal trailing-hour delivery count (over `webhook_seen`, the
/// replay-claim table — the audit chain stores only hashes and cannot
/// count details).
pub(crate) fn signal_flood_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM webhook_seen
          WHERE seen_at >= datetime('now', '-1 hour')",
        [],
        |r| r.get(0),
    )
}

/// The proposal's (content, status) inside the CALLER'S tx — the digest
/// gate's input. None when the id is unknown.
pub(crate) fn draft_proposal_row(
    tx: &Connection,
    proposal_id: i64,
) -> rusqlite::Result<Option<(String, String)>> {
    tx.query_row(
        "SELECT content, status FROM proposals WHERE id=?1",
        params![proposal_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// The digest-gated approve: pending → approved with the decision stamp,
/// ONLY while still pending (the status predicate closes the TOCTOU with a
/// concurrent gate approve). Returns the affected count — the caller owns
/// the n==0 refusal and the evidence.
pub(crate) fn approve_draft_tx(tx: &Connection, proposal_id: i64) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE proposals SET status='approved', decided_at=strftime('%s','now')
          WHERE id=?1 AND status='pending'",
        params![proposal_id],
    )
}

#[cfg(test)]
/// File a pending `draft` proposal (the Signal e2e seeds; kind/novelty/
/// status exactly as the gate's draft flow writes them).
pub(crate) fn file_pending_draft(
    conn: &Connection,
    content: &str,
    created_at: i64,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO proposals(kind, content, created_at, novelty, status) VALUES ('draft', ?1, ?2, 0.5, 'pending')",
        params![content, created_at],
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
/// The proposal's (status, decided_at) — the e2e assertion read.
pub(crate) fn draft_proposal_state(
    conn: &Connection,
    proposal_id: i64,
) -> rusqlite::Result<Option<(String, Option<i64>)>> {
    conn.query_row(
        "SELECT status, decided_at FROM proposals WHERE id=?1",
        params![proposal_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}
