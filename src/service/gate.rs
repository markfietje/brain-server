//! The proposal-gate core — the HITL review queue's storage story.
//!
//! OWNS the `proposals`-table storage for the gate: the review-queue page
//! read (status filter + `since` window + the `LIMIT` ceiling), the review
//! deadline/SLA derivation the queue is rendered against, and the
//! supervisor's owner-scoping filter. The decision CASes, the promote
//! inserts, and the TTL-expire write join this module as the line's last
//! vein drains `handlers/gate.rs`.
//!
//! The review WIRE stays handler-side by contract: `review_digest` (the
//! approve-verb binding), `sanitize_read`/PII masking (the read seam), the
//! screen-verdict BADGE recomputation at emission, and every HTTP status
//! mapping. This module returns the stored forms; the handler shapes what
//! the reviewer sees.
//!
//! Error `Display` carries the exact pre-move message text; the handler
//! wraps it in `HandlerError::internal` unchanged.

use std::fmt;

use rusqlite::{Connection, params, params_from_iter};

/// Max proposals returned per review page. Bounded so a runaway queue can't
/// unbounded a response. The cap rides the core so every future caller of
/// [`pending_page`] inherits it (the fence holds of the FUNCTION).
pub(crate) const MAX_PROPOSALS: usize = 200;

/// A storage failure. `Database`'s Display carries the exact pre-move
/// message; the handler wraps it in `HandlerError::internal` unchanged.
#[derive(Debug)]
pub(crate) enum GateError {
    Database(String),
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateError::Database(m) => f.write_str(m),
        }
    }
}

impl From<rusqlite::Error> for GateError {
    fn from(e: rusqlite::Error) -> Self {
        GateError::Database(e.to_string())
    }
}

/// One row of the human review queue, in its STORED form. The handler adds
/// the reader-dependent layers after the read: `content_digest` (the
/// approve-verb binding) and the `sanitize_read` pass over every emitted
/// text field.
#[derive(Debug, serde::Serialize)]
pub(crate) struct ProposalView {
    pub id: i64,
    pub kind: String,
    /// the READ-canonical form — `sanitize_read` of the
    /// stored row (redact → markdown-ref → invisible strip), i.e. the exact bytes
    /// recall will emit. Every review read returns this, so the reviewer sees
    /// what the system will later recall. Stored bytes stay verbatim (evidence
    /// fidelity in /export + DSAR); this is the display boundary only.
    pub content: String,
    /// `sha256_hex(review_digest(content))` — the
    /// stable, reader-independent fingerprint of the canonical form the approve
    /// verb binds to. Present iff the row was served through a principal-aware
    /// read (list/edit); empty in the bare-`Connection` unit path.
    #[serde(default)]
    pub content_digest: String,
    pub source: Option<String>,
    pub source_prompt: Option<String>,
    pub authority: Option<f32>,
    pub novelty: f32,
    pub conflict_with: Option<i64>,
    pub salience: f32,
    pub created_at: i64,
    /// the injection-screen verdict for `content`,
    /// recomputed deterministically at read time. `clean` or `quarantine` only
    /// (`reject` is never persisted — see the propose handler).
    pub screen_verdict: String,
    /// unix ts of the last content rewrite, `None` if the
    /// pending proposal was never edited. Keys the review badge + read-time view.
    pub edited_at: Option<i64>,
    /// when this proposal ages out of the review window
    /// (unix ts), derived server-side as `created_at + TTL`. The review queue
    /// ticks against this absolute deadline, so an operator override of
    /// `BRAIN_PROPOSAL_TTL_SECS` is authoritative (no client TTL guess).
    pub expires_at: i64,
    /// the SLA band boundaries (secs of remaining life), a
    /// mirror of the alert watcher's `ALERT_WARN_SECS`/`ALERT_CRITICAL_SECS` so
    /// the client colors its countdown from the same thresholds as the server.
    pub warn_secs: i64,
    pub critical_secs: i64,
    /// unix ts of the decision (approve/reject/expire),
    /// `None` while the proposal is still pending. Exposed so a consumer can
    /// compute a decision-latency (`decided_at - created_at`) — the reviewer
    /// calibration signal. The column was written but never read (until this reader).
    #[serde(default)]
    pub decided_at: Option<i64>,
    /// the agent whose interaction produced the candidate.
    /// `None` for loopback/opaque (unowned) writes. Keys the supervisor's owner
    /// scope (role `manages`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// the supervisor's coaching note (set via the coach
    /// verb). `None` until coached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qa_note: Option<String>,
    /// the QA scorecard composed from the proposal's
    /// owner + trace signals. Advisory (never gates approve).
    #[serde(default)]
    pub qa_score: i64,
}

