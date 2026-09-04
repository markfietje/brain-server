//! The domain-administration core — create/delete/vacuum/export and the
//! relabel transaction for the isolation domains, converged onto the service
//! layer out of `handlers/domains.rs`.
//!
//! OWNS (this aggregate's complete storage story):
//! - the shim-mode domain census ([`shim_domain_rows`]): the DISTINCT domain
//!   labels on `knowledge` (the shared file's truth — the registry's file
//!   list is meaningless when every domain shares one DB) plus the per-label
//!   and total counts;
//! - the per-file census ([`file_domain_counts`]) and the emptiness probe
//!   ([`is_empty_store`]) behind create/warm;
//! - the domain erasure ([`delete_domain_data`]): the legal-hold preflight,
//!   the multi-db audit-segment export, and the two erasure plans (whole-file
//!   sweep in multi-db, domain-scoped sweep in shim);
//! - [`vacuum`] and [`export_snapshot`] (the `VACUUM INTO` snapshot path);
//! - the relabel transaction ([`relabel_chunks`]) behind `/domains/move`.
//!
//! FK-children map for the domain erasure (the `knowledge` parent rows and
//! the domain-keyed tables; `PRAGMA foreign_keys=ON`):
//! - `evidence_links.from/to_chunk` → declared NO ACTION on `knowledge(id)`:
//!   deleted EXPLICITLY first, both arms (shim: subselect-scoped; multi-db:
//!   wholesale — the whole file is the domain);
//! - `relationships.knowledge_id` → ON DELETE SET NULL: deleted explicitly
//!   BEFORE `knowledge` (the SET NULL alone would orphan PII-named
//!   entities);
//! - `relationships.from/to_entity_id` → ON DELETE CASCADE from `entities`:
//!   `entities` are PARENTS here — after the relationship sweep, the
//!   orphan-`entities` delete removes only nodes no longer referenced by ANY
//!   relationship in ANY domain (entities may be shared across domains —
//!   shim mode);
//! - `embeddings.knowledge_id` → ON DELETE CASCADE (auto, no statement);
//! - `tombstones.knowledge_id` → soft ref BY DESIGN (the deletion registry
//!   outlives the row); shim: domain-scoped delete, multi-db: wholesale;
//! - `vec_knowledge.knowledge_id` → no declared FK: deleted explicitly;
//! - `knowledge_fts` → fts5 shadow rows, cleaned by the trigger on
//!   `knowledge` DELETE (no explicit statement — deleting them by hand
//!   double-deletes);
//! - `knowledge.source_id`/`revision_id` → knowledge is the CHILD of
//!   `sources`/`source_revisions` (plain REFERENCES): knowledge goes first,
//!   then `sources` (whose `ON DELETE CASCADE` takes `source_revisions`
//!   with it);
//! - `case_articles.knowledge_id` and `kcs_translations.knowledge_id` →
//!   declared NO ACTION, NOT cleared here (pre-existing ceiling, shared
//!   with the purge core's map, documented honestly): deleting a domain
//!   whose chunks carry a case article or a knowledge translation violates
//!   the FK and fails the whole tx LOUDLY (fail-closed, erasure-safe);
//! - `domain_centroids` → domain-keyed: exactly this domain's row (shim) or
//!   the table wholesale (multi-db);
//! - multi-db wholesale-only (the file IS the domain): `sources`,
//!   `source_revisions`, `connector_checkpoints`, `webhook_seen`,
//!   `webhook_queue`;
//! - `audit_events` is deliberately NEVER deleted in either mode: the
//!   immutable chain must survive a domain delete (shim mode has no domain
//!   column to scope by; multi-db mode exports the segment first, then
//!   preserves the file in place).
//!
//! Path safety ceiling: `export_snapshot` writes through the SHARED
//! escaper [`brain_server::backup::vacuum_into`] — never a hand-rolled literal
//! (a `'` in TMPDIR would break out). The quote-escaping and
//! symlink-containment pins stay attached to that primitive in
//! `src/backup.rs` verbatim;
//! `domain_export_routes_through_shared_vacuum_escaper` (below) pins that
//! this module keeps calling it.
//!
//! Transaction shape: [`delete_domain_data`] runs entirely inside the
//! caller's tx (preflight → segment export → sweeps → the `domain_deleted`
//! audit row, SAVEPOINT-nested by the audit writer — the transition and its
//! evidence commit or roll back together). [`relabel_chunks`] is moved
//! VERBATIM and owns its own single tx (the whole relabel is its atomicity
//! unit; it owes no audit row) — a move is not a rewrite.
//!
//! pool authority: the registry stays at the handler — every fn takes
//! `&Connection`/`&Transaction` (or `&mut Connection` where the fn owns its
//! tx); `register_services_receive_no_registry` (service/mod.rs) pins that.
//! The file enumeration/open loop for the multi-db census likewise stays at
//! the handler: filesystem orchestration, not storage logic.

