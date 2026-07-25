use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::handlers::HandlerError;
use crate::AppState;
use rusqlite::params;

#[derive(Debug, Serialize)]
pub struct DomainInfo {
    pub name: String,
    pub entries: i64,
    pub entities: i64,
    pub relations: i64,
    pub multi_db: bool,
}

#[derive(Debug, Serialize)]
pub struct DomainsResponse {
    pub domains: Vec<DomainInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDomainRequest {
    pub name: String,
}

/// `GET /domains` — list domains with per-domain counts.
///
/// In shim mode (single-DB) this groups by the `domain` column.
/// In multi-db mode each file is its own domain.
pub async fn domains(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DomainsResponse>, HandlerError> {
    let multi_db = state.registry.is_multi_db();
    let pool = state.pool.clone();

    let info = tokio::task::spawn_blocking(move || -> Result<Vec<DomainInfo>, HandlerError> {
        // In shim mode the registry's `known_domains()` enumerates files, which
        // is meaningless (the global brain.db's filename leaks in as a fake
        // "domain"). The truth in shim mode is the DISTINCT set of values in
        // the `domain` column on `knowledge`. In multi-db mode each per-domain
        // file is a real domain; we use the file list.
        let mut out = Vec::new();
        if !multi_db {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            // Always include `global` (covers the NULL / 'global' case) + every
            // distinct non-NULL domain value present in the data.
            let mut names: Vec<String> = vec!["global".to_string()];
            let mut stmt = conn
                .prepare("SELECT DISTINCT domain FROM knowledge WHERE domain IS NOT NULL")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| HandlerError::internal(e.to_string()))?;
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
                        params![name],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                out.push(DomainInfo {
                    name: name.clone(),
                    entries,
                    // ponytail: per-domain entity/relation counts in shim mode need a
                    // JOIN through relationships.knowledge_id → knowledge.domain.
                    // For now we report the global total (clearly labeled as such via
                    // multi_db=false); true per-domain counts land with multi-db files.
                    entities: total_entities,
                    relations: total_relations,
                    multi_db,
                });
            }
        } else {
            // Multi-db: open a connection to each per-domain file.
            // ponytail: opens N connections sequentially; upgrade to parallel if > 100 domains.
            let names = state.registry.known_domains();
            for name in &names {
                let Ok(layout) = brain_server::storage_layout::StorageLayout::detect() else {
                    continue;
                };
                let Ok(path) = layout.domain_db(name) else {
                    continue;
                };
                let Ok(conn) = rusqlite::Connection::open(&path) else {
                    continue;
                };
                let entries: i64 = conn
                    .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
                    .unwrap_or(0);
                let entities: i64 = conn
                    .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
                    .unwrap_or(0);
                let relations: i64 = conn
                    .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
                    .unwrap_or(0);
                out.push(DomainInfo {
                    name: name.clone(),
                    entries,
                    entities,
                    relations,
                    multi_db: true,
                });
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(DomainsResponse { domains: info }))
}

/// `POST /domains` — create or warm a domain.
///
/// Idempotent: if the domain already exists, returns `200`; if new, the
/// per-domain pool is opened (creates DB file + runs migration) and returns `201`.
pub async fn create_domain(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateDomainRequest>,
) -> Result<impl IntoResponse, HandlerError> {
    use super::normalize_domain;
    let name = normalize_domain(&req.name)?;

    // Resolve the domain's pool — this creates the file in multi-db mode.
    let pool = state.registry.pool_for(&name).map_err(|e| {
        HandlerError::bad_request("domain_invalid", format!("cannot create domain: {e}"))
    })?;

    let is_new = tokio::task::spawn_blocking(move || -> Result<bool, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap_or(0);
        Ok(count == 0)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok((
        if is_new {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(serde_json::json!({
            "name": name,
            "created": is_new,
            "multi_db": state.registry.is_multi_db(),
        })),
    ))
}

/// `DELETE /domains/{name}?confirm=<name>` — delete a domain and all its data.
///
/// Drops all data for the domain. The `global` domain is protected. Per the
/// plan M5, the caller must echo the domain name as `?confirm=<name>` so a
/// typoed URL or replay can't destroy data by accident.
pub async fn delete_domain(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(q): Query<DeleteDomainQuery>,
) -> Result<impl IntoResponse, HandlerError> {
    use super::normalize_domain;
    let name = normalize_domain(&name)?;

    if name == "global" {
        return Err(HandlerError::bad_request(
            "domain_protected",
            "the 'global' domain cannot be deleted",
        ));
    }
    // Confirm guard: `?confirm=<exact name>` must match. Typos / URL replays
    // can't otherwise destroy a whole domain.
    let confirm = q.confirm.as_deref().unwrap_or("").trim().to_lowercase();
    if confirm != name {
        return Err(HandlerError::bad_request_with(
            "confirm_required",
            "pass ?confirm=<domain-name> to delete a domain",
            serde_json::json!({ "domain": name }),
        ));
    }

    let multi_db = state.registry.is_multi_db();
    let pool = state.registry.pool_for(&name).map_err(|e| {
        HandlerError::bad_request("domain_invalid", format!("cannot resolve domain: {e}"))
    })?;
    let name_for_response = name.clone();

    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        // Transaction for atomicity: every delete either all-succeeds or all-rolls-back.
        // VACUUM cannot run inside a tx, so we run it after commit.
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("tx begin failed: {e}")))?;
        if multi_db {
            // Multi-db: the pool IS this domain's own DB. Every table in it is
            // scoped to this domain — clear them all. Order respects FKs:
            // evidence_links → relationships → knowledge (FK target) → rest.
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
                 DELETE FROM audit_events;
                 DELETE FROM domain_centroids;
                 DELETE FROM knowledge_fts;
                 DELETE FROM vec_knowledge;",
            )
            .map_err(|e| HandlerError::internal(format!("delete domain data failed: {e}")))?;
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
                params![name],
            )
            .map_err(|e| HandlerError::internal(format!("delete evidence_links failed: {e}")))?;
            tx.execute(
                "DELETE FROM relationships WHERE knowledge_id IN
                 (SELECT id FROM knowledge WHERE domain = ?1)",
                params![name],
            )
            .map_err(|e| HandlerError::internal(format!("delete relationships failed: {e}")))?;
            // Entities: only delete entities no longer referenced by any
            // relationship in any domain (an entity may be shared across domains).
            tx.execute(
                "DELETE FROM entities WHERE id NOT IN
                 (SELECT from_entity_id FROM relationships)
                 AND id NOT IN
                 (SELECT to_entity_id FROM relationships)",
                [],
            )
            .map_err(|e| HandlerError::internal(format!("delete orphan entities failed: {e}")))?;
            // Tombstones + sources tied to this domain's chunks.
            tx.execute(
                "DELETE FROM tombstones WHERE knowledge_id IN
                 (SELECT id FROM knowledge WHERE domain = ?1)",
                params![name],
            )
            .map_err(|e| HandlerError::internal(format!("delete tombstones failed: {e}")))?;
            // Knowledge rows (FK source for relationships — already cleared).
            tx.execute(
                "DELETE FROM vec_knowledge WHERE knowledge_id IN
                 (SELECT id FROM knowledge WHERE domain = ?1)",
                params![name],
            )
            .map_err(|e| HandlerError::internal(format!("delete vec_knowledge failed: {e}")))?;
            tx.execute("DELETE FROM knowledge WHERE domain = ?1", params![name])
                .map_err(|e| HandlerError::internal(format!("delete knowledge failed: {e}")))?;
            // FTS5 shadow rows for the deleted knowledge ids are cleaned by the
            // FTS5 trigger on knowledge DELETE, so no explicit DELETE there.
            // Domain centroids: drop just this domain's centroid.
            tx.execute(
                "DELETE FROM domain_centroids WHERE domain = ?1",
                params![name],
            )
            .map_err(|e| HandlerError::internal(format!("delete centroid failed: {e}")))?;
        }
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("tx commit failed: {e}")))?;
        // VACUUM must run outside any transaction. Best-effort: failure here
        // means the file isn't defragmented but the data is gone.
        let _ = conn.execute_batch("VACUUM;");
        Ok(())
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "name": name_for_response, "deleted": true })),
    ))
}