/// the review deadline + SLA bands, shared by every
/// `ProposalView` construction site so the countdown is one definition.
pub(crate) fn proposal_deadline(created_at: i64) -> (i64, i64, i64) {
    (
        created_at + crate::config::proposal_ttl_secs(),
        crate::config::ALERT_WARN_SECS,
        crate::config::ALERT_CRITICAL_SECS,
    )
}

/// the review-queue page — the statement the
/// `decided_at` column + the optional `since` window ride on. Column order is
/// pinned by the index-based `r.get(n)`.
/// A `since` bound still leaves `LIMIT` as the hard ceiling, so a windowed
/// stat fetch MUST pass `limit=MAX_PROPOSALS` or it only samples the default.
pub(crate) fn pending_page(
    conn: &Connection,
    status: &str,
    limit: usize,
    since: Option<i64>,
) -> Result<Vec<ProposalView>, GateError> {
    const COLS: &str =
        "id, kind, content, source, source_prompt, authority, novelty, conflict_with,
                        salience, created_at, edited_at, decided_at, owner, qa_note";
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match since {
        Some(s) => (
            &format!(
                "SELECT {COLS} FROM proposals WHERE status = ?1 AND created_at >= ?3 \
                 ORDER BY created_at DESC LIMIT ?2"
            ),
            vec![Box::new(status), Box::new(limit as i64), Box::new(s)],
        ),
        None => (
            &format!(
                "SELECT {COLS} FROM proposals WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2"
            ),
            vec![Box::new(status), Box::new(limit as i64)],
        ),
    };
    let mut stmt = conn.prepare(sql).map_err(GateError::from)?;
    let rows = stmt
        .query_map(params_from_iter(params), |r| {
            let row_content: String = r.get(2)?;
            let created_at: i64 = r.get(9)?;
            let owner: Option<String> = r.get(12)?;
            let (expires_at, warn_secs, critical_secs) = proposal_deadline(created_at);
            // compose the QA scorecard. Proposals are not
            // recall-trace-linked in schema, so `has_trace` stays false — the
            // absent-trace neutral corner (never NaN). `in_scope` = the candidate
            // is agent-owned (the supervisor QA surface) vs an unowned loopback.
            let qa_score = crate::qa::score_for(owner.is_some(), false, false, false);
            Ok(ProposalView {
                id: r.get(0)?,
                kind: r.get(1)?,
                screen_verdict: crate::screen::screen_verdict_label(crate::screen::screen(
                    &row_content,
                    "",
                ))
                .to_string(),
                content: row_content,
                // the digest is reader-dependent only
                // through None (no PII redaction) — it is banker-settled in the
                // principal-aware HTTP layer (`list_proposals`), not here.
                content_digest: String::new(),
                source: r.get(3)?,
                source_prompt: r.get(4)?,
                authority: r.get(5)?,
                novelty: r.get(6)?,
                conflict_with: r.get(7)?,
                salience: r.get(8)?,
                created_at,
                edited_at: r.get(10)?,
                expires_at,
                warn_secs,
                critical_secs,
                decided_at: r.get(11)?,
                owner,
                qa_note: r.get(13)?,
                qa_score,
            })
        })
        .map_err(GateError::from)?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>();
    Ok(rows)
}

/// narrow a proposal page to the owners a supervisor
/// manages (role `owner IN manages`). Empty `manages` → the whole page
/// (an admin who manages no agents sees the unrestricted queue).
pub(crate) fn owner_in_filtered(rows: Vec<ProposalView>, manages: &[String]) -> Vec<ProposalView> {
    if manages.is_empty() {
        return rows;
    }
    rows.into_iter()
        .filter(|p| {
            p.owner
                .as_deref()
                .is_some_and(|o| manages.iter().any(|m| m == o))
        })
        .collect()
}

/// A candidate awaiting review, exactly as the creation insert persists it.
/// The injection screen + bounds + kind validation stay handler-side (they
/// produce handler-shaped 400s); this is the storage contract only.
pub(crate) struct ProposalInsert<'a> {
    pub kind: &'a str,
    pub content: &'a str,
    pub source: Option<&'a str>,
    pub authority: Option<f32>,
    pub observed_at: Option<i64>,
    pub novelty: f32,
    pub conflict_with: Option<i64>,
    pub salience: f32,
    pub created_at: i64,
    /// the SCREENED form — the caller applies `screen_source_prompt`.
    pub source_prompt: Option<&'a str>,
    pub owner: Option<&'a str>,
    /// the draft advisory lint report (JSON), `None` for non-drafts.
    pub lint_json: Option<&'a str>,
}

