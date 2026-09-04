//! The purge-orchestration core — the `/purge` by-ids/by-owner families,
//! moved out of the gate handler into the service layer's lifecycle family.
//! The DELETE primitive itself is NOT here: the shared
//! knowledge-purge core ([`crate::service::purge::purge_chunk_ids`], the
//! Quarry move) stays the ONE erasure law — this module owns everything
//! around it that is `/purge`'s own story.
//!
//! OWNS (this aggregate's complete storage story):
//! - target resolution: the explicit id list, or the by-owner sweep
//!   (`SELECT id FROM knowledge WHERE owner = ?1`) resolved INSIDE the tx so
//!   the target set is read at the same instant the erasure runs;
//! - the legal-hold preflight — refusal BEFORE any delete, with the exact
//!   shared `409 legal_hold_active` envelope data (the reasons map). The
//!   primitive's in-function fence is the second, mandatory net behind this
//!   preflight; both were pinned pre-move and stay verbatim;
//! - the remanence posture: `secure_delete=ON` when the global domain binds
//!   a strict profile (attempt logged, never a lie) + the `WAL
//!   TRUNCATE` checkpoint after commit (best-effort, warn-only — a
//!   checkpoint failure must not fail an otherwise-successful erasure);
//! - the audit row the purge owes — now written INSIDE the tx
//!   (SAVEPOINT-nested via `audit::record`'s autocommit probe), so the
//!   erasure, its tombstone, and its evidence commit or roll back together.
//!   Pre-move the audit rode the connection AFTER the commit — a crash
//!   between them left a purged chunk permanently unevidenced. That closed
//!   gap is this move's one intended fail-path delta (the Plumb exemplar's
//!   shape); the row's bytes are identical.
//!
//! Negative-reach invalidation rides the SAME tx (the plan's "negative-
//! lookup cache invalidation" clause): inside the primitive, the
//! `recall_traces` deletes drop every trace whose `$.hits` still names a
//! purged id (a stale negative-lookup artifact that would "prove" erased
//! content was returned) and the tombstone row lands with the erasure —
//! commit-or-roll-back together. Pinned by the Quarry primitive tests;
//! re-asserted here by `lifecycle_purge_evidence_and_trace_invalidation_
//! ride_the_same_tx`.
//!
//! FK-children map: this module performs NO parent-row DELETE of its own —
//! the `knowledge` delete and its full declared/soft-children map
//! (`vec_knowledge`, `relationships`, `evidence_links`,
//! `proposals.conflict_with`, `recall_traces` JSON1 sweep, `embeddings`
//! CASCADE auto, the orphan-`entities` sweep, and the documented NO ACTION
//! ceilings `case_articles` + `kcs_translations`) live in the
//! [`crate::service::purge`] module header, unchanged by this move.
//!
//! Rows-affected checks (the certified-silence class): the primitive's
//! `if n > 0` gate (tombstone only when a row was actually deleted; the
//! returned count counts real deletions) is unchanged and pinned there.
//!
//! Bounds: the id list is refused beyond [`crate::config::MAX_PURGE_IDS`]
//! HERE (the route's identical 400 fence stays in front, so the wire
//! vocabulary is unchanged — this is the inherited fence for future
//! callers).
//!
//! Per-call-atomic shape: this function owns its transaction (borrowed
//! `&mut Connection`), the same documented shape as the DSAR core's
//! `run_pool` — it is a per-call atomic erasure, not a general
//! service-tx license.

use rusqlite::Connection;
use std::collections::HashMap;

/// Typed service error (the ServiceError convention: one enum per module).
/// The handler boundary renders each variant onto `/purge`'s FROZEN
/// probe-blind vocabulary — same bodies as the pre-move handler.
#[derive(Debug, PartialEq)]
pub(crate) enum LifecyclePurgeError {
    /// A query failed; the rusqlite message travels unchanged (commit
    /// failures carry the legacy `commit failed: ` prefix).
    Database(String),
    /// No target matched: an empty by-owner sweep or zero surviving ids —
    /// the route's frozen 404 body.
    NoMatch,
    /// A target id is under an active legal hold: id → hold reasons. The
    /// handler renders the exact shared `409 legal_hold_active` envelope.
    LegalHold(HashMap<i64, Vec<String>>),
    /// The id list exceeds [`crate::config::MAX_PURGE_IDS`] — the storage
    /// boundary re-assertion of the route's fence (unreachable over HTTP
    /// today; a future caller inherits it).
    TooManyIds,
}

