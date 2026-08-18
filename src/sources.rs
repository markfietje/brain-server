//! canonical source + revision lifecycle.
//!
//! A `source` is a stable identity for an external document (a vault file, a
//! connector doc, …): identified by its canonical `uri` (the file path / URL),
//! typed by `kind` ('vault', 'manual', …). A `source_revision` is an immutable
//! snapshot of one version of that source's content; a new revision supersedes
//! the prior one. Every knowledge chunk links to a source + revision so a result
//! can be traced to the exact document version it came from.
//!
//! Source-aware uniqueness (plan M1): same source + same revision is a no-op;
//! changed content → new revision atomically supersedes the old chunks.
//!
//! All functions take a `rusqlite::Transaction` so callers compose them into a
//! single atomic ingest; no revision is searchable until its chunk + vector
//! writes commit. Pure w.r.t. the DB: no embedding model, no I/O beyond the tx.

use anyhow::Result;
use rusqlite::{params, Transaction};
use xxhash_rust::xxh3::xxh3_64;

/// The vault connector kind. 'vault' = a file under a vault dir. Stringly-typed
/// in the DB so future connectors add a kind without a schema change.
pub const KIND_VAULT: &str = "vault";

/// The manual-memory kind. 'manual' = an entry written via `POST /ingest/memory`.
/// Distinct from `vault` so a vault reconcile never retires a manual memory
/// (reconcile is kind-scoped — see `reconcile`).
pub const KIND_MANUAL: &str = "manual";

/// Compute the source-level revision hash from the full raw content. This is
/// distinct from the per-chunk `content_hash` (which is namespaced per
/// source_path): the revision summarizes the whole document, so a change
/// anywhere in the file yields a new revision.
pub fn compute_revision(content: &str) -> String {
    format!("{:016x}", xxh3_64(content.as_bytes()))
}

/// Upsert a canonical source record. Returns the source id. Idempotent: a
/// second ingest of the same `uri` reuses the existing row (and reactivates it
/// if it was previously marked deleted — a deleted-then-restored file).
pub fn upsert_source(
    tx: &Transaction<'_>,
    uri: &str,
    kind: &str,
    title: Option<&str>,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO sources (uri, kind, title, state)\n         VALUES (?1, ?2, ?3, 'active')\n         ON CONFLICT(uri) DO UPDATE SET\n             kind = excluded.kind,\n             title = COALESCE(excluded.title, sources.title),\n             state = 'active',\n             updated_at = CURRENT_TIMESTAMP,\n             observed_at = CURRENT_TIMESTAMP",
        params![uri, kind, title],
    )?;
    let id: i64 = tx.query_row("SELECT id FROM sources WHERE uri = ?1", params![uri], |r| {
        r.get(0)
    })?;
    Ok(id)
}

/// Outcome of a revision upsert: either the revision already existed (no-op) or
/// a new one was created (and the prior active revision was superseded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionOutcome {
    /// Revision already current; nothing changed. Carries the existing id.
    Unchanged(i64),
    /// New revision inserted; prior active revision (if any) superseded.
    Created { id: i64, superseded: usize },
}

/// Upsert an immutable revision for `source_id`. Same revision → no-op
/// (`Unchanged`); a different revision → insert + supersede the source's prior
/// active revision + point `sources.current_revision_id` at the new one.
pub fn upsert_revision(
    tx: &Transaction<'_>,
    source_id: i64,
    revision: &str,
    content_hash: Option<&str>,
    chunk_count: usize,
    byte_size: u64,
) -> Result<RevisionOutcome> {
    // Already current?
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM source_revisions\n         WHERE source_id = ?1 AND revision = ?2 AND state = 'active'",
            params![source_id, revision],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        // Refresh observed time on the source even on a no-op revision.
// was `let _ =` — a failed refresh silently
        // misleads the reconcile "last observed" display. Cosmetic: warn.
        if let Err(e) = tx.execute(
            "UPDATE sources SET observed_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![source_id],
        ) {
            tracing::warn!("source observed_at refresh failed: {e}");
        }
        return Ok(RevisionOutcome::Unchanged(id));
    }

    // Supersede the prior active revision(s) for this source. Normally at most
    // one; the loop is defensive against any partial-failure leftover.
    let superseded = tx.execute(
        "UPDATE source_revisions SET state = 'superseded'\n         WHERE source_id = ?1 AND state = 'active'",
        params![source_id],
    )?;
    tx.execute(
        "INSERT INTO source_revisions\n             (source_id, revision, content_hash, chunk_count, byte_size, state)\n         VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
        params![source_id, revision, content_hash, chunk_count as i64, byte_size as i64],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE sources SET current_revision_id = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
        params![id, source_id],
    )?;
    Ok(RevisionOutcome::Created { id, superseded })
}

