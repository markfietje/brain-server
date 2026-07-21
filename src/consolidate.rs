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
                authority REAL
             );
             CREATE TABLE sources(id INTEGER PRIMARY KEY, uri TEXT, kind TEXT, state TEXT);
             CREATE TABLE source_revisions(id INTEGER PRIMARY KEY, source_id INTEGER, revision TEXT, state TEXT);
             CREATE TABLE evidence_links(id INTEGER PRIMARY KEY, from_chunk INTEGER, to_chunk INTEGER, kind TEXT,
                UNIQUE(from_chunk, to_chunk, kind));",
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
}