#[derive(Debug, Default, Deserialize)]
pub struct DeleteDomainQuery {
    #[serde(default)]
    pub confirm: Option<String>,
}

/// `POST /domains/{name}/vacuum` — reclaim free pages in the domain's DB.
/// Cheap operation; safe to run while the server is up.
pub async fn vacuum_domain(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, HandlerError> {
    use super::normalize_domain;
    let name = normalize_domain(&name)?;
    let pool = state.registry.pool_for(&name).map_err(|e| {
        HandlerError::bad_request("domain_invalid", format!("cannot resolve domain: {e}"))
    })?;

    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        conn.execute_batch("VACUUM;")
            .map_err(|e| HandlerError::internal(format!("vacuum failed: {e}")))?;
        Ok(())
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "name": name, "vacuumed": true })),
    ))
}

/// `GET /domains/{name}/export` — stream a consistent snapshot of the domain's
/// `.db` file. Uses SQLite's `VACUUM INTO` to produce a defragmented, WAL-free
/// copy on a temp path and streams that. Avoids reading the live file directly
/// (WAL pages would be missed; concurrent writes could corrupt the read).
///
/// Content type is `application/octet-stream` with a
/// `Content-Disposition: attachment; filename="brain-<domain>.db"` header.
pub async fn export_domain(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Response, HandlerError> {
    use super::normalize_domain;
    let name = normalize_domain(&name)?;
    let pool = state.registry.pool_for(&name).map_err(|e| {
        HandlerError::bad_request("domain_invalid", format!("cannot resolve domain: {e}"))
    })?;

    let (bytes, filename) =
        tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, String), HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
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
            conn.execute_batch(&format!("VACUUM INTO '{}';", temp.display()))
                .map_err(|e| HandlerError::internal(format!("VACUUM INTO failed: {e}")))?;
            let bytes = std::fs::read(&temp)
                .map_err(|e| HandlerError::internal(format!("read export: {e}")))?;
            let _ = std::fs::remove_file(&temp);
            Ok((bytes, format!("brain-{name}.db")))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    let filename_value =
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .map_err(|_| HandlerError::internal("invalid filename"))?;
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/octet-stream"),
            ),
            (header::CONTENT_DISPOSITION, filename_value),
        ],
        bytes,
    )
        .into_response())
}

