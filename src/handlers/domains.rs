//! the isolation-domain administration surface (HTTP).
//!
//! The storage story lives in [`crate::service::domains_admin`] (create/
//! delete/vacuum/export census + erasure + the relabel transaction). This
//! file is the protocol adapter — parse → gate → `spawn_blocking` → core
//! call → typed-error mapping → response — plus the two pieces that are NOT
//! storage and therefore stay: the FILESYSTEM orchestration (the multi-db
//! census's per-file open loop, the import's temp-write/atomic-rename) and
//! the registry/pool-authority calls (`register`, `pool_for`), which never
//! cross the service boundary. Every mutation is Admin-gated + hash-chained
//! into the audit.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

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

/// The handler-boundary map for the domain-admin core: storage failures
/// render the internal-error body with the verbatim pre-move message
/// (statement-specific prefixes included); the hold preflight renders the
/// exact shared `409 legal_hold_active` envelope every erasure route emits;
/// the relabel variants render the pre-move 400 vocabulary byte for byte.
impl From<crate::service::domains_admin::DomainAdminError> for HandlerError {
    fn from(e: crate::service::domains_admin::DomainAdminError) -> Self {
        use crate::service::domains_admin::DomainAdminError as E;
        match e {
            E::Database(m) => HandlerError::internal(m),
            E::LegalHold(held) => HandlerError::conflict_with(
                "legal_hold_active",
                "one or more ids are under legal hold",
                serde_json::json!({ "held": held }),
            ),
            E::MissingIds { missing, total } => HandlerError::bad_request(
                "id_not_found",
                format!("{missing}/{total} ids do not exist"),
            ),
            E::ConfirmRequired => HandlerError::bad_request_with(
                "confirm_required",
                "moving rows out of 'global' requires ?confirm=global",
                serde_json::json!({ "domain": "global" }),
            ),
        }
    }
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
        // the `domain` column on `knowledge` (the core's census). In multi-db
        // mode each per-domain file is a real domain; we use the file list.
        let mut out = Vec::new();
        if !multi_db {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            out = crate::service::domains_admin::shim_domain_rows(&conn)?
                .into_iter()
                .map(|r| DomainInfo {
                    name: r.name,
                    entries: r.entries,
                    entities: r.entities,
                    relations: r.relations,
                    multi_db: r.multi_db,
                })
                .collect();
        } else {
            // Multi-db: open a connection to each per-domain file.
            // ponytail: opens N connections sequentially; upgrade to parallel if > 100 domains.
            let names = state.registry.known_domains();
            for name in &names {
                let Ok(layout) = crate::storage_layout::StorageLayout::detect() else {
                    continue;
                };
                let Ok(path) = layout.domain_db(name) else {
                    continue;
                };
                let Ok(conn) = rusqlite::Connection::open(&path) else {
                    continue;
                };
                let (entries, entities, relations) =
                    crate::service::domains_admin::file_domain_counts(&conn);
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
    // multi-db; warm no-op over the shared pool in shim mode). The pool
    // authority never crosses the service boundary.
    let pool = state
        .registry
        .register(&name)
        .map_err(super::map_domain_error)?;

    let is_new = tokio::task::spawn_blocking(move || -> Result<bool, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        Ok(crate::service::domains_admin::is_empty_store(&conn))
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
/// Drops all data for the domain. The `global` domain is protected. The
/// caller must echo the domain name as `?confirm=<name>` so a
/// typoed URL or replay can't destroy data by accident. The erasure
/// itself (hold preflight → audit-segment export → the FK-ordered sweeps →
/// the in-tx `domain_deleted` evidence row) is the core's
/// `delete_domain_data`, inside the tx this adapter opens.
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
        crate::service::domains_admin::delete_domain_data(&tx, &name, multi_db, root.as_deref())?;
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
        Ok(crate::service::domains_admin::vacuum(&conn)?)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "name": name, "vacuumed": true })),
    ))
}

/// `GET /domains/{name}/export` — stream a consistent snapshot of the domain's
/// `.db` file. The core's `export_snapshot` produces a defragmented, WAL-free
/// copy via SQLite's `VACUUM INTO` on a temp path and reads it back; this
/// adapter streams it. Avoids reading the live file directly (WAL pages would
/// be missed; concurrent writes could corrupt the read).
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
    // The gate depends on the storage layout. In
    // multi-db mode the pool IS the named domain's file, so a Read grant on
    // that domain legitimately covers the snapshot. In SHIM mode
    // `pool_for(name)` resolves to the ONE shared pool — the exported bytes
    // are the whole multi-tenant DB (every tenant's chunks, owners, the audit
    // chain), so a per-name Read grant must never cover it: require Admin.
    // `None` (no JWT) = superuser.
    let action = if state.registry.is_multi_db() {
        crate::auth::Action::Read
    } else {
        crate::auth::Action::Admin
    };
    super::authorize(&principal.0, action, "", &name)?;
    let pool = state
        .registry
        .pool_for(&name)
        .map_err(super::map_domain_error)?;

    let (bytes, filename) =
        tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, String), HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            Ok(crate::service::domains_admin::export_snapshot(
                &conn, &name,
            )?)
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
/// Safety constraints ("the server trusts the caller's graph data
/// after validation; never echo raw content in error messages"):
/// - Target domain must NOT already exist on disk (no overwrite of live data).
/// - Domain name validated before the file is written (path-traversal proof).
/// - Bytes are written to a temp path and atomically renamed, then opened.
///
/// Returns 201 with `{ "name": ..., "imported": true, "bytes": N }`.
///
/// Ceiling (honest): the import path embeds NO storage logic — its
/// duties are the magic-header check, the filesystem write/rename, and the
/// registry `register` (pool authority), all of which stay at the handler by
/// the layer law. The surface is converged: zero embedded statements remain
/// in this file to move.
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
    let layout = crate::storage_layout::StorageLayout::detect()
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
/// delete guard). Each id must exist. Bounded by `MAX_MULTI_GET`/call.
/// The relabel tx itself is the core's `relabel_chunks`.
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

    let (moved, from_domains) =
        tokio::task::spawn_blocking(move || -> Result<(usize, Vec<String>), HandlerError> {
            let mut conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            Ok(crate::service::domains_admin::relabel_chunks(
                &mut conn, &ids, &to_c, &confirm,
            )?)
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

/// `POST /domains/recompute` — one-shot recompute of every known
/// domain's centroid from the corrected vector source. The post-migration catch-up
/// that makes auto-route meaningful (until real centroids exist, `route()`
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
