//! verified webhook HTTP receiver.
//!
//! `POST /webhooks/{kind}` verifies the HMAC-SHA256 signature, enforces
//! idempotency via the delivery id, and enqueues the delivery onto the bounded
//! [`crate::webhook::WebhookQueue`]. It never mutates the index directly — the
//! drain worker does the rest. A forged or replayed webhook is rejected before
//! enqueue.

use crate::AppState;
use crate::handlers::HandlerError;
use crate::webhook::{EnqueueOutcome, WEBHOOK_TS_FUTURE_SKEW_SECS, WebhookQueue};
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
    if let Ok(s) = std::env::var("BRAIN_CONNECTOR_CONFIG_DIR")
        && !s.trim().is_empty()
    {
        return PathBuf::from(s);
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
    // Deterministic: collect the matches and take the LEXICOGRAPHICALLY
    // FIRST — `read_dir` order is filesystem-dependent, so "first match" must
    // not depend on directory enumeration order.
    let mut matches: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_string());
            matches!(n, Some(n) if n.starts_with("github-") && n.ends_with(".json"))
        })
        .collect();
    matches.sort();
    for entry in matches {
        let bytes = std::fs::read(&entry).ok()?;
        let cfg: WhConfig = serde_json::from_slice(&bytes).ok()?;
        if let Some(secret_path) = cfg.webhook_secret_path {
            // the webhook signing secret is a bearer capability —
            // a world-readable copy lets any local user forge signatures.
            // Fail closed like the auth token file: a group/world-accessible
            // secret is refused (None) rather than trusted.
            if crate::auth::check_secret_permissions(std::path::Path::new(&secret_path)).is_err() {
                tracing::warn!(path = %secret_path,
                    "webhook secret file is not owner-only; refusing to trust it");
                return None;
            }
            return std::fs::read(&secret_path).ok();
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
    // The Beacon kb-feedback kind is ALWAYS HMAC-gated (Standard Webhooks),
    // independent of the legacy GitHub-only posture below.
    if kind == "kb-feedback" {
        return receive_kb_feedback(&state, &headers, &body).await;
    }

    // when `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED=1`, the
    // receiver demands the Standard Webhooks header set (id/timestamp/
    // signature) and verifies the signature over `{id}.{timestamp}.{body}`, so
    // a first-party sender cannot re-stamp a stale timestamp. Any kind is
    // accepted here — the flag is an explicit operator opt-in for their own
    // trusted senders. Off by default → the legacy GitHub path below.
    if crate::config::webhook_timestamp_required() {
        return receive_standard(&state, &kind, &headers, &body).await;
    }

    // only GitHub webhooks. Other kinds are unsupported.
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

    // Timestamp check: GitHub sends a standard HTTP
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
            // audit a verified, accepted webhook (delivery id only).
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

/// Standard Webhooks path — requires the spec header
/// set and verifies the `v1,` signature over `{id}.{timestamp}.{body}` before
/// enqueuing with `webhook-id` as the idempotency key.
async fn receive_standard(
    state: &Arc<AppState>,
    kind: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Response {
    let id = headers
        .get("webhook-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    let ts = headers
        .get("webhook-timestamp")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    let sig = headers
        .get("webhook-signature")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    if id.is_empty() || ts.is_empty() || sig.is_empty() {
        deny(state, kind, "missing standard-webhooks headers");
        return HandlerError::unauthorized("missing webhook-id/timestamp/signature")
            .into_response();
    }

    let secret = match load_webhook_secret() {
        Some(s) => s,
        None => {
            warn!("webhook: no connector secret configured");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(json!({ "error": "no connector secret configured" })),
            )
                .into_response();
        }
    };
    if !WebhookQueue::verify_standard_signature(&secret, &id, &ts, body, &sig) {
        deny(state, kind, "bad standard-webhooks signature");
        return HandlerError::unauthorized("standard-webhooks signature verification failed")
            .into_response();
    }

    // `webhook-timestamp` is unix seconds; it rides inside the verified HMAC, so
    // enforcing the replay window here closes the re-stamp gap.
    let received_at = ts
        .parse::<u64>()
        .ok()
        .and_then(|s| std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(s)));

    let queue = WebhookQueue::new(Arc::new(state.pool.clone()));
    match queue.enqueue_ts(kind, "standard", &id, body, received_at) {
        Ok(EnqueueOutcome::Enqueued) => {
            if let Ok(conn) = state.pool.get() {
                crate::audit::record(
                    &conn,
                    crate::audit::AuditKind::Webhook,
                    kind,
                    &id,
                    crate::audit::AuditStatus::Ok,
                    "standard",
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
            deny(state, kind, "timestamp check failed");
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

/// The Beacon kb-feedback signing secret. The operator-hosted relay holds the
/// Standard Webhooks secret; brain-server verifies. `BRAIN_KB_FEEDBACK_SECRET_FILE`
/// (0600-checked, fail-closed like every secret file). Absent/unreadable → None
/// → the route 500s rather than accepting unverified feedback.
fn load_kb_feedback_secret() -> Option<Vec<u8>> {
    let path = std::env::var("BRAIN_KB_FEEDBACK_SECRET_FILE").ok()?;
    if crate::auth::check_secret_permissions(std::path::Path::new(&path)).is_err() {
        tracing::warn!(path = %path,
            "kb-feedback secret file is not owner-only; refusing to trust it");
        return None;
    }
    std::fs::read(path).ok()
}

/// A verified kb-feedback payload: aggregate counters only, no PII by
/// construction. `anonymous_id` is the RELAY's salted day-bucket hash — the
/// raw IP never reaches this handler, and nothing here is stored verbatim
/// beyond these fields.
#[derive(serde::Deserialize)]
struct KbFeedbackPayload {
    slug: String,
    helpful: bool,
    day_bucket: String,
    #[serde(default)]
    anonymous_id: Option<String>,
}

fn day_bucket_valid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && s.chars()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

/// The kb-feedback receiver: ALWAYS Standard-Webhooks HMAC-verified (regardless
/// of the timestamp-required flag — this kind has no legacy path), synchronously
/// converted into one `kb_feedback` finding row (aggregate counters only), with
/// replay protection via the shared seen-window.
async fn receive_kb_feedback(state: &Arc<AppState>, headers: &HeaderMap, body: &Bytes) -> Response {
    let state = Arc::clone(state);
    let id = headers
        .get("webhook-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    let ts = headers
        .get("webhook-timestamp")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    let sig = headers
        .get("webhook-signature")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    if id.is_empty() || ts.is_empty() || sig.is_empty() {
        deny(&state, "kb-feedback", "missing standard-webhooks headers");
        return HandlerError::unauthorized("missing webhook-id/timestamp/signature")
            .into_response();
    }
    let Some(secret) = load_kb_feedback_secret() else {
        warn!("webhook: no kb-feedback secret configured");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({ "error": "no kb-feedback secret configured" })),
        )
            .into_response();
    };
    if !WebhookQueue::verify_standard_signature(&secret, &id, &ts, body, &sig) {
        deny(&state, "kb-feedback", "bad standard-webhooks signature");
        return HandlerError::unauthorized("standard-webhooks signature verification failed")
            .into_response();
    }
    let received_at = ts
        .parse::<u64>()
        .ok()
        .and_then(|s| std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(s)));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Some(t) = received_at {
        let t_secs = t
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);
        if t_secs.abs_diff(now_ms) > WEBHOOK_TS_FUTURE_SKEW_SECS {
            deny(&state, "kb-feedback", "timestamp outside replay window");
            return HandlerError::unauthorized("timestamp check failed").into_response();
        }
    } else {
        deny(&state, "kb-feedback", "unparseable webhook-timestamp");
        return HandlerError::unauthorized("timestamp check failed").into_response();
    }

    let payload: KbFeedbackPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(_) => {
            deny(&state, "kb-feedback", "payload invalid");
            return HandlerError::bad_request(
                "webhook_bad_request",
                "kb-feedback payload must be {slug, helpful, day_bucket}",
            )
            .into_response();
        }
    };
    let slug_ok = brain_server::kb::is_valid_slug(&payload.slug);
    let bucket_ok = day_bucket_valid(&payload.day_bucket);
    let anon_ok = payload.anonymous_id.as_deref().is_none_or(|a| {
        !a.is_empty()
            && a.len() <= 128
            && a.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'=' || b == b'/' || b == b'+')
    });
    if !slug_ok || !bucket_ok || !anon_ok {
        deny(&state, "kb-feedback", "payload fields invalid");
        return HandlerError::bad_request(
            "webhook_bad_request",
            "kb-feedback slug/day_bucket/anonymous_id failed validation",
        )
        .into_response();
    }

    let queue = WebhookQueue::new(Arc::new(state.pool.clone()));
    let first_sight = tokio::task::spawn_blocking(move || queue.seen_claim(&id))
        .await
        .unwrap_or_else(|e| Err(crate::handlers::HandlerError::internal(format!("{e}"))));
    match first_sight {
        Ok(true) => {}
        Ok(false) => {
            return (StatusCode::OK, axum::Json(json!({ "status": "duplicate" }))).into_response();
        }
        Err(e) => return e.into_response(),
    }

    let source = if payload.helpful {
        "kb-feedback:helpful"
    } else {
        "kb-feedback:not_helpful"
    };
    enum Ingest {
        Recorded { hot: Option<i64> },
        Flood,
    }
    let stored = tokio::task::spawn_blocking({
        let slug = payload.slug.clone();
        let state = Arc::clone(&state);
        move || -> Result<Ingest, String> {
            let conn = state.pool.get().map_err(|e| format!("{e}"))?;
            // Flood bound: the synchronous path bypasses the webhook queue
            // cap, so it enforces its own trailing-hour ingest bound (503 =
            // back off; a legitimate relay retries later).
            let recent: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM findings
                     WHERE claim = 'kb_feedback' AND ts > strftime('%s','now') - 3600",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| format!("{e}"))?;
            if recent >= crate::config::KB_FEEDBACK_MAX_PER_HOUR {
                return Ok(Ingest::Flood);
            }
            conn.execute(
                "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
                 VALUES (0, 'kb_feedback', ?1, ?2, 1.0, strftime('%s','now'))",
                rusqlite::params![slug, source],
            )
            .map_err(|e| format!("{e}"))?;
            crate::audit::record(
                &conn,
                crate::audit::AuditKind::Webhook,
                "kb-feedback",
                &slug,
                crate::audit::AuditStatus::Ok,
                "feedback recorded",
            );
            // Rising-repeat signal: when a slug's feedback count first crosses
            // the hot-topic threshold, emit the existing workflow alert kind.
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM findings WHERE claim = 'kb_feedback' AND evidence = ?1",
                    rusqlite::params![slug],
                    |r| r.get(0),
                )
                .map_err(|e| format!("{e}"))?;
            let threshold = crate::config::KB_HOT_TOPIC_THRESHOLD;
            Ok(Ingest::Recorded {
                hot: (count == threshold).then_some(count),
            })
        }
    })
    .await;
    match stored {
        Ok(Ok(Ingest::Flood)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({ "error": "feedback_rate_limited" })),
        )
            .into_response(),
        Ok(Ok(Ingest::Recorded { hot })) => {
            if let Some(n) = hot {
                crate::alert::publish(
                    &state,
                    crate::alert::ALERT_KIND_WORKFLOW,
                    json!({ "hot_topic": payload.slug, "feedback_count": n }),
                );
            }
            (StatusCode::OK, axum::Json(json!({ "status": "recorded" }))).into_response()
        }
        Ok(Err(e)) => HandlerError::internal(e).into_response(),
        Err(e) => HandlerError::internal(format!("{e}")).into_response(),
    }
}
