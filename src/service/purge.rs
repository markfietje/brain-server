//! The knowledge-purge core — the shared hard-delete primitive for every
//! erasure surface (`/purge`, the DSAR workflow, client termination, ump
//! hard-forget), extracted verbatim out of the gate handler by the Quarry
//! milestone (the DSAR core cannot depend on a handler module, and this
//! primitive IS the `knowledge` story of that extraction).
//!
//! OWNS (this aggregate's complete storage story):
//! - the `knowledge` row delete + its declared-FK and soft-ref children:
//!   `vec_knowledge` (explicit), `relationships` (explicit; the FK itself
//!   SET-NULLs), `evidence_links` (explicit, both arms), `proposals`
//!   (`conflict_with`), `recall_traces` (JSON1 path filter over `$.hits`),
//!   `embeddings` (FK ON DELETE CASCADE — auto), and the orphan-`entities`
//!   sweep;
//! - the tombstone row each delete owes (SHA-256 content digest —
//!   `crate::audit::hash`, the lowercase-hex encoder — never the raw text,
//!   never a brute-forceable 64-bit fingerprint);
//! - the structural legal-hold fence, INSIDE the function (the backstop
//!   that makes "every erasure path" true of the FUNCTION, not of today's
//!   call-site discipline — a future caller cannot repeat the ump.forget
//!   miss). Handler-side preflights keep their own `409 legal_hold_active`
//!   listing via `crate::legal_hold`; this fence is the second, mandatory
//!   net.
//!
//! FK-children map for the `knowledge` parent DELETE (declared FKs,
//! `PRAGMA foreign_keys=ON`):
//! - `embeddings.knowledge_id` → ON DELETE CASCADE (auto, no statement);
//! - `relationships.knowledge_id` → ON DELETE SET NULL, rows deleted
//!   explicitly above (the SET NULL alone would orphan PII-named entities);
//! - `evidence_links.from/to_chunk`, `proposals.conflict_with`,
//!   `recall_traces.audit_id` (soft refs, no declared FK) → explicit
//!   DELETEs above; `tombstones.knowledge_id` is a soft ref BY DESIGN (the
//!   registry outlives the row);
//! - `case_articles.knowledge_id`, `kcs_translations.knowledge_id` →
//!   declared NO ACTION, NOT cleared here (pre-existing ceiling, documented
//!   honestly): purging a chunk that carries a case article or a knowledge
//!   translation violates the FK and fails the whole tx LOUDLY (fail-closed
//!   erasure-safe direction; unifying those sweeps is a follow-up, not
//!   silently widened reach).
//!
//! Error convention: [`PurgeError`] is the ServiceError shape — one typed
//! enum, `From<rusqlite::Error>` preserving the message verbatim; the
//! handler boundary (handlers/mod.rs's `From<PurgeError>` impl) renders the
//! route's frozen vocabulary — the exact shared `409 legal_hold_active`
//! envelope every erasure route emits.

use rusqlite::params;
use std::collections::{HashMap, HashSet};

/// Typed service error. `Database` carries the rusqlite text VERBATIM;
/// `LegalHold` carries the active-hold reasons so the handler can render the
/// exact shared `409 legal_hold_active` envelope.
#[derive(Debug)]
pub enum PurgeError {
    /// A query failed; the rusqlite message travels unchanged.
    Database(String),
    /// The in-function legal-hold fence fired: held id → reasons. A held id
    /// is never purged here.
    LegalHold(HashMap<i64, Vec<String>>),
}

impl std::fmt::Display for PurgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PurgeError::Database(e) => write!(f, "database error: {e}"),
            PurgeError::LegalHold(held) => write!(f, "legal hold active on {held:?}"),
        }
    }
}

impl From<rusqlite::Error> for PurgeError {
    fn from(e: rusqlite::Error) -> Self {
        PurgeError::Database(e.to_string())
    }
}

