//! v0.9.4 Sources — source lifecycle HTTP handlers.
//!
//! - `POST /sources/reconcile`: kind-scoped reconciliation. The caller passes
//!   the live URI set (typically the files currently on disk under a vault
//!   dir); the server retires any active source of that kind whose URI is not
//!   in the set, sweeping its chunks from retrieval.
//! - `DELETE /sources/{id}`: retire a single source by id. Sweeps chunks and
//!   marks the source + active revision tombstoned.
//!
//! Both handlers wrap `crate::sources::{reconcile, delete_source}` so the DB
//! logic stays in the testable module. Wire contract per `API_CONTRACT.md`.

use axum::extract::{Path, State};
use axum::response::Json;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::handlers::HandlerError;
use crate::AppState;

/// Hard cap on the size of `live_uris` in a reconcile request. Matches the
/// `MAX_INGEST_FILES` ceiling in `brain ingest-dir` (50k) — a vault larger
/// than this needs the paid live-sync tier, not one-shot reconcile.
const MAX_LIVE_URIS: usize = 50_000;

#[derive(Debug, Deserialize)]
pub struct ReconcileRequest {
    /// Source kind to reconcile against (e.g. "vault"). Cannot be empty.
    pub kind: String,
    /// Canonical URIs of sources that still exist. Any active source of `kind`
    /// whose URI is NOT in this set is retired.
    #[serde(default)]
    pub live_uris: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ReconcileResponse {
    pub kind: String,
    pub deleted_sources: usize,
    pub deleted_chunks: usize,
    pub orphan_uris: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DeleteSourceResponse {
    pub deleted: bool,
    pub source_id: i64,
}

/// `POST /sources/reconcile`
pub async fn reconcile(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ReconcileRequest>,
) -> Result<Json<ReconcileResponse>, HandlerError> {
    let kind = req.kind.trim().to_lowercase();
    if kind.is_empty() {
        return Err(HandlerError::bad_request(
            "kind_invalid",
            "kind must not be empty",
        ));
    }
    // ponytail ceiling: a URI cap on the request body. The request-size limit
    // (MAX_REQUEST_SIZE) already bounds the wire, but a bounded Vec makes the
    // abuse surface explicit. The server does NOT walk the filesystem — the
    // caller supplies the live set, preserving the client/server boundary.
    if req.live_uris.len() > MAX_LIVE_URIS {
        return Err(HandlerError::bad_request_with(
            "too_many_uris",
            format!("live_uris exceeds {MAX_LIVE_URIS}"),
            serde_json::json!({ "max": MAX_LIVE_URIS, "got": req.live_uris.len() }),
        ));
    }
    let live: std::collections::HashSet<String> = req.live_uris.iter().cloned().collect();
    let kind_for_db = kind.clone();
    let audit_state = Arc::clone(&state);

    let report = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let mut conn = state
            .pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("transaction failed: {e}")))?;
        let report = crate::sources::reconcile(&tx, &kind_for_db, &live)
            .map_err(|e| HandlerError::internal(format!("reconcile failed: {e}")))?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok::<_, HandlerError>(report)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    // v0.9.7 Guard: audit successful reconcile (kind-scoped; no secret URIs).
    if let Ok(conn) = audit_state.pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("kind={kind}"),
            crate::audit::AuditStatus::Ok,
            &format!("deleted={}", report.deleted_sources),
        );
    }

    Ok(Json(ReconcileResponse {
        kind,
        deleted_sources: report.deleted_sources,
        deleted_chunks: report.deleted_chunks,
        orphan_uris: report.orphan_uris,
    }))
}

/// `DELETE /sources/{id}`
pub async fn delete_source(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<DeleteSourceResponse>, HandlerError> {
    let pool = state.pool.clone();

    let deleted = tokio::task::spawn_blocking(move || -> Result<bool, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("transaction failed: {e}")))?;
        // Surface the URI for the tombstone audit before the source row is
        // marked deleted — same provenance rationale as `forget`'s tombstone.
        let _uri: Option<String> = tx
            .query_row("SELECT uri FROM sources WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .ok();
        let existed = crate::sources::delete_source(&tx, id)
            .map_err(|e| HandlerError::internal(format!("delete_source failed: {e}")))?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(existed)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    if !deleted {
        return Err(HandlerError::not_found(format!("no source with id {id}")));
    }

    // v0.9.7 Guard: audit successful source deletion (id only, never the URI).
    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("source_id={id}"),
            crate::audit::AuditStatus::Ok,
            "deleted",
        );
    }

    Ok(Json(DeleteSourceResponse {
        deleted: true,
        source_id: id,
    }))
}