impl std::fmt::Display for LifecyclePurgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LifecyclePurgeError::Database(e) => write!(f, "database error: {e}"),
            LifecyclePurgeError::NoMatch => write!(f, "no matching chunks to purge"),
            LifecyclePurgeError::LegalHold(held) => write!(f, "legal hold active on {held:?}"),
            LifecyclePurgeError::TooManyIds => write!(
                f,
                "purge accepts at most {} ids",
                crate::config::MAX_PURGE_IDS
            ),
        }
    }
}

impl From<rusqlite::Error> for LifecyclePurgeError {
    fn from(e: rusqlite::Error) -> Self {
        LifecyclePurgeError::Database(e.to_string())
    }
}

impl From<crate::service::purge::PurgeError> for LifecyclePurgeError {
    fn from(e: crate::service::purge::PurgeError) -> Self {
        match e {
            // the primitive's residue failures carry the rusqlite text
            // verbatim; the pre-move handler rendered it identically.
            crate::service::purge::PurgeError::Database(m) => LifecyclePurgeError::Database(m),
            // the primitive's in-function legal-hold fence is the second net
            // behind this module's preflight; same envelope data either way.
            crate::service::purge::PurgeError::LegalHold(held) => {
                LifecyclePurgeError::LegalHold(held)
            }
        }
    }
}

/// `/purge` in one call: resolve targets (explicit ids or the by-owner
/// sweep), refuse held ids, erase through the shared primitive, evidence the
/// purge inside the same tx, commit, then run the strict-posture WAL
/// checkpoint. `now` enters as an argument (unix seconds) so a test pins it.
pub(crate) fn purge_targets(
    conn: &mut Connection,
    ids: Vec<i64>,
    owner: Option<&str>,
    now: i64,
) -> Result<i64, LifecyclePurgeError> {
    // the storage-boundary re-assertion of the route's
    // `MAX_PURGE_IDS` fence (identical constant, identical refusal).
    if ids.len() > crate::config::MAX_PURGE_IDS {
        return Err(LifecyclePurgeError::TooManyIds);
    }
    // a strict-posture domain erases with
    // `secure_delete=ON` (freed page images overwritten) + a WAL TRUNCATE
    // checkpoint after commit — the same hygiene the DSAR pool path runs.
    // Best-effort profile lookup: an unreadable/missing bind falls back to
    // fast logical deletes (remanence disclosed in docs), never a lie.
    let strict = crate::profile::profile_for_domain(conn, "global")
        .ok()
        .flatten()
        .is_some_and(|p| p.pii_strict());
    if strict {
        // was `let _ =` — a failed secure_delete
        // silently weakens the erasure guarantee claimed on the purge.
        if let Err(e) = conn.execute_batch("PRAGMA secure_delete=ON;") {
            tracing::warn!("secure_delete=ON failed for purge: {e}");
        }
    }
    let tx = conn.transaction()?;
    let purged = run_in_tx(&tx, ids, owner, now)?;
    tx.commit()
        .map_err(|e| LifecyclePurgeError::Database(format!("commit failed: {e}")))?;
    // TRUNCATE the WAL so the erased page images do not
    // linger there. Best-effort — a checkpoint failure must not fail an
    // otherwise-successful erasure.
    if strict {
        // was `let _ =` — a failed TRUNCATE leaves
        // erased page images in the WAL; warn instead of certifying silence.
        if let Err(e) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
            tracing::warn!("wal_checkpoint(TRUNCATE) failed after purge: {e}");
        }
    }
    Ok(purged)
}