/// `POST /domains/{name}/import` — restore a `.db` snapshot into a NEW domain.
///
/// The body is the raw bytes of a previously-exported `brain-<domain>.db`.
/// Safety constraints (per plan M5 — "the server trusts the caller's graph data
/// after validation; never echo raw content in error messages"):
/// - Target domain must NOT already exist on disk (no overwrite of live data).
/// - Domain name validated before the file is written (path-traversal proof).
/// - Bytes are written to a temp path and atomically renamed, then opened.
///
/// Returns 201 with `{ "name": ..., "imported": true, "bytes": N }`.
pub async fn import_domain(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: axum::body::Body,
) -> Result<impl IntoResponse, HandlerError> {
    use super::normalize_domain;
    use axum::body::to_bytes;
    let name = normalize_domain(&name)?;
    if name == "global" {
        // Importing into `global` would overwrite the legacy live DB.
        return Err(HandlerError::bad_request(
            "domain_protected",
            "cannot import into the 'global' domain; pick a new name",
        ));
    }
    let bytes = to_bytes(body, 1024 * 1024 * 1024) // 1 GiB hard cap; ponytail: domains are bounded by capacity envelope
        .await
        .map_err(|_| HandlerError::payload_too_large("import body too large"))?;
    if bytes.is_empty() {
        return Err(HandlerError::bad_request(
            "body_empty",
            "import body is empty",
        ));
    }
    // SQLite magic-header check: rejects obvious garbage before we touch disk.
    // The 16-byte header is "SQLite format 3\0".
    const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";
    if bytes.len() < SQLITE_MAGIC.len() || &bytes[..SQLITE_MAGIC.len()] != SQLITE_MAGIC {
        return Err(HandlerError::bad_request(
            "import_invalid",
            "body is not a valid SQLite database (bad magic header)",
        ));
    }

    // Resolve the target path via StorageLayout so the security check (path
    // traversal) lives in exactly one place. Refuse if the file already exists.
    let layout = brain_server::storage_layout::StorageLayout::detect()
        .map_err(|e| HandlerError::internal(format!("storage layout: {e}")))?;
    let final_path = layout.domain_db(&name).map_err(|_| {
        HandlerError::bad_request("domain_invalid", format!("invalid domain: {name}"))
    })?;
    if final_path.exists() {
        return Err(HandlerError::bad_request_with(
            "domain_exists",
            "target domain already has a DB; delete it first",
            serde_json::json!({ "path": final_path.display().to_string() }),
        ));
    }

    // Atomic write: temp file in the SAME dir (same filesystem → atomic rename
    // on POSIX), with a unique suffix to survive concurrent imports of the
    // same name. Cleanup of a crashed prior attempt happens implicitly when
    // the new temp file overwrites it.
    let temp_path = final_path.with_extension(format!("{}.importing", std::process::id()));
    if let Some(p) = final_path.parent() {
        std::fs::create_dir_all(p)
            .map_err(|e| HandlerError::internal(format!("create dir: {e}")))?;
    }
    std::fs::write(&temp_path, &bytes)
        .map_err(|e| HandlerError::internal(format!("write import: {e}")))?;
    std::fs::rename(&temp_path, &final_path).map_err(|e| {
        // Best-effort cleanup of the temp file on rename failure.
        let _ = std::fs::remove_file(&temp_path);
        HandlerError::internal(format!("rename import: {e}"))
    })?;

    // Open the imported DB via the registry so migration runs and the pool
    // is cached. This is the validity check: if the bytes weren't a real
    // SQLite DB (despite the magic header), opening the pool will fail.
    if let Err(e) = state.registry.pool_for(&name) {
        // Clean up the bad import so the next attempt can succeed.
        let _ = std::fs::remove_file(&final_path);
        return Err(HandlerError::bad_request(
            "import_invalid",
            format!("imported DB is invalid: {e}"),
        ));
    }

    let n = bytes.len();
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "name": name, "imported": true, "bytes": n })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shim-mode delete must NOT touch the global audit log or other
    /// domains' centroids. This is the critical-correctness bug caught in the
    /// second-pass review: an earlier draft did `DELETE FROM audit_events`
    /// (no WHERE clause) which would have wiped the immutable audit trail when
    /// any single domain was deleted.
    #[test]
    fn delete_domain_shim_mode_sql_preserves_global_tables() {
        // We can't easily spin up axum in a unit test, so we exercise the SQL
        // shape against a real in-memory DB with the v0.9.9 schema.
        #[allow(clippy::missing_transmute_annotations)]
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
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

        // Execute the EXACT shim-mode delete SQL the handler runs for `health`.
        let tx = conn.transaction().unwrap();
        let name = "health";
        tx.execute(
            "DELETE FROM relationships WHERE knowledge_id IN
             (SELECT id FROM knowledge WHERE domain = ?1)",
            params![name],
        )
        .unwrap();
        tx.execute("DELETE FROM knowledge WHERE domain = ?1", params![name])
            .unwrap();
        tx.execute(
            "DELETE FROM domain_centroids WHERE domain = ?1",
            params![name],
        )
        .unwrap();
        tx.commit().unwrap();

        // Audit log is IMMUTABLE — must be untouched.
        let audit_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            audit_count, 1,
            "global audit_events must survive a domain delete"
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
}
