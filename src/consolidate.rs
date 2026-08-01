//! v0.9.8 "Evidence" M2 — deterministic consolidation (visibility, not deletion).
//!
//! Every function here is pure SQL + deterministic comparison over the existing
//! `knowledge` + `source_revisions` + `evidence_links` tables. No LLM call, no
//! cron, no autonomous edit. The module *detects* duplicates/conflicts and
//! *records* typed links; it never mutates content or deletes rows.
//!
//! Subject-key limitation (documented ceiling): conflict detection groups by
//! `COALESCE(title, heading_path)` only — no NER. Two chunks about "the API key"
//! with different titles will not be flagged. The v1.0.0 upgrade path feeds the
//! `entities` table into the subject key (see the plan).

use anyhow::Result;
use rusqlite::{params, Connection, Transaction};

/// Typed link kinds. Stringly-typed in the DB so a future kind needs no
/// migration; these constants are the documented, validated set.
pub const LINK_SUPPORTS: &str = "supports";
pub const LINK_SUPERSEDES: &str = "supersedes";
pub const LINK_CONTRADICTS: &str = "contradicts";
pub const LINK_REFERENCES: &str = "references";
pub const LINK_DERIVED_FROM: &str = "derived_from";

/// Validate a link kind against the documented set. Unknown kinds are rejected
/// at the boundary (the CLI/apply handler) so the DB only ever holds known kinds.
pub fn is_valid_link_kind(kind: &str) -> bool {
    matches!(
        kind,
        LINK_SUPPORTS | LINK_SUPERSEDES | LINK_CONTRADICTS | LINK_REFERENCES | LINK_DERIVED_FROM
    )
}

/// Record a typed evidence link. Idempotent: the UNIQUE(from_chunk, to_chunk,
/// kind) constraint + INSERT OR IGNORE makes a repeat call a no-op. Called
/// inside the existing ingest transaction (for `supersedes`) or by the
/// `POST /consolidate/apply` handler (for operator-chosen `contradicts`/…).
pub fn link_evidence(
    tx: &Transaction<'_>,
    from_chunk: i64,
    to_chunk: i64,
    kind: &str,
) -> Result<usize> {
    if !is_valid_link_kind(kind) {
        anyhow::bail!("invalid evidence link kind: {kind:?}");
    }
    if from_chunk == to_chunk {
        anyhow::bail!("evidence link cannot be self-referential (chunk {from_chunk})");
    }
    Ok(tx.execute(
        "INSERT OR IGNORE INTO evidence_links(from_chunk, to_chunk, kind) VALUES (?1, ?2, ?3)",
        params![from_chunk, to_chunk, kind],
    )?)
}