/// Shared hard-delete for a list of chunk ids, run inside the caller's
/// transaction. Removes the `knowledge` row + its `vec_knowledge` embedding +
/// graph nodes/edges + supersession/derivation pointers + `proposals`
/// references, and appends a tombstone row (hash-only). Used by `/purge`
/// (reason `explicit`) and the DSAR workflow (reason `owner:<subject>`, with
/// derived descendants carrying `derived` + the purge root's origin id).
/// Returns the number of chunks actually deleted.
pub fn purge_chunk_ids(
    tx: &rusqlite::Transaction<'_>,
    ids: &[i64],
    now: i64,
    reason: &str,
    origin_id: Option<i64>,
) -> Result<i64, PurgeError> {
    // the structural legal-hold fence. Every production
    // caller preflights (`/purge` 409s, DSAR defers + lists, client
    // termination filters, ump forget refuses), so this guard is the backstop
    // that makes "every erasure path" true of the FUNCTION, not of today's
    // call-site discipline — a future caller cannot repeat the ump.forget miss.
    let held = crate::legal_hold::active_reasons(tx, ids)?;
    if !held.is_empty() {
        return Err(PurgeError::LegalHold(held));
    }
    let mut purged = 0i64;
    // Entity ids referenced by the purged chunks' relationships, collected so the
    // post-loop orphan sweep can drop graph nodes that no longer link to any
    // surviving knowledge. Scoped (never global) so standalone entities unrelated
    // to this purge are untouched.
    let mut affected_entities: HashSet<i64> = HashSet::new();
    for id in ids {
        // Capture the entity ids this chunk's relationships reference (the only
        // reliable link — `entities` has no `knowledge_id` column).
        if let Ok(mut s) = tx.prepare(
            "SELECT from_entity_id FROM relationships WHERE knowledge_id = ?1
             UNION SELECT to_entity_id FROM relationships WHERE knowledge_id = ?1",
        ) && let Ok(rows) = s.query_map([id], |r| r.get::<_, i64>(0))
        {
            for e in rows.flatten() {
                affected_entities.insert(e);
            }
        }
        // Capture a SHA-256 of the row content for the tombstone before
        // deletion (the deletion registry's digest of
        // DELETED content must not be an offline-brute-forceable xxh3-64 —
        // low-entropy personal values are recoverable from 64-bit hashes).
        // ponytail: still a one-way digest, not a secrecy mechanism — a full
        // at-rest compromise (key beside data) is out of scope.
        let content_digest: Option<String> = tx
            .query_row(
                "SELECT content FROM knowledge WHERE id = ?1",
                params![id],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .map(|c| crate::audit::hash(&c));
        // Graph edges are removed by their knowledge link (the FK on
        // `relationships.knowledge_id` only SET NULLs, it does not delete, so
        // the row must go explicitly). The old clause also referenced
        // `entities.knowledge_id`, a column that does NOT exist — that subquery
        // raised "no such column" and silently aborted the whole DELETE, so
        // relationships (and with them their PII-bearing entity names) survived
        // every purge. Now removed; entity-level cleanup runs in the post-loop
        // orphan sweep. vec0 rows are deleted by knowledge_id.
        // these residue DELETEs were `let _ =` — a single
        // silent failure left relationships/vec/evidence/traces for a chunk the
        // purge then tombstoned as erased (partial erasure certified complete).
        // All now propagate: a residue failure rolls back the whole purge tx.
        tx.execute(
            "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM relationships WHERE knowledge_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM evidence_links WHERE from_chunk = ?1 OR to_chunk = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM proposals WHERE conflict_with = ?1",
            params![id],
        )?;
        // cascade to recall_traces. The trace side table (read-event
        // replay artifact) embeds hit chunk ids in its JSON; a purged chunk
        // must not leave a trace that still "proves" it was returned. JSON1
        // is compiled into the bundled SQLite (rusqlite "bundled"), so the
        // path filter is exact, not a LIKE. A trace with an unparseable JSON
        // body is skipped by the json_valid filter rather than failing the
        // purge; only a genuine SQL error now propagates.
        tx.execute(
            "DELETE FROM recall_traces WHERE audit_id IN (
                 SELECT rt.audit_id FROM recall_traces rt
                  WHERE json_valid(rt.trace_json)
                    AND EXISTS (
                        SELECT 1 FROM json_each(rt.trace_json, '$.hits')
                         WHERE json_extract(value, '$.id') = ?1
                    )
             )",
            params![id],
        )?;
        let n = tx.execute("DELETE FROM knowledge WHERE id = ?1", params![id])?;
        if n > 0 {
            tx.execute(
                "INSERT INTO tombstones(knowledge_id, content_hash, purged_at, reason, origin_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    content_digest.unwrap_or_else(|| "unknown".into()),
                    now,
                    reason,
                    origin_id
                ],
            )?;
            purged += 1;
        }
    }

    // orphan-entity sweep. An entity referenced by a purged chunk
    // whose relationships are now all gone is erased too — an entity *name* can
    // itself be PII (a person/email/account label), so "memory you can see,
    // approve, and erase" must not leave a graph node behind after erasure.
    // Scoped to the affected set + the "no remaining relationship" guard, so a
    // shared entity still linked to surviving knowledge survives. Best-effort.
    for e in &affected_entities {
        let alive: i64 = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM relationships
                                WHERE from_entity_id = ?1 OR to_entity_id = ?1)",
                params![e],
                |r| r.get(0),
            )
            .unwrap_or(1);
        if alive == 0 {
            // Clear any residual relationship rows first (FK-off safety; if FKs
            // are on the entity DELETE cascades them anyway).
            // was `let _ =` — an orphan PII-named entity
            // surviving a purge is a partial erasure; propagate.
            tx.execute(
                "DELETE FROM relationships WHERE from_entity_id = ?1 OR to_entity_id = ?1",
                params![e],
            )?;
            tx.execute("DELETE FROM entities WHERE id = ?1", params![e])?;
        }
    }

    Ok(purged)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the deletion registry's digest is the SHA-256 of
    /// the deleted content — NOT the row's own content_hash (the 64-bit xxh3
    /// that was brute-forceable offline for low-entropy values). The tombstone
    /// must carry the new digest, and it must not be the stored hash.
    /// (Moved verbatim with the primitive from the gate handler tests; the
    /// digest helper is now the canonical `crate::audit::hash` — byte-for-byte
    /// the same lowercase-hex SHA-256 the gate-local copy produced.)
    #[test]
    fn purge_tombstone_digest_is_sha256_of_content() {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        crate::migration::run_migration(&mut conn, 1).expect("migration");
        conn.execute(
            "INSERT INTO knowledge (content, content_hash, node_kind) \
             VALUES ('SSN 123-45-6789', 'xxh3-of-content', 'fact')",
            [],
        )
        .unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM knowledge", [], |r| r.get(0))
            .unwrap();
        let now = chrono::Utc::now().timestamp();
        let tx = conn.transaction().unwrap();
        purge_chunk_ids(&tx, &[id], now, "test", None).unwrap();
        tx.commit().unwrap();
        let digest: String = conn
            .query_row(
                "SELECT content_hash FROM tombstones WHERE knowledge_id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(digest, crate::audit::hash("SSN 123-45-6789"));
        assert_eq!(digest.len(), 64, "SHA-256 hex is 64 chars");
        assert_ne!(digest, "xxh3-of-content");
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    /// the in-function legal-hold fence is the BACKSTOP every erasure
    /// caller inherits by calling this primitive — even one that forgot its
    /// own preflight. A held id is never purged; the error carries the hold
    /// reasons for the shared `409 legal_hold_active` envelope.
    #[test]
    fn purge_chunk_ids_backstop_refuses_held_id() {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().expect("db");
        crate::migration::run_migration(&mut conn, 1).expect("migration");
        conn.execute(
            "INSERT INTO knowledge (content, content_hash) VALUES ('litigation evidence', 'h1')",
            [],
        )
        .unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM knowledge", [], |r| r.get(0))
            .unwrap();
        let now = chrono::Utc::now().timestamp();
        let tx = conn.transaction().unwrap();
        crate::legal_hold::insert_holds(&tx, &[id], "case-42 litigation", Some("dpo"), now)
            .unwrap();
        let err = purge_chunk_ids(&tx, &[id], now, "test", None).unwrap_err();
        match err {
            PurgeError::LegalHold(held) => {
                assert_eq!(
                    held.get(&id),
                    Some(&vec!["case-42 litigation".to_string()]),
                    "the fence reports the hold reasons"
                );
            }
            other => panic!("expected the legal-hold backstop, got {other:?}"),
        }
        tx.commit().unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1, "a held id is never purged");
    }
}