use rusqlite::Connection;
use std::collections::HashMap;

/// One census row: the plain (non-wire) form the handler maps onto its
/// `DomainInfo` JSON 1:1.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DomainRow {
    pub name: String,
    pub entries: i64,
    pub entities: i64,
    pub relations: i64,
    pub multi_db: bool,
}

/// Typed service error (the ServiceError convention: one enum per module).
/// `Database` carries the verbatim pre-move message (statement-specific
/// prefixes included — the handler renders the internal-error body with the
/// string byte-for-byte); `LegalHold` carries the active-hold map for the
/// shared `409 legal_hold_active` envelope; the relabel variants carry the
/// exact pre-move 400 vocabulary.
#[derive(Debug)]
pub(crate) enum DomainAdminError {
    /// A query/file operation failed; the pre-move message travels intact.
    Database(String),
    /// The hold preflight fired: held knowledge id → its reasons. Nothing is
    /// erased while any chunk in the domain is frozen.
    LegalHold(HashMap<i64, Vec<String>>),
    /// Some relabel ids do not exist (provenance safety).
    MissingIds { missing: usize, total: usize },
    /// Draining rows OUT of the `global` fallback bucket requires the
    /// explicit `?confirm=global` echo.
    ConfirmRequired,
}

impl std::fmt::Display for DomainAdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainAdminError::Database(e) => write!(f, "{e}"),
            DomainAdminError::LegalHold(held) => write!(f, "legal hold active on {held:?}"),
            DomainAdminError::MissingIds { missing, total } => {
                write!(f, "{missing}/{total} ids do not exist")
            }
            DomainAdminError::ConfirmRequired => {
                write!(f, "moving rows out of 'global' requires ?confirm=global")
            }
        }
    }
}

impl From<rusqlite::Error> for DomainAdminError {
    fn from(e: rusqlite::Error) -> Self {
        DomainAdminError::Database(e.to_string())
    }
}

/// The shim-mode census: always `global` first (it covers its own rows plus
/// any NULL-domain legacy rows), then every distinct non-NULL label, sorted.
/// Totals are server-wide (per-domain entity/relation counts need a JOIN
/// through `relationships.knowledge_id`; labeled as such via
/// `multi_db=false`). Count failures report 0 — the pre-move
/// `unwrap_or(0)` posture, kept verbatim (a degraded census must not
/// 500 the listing).
pub(crate) fn shim_domain_rows(conn: &Connection) -> Result<Vec<DomainRow>, DomainAdminError> {
    let mut out = Vec::new();
    let mut names: Vec<String> = vec!["global".to_string()];
    let mut stmt =
        conn.prepare("SELECT DISTINCT domain FROM knowledge WHERE domain IS NOT NULL")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for r in rows.flatten() {
        if r != "global" {
            names.push(r);
        }
    }
    names.sort();
    names.dedup();
    let total_entities: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
        .unwrap_or(0);
    let total_relations: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
        .unwrap_or(0);
    for name in &names {
        let entries: i64 = conn
            .query_row(
                // `global` covers its own rows + any NULL-domain legacy rows.
                "SELECT COUNT(*) FROM knowledge
                 WHERE domain = ?1 OR (?1 = 'global' AND domain IS NULL)",
                rusqlite::params![name],
                |r| r.get(0),
            )
            .unwrap_or(0);
        out.push(DomainRow {
            name: name.clone(),
            entries,
            entities: total_entities,
            relations: total_relations,
            multi_db: false,
        });
    }
    Ok(out)
}

/// The multi-db per-file census: each connection IS one domain's file.
/// Count failures report 0 (the pre-move posture, verbatim).
pub(crate) fn file_domain_counts(conn: &Connection) -> (i64, i64, i64) {
    let entries: i64 = conn
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
        .unwrap_or(0);
    let entities: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
        .unwrap_or(0);
    let relations: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
        .unwrap_or(0);
    (entries, entities, relations)
}

/// The create/warm probe: a store with zero knowledge rows is NEW (201) —
/// one that already carries data was warmed (200). Failure reads as empty
/// (the pre-move `unwrap_or(0)` posture, verbatim).
pub(crate) fn is_empty_store(conn: &Connection) -> bool {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
        .unwrap_or(0);
    count == 0
}