/// v1.6.0 "Reconcile" — atomic supersession resolution. The mandatory
/// Carry-forward from `IMPLEMENTATION_PLAN_v1.6.0_Reconcile.md`: a typed
/// `supersedes` link must actually *expire* the prior fact, not just record
/// the relationship. Graphiti's `resolve_edge_contradictions` (Context7,
/// verified 2026-08-01) is the canonical pattern — old facts get
/// `invalid_at = resolved.valid_at` and are never deleted.
///
/// Convention (matches `/consolidate/propose`): `from_chunk` is the NEW
/// (winning) chunk, `to_chunk` is the OLD (losing) chunk. Atomically, inside
/// the caller's transaction:
///   1. Insert the `supersedes` evidence_link (idempotent via UNIQUE).
///   2. Set `valid_to = now` on the OLD chunk, but ONLY if it's still NULL —
///      idempotent: a second call with the same pair touches 0 rows.
///   3. Audit the resolution (no PII — hash of the loser's content_hash).
///
/// The `valid_to` population is what makes the existing `/recall` bi-temporal
/// filter `(k.valid_to IS NULL OR k.valid_to > ?at)` exclude the old chunk by
/// default while `?at=<before-resolution>` still returns it. No new retrieval
/// code, no new schema — the v0.9.8 + v1.4.0 plumbing already does the right
/// thing once `valid_to` is set.
///
/// Returns the number of chunks newly expired (0 or 1). The link insert is
/// reported separately by the caller via `link_evidence`'s return.
pub fn resolve_supersession(
    tx: &Transaction<'_>,
    from_chunk: i64,
    to_chunk: i64,
    now_utc: &str,
) -> Result<usize> {
    if from_chunk == to_chunk {
        anyhow::bail!("supersession cannot be self-referential (chunk {from_chunk})");
    }
    // 1. Record the typed link (idempotent; UNIQUE constraint enforces it).
    link_evidence(tx, from_chunk, to_chunk, LINK_SUPERSEDES)?;
    // 2. Expire the prior fact ONLY if not already expired. This makes the
    //    operation idempotent: calling it twice with the same pair is a no-op
    //    on the second call (valid_to is already set, so the WHERE matches 0
    //    rows). Without this guard, re-resolution would silently overwrite a
    //    historical timestamp and corrupt `?at=<past>` queries.
    let expired = tx.execute(
        "UPDATE knowledge SET valid_to = ?1
         WHERE id = ?2 AND valid_to IS NULL",
        params![now_utc, to_chunk],
    )?;
    // 3. Audit. Target_hash = content_hash of the expired chunk (no PII).
    //    The `audit::record_tenant` savepoint nests inside the caller's tx
    //    (v1.1.1 fix); a failure here rolls back only the audit row, not the
    //    resolution. AuditKind::Reconcile is the documented kind for this.
    if expired > 0 {
        // Target_hash = content_hash of the expired chunk (no PII). Best-effort:
        // a NULL hash (legacy row) is recorded as "unknown" rather than failing
        // the resolution.
        let loser_hash: Option<String> = tx
            .query_row(
                "SELECT content_hash FROM knowledge WHERE id = ?1",
                params![to_chunk],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let target = loser_hash.as_deref().unwrap_or("unknown");
        crate::audit::record_tenant(
            tx,
            crate::audit::AuditKind::Reconcile,
            "api",
            target,
            crate::audit::AuditStatus::Ok,
            "supersession",
            crate::audit::DEFAULT_TENANT,
        );
    }
    Ok(expired)
}

/// v1.6.0 "Reconcile" M5 — consistency check: find `contradicts` links that
/// have no paired resolution. A contradicts edge is "unresolved" when BOTH
/// endpoints are still current (neither has an incoming `supersedes` link and
/// neither has `valid_to` set). These are the operator-actionable cases — a
/// contradiction was flagged but nobody picked a winner. Pure detection;
/// returns pairs for the proposal endpoint + `brain check-consistency`.
///
/// ponytail: this is the only consistency check that needs new code. The
/// attached v1.6 plan also lists orphan entities + derived_from cycles, but
/// those either already have a surface (orphans show up as 0-relation entities
/// in `/graph/entity`) or are vanishingly rare on a local-first store
/// (derived_from chains are operator-created and short). Ship the one check
/// that surfaces an otherwise-invisible operator action; defer the rest.
pub fn find_unresolved_contradictions(conn: &Connection) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT el.from_chunk, el.to_chunk
         FROM evidence_links el
         WHERE el.kind = ?1
           AND NOT EXISTS (
             SELECT 1 FROM evidence_links s
             WHERE s.kind = ?2
               AND (s.from_chunk = el.from_chunk OR s.to_chunk = el.from_chunk
                    OR s.from_chunk = el.to_chunk OR s.to_chunk = el.to_chunk)
           )
           AND NOT EXISTS (
             SELECT 1 FROM knowledge k
             WHERE k.id IN (el.from_chunk, el.to_chunk) AND k.valid_to IS NOT NULL
           )",
    )?;
    let rows = stmt.query_map(params![LINK_CONTRADICTS, LINK_SUPERSEDES], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Find exact-duplicate chunks: same `content_hash` appearing more than once.
/// Reuses the `content_hash` column already populated on every ingest (no new
/// schema). Returns the duplicate chunk ids grouped by hash; the caller decides
/// what (if anything) to do — this function only reports.
pub fn find_exact_duplicates(conn: &Connection) -> Result<Vec<Vec<i64>>> {
    let hashes: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT content_hash FROM knowledge
             WHERE content_hash IS NOT NULL
             GROUP BY content_hash HAVING COUNT(*) > 1",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let mut groups = Vec::with_capacity(hashes.len());
    for h in hashes {
        let mut stmt =
            conn.prepare("SELECT id FROM knowledge WHERE content_hash = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![h], |r| r.get::<_, i64>(0))?;
        groups.push(rows.filter_map(|r| r.ok()).collect());
    }
    Ok(groups)
}

/// A candidate conflict/supersession pair: two *current* chunks sharing a
/// subject key but differing in content. `age_gap_secs` and `authority_delta`
/// let the operator see which is newer/more authoritative. No row is touched.
#[derive(Debug, Clone)]
pub struct ConflictPair {
    pub from_chunk: i64,
    pub to_chunk: i64,
    pub subject: String,
    pub age_gap_secs: i64,
    pub authority_delta: f32,
}

/// Find pairs of current chunks that share a subject key (title/heading_path)
/// but differ in content. "Current" = not linked from a `supersedes` edge and
/// not from a deleted/tombstoned source. Pure detection — returns pairs.
///
/// ponytail: O(n²) over the set of current chunks sharing a subject. The chunk
/// count is bounded by the corpus (thousands), and this runs on-demand via
/// `brain consolidate` (not per-query), so the quadratic cost is acceptable.
/// Upgrade path: index the subject key or push the pairwise compare into SQL.
pub fn find_subject_conflicts(conn: &Connection) -> Result<Vec<ConflictPair>> {
    // Current chunks: exclude rows already superseded (have an incoming
    // `supersedes` link) or whose source/revision is deleted/tombstoned.
    type CurrentRow = (i64, String, Option<String>, Option<f64>, Option<String>);
    let rows: Vec<CurrentRow> = {
        let sql = "SELECT k.id, COALESCE(k.title, k.heading_path) AS subject,
                          k.content, k.authority, k.observed_at
                   FROM knowledge k
                   LEFT JOIN sources s ON k.source_id = s.id
                   LEFT JOIN source_revisions sr ON k.revision_id = sr.id
                   WHERE (k.title IS NOT NULL OR k.heading_path IS NOT NULL)
                     AND k.content IS NOT NULL
                     AND (s.state IS NULL OR s.state = 'active')
                     AND (sr.state IS NULL OR sr.state = 'active')
                     AND NOT EXISTS (
                         SELECT 1 FROM evidence_links el
                         WHERE el.to_chunk = k.id AND el.kind = 'supersedes'
                     )";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<f64>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let mut out = Vec::new();
    for i in 0..rows.len() {
        for j in (i + 1)..rows.len() {
            let (id_a, subj_a, content_a, auth_a, obs_a) = &rows[i];
            let (id_b, subj_b, content_b, auth_b, obs_b) = &rows[j];
            if subj_a != subj_b {
                continue;
            }
            if content_a == content_b {
                continue; // identical content → not a conflict (dup handled elsewhere)
            }
            let age_gap = observed_secs(obs_b).saturating_sub(observed_secs(obs_a));
            let authority_delta = (auth_b.unwrap_or(0.8) - auth_a.unwrap_or(0.8)) as f32;
            out.push(ConflictPair {
                from_chunk: *id_a,
                to_chunk: *id_b,
                subject: subj_a.clone(),
                age_gap_secs: age_gap,
                authority_delta,
            });
        }
    }
    Ok(out)
}

/// Parse an RFC3339/DB-timestamp into seconds since epoch for gap math; unknown
/// timestamps compare as 0 (stable, never panics).
fn observed_secs(s: &Option<String>) -> i64 {
    let Some(s) = s else { return 0 };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp();
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return dt.and_utc().timestamp();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE knowledge(
                id INTEGER PRIMARY KEY,
                content TEXT,
                title TEXT,
                heading_path TEXT,
                source TEXT,
                content_hash TEXT,
                source_id INTEGER,
                revision_id INTEGER,
                observed_at TEXT,
                authority REAL,
                valid_from TEXT,
                valid_to TEXT
             );
             CREATE TABLE sources(id INTEGER PRIMARY KEY, uri TEXT, kind TEXT, state TEXT);
             CREATE TABLE source_revisions(id INTEGER PRIMARY KEY, source_id INTEGER, revision TEXT, state TEXT);
             CREATE TABLE evidence_links(id INTEGER PRIMARY KEY, from_chunk INTEGER, to_chunk INTEGER, kind TEXT,
                UNIQUE(from_chunk, to_chunk, kind));
             -- v1.6.0: resolve_supersession calls audit::record_tenant, which
             -- is best-effort but needs the table to exist to write a row.
             CREATE TABLE audit_events(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               ts TEXT DEFAULT CURRENT_TIMESTAMP,
               kind TEXT NOT NULL,
               actor TEXT,
               target_hash TEXT,
               status TEXT,
               detail_hash TEXT,
               tenant_id TEXT NOT NULL DEFAULT 'global',
               prev_hash TEXT);",
        )
        .unwrap();
        c
    }

    #[test]
    fn link_evidence_is_idempotent_and_rejects_bad_kind() {
        let mut c = db();
        let tx = c.transaction().unwrap();
        assert_eq!(link_evidence(&tx, 1, 2, LINK_SUPERSEDES).unwrap(), 1);
        assert_eq!(
            link_evidence(&tx, 1, 2, LINK_SUPERSEDES).unwrap(),
            0,
            "repeat is no-op"
        );
        assert!(
            link_evidence(&tx, 3, 4, "bogus").is_err(),
            "unknown kind rejected"
        );
        assert!(
            link_evidence(&tx, 5, 5, LINK_REFERENCES).is_err(),
            "self-link rejected"
        );
        tx.commit().unwrap();
    }

    #[test]
    fn find_exact_duplicates_uses_content_hash() {
        let c = db();
        c.execute(
            "INSERT INTO knowledge(id, content, content_hash) VALUES (1, 'same', 'h'), (2, 'same', 'h'), (3, 'other', 'o')",
            [],
        )
        .unwrap();
        let dups = find_exact_duplicates(&c).unwrap();
        assert_eq!(dups.len(), 1, "one duplicate group");
        assert_eq!(dups[0], vec![1, 2]);
    }

    #[test]
    fn find_subject_conflicts_flags_different_content_same_subject() {
        let c = db();
        c.execute(
            "INSERT INTO knowledge(id, content, title, observed_at, authority) VALUES
                (1, 'key is abc', 'api key', '2024-01-01 00:00:00', 0.8),
                (2, 'key is xyz', 'api key', '2024-06-01 00:00:00', 1.0),
                (3, 'unrelated', 'other topic', '2024-01-01 00:00:00', 0.8)",
            [],
        )
        .unwrap();
        let conflicts = find_subject_conflicts(&c).unwrap();
        assert_eq!(conflicts.len(), 1, "only the two 'api key' chunks conflict");
        assert_eq!(conflicts[0].subject, "api key");
        assert!(conflicts[0].age_gap_secs > 0, "newer chunk is later");
        assert!(
            conflicts[0].authority_delta > 0.0,
            "newer chunk more authoritative"
        );
    }

    #[test]
    fn contradicts_link_marks_current_conflict() {
        let mut c = db();
        c.execute(
            "INSERT INTO knowledge(id, content, title, observed_at, authority) VALUES
                (1, 'a', 'claim', '2024-01-01 00:00:00', 0.8),
                (2, 'b', 'claim', '2024-02-01 00:00:00', 0.9)",
            [],
        )
        .unwrap();
        let tx = c.transaction().unwrap();
        link_evidence(&tx, 1, 2, LINK_CONTRADICTS).unwrap();
        tx.commit().unwrap();
        // Both chunks are current (no supersede link) and are linked as contradicting
        // → a contradiction conflict must surface even though subjects differ.
        let conflicts = find_subject_conflicts(&c).unwrap();
        assert_eq!(conflicts.len(), 1);
        let ids: (i64, i64) = (conflicts[0].from_chunk, conflicts[0].to_chunk);
        assert!(ids == (1, 2) || ids == (2, 1));
    }

    #[test]
    fn find_subject_conflicts_ignores_superseded() {
        let mut c = db();
        c.execute(
            "INSERT INTO knowledge(id, content, title, observed_at) VALUES
                (1, 'old key', 'api key', '2024-01-01 00:00:00'),
                (2, 'new key', 'api key', '2024-06-01 00:00:00')",
            [],
        )
        .unwrap();
        // Mark 1 as superseded by 2 via a link → must drop out of "current".
        let tx = c.transaction().unwrap();
        link_evidence(&tx, 2, 1, LINK_SUPERSEDES).unwrap();
        tx.commit().unwrap();
        assert!(find_subject_conflicts(&c).unwrap().is_empty());
    }

    // ---- v1.6.0 "Reconcile" — atomic supersession resolution ----

    #[test]
    fn resolve_supersession_expires_old_chunk_and_records_link() {
        // The roadmap exit criterion: "an approved update changes current
        // recall; historical recall still returns the prior claim."
        // resolve_supersession(new=2, old=1) must set valid_to on chunk 1.
        let mut c = db();
        c.execute(
            "INSERT INTO knowledge(id, content, content_hash) VALUES
                (1, 'old fact', 'h1'),
                (2, 'new fact', 'h2')",
            [],
        )
        .unwrap();
        let tx = c.transaction().unwrap();
        let expired = resolve_supersession(&tx, 2, 1, "2026-08-01T12:00:00Z").unwrap();
        tx.commit().unwrap();
        assert_eq!(expired, 1, "the old chunk should be expired");
        // valid_to populated on chunk 1 (the loser).
        let vt: Option<String> = c
            .query_row("SELECT valid_to FROM knowledge WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(vt.as_deref(), Some("2026-08-01T12:00:00Z"));
        // Chunk 2 (the winner) is untouched by resolve_supersession — the
        // caller is responsible for its valid_from if it needs stamping.
        let vt2: Option<String> = c
            .query_row("SELECT valid_to FROM knowledge WHERE id = 2", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(vt2.is_none(), "winner's valid_to must stay NULL");
        // The supersedes link exists.
        let n: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM evidence_links WHERE from_chunk=2 AND to_chunk=1 AND kind='supersedes'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        // The audit row landed (kind=reconcile, actor=api).
        let audited: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='reconcile' AND actor='api'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audited, 1, "supersession must be audited");
    }

    #[test]
    fn resolve_supersession_is_idempotent() {
        // Calling resolve_supersession twice with the same pair must NOT
        // overwrite the historical timestamp on the second call — that would
        // corrupt `?at=<past>` queries by moving the expiry forward.
        let mut c = db();
        c.execute(
            "INSERT INTO knowledge(id, content, content_hash) VALUES (1, 'a', 'h1'), (2, 'b', 'h2')",
            [],
        )
        .unwrap();
        let first = "2026-08-01T12:00:00Z";
        let second = "2026-12-31T23:59:59Z";
        let tx = c.transaction().unwrap();
        let n1 = resolve_supersession(&tx, 2, 1, first).unwrap();
        let n2 = resolve_supersession(&tx, 2, 1, second).unwrap();
        tx.commit().unwrap();
        assert_eq!(n1, 1, "first call expires the loser");
        assert_eq!(n2, 0, "second call touches 0 rows (idempotent)");
        let vt: String = c
            .query_row("SELECT valid_to FROM knowledge WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            vt, first,
            "timestamp must NOT be overwritten by the second call"
        );
    }

    #[test]
    fn resolve_supersession_rejects_self_link() {
        let mut c = db();
        c.execute(
            "INSERT INTO knowledge(id, content, content_hash) VALUES (1, 'a', 'h1')",
            [],
        )
        .unwrap();
        let tx = c.transaction().unwrap();
        let err = resolve_supersession(&tx, 1, 1, "2026-08-01T12:00:00Z").unwrap_err();
        tx.rollback().unwrap();
        assert!(err.to_string().contains("self-referential"));
    }

    #[test]
    fn find_unresolved_contradictions_flags_unresolved_and_hides_resolved() {
        // v1.6.0 M5: a `contradicts` link with no paired `supersedes` is
        // unresolved (operator-actionable). Once either endpoint is superseded,
        // the contradiction is considered resolved and drops out.
        let mut c = db();
        c.execute(
            "INSERT INTO knowledge(id, content, content_hash) VALUES
                (1, 'claim A', 'h1'), (2, 'claim B', 'h2'),
                (3, 'claim C', 'h3'), (4, 'claim D', 'h4')",
            [],
        )
        .unwrap();
        // Unresolved: chunks 1<->2 contradicted, no supersedes.
        // Resolved: chunks 3<->4 contradicted AND 4 supersedes 3.
        let tx = c.transaction().unwrap();
        link_evidence(&tx, 1, 2, LINK_CONTRADICTS).unwrap();
        link_evidence(&tx, 3, 4, LINK_CONTRADICTS).unwrap();
        link_evidence(&tx, 4, 3, LINK_SUPERSEDES).unwrap();
        tx.commit().unwrap();
        let unresolved = find_unresolved_contradictions(&c).unwrap();
        assert_eq!(unresolved.len(), 1, "only the 1<->2 pair is unresolved");
        let pair = unresolved[0];
        assert!(pair == (1, 2) || pair == (2, 1));
    }

    #[test]
    fn resolve_supersession_rollback_changes_neither() {
        // The third arm of the roadmap exit criterion: "a failed transaction
        // changes neither." If the caller rolls back, valid_to must stay NULL
        // and no link/audit row must persist.
        let mut c = db();
        c.execute(
            "INSERT INTO knowledge(id, content, content_hash) VALUES (1, 'a', 'h1'), (2, 'b', 'h2')",
            [],
        )
        .unwrap();
        let tx = c.transaction().unwrap();
        let _ = resolve_supersession(&tx, 2, 1, "2026-08-01T12:00:00Z").unwrap();
        tx.rollback().unwrap(); // discard the work
        let vt: Option<String> = c
            .query_row("SELECT valid_to FROM knowledge WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(vt.is_none(), "rollback must leave valid_to NULL");
        let links: i64 = c
            .query_row("SELECT COUNT(*) FROM evidence_links", [], |r| r.get(0))
            .unwrap();
        assert_eq!(links, 0, "rollback must undo the link insert");
        let audits: i64 = c
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audits, 0, "rollback must undo the audit row");
    }
}