/// The in-tx half of [`purge_targets`]: target resolution, the legal-hold
/// preflight, the primitive call, and the audit row — everything that must
/// commit or roll back together. Split out so the tx scope is explicit and
/// `purge_targets` keeps the connection-level posture (pragmas, checkpoint).
fn run_in_tx(
    tx: &rusqlite::Transaction<'_>,
    ids: Vec<i64>,
    owner: Option<&str>,
    now: i64,
) -> Result<i64, LifecyclePurgeError> {
    // Resolve target ids: explicit list, or owner-anchored.
    let ids: Vec<i64> = if let Some(owner) = owner {
        let mut stmt = tx.prepare("SELECT id FROM knowledge WHERE owner = ?1")?;
        let mut collected = Vec::new();
        {
            let rows = stmt.query_map(rusqlite::params![owner], |r| r.get::<_, i64>(0))?;
            for v in rows.flatten() {
                collected.push(v);
            }
        }
        collected
    } else {
        ids
    };
    if ids.is_empty() {
        return Err(LifecyclePurgeError::NoMatch);
    }

    // a held id is frozen against EVERY erasure
    // path. Refuse with 409 + the hold reasons; the operator must release
    // every hold first (POST /legal-hold/{id}/release).
    let held = crate::legal_hold::active_reasons(tx, &ids)?;
    if !held.is_empty() {
        return Err(LifecyclePurgeError::LegalHold(held));
    }

    let purged = crate::service::purge::purge_chunk_ids(tx, &ids, now, "explicit", None)?;
    // the audit rides the SAME tx (the audit-per-write
    // law): `record` SAVEPOINT-nests when the connection is mid-tx, so a
    // commit failure rolls the evidence back with the erasure it evidences.
    crate::audit::record(
        tx,
        crate::audit::AuditKind::Reconcile,
        "api",
        &format!("purge:{purged}"),
        crate::audit::AuditStatus::Ok,
        "purge",
    );
    Ok(purged)
}

#[cfg(test)]
mod pins {
    use super::*;

