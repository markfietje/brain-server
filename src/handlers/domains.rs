use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::handlers::auth::OptPrincipal;
use crate::handlers::HandlerError;
use crate::AppState;
use rusqlite::{params, Connection};

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

#[derive(Debug, Deserialize)]
pub struct MoveDomainsRequest {
    pub ids: Vec<i64>,
    pub to: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct MoveDomainsQuery {
    #[serde(default)]
    pub confirm: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MoveDomainsResponse {
    pub to: String,
    pub moved: usize,
    pub from_domains: Vec<String>,
    pub recomputed: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RecomputeResponse {
    pub recomputed: Vec<(String, usize)>,
}

/// `GET /domains` — list domains with per-domain counts.
///
/// In shim mode (single-DB) this groups by the `domain` column.
/// In multi-db mode each file is its own domain.
pub async fn domains(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<DomainsResponse>, HandlerError> {
    // AuthZ read gate. `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
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
    principal: OptPrincipal,
    Json(req): Json<CreateDomainRequest>,
) -> Result<impl IntoResponse, HandlerError> {
    use super::normalize_domain;
    let name = normalize_domain(&req.name)?;
    // write gate (creating a domain is a write). `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Write, "", &name)?;

    // registration is the ONE creation path —
    // this is the only place a domain file comes into being (cap-bounded in
    // multi-db; warm no-op over the shared pool in shim mode).
    let pool = state
        .registry
        .register(&name)
        .map_err(super::map_domain_error)?;

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
    principal: OptPrincipal,
    Path(name): Path<String>,
    Query(q): Query<DeleteDomainQuery>,
) -> Result<impl IntoResponse, HandlerError> {
    use super::normalize_domain;
    let name = normalize_domain(&name)?;
    // admin gate (destructive lifecycle op). `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", &name)?;

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
    let pool = state
        .registry
        .pool_for(&name)
        .map_err(super::map_domain_error)?;
    let name_for_response = name.clone();
    let name_for_audit = name.clone();
    let root = state.db_path.parent().map(ToOwned::to_owned);

    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        // Transaction for atomicity: every delete either all-succeeds or all-rolls-back.
        // VACUUM cannot run inside a tx, so we run it after commit.
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("tx begin failed: {e}")))?;
        // a domain holding any actively-held chunk
        // refuses deletion entirely (all-or-nothing). The operator must release
        // every hold or scope the delete before the domain can go.
        {
            let ids: Vec<i64> = if multi_db {
                let mut stmt = tx
                    .prepare("SELECT id FROM knowledge")
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, i64>(0))
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                rows.filter_map(|r| r.ok()).collect()
            } else {
                let mut stmt = tx
                    .prepare("SELECT id FROM knowledge WHERE domain = ?1")
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                let rows = stmt
                    .query_map(params![name], |r| r.get::<_, i64>(0))
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                rows.filter_map(|r| r.ok()).collect()
            };
            crate::legal_hold::refuse_if_held(&tx, &ids)?;
        }
        if multi_db {
            // export the domain's audit segment
            // before erasure so the chain survives as an operator-reviewable
            // artifact, then preserve it in the live file too (never unlink).
            export_audit_segment(
                &tx,
                &name,
                if let Some(r) = &root {
                    r.as_path()
                } else {
                    std::path::Path::new(".")
                },
            )
            .map_err(|e| HandlerError::internal(format!("archive domain audit: {e}")))?;
            // Multi-db: the pool IS this domain's own DB. Every table in it is
            // scoped to this domain — clear them all. Order respects FKs:
            // evidence_links → relationships → knowledge (FK target) → rest.
            // `audit_events` is deliberately NOT deleted: the immutible chain
            // must survive a domain delete (F-02).
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
        // record the deletion on the surviving chain (the global
        // chain in shim mode; the domain's own preserved chain in multi-db).
        let _ = crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "operator",
            &name_for_audit,
            crate::audit::AuditStatus::Ok,
            "domain_deleted",
        );
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
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, HandlerError> {
    use super::normalize_domain;
    let name = normalize_domain(&name)?;
    // admin gate (maintenance op). `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", &name)?;
    let pool = state
        .registry
        .pool_for(&name)
        .map_err(super::map_domain_error)?;

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
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Response, HandlerError> {
    use super::normalize_domain;
    let name = normalize_domain(&name)?;
    // read gate (streams a full DB snapshot). `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Read, "", &name)?;
    let pool = state
        .registry
        .pool_for(&name)
        .map_err(super::map_domain_error)?;

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
    principal: OptPrincipal,
    Path(name): Path<String>,
    body: axum::body::Body,
) -> Result<impl IntoResponse, HandlerError> {
    use super::normalize_domain;
    use axum::body::to_bytes;
    let name = normalize_domain(&name)?;
    // admin gate (overwrites domain data). `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", &name)?;
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
    // `register` — an import is an admin-created resource
    // (an unregistered name must not lazily create a file).
    if let Err(e) = state.registry.register(&name) {
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

/// `POST /domains/move` — relabel chunks from one domain to another.
///
/// the migration mechanism for fixing the 99%-in-`global`
/// corpus. Relabels `knowledge.domain` in ONE transaction (provenance fields
/// `source`/`authority`/`observed_at` are untouched), then recomputes the
/// centroids of every domain touched so routing sees the move. This is the
/// non-re-ingest cure: re-ingesting is blocked by the global `content_hash`
/// dedup and wastes a re-embed, while a relabel needs no schema change
/// (`vec_knowledge` has no domain column — filtering is on `knowledge.domain`).
///
/// Guards: `to` may not be `global` (the fallback bucket); moving rows OUT of
/// `global` requires `?confirm=global` (typo-replay protection, mirror of the
/// v1.0 delete guard). Each id must exist. Bounded by `MAX_MULTI_GET`/call.
pub async fn move_domains(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<MoveDomainsQuery>,
    Json(req): Json<MoveDomainsRequest>,
) -> Result<Json<MoveDomainsResponse>, HandlerError> {
    use super::normalize_domain;
    let to = normalize_domain(&req.to)?;
    // Admin gate (bulk relabel of memory).
    super::authorize(&principal.0, crate::auth::Action::Admin, "", &to)?;
    if to == "global" {
        return Err(HandlerError::bad_request(
            "domain_protected",
            "the 'global' domain is the fallback bucket; do not move INTO it",
        ));
    }
    if req.ids.is_empty() {
        return Err(HandlerError::bad_request("ids_empty", "no ids to move"));
    }
    if req.ids.len() > crate::config::MAX_MULTI_GET {
        return Err(HandlerError::bad_request_with(
            "too_many_ids",
            format!(
                "ids exceed {} per call; batch the migration",
                crate::config::MAX_MULTI_GET
            ),
            serde_json::json!({ "max": crate::config::MAX_MULTI_GET }),
        ));
    }
    let confirm = q.confirm.as_deref().unwrap_or("").trim().to_lowercase();

    let pool = state
        .registry
        .pool_for(&to)
        .map_err(super::map_domain_error)?;
    let to_c = to.clone();
    let ids = req.ids;

    let (moved, from_domains) = tokio::task::spawn_blocking(move || {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        relabel_chunks(&mut conn, &ids, &to_c, &confirm)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    // Recompute centroids for every domain touched so routing sees the move.
    // Best-effort: a centroid failure must not fail an otherwise-successful move.
    let mut recomputed: Vec<String> = Vec::new();
    let mut domains = from_domains.clone();
    if !domains.contains(&to) {
        domains.push(to.clone());
    }
    for d in domains {
        if let Ok(dp) = state.registry.pool_for(&d) {
            let _ = crate::domain_router::recompute_centroid(&dp, &d, &state.pool);
            recomputed.push(d);
        }
    }

    Ok(Json(MoveDomainsResponse {
        to,
        moved,
        from_domains,
        recomputed,
    }))
}

/// `POST /domains/recompute` — v1.13.0 M4: one-shot recompute of every known
/// domain's centroid from the corrected M1 source. The post-migration catch-up
/// that makes M2's auto-route meaningful (until real centroids exist, `route()`
/// only ever sees `global`). Runs on the caller's pool in a blocking task.
/// Admin-gated.
pub async fn recompute_domains(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<RecomputeResponse>, HandlerError> {
    // Admin gate (operator sweep over all domains).
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = state.pool.clone();
    let result =
        tokio::task::spawn_blocking(move || crate::domain_router::recompute_all_centroids(&pool))
            .await
            .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?
            .map_err(|e| HandlerError::internal(format!("recompute sweep failed: {e}")))?;
    Ok(Json(RecomputeResponse { recomputed: result }))
}

/// The relabel transaction core of `move_domains`, extracted for testability.
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
) -> Result<(usize, Vec<String>), HandlerError> {
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
        .map_err(|e| HandlerError::internal(format!("id check failed: {e}")))?;
    if existing as usize != ids.len() {
        return Err(HandlerError::bad_request(
            "id_not_found",
            format!(
                "{}/{} ids do not exist",
                ids.len() - existing as usize,
                ids.len()
            ),
        ));
    }

    // Source domains involved; draining `global` needs ?confirm=global.
    let mut from_domains: Vec<String> = Vec::new();
    {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT DISTINCT domain FROM knowledge WHERE id IN ({ph})"
            ))
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        for r in rows.flatten() {
            from_domains.push(r);
        }
    }
    if from_domains.iter().any(|d| d == "global") && confirm != "global" {
        return Err(HandlerError::bad_request_with(
            "confirm_required",
            "moving rows out of 'global' requires ?confirm=global",
            serde_json::json!({ "domain": "global" }),
        ));
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
        .map_err(|e| HandlerError::internal(format!("tx begin failed: {e}")))?;
    let changed = tx
        .execute(
            &format!("UPDATE knowledge SET domain = ?1 WHERE id IN ({ph}) AND domain != ?1"),
            param_refs.as_slice(),
        )
        .map_err(|e| HandlerError::internal(format!("relabel failed: {e}")))?;
    tx.commit()
        .map_err(|e| HandlerError::internal(format!("tx commit failed: {e}")))?;
    Ok((changed, from_domains))
}

/// stream a domain's audit segment to `<layout>/archives/<domain>-audit-<date>.ndjson`
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
        // prev_hash (and pre-v1.1 rows NULL actors); a bare String read would
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
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// domains' centroids. This is the critical-correctness bug caught in the
    /// second-pass review: an earlier draft did `DELETE FROM audit_events`
    /// (no WHERE clause) which would have wiped the immutable audit trail when
    /// any single domain was deleted.
    #[test]
    fn delete_domain_shim_mode_sql_preserves_global_tables() {
        // We can't easily spin up axum in a unit test, so we exercise the SQL
        // shape against a real in-memory DB with the v0.9.9 schema.
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
        let err = super::relabel_chunks(&mut conn, &global_ids, "business", "").unwrap_err();
        assert_eq!(
            err.inner.code, "confirm_required",
            "got: {:?}",
            err.inner.code
        );

        // With confirm -> both rows move; business rows untouched.
        let (moved, from) =
            super::relabel_chunks(&mut conn, &global_ids, "business", "global").unwrap();
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
        let err = super::relabel_chunks(&mut conn, &[999999], "business", "global").unwrap_err();
        assert_eq!(
            err.inner.code, "id_not_found",
            "expected id_not_found, got: {:?}",
            err.inner.code
        );
    }

    /// the one-shot sweep recomputes every known domain's centroid
    /// from vec_knowledge and cleans a stale centroid for an emptied domain.
    /// Driven through the real vec0 + `recompute_all_centroids` path (a Pool is
    /// required, so this spins up a real pool on a temp file like the M1 tests).
    #[test]
    fn recompute_sweep_recomputes_all_and_cleans_stale() {
        crate::register_sqlite_vec();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sweep.db");
        // Schema first on a raw connection (the pool reads the same file).
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            brain_server::migration::run_migration(&mut conn, crate::config::DB_MMAP_SIZE_MIB)
                .expect("migration");
            conn.execute(
                "INSERT INTO knowledge(id, content, content_hash, domain) VALUES
                    (1, 'a', 'a', 'visa')",
                [],
            )
            .unwrap();
            let v: Vec<f32> = (0..512).map(|i| (i as f32 * 0.01).sin()).collect();
            let blob: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
            conn.execute(
                "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
                 VALUES (1, vec_quantize_int8(?1, 'unit'), vec_quantize_binary(?1), 'test', datetime('now'))",
                rusqlite::params![blob],
            )
            .unwrap();
            // A stale centroid for a domain with zero rows must be cleaned.
            conn.execute(
                "INSERT INTO domain_centroids (domain, centroid, count) VALUES ('dead', X'ABCD', 3)",
                [],
            )
            .unwrap();
        }
        let pool: crate::Pool = r2d2::Pool::builder()
            .build(r2d2_sqlite::SqliteConnectionManager::file(&path))
            .expect("pool build");
        let out = crate::domain_router::recompute_all_centroids(&pool).unwrap();
        let rows: std::collections::BTreeMap<String, usize> = out.into_iter().collect();
        assert_eq!(rows.get("visa"), Some(&1), "visa recomputed from vec0");
        assert_eq!(
            rows.get("dead"),
            Some(&0),
            "emptied domain processed with count 0 (centroid cleaned)"
        );
        let dead_rows: i64 = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM domain_centroids WHERE domain='dead'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dead_rows, 0, "stale centroid deleted");
    }
}