/// Link a set of chunk ids to their source + revision. Called after the chunks
/// are inserted so the link only lands for chunks that exist.
pub fn link_chunks(
    tx: &Transaction<'_>,
    source_id: i64,
    revision_id: i64,
    chunk_ids: &[i64],
) -> Result<usize> {
    let mut linked = 0;
    for cid in chunk_ids {
        linked += tx.execute(
            "UPDATE knowledge SET source_id = ?1, revision_id = ?2 WHERE id = ?3",
            params![source_id, revision_id, cid],
        )?;
    }
    Ok(linked)
}

/// temporal authority for a chunk. Source-authority is a
/// documented tie-breaker only (M2.4); it is never fed into RRF/BM25 scores.
/// Defaults chosen so the common ingest kinds are distinguishable without
/// per-source tuning.
pub const AUTHORITY_MANUAL: f32 = 1.0;
pub const AUTHORITY_VAULT: f32 = 0.8;
// ponytail: connector ingests currently flow through the vault ingest path
// (they set `source_path`), so they are stamped with AUTHORITY_VAULT today.
// This constant is reserved for when the connector path is split out;
// keep it so the documented authority tiers stay explicit.
#[allow(dead_code)]
pub const AUTHORITY_CONNECTOR: f32 = 0.6;

/// Stamp the temporal window + authority for a freshly-linked chunk. Pure
/// w.r.t. the tx; called once per chunk inside the existing ingest transaction
/// (vault, manual, connector). All three temporal columns were added in v0.9.1
/// but never populated — this is the write side of M1.1.
///
/// `observed_at` is when brain-server learned the fact (ingest/sync time).
/// `valid_from` is when the fact became true in the world (file mtime, issue
/// created_at, …); `None` ⇒ treat as observed_at (no separate world-time known).
/// `valid_to` is `None` ⇒ still current.
pub fn stamp_evidence(
    tx: &Transaction<'_>,
    chunk_id: i64,
    observed_at: &str,
    valid_from: Option<&str>,
    valid_to: Option<&str>,
    authority: f32,
) -> Result<usize> {
    let valid_from = valid_from.unwrap_or(observed_at);
    tx.execute(
        "UPDATE knowledge
         SET observed_at = ?2, valid_from = ?3, valid_to = ?4,
             authority = ?5
         WHERE id = ?1",
        params![chunk_id, observed_at, valid_from, valid_to, authority],
    )
    .map_err(Into::into)
}

/// Reconciliation report (plan M2): which sources fell out of the live set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub deleted_sources: usize,
    pub deleted_chunks: usize,
    pub orphan_uris: Vec<String>,
}

/// Reconcile sources of `kind` against a live set of URIs. Any active source of
/// that kind whose uri is NOT in `live_uris` is marked deleted and its chunks
/// are swept from retrieval (vec0 + FTS via trigger + knowledge rows). This is
/// how a vault delete or rename is detected: the caller enumerates the files on
/// disk; anything indexed but no longer on disk is retired.
///
/// Never deletes blindly across kinds — only `kind`-scoped sources are touched,
/// so reconciling a vault never retires a manual memory.
pub fn reconcile(
    tx: &Transaction<'_>,
    kind: &str,
    live_uris: &std::collections::HashSet<String>,
) -> Result<ReconcileReport> {
    let indexed = orphaned_sources(tx, kind, live_uris)?;

    let mut report = ReconcileReport::default();
    for (sid, uri) in &indexed {
        report.orphan_uris.push(uri.clone());
        report.deleted_chunks += sweep_source_chunks(tx, *sid)?;
        tx.execute(
            "UPDATE sources SET state = 'deleted', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![sid],
        )?;
        tx.execute(
            "UPDATE source_revisions SET state = 'tombstoned'\n             WHERE source_id = ?1 AND state = 'active'",
            params![sid],
        )?;
        report.deleted_sources += 1;
    }
    Ok(report)
}