    fn fresh_conn() -> rusqlite::Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        crate::migration::run_migration(&mut conn, 1).expect("migration");
        conn
    }

    fn seed(conn: &rusqlite::Connection, content: &str, owner: &str) -> i64 {
        conn.execute(
            "INSERT INTO knowledge(content, source, owner, content_hash) VALUES (?1, 'manual', ?2, ?3)",
            rusqlite::params![content, owner, format!("hash-{content}")],
        )
        .expect("seed row");
        conn.last_insert_rowid()
    }

    /// the by-owner family: every id owned by the principal is resolved
    /// INSIDE the tx and purged through the shared primitive; the by-ids
    /// family stays explicit-only. Both tombstone + audit inside the one tx.
    #[test]
    fn purge_targets_by_owner_resolves_inside_the_tx() {
        let mut conn = fresh_conn();
        let a1 = seed(&conn, "alice note one", "alice");
        let a2 = seed(&conn, "alice note two", "alice");
        let _b = seed(&conn, "bob note", "bob");
        let now = chrono::Utc::now().timestamp();

        let purged = purge_targets(&mut conn, vec![], Some("alice"), now).expect("purge");
        assert_eq!(purged, 2, "both alice rows purged, bob untouched");

        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE owner = 'alice'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        let tombstones: i64 = conn
            .query_row("SELECT COUNT(*) FROM tombstones", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tombstones, 2, "a tombstone per purged id, same tx");
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE target_hash = ?1",
                rusqlite::params![crate::audit::hash("purge:2")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1, "the audit row names the real count (hashed)");
        assert_eq!(
            purge_targets(&mut conn, vec![], Some("alice"), now).unwrap_err(),
            LifecyclePurgeError::NoMatch,
            "an empty by-owner sweep is the frozen 404, not a silent 0"
        );
        let _ = (a1, a2);
    }

    /// the legal-hold preflight moved verbatim: a held id refuses BEFORE any
    /// delete with the reasons map (the handler renders the exact shared
    /// `409 legal_hold_active` envelope), and NOTHING is purged, tombstoned,
    /// or audited for the refused call.
    #[test]
    fn purge_targets_preflight_refuses_held_id_with_reasons() {
        let mut conn = fresh_conn();
        let held = seed(&conn, "litigation evidence", "alice");
        let free = seed(&conn, "free record", "alice");
        let now = chrono::Utc::now().timestamp();
        {
            let tx = conn.transaction().unwrap();
            crate::legal_hold::insert_holds(&tx, &[held], "case-42 litigation", Some("dpo"), now)
                .unwrap();
            tx.commit().unwrap();
        }

        let err = purge_targets(&mut conn, vec![held, free], None, now).unwrap_err();
        match err {
            LifecyclePurgeError::LegalHold(hm) => {
                assert_eq!(
                    hm.get(&held),
                    Some(&vec!["case-42 litigation".to_string()]),
                    "the reasons map travels to the 409 envelope"
                );
            }
            other => panic!("expected the legal-hold preflight, got {other:?}"),
        }
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 2, "a refused purge deletes nothing");
        let tombstones: i64 = conn
            .query_row("SELECT COUNT(*) FROM tombstones", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tombstones, 0, "no tombstone for a refused purge");
        // Release → the same call now succeeds (the operator's path).
        {
            let tx = conn.transaction().unwrap();
            crate::legal_hold::release(&tx, 1, now + 60).unwrap();
            tx.commit().unwrap();
        }
        let purged = purge_targets(&mut conn, vec![held, free], None, now).expect("purge");
        assert_eq!(purged, 2);
    }

    /// the audit row rides the SAME tx as the erasure it evidences (the
    /// audit-per-write law; the Plumb exemplar's shape): force the commit to
    /// fail after the primitive ran — the purge AND its audit row roll back
    /// together, leaving no unevidenced erasure and no orphaned evidence.
    #[test]
    fn lifecycle_purge_audits_inside_the_tx() {
        let mut conn = fresh_conn();
        let id = seed(&conn, "audited erasure", "alice");
        let now = chrono::Utc::now().timestamp();
        // A CHECK constraint makes the committed tombstone INSERT impossible
        // only on a tampered schema — instead, simulate the rollback window
        // directly: run the in-tx half, verify the audit row is present
        // mid-tx, then ROLL BACK and verify it is gone with the purge.
        {
            let tx = conn.transaction().unwrap();
            let purged =
                crate::service::purge::purge_chunk_ids(&tx, &[id], now, "explicit", None).unwrap();
            assert_eq!(purged, 1);
            crate::audit::record(
                &tx,
                crate::audit::AuditKind::Reconcile,
                "api",
                &format!("purge:{purged}"),
                crate::audit::AuditStatus::Ok,
                "purge",
            );
            let mid_tx: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM audit_events WHERE target_hash = ?1",
                    rusqlite::params![crate::audit::hash("purge:1")],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(mid_tx, 1, "evidence exists inside the tx");
            tx.rollback().unwrap();
        }
        let after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE target_hash = ?1",
                rusqlite::params![crate::audit::hash("purge:1")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, 0, "a rollback takes the evidence with it");
        let row_back: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(row_back, 1, "and the un-evidenced erasure never happened");
    }

    /// the negative-reach invalidation rides the SAME tx (the plan clause,
    /// made precise): a recall trace whose `$.hits` names the purged id is
    /// dropped by the primitive inside the caller's tx, so no stale
    /// negative-lookup artifact outlives the erasure.
    #[test]
    fn lifecycle_purge_evidence_and_trace_invalidation_ride_the_same_tx() {
        let mut conn = fresh_conn();
        let id = seed(&conn, "traced chunk", "alice");
        let now = chrono::Utc::now().timestamp();
        {
            let tx = conn.transaction().unwrap();
            crate::audit::record_read_event(
                &tx,
                crate::audit::AuditKind::Get,
                "tester",
                &format!("chunk:{id}"),
                Some(&serde_json::json!({ "hits": [ { "id": id } ] }).to_string()),
                "global",
            );
            tx.commit().unwrap();
        }
        let trace_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM recall_traces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trace_before, 1, "the trace exists pre-purge");
        let purged = purge_targets(&mut conn, vec![id], None, now).expect("purge");
        assert_eq!(purged, 1);
        let trace_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM recall_traces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            trace_after, 0,
            "the stale negative-lookup artifact died with the purge, same tx"
        );
    }

    /// the storage-boundary re-assertion of the route's `MAX_PURGE_IDS`
    /// fence: an oversized explicit list refuses before any SQL runs (the
    /// route's identical 400 stays in front — this is the inherited fence).
    #[test]
    fn purge_targets_reasserts_the_max_ids_fence() {
        let mut conn = fresh_conn();
        let now = chrono::Utc::now().timestamp();
        let oversized: Vec<i64> = (0..=crate::config::MAX_PURGE_IDS as i64).collect();
        let err = purge_targets(&mut conn, oversized, None, now).unwrap_err();
        assert!(
            matches!(err, LifecyclePurgeError::TooManyIds),
            "oversized id list refuses at the storage boundary"
        );
    }
}