/// Stream a domain's audit segment to `<root>/archives/<domain>-audit-<epoch>.ndjson`
/// (0600) before its rows are erased, so the deletion registry survives as an
/// operator-reviewable artifact. The path is derived from the data root the
/// handler threads in (`state.db_path`'s parent — the same root
/// `StorageLayout::detect()` resolves in production, without the env
/// dependence that would race tests). Only meaningful in multi-db mode (the
/// whole file is the domain's); shim-mode audit is global.
fn export_audit_segment(
    tx: &rusqlite::Transaction<'_>,
    domain: &str,
    root: &std::path::Path,
) -> Result<std::path::PathBuf, anyhow::Error> {
    let archives = root.join("archives");
    std::fs::create_dir_all(&archives)?;
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = archives.join(format!("{domain}-audit-{epoch}.ndjson"));
    let mut stmt = tx.prepare(
        "SELECT id, ts, kind, actor, target_hash, status, detail_hash, tenant_id, prev_hash
           FROM audit_events ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        // NULLable columns read as Option — the chain's FIRST row has a NULL
        // prev_hash (and legacy rows NULL actors); a bare String read would
        // drop it at the flatten boundary. Nulls serialize as JSON null.
        Ok(serde_json::json!({
            "id": r.get::<_, i64>(0)?,
            "ts": r.get::<_, String>(1)?,
            "kind": r.get::<_, String>(2)?,
            "actor": r.get::<_, Option<String>>(3)?,
            "target_hash": r.get::<_, Option<String>>(4)?,
            "status": r.get::<_, Option<String>>(5)?,
            "detail_hash": r.get::<_, Option<String>>(6)?,
            "tenant_id": r.get::<_, Option<String>>(7)?,
            "prev_hash": r.get::<_, Option<String>>(8)?,
        }))
    })?;
    use std::io::Write;
    let mut out = std::fs::File::create(&path)?;
    // 0600 explicitly — `File::create` honors the process umask (this is a
    // deletion-judgment artifact an operator reviews; same posture as the
    // auth-token rotate path).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        out.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    for row in rows.flatten() {
        writeln!(out, "{row}")?;
    }
    // The deletion registry is evidence too — tombstones
    // (SHA-256 content digests of purged chunks) + evidence_links were
    // previously destroyed by the delete the segment was supposed to
    // memorialize. Append them to the same operator artifact.
    let mut ts = tx.prepare(
        "SELECT knowledge_id, document_id, content_hash, reason, origin_id, deleted_at, purged_at \
           FROM tombstones ORDER BY knowledge_id",
    )?;
    let ts_rows = ts.query_map([], |r| {
        Ok(serde_json::json!({
            "knowledge_id": r.get::<_, i64>(0)?,
            "document_id": r.get::<_, Option<String>>(1)?,
            "content_hash": r.get::<_, Option<String>>(2)?,
            "reason": r.get::<_, Option<String>>(3)?,
            "origin_id": r.get::<_, Option<i64>>(4)?,
            "deleted_at": r.get::<_, Option<String>>(5)?,
            "purged_at": r.get::<_, Option<i64>>(6)?,
        }))
    })?;
    for row in ts_rows.flatten() {
        writeln!(out, "{{\"segment\":\"tombstones\",\"row\":{row}}}")?;
    }
    let mut el = tx.prepare(
        "SELECT from_chunk, to_chunk, kind, created_at FROM evidence_links ORDER BY from_chunk, to_chunk",
    )?;
    let el_rows = el.query_map([], |r| {
        Ok(serde_json::json!({
            "from_chunk": r.get::<_, i64>(0)?,
            "to_chunk": r.get::<_, i64>(1)?,
            "kind": r.get::<_, Option<String>>(2)?,
            "created_at": r.get::<_, Option<String>>(3)?,
        }))
    })?;
    for row in el_rows.flatten() {
        writeln!(out, "{{\"segment\":\"evidence_links\",\"row\":{row}}}")?;
    }
    Ok(path)
}