/// Queue a scored candidate: the `proposals` insert + its `proposal_pending`
/// audit row (the evidence lands inside whatever tx context the caller
/// holds — audit-per-write).
pub(crate) fn insert_proposal(conn: &Connection, p: &ProposalInsert<'_>) -> Result<i64, GateError> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO proposals(kind, content, source, authority, observed_at,
                               novelty, conflict_with, salience, created_at, source_prompt, owner, lint_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         RETURNING id",
            rusqlite::params![
                p.kind,
                p.content,
                p.source,
                p.authority,
                p.observed_at,
                p.novelty,
                p.conflict_with,
                p.salience,
                p.created_at,
                p.source_prompt,
                p.owner,
                p.lint_json,
            ],
            |r| r.get(0),
        )
        .map_err(|e| GateError::Database(format!("proposal insert failed: {e}")))?;
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Ingest,
        p.owner.unwrap_or("api"),
        &format!("proposal:{id}"),
        crate::audit::AuditStatus::Ok,
        "proposal_pending",
    );
    Ok(id)
}

/// Find a live chunk whose subject conflicts with the candidate content. Reuses
/// [`crate::consolidate::find_subject_conflicts`]'s signal: a candidate that
/// contradicts an existing current claim is flagged in the review queue so the
/// human sees the conflict, not a silent overwrite.
pub(crate) fn find_conflict(conn: &Connection, content: &str) -> Option<i64> {
    // Cheap exact-subject pre-check before the O(n²) pairwise scan: only run
    // the full conflict scan when the candidate's subject appears somewhere.
    let subject = content
        .lines()
        .next()
        .unwrap_or(content)
        .chars()
        .take(120)
        .collect::<String>();
    let mut stmt = conn
        .prepare("SELECT id FROM knowledge WHERE (title IS NOT NULL AND title = ?1) OR (heading_path IS NOT NULL AND heading_path = ?1) AND valid_to IS NULL LIMIT 1")
        .ok()?;
    let matched: Option<i64> = stmt
        .query_row(rusqlite::params![subject], |r| r.get(0))
        .ok();
    drop(stmt);
    // Full pairwise conflict scan only when we have a subject-anchored hit.
    if matched.is_some()
        && let Ok(pairs) = crate::consolidate::find_subject_conflicts(conn)
    {
        return pairs.into_iter().map(|p| p.from_chunk).next();
    }
    None
}

/// The pending-fence read behind approve/reject/edit: `created_at` iff the
/// row is still pending. `None` = already decided (or absent) — the caller
/// treats that as "nothing to expire".
pub(crate) fn pending_created_at(conn: &Connection, id: i64) -> Option<i64> {
    conn.query_row(
        "SELECT created_at FROM proposals WHERE id = ?1 AND status = 'pending'",
        params![id],
        |r| r.get(0),
    )
    .ok()
}

/// if the proposal is older than
/// [`crate::config::proposal_ttl_secs`], mark it rejected + audit
/// `proposal_expired` and return `false`. A stale auto-capture prompt's
/// context is unrecoverable, so the queue refuses to act on it (neither
/// approve nor reject — it's already beyond verification). Wall-clock enters
/// as an argument so a test pins the instant.
pub(crate) fn expire_if_stale(
    conn: &Connection,
    id: i64,
    created_at: i64,
    now: i64,
) -> Result<bool, GateError> {
    if now - created_at <= crate::config::proposal_ttl_secs() {
        return Ok(true);
    }
    conn.execute(
        "UPDATE proposals SET status = 'rejected', decided_at = ?1
         WHERE id = ?2 AND status = 'pending'",
        params![now, id],
    )
    .map_err(GateError::from)?;
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Reconcile,
        "api",
        &format!("proposal:{id}"),
        crate::audit::AuditStatus::Ok,
        "proposal_expired",
    );
    Ok(false)
}

/// The reject CAS — `AND status = 'pending'` so a concurrent
/// approve/reject can't both succeed. Rows-affected belongs to the caller
/// (0 = the 404 path).
pub(crate) fn cas_rejected(
    tx: &rusqlite::Transaction<'_>,
    id: i64,
    now: i64,
) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE proposals SET status = 'rejected', decided_at = ?1
         WHERE id = ?2 AND status = 'pending'",
        params![now, id],
    )
}

