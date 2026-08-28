//! UMP ops storage: the id-resolution lookup, the bi-temporal supersession
//! read, the relations read, the soft-forget write, and the consent-denial
//! audit emission.
//!
//! OWNS the `/ump` family's storage story EXCEPT what already converged:
//! row loads ride `service::lifecycle::fetch::load_knowledge_row`, hard
//! erasure rides `service::purge::purge_chunk_ids` (whose header owns the
//! FK-children map), and the row access-meta read is `service::procedure::
//! row_access_meta` — one definition, dup-guard enforced. What lives here:
//! - `row_id_for_ump_id`: the content-addressed id lookup (ingest computes
//!   `ump_id` from `domain \0 content`, so a peer-sent id round-trips);
//! - `superseded_by_for`: the L2 bi-temporal `superseded_by` — `supersedes`
//!   evidence links pointing AT the chunk, resolved to successor urns;
//! - `raw_relations`: the chunk's outgoing graph edges as RAW triples —
//!   the stored-text sanitize stays handler-side (read seam);
//! - `chunk_exists` + `forget_soft`: the soft-forget block — flag +
//!   hash-only tombstone + its audit row INSIDE the caller's tx (the
//!   evidence commits with the write it evidences); the hard path is the
//!   purge core behind the legal-hold fence, orchestration stays at the
//!   handler;
//! - `record_forbidden_scope`: the §3.7 consent-mismatch `Denied` auth
//!   row — best-effort by contract, never fails the request.
//!
//! Error Display carries the exact pre-move message text; the handler
//! renders it verbatim.

use std::fmt;

use rusqlite::{Connection, OptionalExtension, params};

/// A storage failure. `Display` carries the exact pre-move message; the
/// handler wraps it in `HandlerError::internal` unchanged.
#[derive(Debug)]
pub(crate) enum UmpOpsError {
    Database(String),
}

impl fmt::Display for UmpOpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UmpOpsError::Database(m) => f.write_str(m),
        }
    }
}

impl From<rusqlite::Error> for UmpOpsError {
    fn from(e: rusqlite::Error) -> Self {
        UmpOpsError::Database(e.to_string())
    }
}

/// The indexed `ump_id` lookup behind the `urn:ump:` id resolution (plain
/// integers and the legacy trailing-numeric shape parse without storage).
pub(crate) fn row_id_for_ump_id(conn: &Connection, ump_id: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM knowledge WHERE ump_id = ?1",
        params![ump_id],
        |r| r.get::<_, i64>(0),
    )
    .optional()
}

/// The UMP urns of the chunks that superseded this one (L2 bi-temporal
/// `superseded_by`): `supersedes` evidence links pointing AT this chunk,
/// resolved to the successor's content-addressed id. Empty when current.
pub(crate) fn superseded_by_for(conn: &Connection, id: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT k.ump_id FROM evidence_links el
           JOIN knowledge k ON k.id = el.from_chunk
          WHERE el.kind = 'supersedes' AND el.to_chunk = ?1
            AND k.ump_id IS NOT NULL AND k.ump_id != ''
          ORDER BY el.id",
    )?;
    let rows = stmt.query_map(params![id], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// One graph relation as a raw (from-name, to-name, relation-type) triple.
pub(crate) type RelationTriple = (String, String, String);

/// The chunk's outgoing relations (the raw input to the handler's
/// `{from, to, type}` rows — the record engine renders them into
/// `about`/typed `body.structured.relations`). Entity names arrive from
/// vault wikilinks/frontmatter with no vocabulary gate and the relation
/// type is linker text — stored text; the sanitize stays handler-side.
pub(crate) fn raw_relations(conn: &Connection, id: i64) -> rusqlite::Result<Vec<RelationTriple>> {
    let mut stmt = conn.prepare(
        "SELECT e1.name, e2.name, r.relation_type
           FROM relationships r
           JOIN entities e1 ON r.from_entity_id = e1.id
           JOIN entities e2 ON r.to_entity_id = e2.id
          WHERE r.knowledge_id = ?1
            AND r.superseded_at IS NULL",
    )?;
    let rows = stmt.query_map(params![id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    rows.collect()
}

/// Existence probe behind the forget fence (404 before any write).
pub(crate) fn chunk_exists(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM knowledge WHERE id = ?1)",
        params![id],
        |r| r.get(0),
    )?;
    Ok(n != 0)
}

/// The soft-forget block, inside the CALLER'S tx: quarantine-style flag,
/// hash-only tombstone (the content hash rides along when the row is still
/// readable — a best-effort read by design), and the audit row in the SAME
/// tx so the evidence commits with the write it evidences. The row stays
/// retrievable with `include_flagged` — soft forget is reversible in
/// posture; the tombstone is the registry row, not an erasure.
pub(crate) fn forget_soft(
    tx: &rusqlite::Transaction<'_>,
    id: i64,
    reason: &str,
    now: i64,
) -> Result<(), UmpOpsError> {
    let hash: Option<String> = tx
        .query_row(
            "SELECT content_hash FROM knowledge WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    tx.execute(
        "UPDATE knowledge SET flagged = 1 WHERE id = ?1",
        params![id],
    )?;
    tx.execute(
        "INSERT INTO tombstones(knowledge_id, content_hash, purged_at, reason, origin_id)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, hash, now, reason, None::<i64>],
    )?;
    crate::audit::record(
        tx,
        crate::audit::AuditKind::Ingest,
        "ump",
        &id.to_string(),
        crate::audit::AuditStatus::Ok,
        "ump-forget-soft",
    );
    Ok(())
}

/// Record a `Denied` auth audit row for a §3.7 consent mismatch.
/// Best-effort: a failure is ignored so a missing audit log never fails the
/// request. The detail rides the target slot as a bounded hash, never the
/// raw declared owner.
pub(crate) fn record_forbidden_scope(
    conn: &Connection,
    principal_sub: &str,
    declared_owner: &str,
) -> bool {
    let detail = format!("ump.remember scope.owner={declared_owner} does not match principal");
    crate::audit::record(
        conn,
        crate::audit::AuditKind::Auth,
        principal_sub,
        &detail,
        crate::audit::AuditStatus::Denied,
        "api",
    )
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a §3.7 consent mismatch is audited as a `Denied` auth
    /// event on the read-event-capable audit chain (COMPLIANCE.md §3.5
    /// promised it; the gap this closes was a silent 400 with no footprint).
    #[test]
    fn forbidden_scope_is_audited_as_denied_auth_event() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, 1).unwrap();
        assert!(
            record_forbidden_scope(&conn, "alice", "eve"),
            "the audit write succeeds on the migrated chain"
        );
        let (kind, status, target_hash): (String, String, Option<String>) = conn
            .query_row(
                "SELECT kind, status, target_hash FROM audit_events WHERE kind = 'auth' ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "auth");
        assert_eq!(status, "denied");
        // The scope detail rides in the target slot (record(kind, actor,
        // target, status, detail)) as a bounded xxh3 hash, never the raw
        // string. Derive from the helper's OWN format so the test cannot
        // drift; asserting the exact hash also proves nothing raw persisted.
        let expected = crate::audit::hash("ump.remember scope.owner=eve does not match principal");
        assert_eq!(target_hash.as_deref(), Some(expected.as_str()));
        // The chain still verifies (the denial joined the tamper-evident log).
        assert!(crate::audit::verify_chain(&conn));
    }
}