/// Active sources of `kind` absent from `live_uris` — exactly the set a
/// reconcile WOULD retire. The reconcile handler preflights this set against
/// legal holds (a held chunk refuses the whole sweep with 409),
/// and `reconcile` itself re-uses it so both sites share one query.
pub fn orphaned_sources(
    tx: &Transaction<'_>,
    kind: &str,
    live_uris: &std::collections::HashSet<String>,
) -> Result<Vec<(i64, String)>> {
    let mut stmt =
        tx.prepare("SELECT id, uri FROM sources WHERE kind = ?1 AND state = 'active'")?;
    let rows = stmt.query_map(params![kind], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(rows
        .filter_map(|r| r.ok())
        .filter(|(_, uri)| !live_uris.contains(uri))
        .collect())
}

/// Chunk ids belonging to `source_id`. Shared by the erasure guard so a held
/// chunk can be refused/deferred before the sweep deletes it.
pub fn chunk_ids_for_source(tx: &Transaction<'_>, source_id: i64) -> Result<Vec<i64>> {
    let mut stmt = tx.prepare("SELECT id FROM knowledge WHERE source_id = ?1")?;
    let rows = stmt.query_map(params![source_id], |r| r.get::<_, i64>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Delete a single source by id (plan M2 explicit delete). Sweeps chunks and
/// marks the source + active revision tombstoned. Returns false if the source
/// id does not exist.
pub fn delete_source(tx: &Transaction<'_>, source_id: i64) -> Result<bool> {
    let existed: i64 = tx.query_row(
        "SELECT COUNT(*) FROM sources WHERE id = ?1",
        params![source_id],
        |r| r.get(0),
    )?;
    if existed == 0 {
        return Ok(false);
    }
    sweep_source_chunks(tx, source_id)?;
    tx.execute(
        "UPDATE sources SET state = 'deleted', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        params![source_id],
    )?;
    tx.execute(
        "UPDATE source_revisions SET state = 'tombstoned'\n         WHERE source_id = ?1 AND state = 'active'",
        params![source_id],
    )?;
    Ok(true)
}

/// Remove every chunk belonging to `source_id` from retrieval: vec0 rows, FTS
/// rows (via the knowledge_ad trigger), and the knowledge rows themselves.
/// A chunk under an active legal hold REFUSES the
/// sweep. The HTTP handlers preflight with `refuse_if_held` so operators see
/// the `409 legal_hold_active` envelope; this core guard is the backstop that
/// keeps a non-HTTP caller from retiring a source and orphaning the frozen
/// chunk's provenance. A failed vec0 delete propagates (a sweep that leaves an
/// orphan vector is a partial erasure, not a retirement).
fn sweep_source_chunks(tx: &Transaction<'_>, source_id: i64) -> Result<usize> {
    let ids = chunk_ids_for_source(tx, source_id)?;
    // Active holds are tiny + partial-index-served; a missing legal_holds table
    // (a unit-test schema) means no holds.
    if let Err(e) = crate::legal_hold::refuse_if_held(tx, &ids) {
        if !(e.inner.code == "internal_error" && is_missing_table(&e.inner.message)) {
            return Err(anyhow::anyhow!(
                "source {source_id} sweep refused: {}",
                e.inner.message
            ));
        }
    }
    for id in &ids {
        tx.execute(
            "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
            params![id],
        )?;
    }
    let mut n = 0usize;
    for id in &ids {
        n += tx.execute("DELETE FROM knowledge WHERE id = ?1", params![id])?;
    }
    Ok(n)
}

fn is_missing_table(msg: &str) -> bool {
    msg.contains("no such table")
}

/// Extension trait so `query_row(...).optional()` works without importing the
/// trait at every call site. rusqlite ships `OptionalExtension` for this.
trait OptionalExt {
    type T;
    fn optional(self) -> Result<Option<Self::T>>;
}

impl<T> OptionalExt for Result<T, rusqlite::Error> {
    type T = T;
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    // Lifecycle vocabulary used by the assertions below. Kept here (not exported)
    // because the production binary only exercises KIND_VAULT today; the other
    // kinds/states are the documented set the connector layer will use.
    const KIND_MANUAL: &str = "manual";
    const STATE_ACTIVE: &str = "active";
    const STATE_DELETED: &str = "deleted";
    const REV_SUPERSEDED: &str = "superseded";
    const REV_TOMBSTONED: &str = "tombstoned";

    /// Build an in-memory DB with the v0.9.4 source tables + a minimal
    /// knowledge table (enough for the link/sweep tests). Does not run the full
    /// migration (no vec0/FTS) — these tests exercise the source/revision logic
    /// only, which is pure SQL over the source tables.
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE knowledge(\n                id INTEGER PRIMARY KEY,\n                content TEXT,\n                source_id INTEGER,\n                revision_id INTEGER,\n                observed_at TEXT,\n                valid_from TEXT,\n                valid_to TEXT,\n                authority REAL,\n                flagged INTEGER DEFAULT 0\n             );\n             CREATE TABLE sources(\n                id INTEGER PRIMARY KEY AUTOINCREMENT,\n                uri TEXT NOT NULL UNIQUE,\n                kind TEXT NOT NULL DEFAULT 'vault',\n                title TEXT,\n                current_revision_id INTEGER,\n                state TEXT NOT NULL DEFAULT 'active',\n                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,\n                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,\n                observed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP\n             );\n             CREATE TABLE source_revisions(\n                id INTEGER PRIMARY KEY AUTOINCREMENT,\n                source_id INTEGER NOT NULL,\n                revision TEXT NOT NULL,\n                content_hash TEXT,\n                chunk_count INTEGER NOT NULL DEFAULT 0,\n                byte_size INTEGER NOT NULL DEFAULT 0,\n                state TEXT NOT NULL DEFAULT 'active',\n                fetched_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,\n                FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE\n             );\n             CREATE TABLE vec_knowledge(\n                knowledge_id INTEGER PRIMARY KEY\n             );\n             CREATE TABLE legal_holds(\n                id INTEGER PRIMARY KEY AUTOINCREMENT,\n                knowledge_id INTEGER NOT NULL,\n                reason TEXT NOT NULL,\n                held_by TEXT,\n                held_at INTEGER NOT NULL,\n                released_at INTEGER\n             );\n             CREATE UNIQUE INDEX idx_source_revisions_src_rev\n                ON source_revisions(source_id, revision);",
        )
        .unwrap();
        c
    }

    fn chunk(tx: &Transaction<'_>, content: &str) -> i64 {
        tx.execute(
            "INSERT INTO knowledge(content) VALUES (?1)",
            params![content],
        )
        .unwrap();
        tx.last_insert_rowid()
    }

    #[test]
    fn revision_is_full_content_hash() {
        let a = compute_revision("hello world");
        let b = compute_revision("hello world");
        let c = compute_revision("hello world!");
        assert_eq!(a, b, "same content → same revision");
        assert_ne!(a, c, "changed content → new revision");
    }

    #[test]
    fn upsert_source_is_idempotent_and_reactivates() {
        let mut c = db();
        let tx = c.transaction().unwrap();
        let id1 = upsert_source(&tx, "/v/a.md", KIND_VAULT, Some("A")).unwrap();
        let id2 = upsert_source(&tx, "/v/a.md", KIND_VAULT, Some("A")).unwrap();
        assert_eq!(id1, id2, "same uri reuses the source row");

        // Mark deleted, then re-upsert → reactivated.
        tx.execute(
            "UPDATE sources SET state = 'deleted' WHERE id = ?1",
            params![id1],
        )
        .unwrap();
        let id3 = upsert_source(&tx, "/v/a.md", KIND_VAULT, Some("A")).unwrap();
        let state: String = tx
            .query_row(
                "SELECT state FROM sources WHERE id = ?1",
                params![id3],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(id3, id1);
        assert_eq!(
            state, STATE_ACTIVE,
            "re-ingest reactivates a deleted source"
        );
    }

    #[test]
    fn same_revision_is_noop_then_new_supersedes() {
        let mut c = db();
        let tx = c.transaction().unwrap();
        let sid = upsert_source(&tx, "/v/a.md", KIND_VAULT, None).unwrap();

        // First revision → created.
        let r1 = upsert_revision(&tx, sid, "r1", None, 1, 100).unwrap();
        assert!(matches!(r1, RevisionOutcome::Created { superseded: 0, .. }));

        // Same revision again → unchanged.
        let r1b = upsert_revision(&tx, sid, "r1", None, 1, 100).unwrap();
        assert!(matches!(r1b, RevisionOutcome::Unchanged(_)));

        // New revision → created + prior superseded.
        let r2 = upsert_revision(&tx, sid, "r2", None, 2, 200).unwrap();
        match r2 {
            RevisionOutcome::Created { id, superseded } => {
                assert_eq!(superseded, 1, "the one prior active revision superseded");
                let current: i64 = tx
                    .query_row(
                        "SELECT current_revision_id FROM sources WHERE id = ?1",
                        params![sid],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(current, id, "source points at the new revision");
            }
            _ => panic!("expected Created"),
        }

        // The old revision is now superseded, not active.
        let old_state: String = tx
            .query_row(
                "SELECT state FROM source_revisions WHERE revision = 'r1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_state, REV_SUPERSEDED);
    }

    #[test]
    fn link_chunks_sets_source_and_revision() {
        let mut c = db();
        let tx = c.transaction().unwrap();
        let sid = upsert_source(&tx, "/v/a.md", KIND_VAULT, None).unwrap();
        let rid = match upsert_revision(&tx, sid, "r1", None, 2, 100).unwrap() {
            RevisionOutcome::Created { id, .. } => id,
            _ => panic!("expected created"),
        };
        let c1 = chunk(&tx, "para one");
        let c2 = chunk(&tx, "para two");
        let n = link_chunks(&tx, sid, rid, &[c1, c2]).unwrap();
        assert_eq!(n, 2);
        for id in [c1, c2] {
            let (s, r): (i64, i64) = tx
                .query_row(
                    "SELECT source_id, revision_id FROM knowledge WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(s, sid);
            assert_eq!(r, rid);
        }
    }

    #[test]
    fn reconcile_retires_sources_not_in_live_set() {
        let mut c = db();
        let tx = c.transaction().unwrap();
        let a = upsert_source(&tx, "/v/a.md", KIND_VAULT, None).unwrap();
        let b = upsert_source(&tx, "/v/b.md", KIND_VAULT, None).unwrap();
        let manual = upsert_source(&tx, "manual:1", KIND_MANUAL, None).unwrap();
        for sid in [a, b] {
            let rid = match upsert_revision(&tx, sid, "r1", None, 1, 10).unwrap() {
                RevisionOutcome::Created { id, .. } => id,
                _ => panic!(),
            };
            let cid = chunk(&tx, "x");
            link_chunks(&tx, sid, rid, &[cid]).unwrap();
        }
        // manual source has no chunks; ensure it survives a vault reconcile.

        let live: std::collections::HashSet<String> = ["/v/a.md".to_string()].into_iter().collect();
        let report = reconcile(&tx, KIND_VAULT, &live).unwrap();

        assert_eq!(report.deleted_sources, 1, "b.md retired");
        assert_eq!(report.deleted_chunks, 1, "b.md's chunk swept");
        assert_eq!(report.orphan_uris, vec!["/v/b.md".to_string()]);
        assert_eq!(report.deleted_chunks, 1);

        // a.md survived.
        let a_state: String = tx
            .query_row("SELECT state FROM sources WHERE id = ?1", params![a], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(a_state, STATE_ACTIVE);
        // manual source untouched by a vault reconcile.
        let m_state: String = tx
            .query_row(
                "SELECT state FROM sources WHERE id = ?1",
                params![manual],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(m_state, STATE_ACTIVE);
    }

    #[test]
    fn slack_reconcile_sweeps_deleted_channel_and_spares_other_kinds() {
        // removing a channel from the allowed list
        // retires that channel's sources via a kind-scoped reconcile — a Slack
        // reconcile never touches a CRM source (kind-scoping is per-connector).
        let mut c = db();
        let tx = c.transaction().unwrap();
        let dropped = upsert_source(&tx, "slack://sales/1700000000.1", "slack", None).unwrap();
        let kept = upsert_source(&tx, "slack://sales/1700000000.2", "slack", None).unwrap();
        let crm = upsert_source(&tx, "crm://acme/opp-1", "crm", None).unwrap();
        for sid in [dropped, kept, crm] {
            let rid = match upsert_revision(&tx, sid, "r1", None, 1, 10).unwrap() {
                RevisionOutcome::Created { id, .. } => id,
                _ => panic!(),
            };
            let cid = chunk(&tx, "x");
            link_chunks(&tx, sid, rid, &[cid]).unwrap();
        }

        // The channel was removed → only "kept" is in the live set.
        let live: std::collections::HashSet<String> = ["slack://sales/1700000000.2".to_string()]
            .into_iter()
            .collect();
        let report = reconcile(&tx, "slack", &live).unwrap();

        assert_eq!(report.deleted_sources, 1, "dropped channel retired");
        assert_eq!(report.deleted_chunks, 1, "its chunk swept");
        assert_eq!(
            report.orphan_uris,
            vec!["slack://sales/1700000000.1".to_string()]
        );

        // kept + the CRM source survive a slack reconcile.
        let count_active: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM sources WHERE state = ?1 AND id IN (?2, ?3)",
                params![STATE_ACTIVE, kept, crm],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_active, 2);
    }

    #[test]
    fn sweep_refuses_when_a_chunk_is_held() {
        // the core sweep refuses on an active hold instead of
        // deferring — retiring the source would orphan the frozen chunk's
        // provenance. The HTTP preflight owns the 409 envelope; this is the
        // backstop for any non-HTTP caller.
        let mut c = db();
        let tx = c.transaction().unwrap();
        let sid = upsert_source(&tx, "/v/a.md", KIND_VAULT, None).unwrap();
        let rid = match upsert_revision(&tx, sid, "r1", None, 1, 10).unwrap() {
            RevisionOutcome::Created { id, .. } => id,
            _ => panic!("expected created"),
        };
        let cid = chunk(&tx, "evidence");
        link_chunks(&tx, sid, rid, &[cid]).unwrap();
        crate::legal_hold::insert_holds(&tx, &[cid], "litigation", Some("dpo"), 1).unwrap();

        let err = delete_source(&tx, sid).unwrap_err();
        assert!(
            err.to_string().contains("refused"),
            "held chunk must refuse the sweep: {err}"
        );
        // Nothing was destroyed: chunk + source survive, whole sweep rolled
        // back by the caller's tx (here we just assert the rows still exist).
        let chunks: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE id = ?1",
                params![cid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(chunks, 1, "held chunk untouched");
    }

    #[test]
    fn delete_source_sweeps_chunks_and_tombstones() {
        let mut c = db();
        let tx = c.transaction().unwrap();
        let sid = upsert_source(&tx, "/v/a.md", KIND_VAULT, None).unwrap();
        let rid = match upsert_revision(&tx, sid, "r1", None, 1, 10).unwrap() {
            RevisionOutcome::Created { id, .. } => id,
            _ => panic!(),
        };
        let cid = chunk(&tx, "content");
        link_chunks(&tx, sid, rid, &[cid]).unwrap();

        assert!(delete_source(&tx, sid).unwrap());
        // Chunk gone from retrieval.
        let remaining: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE source_id = ?1",
                params![sid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        // Source retained as deleted; revision tombstoned.
        let (sstate, rstate): (String, String) = tx
            .query_row(
                "SELECT s.state, rv.state FROM sources s\n                 LEFT JOIN source_revisions rv ON rv.source_id = s.id\n                 WHERE s.id = ?1",
                params![sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(sstate, STATE_DELETED);
        assert_eq!(rstate, REV_TOMBSTONED);

        // Nonexistent id → false.
        assert!(!delete_source(&tx, 9999).unwrap());
    }

    #[test]
    fn superseded_chunks_are_not_queryable() {
        // The sweep happens on reconcile/delete; on a content change the ingest
        // path sweeps the old source_path chunks before re-inserting (existing
        // v0.9.2 logic). Here we verify delete makes the chunk unqueryable via
        // the source link, which is the retrieval-relevance invariant.
        let mut c = db();
        let tx = c.transaction().unwrap();
        let sid = upsert_source(&tx, "/v/a.md", KIND_VAULT, None).unwrap();
        let rid = match upsert_revision(&tx, sid, "r1", None, 1, 10).unwrap() {
            RevisionOutcome::Created { id, .. } => id,
            _ => panic!(),
        };
        let cid = chunk(&tx, "secret");
        link_chunks(&tx, sid, rid, &[cid]).unwrap();
        delete_source(&tx, sid).unwrap();
        let found: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE id = ?1",
                params![cid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(found, 0, "deleted source's chunk must be gone");
    }

    #[test]
    fn stamp_evidence_writes_temporal_columns_and_authority() {
        let mut c = db();
        let tx = c.transaction().unwrap();
        let cid = chunk(&tx, "a fact");
        stamp_evidence(
            &tx,
            cid,
            "2024-03-01 00:00:00",
            Some("2024-03-01 00:00:00"),
            None,
            AUTHORITY_MANUAL,
        )
        .unwrap();
        tx.commit().unwrap();
        let (obs, vf, vt, auth, flag): (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<f64>,
            i64,
        ) = c
            .query_row(
                "SELECT observed_at, valid_from, valid_to, authority, flagged \
                 FROM knowledge WHERE id = ?1",
                params![cid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(obs.as_deref(), Some("2024-03-01 00:00:00"));
        assert_eq!(vf.as_deref(), Some("2024-03-01 00:00:00"));
        assert!(vt.is_none(), "a freshly-stamped fact has no expiry");
        assert_eq!(auth, Some(AUTHORITY_MANUAL as f64));
        assert_eq!(flag, 0);
    }
}