/// The stored proposal content — the digest input for the decided event +
/// the oversight basis. Best-effort callers keep their `ok()`/`unwrap_or`
/// shapes against the returned `rusqlite::Result`.
pub(crate) fn content_of(conn: &Connection, id: i64) -> rusqlite::Result<String> {
    conn.query_row(
        "SELECT content FROM proposals WHERE id = ?1",
        params![id],
        |r| r.get::<_, String>(0),
    )
}

/// The edit-path pending row: the 8-column projection the edit verb loads
/// inside its tx before rewriting. `None` = no pending row with that id
/// (the handler renders the frozen 404).
pub(crate) struct PendingEditRow {
    pub kind: String,
    pub content: String,
    pub source: Option<String>,
    pub source_prompt: Option<String>,
    pub authority: Option<f32>,
    pub created_at: i64,
    pub owner: Option<String>,
    pub qa_note: Option<String>,
}

pub(crate) fn pending_edit_row(conn: &Connection, id: i64) -> Option<PendingEditRow> {
    conn.query_row(
        "SELECT kind, content, source, source_prompt, authority, created_at, owner, qa_note
         FROM proposals WHERE id = ?1 AND status = 'pending'",
        params![id],
        |r| {
            Ok(PendingEditRow {
                kind: r.get(0)?,
                content: r.get(1)?,
                source: r.get(2)?,
                source_prompt: r.get(3)?,
                authority: r.get(4)?,
                created_at: r.get(5)?,
                owner: r.get(6)?,
                qa_note: r.get(7)?,
            })
        },
    )
    .ok()
}

