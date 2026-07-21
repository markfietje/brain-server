//! v0.9.7 "Guard" — verified webhook HTTP receiver.
//!
//! `POST /webhooks/{kind}` verifies the HMAC-SHA256 signature, enforces
//! idempotency via the delivery id, and enqueues the delivery onto the bounded
//! [`crate::webhook::WebhookQueue`]. It never mutates the index directly — the
//! drain worker does the rest. A forged or replayed webhook is rejected before
//! enqueue.

use crate::handlers::HandlerError;
use crate::webhook::{EnqueueOutcome, WebhookQueue};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::warn;

/// Tiny, feature-free view of a connector config: just the webhook secret path.
/// Parsed via `serde_json::Value` so this handler stays free of the
/// `connector-github` feature gate (the server binary never enables it).
#[derive(serde::Deserialize)]
struct WhConfig {
    #[serde(default)]
    webhook_secret_path: Option<String>,
}

/// Resolve the directory holding per-connector config files, honoring
/// `BRAIN_CONNECTOR_CONFIG_DIR` then `~/.config/brain-server/connectors`.
fn connector_config_dir() -> PathBuf {
    if let Ok(s) = std::env::var("BRAIN_CONNECTOR_CONFIG_DIR") {
        if !s.trim().is_empty() {
            return PathBuf::from(s);
        }
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".config/brain-server/connectors")
}

/// Load the first `github-*.json` config found and return its webhook secret.
/// The dispatcher only knows `kind` (no instance in the path), so we glob the
/// kind directory and use the first match. Returns `None` if no github
/// connector is configured.
fn load_webhook_secret() -> Option<Vec<u8>> {
    let dir = connector_config_dir();
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("github-") || !name.ends_with(".json") {
            continue;
        }
        let bytes = std::fs::read(entry.path()).ok()?;
        let cfg: WhConfig = serde_json::from_slice(&bytes).ok()?;
        if let Some(secret_path) = cfg.webhook_secret_path {
            return std::fs::read(secret_path).ok();
        }
    }
    None
}

pub async fn receive(
    State(state): State<Arc<AppState>>,
    Path(kind): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // v0.9.7: only GitHub webhooks. Other kinds are unsupported.
    if kind != "github" {
        return HandlerError::bad_request(
            "unsupported_kind",
            format!("kind '{kind}' not supported"),
        )
        .into_response();
    }

    let sig = match headers
        .get("x-hub-signature-256")
        .and_then(|h| h.to_str().ok())
    {
        Some(s) => s.to_string(),
        None => {
            deny(&state, &kind, "missing signature");
            return HandlerError::unauthorized("missing x-hub-signature-256").into_response();
        }
    };

    let secret = match load_webhook_secret() {
        Some(s) => s,
        None => {
            warn!("webhook: no github connector configured");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({ "error": "no github connector configured" })),
            )
                .into_response();
        }
    };

    if !WebhookQueue::verify_github_signature(&secret, &body, &sig) {
        deny(&state, &kind, "bad signature");
        return HandlerError::unauthorized("signature verification failed").into_response();
    }

    // Timestamp check is enforced via `enqueue_ts` using the request `Date`
    // header (GitHub sends one on every request). If `Date` is absent or
    // unparseable we fall back to delivery-id idempotency + the `webhook_seen`
    // replay window — GitHub has no signed timestamp, so this is still
    // protected against replays.
    let delivery_id = headers
        .get("x-github-delivery")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    if delivery_id.is_empty() {
        deny(&state, &kind, "missing delivery id");
        return HandlerError::bad_request("webhook_bad_request", "missing x-github-delivery")
            .into_response();
    }
    let event = headers
        .get("x-github-event")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Timestamp check (plan M1 "timestamp check"): GitHub sends a standard HTTP
    // `Date` header on every request. If present and parseable, enforce the
    // replay window against it; if absent/unparseable we fall back to the
    // delivery-id idempotency + `webhook_seen` window (GitHub has no signed
    // timestamp, so this is still protected against replays).
    let received_at = headers
        .get(axum::http::header::DATE)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
        .and_then(|dt| {
            // Guard against pre-epoch timestamps so the `u64` cast can't panic.
            if dt.timestamp() < 0 {
                return None;
            }
            let secs = dt.timestamp() as u64;
            let nanos = dt.timestamp_subsec_nanos();
            Some(std::time::UNIX_EPOCH + std::time::Duration::new(secs, nanos))
        });

    let queue = WebhookQueue::new(Arc::new(state.pool.clone()));
    match queue.enqueue_ts(&kind, &event, &delivery_id, &body, received_at) {
        Ok(EnqueueOutcome::Enqueued) => {
            // v0.9.7 Guard: audit a verified, accepted webhook (delivery id only).
            if let Ok(conn) = state.pool.get() {
                crate::audit::record(
                    &conn,
                    crate::audit::AuditKind::Webhook,
                    &kind,
                    &delivery_id,
                    crate::audit::AuditStatus::Ok,
                    &event,
                );
            }
            (StatusCode::OK, axum::Json(json!({ "status": "enqueued" }))).into_response()
        }
        Ok(EnqueueOutcome::Duplicate) => {
            (StatusCode::OK, axum::Json(json!({ "status": "duplicate" }))).into_response()
        }
        Ok(EnqueueOutcome::Full) => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({ "error": "queue_full" })),
        )
            .into_response(),
        Ok(EnqueueOutcome::Rejected) => {
            deny(&state, &kind, "timestamp check failed");
            HandlerError::unauthorized("timestamp check failed").into_response()
        }
        Err(e) => e.into_response(),
    }
}

fn deny(state: &Arc<AppState>, kind: &str, detail: &str) {
    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Webhook,
            kind,
            detail,
            crate::audit::AuditStatus::Denied,
            detail,
        );
    }
}