/// The domain erasure, inside the caller's tx: the hold preflight (a domain
/// holding any actively-held chunk refuses deletion entirely — all-or-
/// nothing), the multi-db audit-segment export, then the erasure plan for
/// the mode (see the module-header FK-children map), then the
/// `domain_deleted` audit row INSIDE the same tx (the audit writer
/// SAVEPOINT-nests, so the transition and its evidence commit or roll back
/// together — the pre-move code recorded after the commit, leaving a crash
/// window the audit-per-write law closes; the writer's own fail-safe
/// posture — drop + alert, never forge — is unchanged).
pub(crate) fn delete_domain_data(
    tx: &rusqlite::Transaction<'_>,
    name: &str,
    multi_db: bool,
    root: Option<&std::path::Path>,
) -> Result<(), DomainAdminError> {
    // a domain holding any actively-held chunk
    // refuses deletion entirely (all-or-nothing). The operator must release
    // every hold or scope the delete before the domain can go.
    {
        let ids: Vec<i64> = if multi_db {
            let mut stmt = tx.prepare("SELECT id FROM knowledge")?;
            let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        } else {
            let mut stmt = tx.prepare("SELECT id FROM knowledge WHERE domain = ?1")?;
            let rows = stmt.query_map(rusqlite::params![name], |r| r.get::<_, i64>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        let held = crate::legal_hold::active_reasons(tx, &ids)?;
        if !held.is_empty() {
            return Err(DomainAdminError::LegalHold(held));
        }
    }
    if multi_db {
        // export the domain's audit segment
        // before erasure so the chain survives as an operator-reviewable
        // artifact, then preserve it in the live file too (never unlink).
        export_audit_segment(tx, name, root.unwrap_or_else(|| std::path::Path::new(".")))
            .map_err(|e| DomainAdminError::Database(format!("archive domain audit: {e}")))?;
        // Multi-db: the pool IS this domain's own DB. Every table in it is
        // scoped to this domain — clear them all. Order respects FKs:
        // evidence_links → relationships → knowledge (FK target) → rest.
        // `audit_events` is deliberately NOT deleted: the immutable chain
        // must survive a domain delete.
        tx.execute_batch(
            "DELETE FROM evidence_links;
             DELETE FROM relationships;
             DELETE FROM knowledge;
             DELETE FROM entities;
             DELETE FROM tombstones;
             DELETE FROM sources;
             DELETE FROM source_revisions;
             DELETE FROM connector_checkpoints;
             DELETE FROM webhook_seen;
             DELETE FROM webhook_queue;
             DELETE FROM domain_centroids;
             DELETE FROM knowledge_fts;
             DELETE FROM vec_knowledge;",
        )
        .map_err(|e| DomainAdminError::Database(format!("delete domain data failed: {e}")))?;
    } else {
        // Shim mode: the pool is the GLOBAL shared DB. Delete ONLY this
        // domain's rows. `audit_events` has no domain column → leave it
        // untouched (the immutable audit log MUST survive a domain delete).
        // `domain_centroids` is keyed by domain → delete just this one.
        tx.execute(
            "DELETE FROM evidence_links WHERE from_chunk IN
             (SELECT id FROM knowledge WHERE domain = ?1)
             OR to_chunk IN
             (SELECT id FROM knowledge WHERE domain = ?1)",
            rusqlite::params![name],
        )
        .map_err(|e| DomainAdminError::Database(format!("delete evidence_links failed: {e}")))?;
        tx.execute(
            "DELETE FROM relationships WHERE knowledge_id IN
             (SELECT id FROM knowledge WHERE domain = ?1)",
            rusqlite::params![name],
        )
        .map_err(|e| DomainAdminError::Database(format!("delete relationships failed: {e}")))?;
        // Entities: only delete entities no longer referenced by any
        // relationship in any domain (an entity may be shared across domains).
        tx.execute(
            "DELETE FROM entities WHERE id NOT IN
             (SELECT from_entity_id FROM relationships)
             AND id NOT IN
             (SELECT to_entity_id FROM relationships)",
            [],
        )
        .map_err(|e| DomainAdminError::Database(format!("delete orphan entities failed: {e}")))?;
        // Tombstones + sources tied to this domain's chunks.
        tx.execute(
            "DELETE FROM tombstones WHERE knowledge_id IN
             (SELECT id FROM knowledge WHERE domain = ?1)",
            rusqlite::params![name],
        )
        .map_err(|e| DomainAdminError::Database(format!("delete tombstones failed: {e}")))?;
        // Knowledge rows (FK source for relationships — already cleared).
        tx.execute(
            "DELETE FROM vec_knowledge WHERE knowledge_id IN
             (SELECT id FROM knowledge WHERE domain = ?1)",
            rusqlite::params![name],
        )
        .map_err(|e| DomainAdminError::Database(format!("delete vec_knowledge failed: {e}")))?;
        tx.execute(
            "DELETE FROM knowledge WHERE domain = ?1",
            rusqlite::params![name],
        )
        .map_err(|e| DomainAdminError::Database(format!("delete knowledge failed: {e}")))?;
        // FTS5 shadow rows for the deleted knowledge ids are cleaned by the
        // FTS5 trigger on knowledge DELETE, so no explicit statement there.
        // Domain centroids: drop just this domain's centroid.
        tx.execute(
            "DELETE FROM domain_centroids WHERE domain = ?1",
            rusqlite::params![name],
        )
        .map_err(|e| DomainAdminError::Database(format!("delete centroid failed: {e}")))?;
    }
    // record the deletion on the surviving chain (the global
    // chain in shim mode; the domain's own preserved chain in multi-db) —
    // INSIDE the caller's tx, so the evidence is atomic with the erasure.
    crate::audit::record(
        tx,
        crate::audit::AuditKind::Reconcile,
        "operator",
        name,
        crate::audit::AuditStatus::Ok,
        "domain_deleted",
    );
    Ok(())
}

/// Reclaim free pages in the domain's DB. Cheap; safe while the server is up.
pub(crate) fn vacuum(conn: &Connection) -> Result<(), DomainAdminError> {
    conn.execute_batch("VACUUM;")
        .map_err(|e| DomainAdminError::Database(format!("vacuum failed: {e}")))?;
    Ok(())
}

/// The export snapshot: a consistent, defragmented, WAL-free copy of the
/// domain's DB via SQLite's `VACUUM INTO`, read back as bytes. Avoids
/// reading the live file directly (WAL pages would be missed; concurrent
/// writes could corrupt the read). The path goes through the SHARED
/// quote-escaping primitive [`brain_server::backup::vacuum_into`] — never a
/// hand-rolled literal (pinned below); the temp file is best-effort removed
/// after the read (maintenance scratch, not evidence).
pub(crate) fn export_snapshot(
    conn: &Connection,
    name: &str,
) -> Result<(Vec<u8>, String), DomainAdminError> {
    let temp = std::env::temp_dir().join(format!(
        "brain-export-{}-{}.db",
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    // VACUUM INTO writes a consistent snapshot to `temp` without holding
    // a write lock on the source. Safe under concurrent writes.
    brain_server::backup::vacuum_into(conn, &temp)
        .map_err(|e| DomainAdminError::Database(format!("VACUUM INTO failed: {e}")))?;
    let bytes = std::fs::read(&temp)
        .map_err(|e| DomainAdminError::Database(format!("read export: {e}")))?;
    let _ = std::fs::remove_file(&temp);
    Ok((bytes, format!("brain-{name}.db")))
}

/// The relabel transaction core of `/domains/move`, moved verbatim.
/// Validates every id exists, derives the source domains, enforces the
/// `?confirm=global` guard when draining the fallback bucket, then relabels in
/// ONE transaction (only rows currently in a different domain; provenance
/// fields `source`/`authority`/`observed_at` are untouched). Returns the
/// number actually moved + the distinct source domains.
pub(crate) fn relabel_chunks(
    conn: &mut Connection,
    ids: &[i64],
    to: &str,
    confirm: &str,
) -> Result<(usize, Vec<String>), DomainAdminError> {
    let ph = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");

    // Validate every id exists before touching anything (provenance safety).
    let existing: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM knowledge WHERE id IN ({ph})"),
            rusqlite::params_from_iter(ids.iter()),
            |r| r.get(0),
        )
        .map_err(|e| DomainAdminError::Database(format!("id check failed: {e}")))?;
    if existing as usize != ids.len() {
        return Err(DomainAdminError::MissingIds {
            missing: ids.len() - existing as usize,
            total: ids.len(),
        });
    }

    // Source domains involved; draining `global` needs ?confirm=global.
    let mut from_domains: Vec<String> = Vec::new();
    {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT DISTINCT domain FROM knowledge WHERE id IN ({ph})"
            ))
            .map_err(DomainAdminError::from)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
                r.get::<_, String>(0)
            })
            .map_err(DomainAdminError::from)?;
        for r in rows.flatten() {
            from_domains.push(r);
        }
    }
    if from_domains.iter().any(|d| d == "global") && confirm != "global" {
        return Err(DomainAdminError::ConfirmRequired);
    }

    // One tx: relabel only rows currently in a different domain.
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(ids.len() + 1);
    params_vec.push(Box::new(to.to_string()));
    for id in ids {
        params_vec.push(Box::new(*id));
    }
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let tx = conn
        .transaction()
        .map_err(|e| DomainAdminError::Database(format!("tx begin failed: {e}")))?;
    let changed = tx
        .execute(
            &format!("UPDATE knowledge SET domain = ?1 WHERE id IN ({ph}) AND domain != ?1"),
            param_refs.as_slice(),
        )
        .map_err(|e| DomainAdminError::Database(format!("relabel failed: {e}")))?;
    tx.commit()
        .map_err(|e| DomainAdminError::Database(format!("tx commit failed: {e}")))?;
    Ok((changed, from_domains))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert::ChainWatchState;
    use crate::auth::jwks::KeyStore;
    use crate::domain_registry::DomainRegistry;
    use crate::handlers::domains::delete_domain;
    use crate::integrity::SnapshotState;
    use crate::{AppState, ConnectionTracker, RateLimiter};
    use axum::extract::{Path, Query, State};
    use axum::http::StatusCode;
    use rusqlite::params;
    use std::sync::Arc;

    /// domains' centroids. This is the critical-correctness bug caught in the
    /// second-pass review: an earlier draft did `DELETE FROM audit_events`
    /// (no WHERE clause) which would have wiped the immutable audit trail when
    /// any single domain was deleted.
    #[test]
    fn delete_domain_shim_mode_sql_preserves_global_tables() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, crate::config::DB_MMAP_SIZE_MIB)
            .expect("migration");

        // Seed two domains + global audit + a global centroid.
        for (domain, n) in [("health", 1), ("business", 2)] {
            for i in 0..n {
                conn.execute(
                    "INSERT INTO knowledge (title, content, source, content_hash, domain)
                     VALUES (?1, ?2, 'structured', ?3, ?4)",
                    params![
                        format!("{domain}{i}"),
                        format!("content {i}"),
                        format!("h{domain}{i}"),
                        domain
                    ],
                )
                .unwrap();
            }
        }
        // An unrelated audit row (must survive a domain delete).
        conn.execute(
            "INSERT INTO audit_events (kind, actor, target_hash, status)
             VALUES ('auth', 'tester', 'abcdef', 'allowed')",
            [],
        )
        .unwrap();
        // Two domain centroids (only the deleted domain's should go).
        conn.execute(
            "INSERT INTO domain_centroids (domain, centroid, count) VALUES ('health', X'AABB', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO domain_centroids (domain, centroid, count) VALUES ('business', X'CCDD', 2)",
            [],
        )
        .unwrap();

        // Execute the REAL shim-mode erasure core for `health` inside a tx
        // (pre-move this pin replayed the SQL by hand; the core is now
        // directly callable, so the pin drives the code itself — the
        // expected audit count grows by exactly the one `domain_deleted`
        // evidence row the in-tx audit owes).
        let tx = conn.transaction().unwrap();
        delete_domain_data(&tx, "health", false, None).unwrap();
        tx.commit().unwrap();

        // Audit log is IMMUTABLE — the pre-existing row survives, plus the
        // one `domain_deleted` evidence row.
        let audit_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            audit_count, 2,
            "global audit_events must survive a domain delete (+1 evidence row)"
        );

        // Other domains' centroids must survive.
        let biz_centroids: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM domain_centroids WHERE domain = 'business'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(biz_centroids, 1, "other domains' centroids must survive");

        // Deleted domain's rows gone; other domains' rows intact.
        let health_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE domain = 'health'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let business_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE domain = 'business'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(health_rows, 0, "deleted domain's rows are gone");
        assert_eq!(business_rows, 2, "other domains' rows are intact");
    }

    /// the audit-per-write twin: the erasure and its `domain_deleted`
    /// evidence row commit — or roll back — TOGETHER.
    #[test]
    fn domain_delete_rolls_back_with_its_audit() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, crate::config::DB_MMAP_SIZE_MIB)
            .expect("migration");
        conn.execute(
            "INSERT INTO knowledge (title, content, source, content_hash, domain)
             VALUES ('h0', 'c', 'structured', 'hh0', 'health')",
            [],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        delete_domain_data(&tx, "health", false, None).unwrap();
        tx.rollback().unwrap();

        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE domain = 'health'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "a rolled-back erasure leaves the rows in place");
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 0, "the evidence row rolled back WITH the erasure");
    }

    /// the relabel core moves only the requested ids into the target
    /// domain, reports the distinct source domains, and leaves provenance
    /// fields (`source`/`authority`/`observed_at`) untouched. Draining rows OUT
    /// of the fallback bucket requires `?confirm=global`.
    #[test]
    fn relabel_chunks_moves_rows_and_preserves_provenance() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, crate::config::DB_MMAP_SIZE_MIB)
            .expect("migration");

        let mut global_ids = Vec::new();
        for i in 0..2 {
            conn.execute(
                "INSERT INTO knowledge (title, content, source, content_hash, domain)
                 VALUES (?1, ?2, 'structured', ?3, 'global')",
                params![
                    format!("g{i}"),
                    format!("global content {i}"),
                    format!("hg{i}")
                ],
            )
            .unwrap();
            global_ids.push(conn.last_insert_rowid());
        }
        conn.execute(
            "INSERT INTO knowledge (title, content, source, content_hash, domain)
             VALUES ('b0', 'biz', 'structured', 'hb0', 'business')",
            [],
        )
        .unwrap();

        // No confirm -> draining global is refused.
        let err = relabel_chunks(&mut conn, &global_ids, "business", "").unwrap_err();
        assert!(
            matches!(err, DomainAdminError::ConfirmRequired),
            "got: {err:?}"
        );

        // With confirm -> both rows move; business rows untouched.
        let (moved, from) = relabel_chunks(&mut conn, &global_ids, "business", "global").unwrap();
        assert_eq!(moved, 2);
        assert_eq!(from, vec!["global".to_string()]);

        let biz_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE domain = 'business' AND id IN (?, ?)",
                params![global_ids[0], global_ids[1]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(biz_rows, 2, "relabeled rows now live in the target domain");
        // Provenance preserved.
        let src: String = conn
            .query_row(
                "SELECT source FROM knowledge WHERE id = ?1",
                params![global_ids[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(src, "structured", "provenance field untouched by relabel");
    }

    #[test]
    fn relabel_chunks_rejects_missing_ids() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, crate::config::DB_MMAP_SIZE_MIB)
            .expect("migration");
        let err = relabel_chunks(&mut conn, &[999_999], "business", "global").unwrap_err();
        match err {
            DomainAdminError::MissingIds { missing, total } => {
                assert_eq!((missing, total), (1, 1), "1/1 ids do not exist");
            }
            other => panic!("expected MissingIds, got: {other:?}"),
        }
    }

    // ── the route-level domain-delete pins, repointed verbatim from the
    // pre-move handlers/clients.rs test home (they exercise the erasure
    // through the handler fn, which now delegates to this core).

    fn app_state(dir: &tempfile::TempDir) -> Arc<AppState> {
        app_state_with(dir, true, 4)
    }

    static TEST_EMBEDDER: std::sync::OnceLock<Arc<dyn brain_server::embed::Embedder>> =
        std::sync::OnceLock::new();

    fn app_state_with(dir: &tempfile::TempDir, multi_db: bool, max_size: u32) -> Arc<AppState> {
        brain_server::register_sqlite_vec::register_sqlite_vec();
        let path = dir.path().join("brain.db");
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(&path);
        let pool: crate::Pool = r2d2::Pool::builder()
            .max_size(max_size)
            .build(mgr)
            .expect("pool");
        brain_server::migration::run_migration(
            &mut pool.get().unwrap(),
            crate::config::DB_MMAP_SIZE_MIB,
        )
        .expect("migration");
        let model: Arc<dyn brain_server::embed::Embedder> = TEST_EMBEDDER
            .get_or_init(|| {
                Arc::new(
                    brain_server::embed::StaticEmbedder::new(crate::config::MODEL_ID)
                        .expect("model"),
                )
            })
            .clone();
        Arc::new(AppState {
            token_store: crate::auth::TokenStore::new(),
            jwt_middleware_state: std::sync::Arc::new(
                crate::server::router::auth::JwtMiddlewareState::opaque_for_tests(
                    pool.clone(),
                    path.clone(),
                ),
            ),
            cors: tower_http::cors::CorsLayer::new(),
            model,
            registry: DomainRegistry::new(pool.clone(), &path, multi_db),
            pool,
            db_path: path.clone(),
            connection_tracker: Arc::new(ConnectionTracker::new()),
            rate_limiter: Arc::new(RateLimiter::new()),
            snapshot: SnapshotState::default(),
            audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
            auth_mode: crate::auth::AuthMode::Opaque,
            key_store: KeyStore::load(std::path::Path::new("/nonexistent"))
                .expect("empty key store"),
            revocation_cache: Arc::new(crate::auth::revocation::RevocationCache::new()),
            jwt_issuer: String::new(),
            jwt_audience: String::new(),
            oidc_config: crate::handlers::well_known::OidcConfig::unconfigured(),
            ump_events: tokio::sync::broadcast::channel(crate::config::UMP_EVENT_BUFFER).0,
            alert_events: tokio::sync::broadcast::channel(crate::config::ALERT_EVENT_BUFFER).0,
            alert_seq: std::sync::atomic::AtomicU64::new(0),
            chain_watch: ChainWatchState::default(),
        })
    }

    fn seed_rows(state: &AppState, domain: &str, n: i64) -> Vec<i64> {
        let pool = state.registry.register(domain).unwrap();
        let conn = pool.get().unwrap();
        let mut ids = Vec::new();
        for i in 0..n {
            conn.execute(
                "INSERT INTO knowledge(content, content_hash, owner) VALUES (?1, ?2, 'o')",
                rusqlite::params![format!("data-{i}"), format!("h{i}")],
            )
            .expect("seed row");
            ids.push(conn.last_insert_rowid());
        }
        ids
    }

    fn count_knowledge(state: &AppState, domain: &str) -> i64 {
        state
            .registry
            .register(domain)
            .unwrap()
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap_or(0)
    }

    fn hold(domain_pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>, ids: &[i64]) {
        let mut conn = domain_pool.get().unwrap();
        let tx = conn.transaction().unwrap();
        crate::legal_hold::insert_holds(&tx, ids, "litigation 2026-118", Some("dpo"), 60).unwrap();
        tx.commit().unwrap();
    }

    async fn delete_domain_ok(state: &Arc<AppState>, name: &str) {
        match delete_domain(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path(name.to_string()),
            Query(crate::handlers::domains::DeleteDomainQuery {
                confirm: Some(name.to_string()),
            }),
        )
        .await
        {
            Ok(_) => {}
            Err(e) => panic!("domain delete failed: {} {}", e.inner.code, e.inner.message),
        }
    }

    #[tokio::test]
    async fn domain_delete_refuses_while_holds_active() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        let ids = seed_rows(&state, "acme", 2);
        hold(&state.registry.register("acme").unwrap(), &[ids[0]]);

        let err = match delete_domain(
            State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            Path("acme".to_string()),
            Query(crate::handlers::domains::DeleteDomainQuery {
                confirm: Some("acme".to_string()),
            }),
        )
        .await
        {
            Ok(_) => panic!("a domain with a held chunk must refuse deletion"),
            Err(e) => e,
        };
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.inner.code, "legal_hold_active");
        assert_eq!(
            count_knowledge(&state, "acme"),
            2,
            "nothing erased while the hold is active"
        );

        // The domain file also survives (multi-db: no unlink, no rename).
        let archived: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("acme"))
            .collect();
        assert!(
            archived
                .iter()
                .any(|e| e.file_name().to_string_lossy().ends_with(".db")),
            "the domain DB file is still in place"
        );
    }

    // ── the audit chain survives a domain
    // delete. Multi-db mode exports the domain's audit segment to
    // `archives/<domain>-audit-<epoch>.ndjson` (0600), keeps the file in
    // place with its chain intact, and records a `domain_deleted` event.
    // (Deviation from the plan's `.db.archived-<epoch>` rename: the live DB
    // file is retained at its path — the pool holds open connections to it,
    // and a rename would resurrect an empty domain on the next `pool_for`.
    // In-place preservation serves the same intent — the chain stays
    // verifiable — and is strictly more recoverable.)

    #[tokio::test]
    async fn domain_delete_archives_audit_segment() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        seed_rows(&state, "acme", 2);
        {
            let conn = state.registry.register("acme").unwrap().get().unwrap();
            crate::audit::record(
                &conn,
                crate::audit::AuditKind::Ingest,
                "operator",
                "seed-1",
                crate::audit::AuditStatus::Ok,
                "pre-delete evidence",
            );
            crate::audit::record(
                &conn,
                crate::audit::AuditKind::Recall,
                "operator",
                "seed-2",
                crate::audit::AuditStatus::Ok,
                "pre-delete evidence 2",
            );
        }

        delete_domain_ok(&state, "acme").await;

        let archive = std::fs::read_dir(dir.path().join("archives"))
            .expect("archives dir exists")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("acme-audit-"))
                    .unwrap_or(false)
            })
            .expect("the audit segment was exported");
        let raw = std::fs::read_to_string(&archive).unwrap();
        let mut segments = raw.lines();
        let first = segments.next().expect("segment is non-empty");
        assert!(
            first.contains("\"prev_hash\":null"),
            "the chain head (NULL prev_hash) must serialize, not drop: {first}"
        );
        assert!(
            segments.next().is_some(),
            "both pre-delete chain rows stream (ORDER BY id)"
        );
        assert!(
            raw.contains(&crate::audit::hash("pre-delete evidence").to_string()),
            "the segment carries the pre-delete chain rows"
        );
        assert!(
            raw.contains(&crate::audit::hash("pre-delete evidence 2").to_string()),
            "both pre-delete rows stream (ORDER BY id)"
        );
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&archive).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the archive is 0600");
        // The export ran BEFORE erasure — the deletion event is not in it.
        assert!(
            !raw.contains(&crate::audit::hash("domain_deleted").to_string()),
            "segment is the pre-deletion snapshot"
        );
        assert_eq!(count_knowledge(&state, "acme"), 0, "domain rows erased");
        assert!(
            dir.path().join("brain-acme.db").exists(),
            "file retained (no unlink, no rename)"
        );
    }

    #[tokio::test]
    async fn domain_delete_writes_domain_deleted_event() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        seed_rows(&state, "acme", 1);

        delete_domain_ok(&state, "acme").await;

        let (count, target): (i64, String) = state
            .registry
            .register("acme")
            .unwrap()
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*), target_hash FROM audit_events
                 WHERE detail_hash = ?1 AND kind = 'reconcile'",
                [crate::audit::hash("domain_deleted")],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "exactly one domain_deleted event on the chain");
        assert_eq!(
            target,
            crate::audit::hash("acme").to_string(),
            "the event names the deleted domain (hash-only)"
        );
    }

    #[tokio::test]
    async fn audit_chain_survives_domain_delete() {
        let dir = tempfile::tempdir().unwrap();
        let state = app_state(&dir);
        seed_rows(&state, "acme", 1);
        {
            let conn = state.registry.register("acme").unwrap().get().unwrap();
            for i in 0..3 {
                crate::audit::record(
                    &conn,
                    crate::audit::AuditKind::Recall,
                    "operator",
                    &format!("pre-{i}"),
                    crate::audit::AuditStatus::Ok,
                    "evidence",
                );
            }
        }

        delete_domain_ok(&state, "acme").await;

        let conn = state.registry.register("acme").unwrap().get().unwrap();
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            total, 4,
            "3 pre-delete + 1 domain_deleted on the surviving chain"
        );
        assert!(
            crate::audit::verify_chain(&conn),
            "the preserved in-file chain still verifies"
        );
    }

    /// The export path routes through the shared `VACUUM INTO` escaper —
    /// never a hand-rolled literal (source inspection; the escaping +
    /// symlink-containment pins themselves stay attached to
    /// `backup::vacuum_into` verbatim in src/backup.rs).
    #[test]
    fn domain_export_routes_through_shared_vacuum_escaper() {
        let text = include_str!("../service/domains_admin.rs");
        let prod = text.split("#[cfg(test)]").next().unwrap();
        assert!(
            prod.contains("backup::vacuum_into"),
            "the export must call the shared escaper, not roll its own SQL literal"
        );
        assert!(
            !prod.contains("VACUUM INTO '") && !prod.contains("VACUUM INTO \""),
            "a hand-rolled quoted VACUUM INTO literal would break out on a quoted path"
        );
    }
}