/// The edit CAS: rewrite content + re-scored components + the `edited_at`
/// stamp, `AND status = 'pending'` so a concurrent decision wins cleanly
/// (rows-affected 0 = the caller's rollback + 409).
pub(crate) fn apply_edit(
    tx: &rusqlite::Transaction<'_>,
    id: i64,
    content: &str,
    novelty: f32,
    salience: f32,
    conflict_with: Option<i64>,
    edited_at: i64,
) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE proposals SET content = ?1, novelty = ?2, salience = ?3,
                conflict_with = ?4, edited_at = ?5
         WHERE id = ?6 AND status = 'pending'",
        params![content, novelty, salience, conflict_with, edited_at, id],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// an ingested proposal records its agent `owner`, and
    /// `pending_page` returns it alongside a `qa_score` — in-scope
    /// (owned) gets the absent-trace neutral corner, unowned degrades to
    /// out-of-scope. The supervisor page filter (R1 `manages`) keeps only
    /// owned rows.
    #[test]
    fn proposal_owner_and_scorecard_round_trip() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        brain_server::migration::run_migration(&mut conn, 1).expect("migration");
        let now = chrono::Utc::now().timestamp();
        let owned: i64 = conn
            .query_row(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at, owner)
                 VALUES ('fact', 'agent body', 0.9, 0.5, ?1, 'agent-1') RETURNING id",
                [now],
                |r| r.get(0),
            )
            .unwrap();
        let unowned: i64 = conn
            .query_row(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at)
                 VALUES ('fact', 'unowned body', 0.9, 0.5, ?1) RETURNING id",
                [now],
                |r| r.get(0),
            )
            .unwrap();

        let pending = pending_page(&conn, "pending", MAX_PROPOSALS, None).expect("pending");
        let owned_v = pending
            .iter()
            .find(|v| v.id == owned)
            .expect("owned present");
        assert_eq!(owned_v.owner.as_deref(), Some("agent-1"));
        assert_eq!(
            owned_v.qa_score, 90,
            "owned + absent trace = cited-neutral in-scope"
        );
        assert!(owned_v.qa_note.is_none());
        let unowned_v = pending
            .iter()
            .find(|v| v.id == unowned)
            .expect("unowned present");
        assert_eq!(unowned_v.owner, None);
        assert_eq!(
            unowned_v.qa_score, 40,
            "unowned = out-of-scope neutral corner"
        );

        let manages = vec!["agent-1".to_string()];
        assert!(
            owner_in_filtered(Vec::new(), &[]).is_empty(),
            "empty manages → whole queue (short-circuit, keeps rows)"
        );
        let scoped = owner_in_filtered(pending, &manages);
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].id, owned, "outside-manages proposal excluded");
    }

    /// `decided_at` surfaces on every `ProposalView`
    /// — `None` while pending, set after a decision (the write paths stamp it),
    /// and set on a TTL auto-expire. The round-trip proves the column now reads
    /// where the writers always wrote it.
    #[test]
    fn proposal_view_round_trips_decided_at() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        brain_server::migration::run_migration(&mut conn, 1).expect("migration");
        let now = chrono::Utc::now().timestamp();
        let pending: i64 = conn
            .query_row(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at)
                 VALUES ('fact', 'still open', 0.9, 0.5, ?1) RETURNING id",
                [now],
                |r| r.get(0),
            )
            .unwrap();
        let decided: i64 = conn
            .query_row(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at)
                 VALUES ('fact', 'decided body', 0.9, 0.5, ?1) RETURNING id",
                [now - 100],
                |r| r.get(0),
            )
            .unwrap();
        // Mirror the approve/reject write site (gate.rs:424 / :618 / :753).
        conn.execute(
            "UPDATE proposals SET status = 'approved', decided_at = ?1 WHERE id = ?2",
            rusqlite::params![now - 5, decided],
        )
        .unwrap();
        let expired: i64 = conn
            .query_row(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at)
                 VALUES ('fact', 'expired body', 0.9, 0.5, ?1) RETURNING id",
                [now - crate::config::proposal_ttl_secs() - 1],
                |r| r.get(0),
            )
            .unwrap();
        // TTL auto-expire stamps decided_at (expire_if_stale's write).
        conn.execute(
            "UPDATE proposals SET status = 'rejected', decided_at = ?1 WHERE id = ?2",
            rusqlite::params![now - 1, expired],
        )
        .unwrap();

        let views = pending_page(&conn, "approved", MAX_PROPOSALS, None).expect("approved");
        let decided_view = views
            .iter()
            .find(|v| v.id == decided)
            .expect("decided present");
        assert_eq!(
            decided_view.decided_at,
            Some(now - 5),
            "approved carries its decision"
        );

        let pending_views = pending_page(&conn, "pending", MAX_PROPOSALS, None).expect("pending");
        let pending_view = pending_views
            .iter()
            .find(|v| v.id == pending)
            .expect("pending present");
        assert_eq!(
            pending_view.decided_at, None,
            "a pending proposal has no decision"
        );

        let rejected_views =
            pending_page(&conn, "rejected", MAX_PROPOSALS, None).expect("rejected");
        let expired_view = rejected_views
            .iter()
            .find(|v| v.id == expired)
            .expect("expired present");
        assert_eq!(
            expired_view.decided_at,
            Some(now - 1),
            "an expired (auto-rejected) proposal still records a latency"
        );
    }

    /// `since` bounds the page by `created_at`, and
    /// its absence returns the legacy full query (back-compat pinned).
    #[test]
    fn proposals_since_filters_created_at_and_is_optional() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        brain_server::migration::run_migration(&mut conn, 1).expect("migration");
        // Three approved rows at distinct created_at (newest first by default).
        conn.execute_batch(
            "INSERT INTO proposals(kind, content, novelty, salience, created_at, status) VALUES
                 ('fact', 'oldest', 0.9, 0.5, 1000, 'approved'),
                 ('fact', 'middle', 0.9, 0.5, 2000, 'approved'),
                 ('fact', 'newest', 0.9, 0.5, 3000, 'approved');",
        )
        .unwrap();

        // Absent `since` → all rows, newest first (legacy behavior unchanged).
        let all = pending_page(&conn, "approved", MAX_PROPOSALS, None).expect("all");
        let ids: Vec<i64> = all.iter().map(|v| v.id).collect();
        assert_eq!(ids.len(), 3, "no since → every row");
        let newest = all.iter().find(|v| v.content == "newest").unwrap();
        assert_eq!(ids[0], newest.id, "newest first preserved");

        // `since=2000` excludes rows created before the bound.
        let windowed =
            pending_page(&conn, "approved", MAX_PROPOSALS, Some(2000)).expect("windowed");
        let wids: Vec<i64> = windowed.iter().map(|v| v.id).collect();
        assert_eq!(wids.len(), 2, "since=2000 keeps created_at >= 2000");
        assert!(
            windowed.iter().all(|v| v.created_at >= 2000),
            "no row older than the bound"
        );
        assert!(
            all.iter().any(|v| v.created_at < 2000),
            "the old row still exists without the bound"
        );
    }
}
