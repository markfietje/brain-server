//! The channel bridge webhook receiver.
//!
//! `POST /webhooks/channel/{kind}` — a GOVERNED EDGE, never a server feature.
//! A bridge process (zero-dep edge or the Rust signal-gateway) signs platform
//! messages Standard-Webhooks style with its OWN 0600 secret and POSTs the
//! normalized envelope. The handler NEVER trusts anything but the HMAC:
//!
//! verify → skew → bounds → replay claim `(bridge, external_id)` → flood cap
//! → SCREEN (inside the landing tx, before any state) → thread/auto-open →
//! case note + audit chain, all in ONE `BEGIN IMMEDIATE` transaction.
//!
//! `POST /webhooks/channel/{kind}/drain` — the outbound half: pull-model
//! delivery of `channel/out` envelopes (approved acts / consented alert
//! forwards ONLY — the topic never touches a broadcast bus), claimed
//! atomically by the bridge's cron crank. Delivery is at-least-once BY EVENT
//! ID: senders dedupe on `event_id`.
//!
//! Both routes authenticate BRIDGE identity purely by HMAC against the
//! discovered per-config secrets (`channel-{kind}-{tenant}.json`, 0600,
//! shared substrate with the edge). No bearer, ever; no brain token in any
//! bridge (pinned house-wide by self-grep).

use crate::AppState;
use crate::handlers::HandlerError;
use crate::webhook::{WEBHOOK_TS_FUTURE_SKEW_SECS, WebhookQueue};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::Arc;

use crate::workflow::channels::{self, ChannelBridgeConfig, InboundEnvelope, LandError};

/// Flood bound for synchronous bridge paths (503 = back off; the bridge
/// retries later). Counted over the shared `webhook_seen` trailing hour like
/// the signal path's cap. Pinned by test below.
pub(crate) const CHANNEL_MAX_PER_HOUR: i64 = 1_000;

fn deny_channel(state: &Arc<AppState>, actor: &str, detail: &str) {
    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Webhook,
            actor,
            "channel",
            crate::audit::AuditStatus::Denied,
            detail,
        );
    }
}

/// Resolve WHICH configured bridge sent this request: the Standard Webhooks
/// signature must verify against that config's secret (constant-time per
/// candidate, lexicographic candidate order). Returns the verified config or
/// None (none matched = unverified = denied).
fn verify_bridge(
    state: &Arc<AppState>,
    kind: &str,
    headers: &HeaderMap,
    body: &Bytes,
    route_tag: &str,
) -> Option<ChannelBridgeConfig> {
    let id = header_str(headers, "webhook-id");
    let ts = header_str(headers, "webhook-timestamp");
    let sig = header_str(headers, "webhook-signature");
    if id.is_empty() || ts.is_empty() || sig.is_empty() {
        return None;
    }
    // Candidate discovery happens on EVERY request: configs are few, disk is
    // loopback-fast, and a fresh read means an operator can add/remove a
    // bridge without restarting the kernel (config-off by default = rollback).
    let dir = connector_config_dir();
    let candidates: Vec<ChannelBridgeConfig> = channels::discover_bridge_configs(&dir)
        .into_iter()
        .filter(|c| c.kind == kind)
        .collect();
    for cfg in candidates {
        if channels::verify_bridge_signature(&cfg.webhook_secret, &id, &ts, body, &sig) {
            return Some(cfg);
        }
    }
    deny_channel(
        state,
        route_tag,
        "no candidate config verified the signature",
    );
    None
}

fn header_str(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// Resolve the directory holding connector/bridge config files, honoring
/// `BRAIN_CONNECTOR_CONFIG_DIR` then `~/.config/brain-server/connectors`
/// (same convention as the GitHub webhook secret loader).
pub(crate) fn connector_config_dir() -> std::path::PathBuf {
    if let Ok(s) = std::env::var("BRAIN_CONNECTOR_CONFIG_DIR")
        && !s.trim().is_empty()
    {
        return std::path::PathBuf::from(s);
    }
    let home = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    home.join(".config/brain-server/connectors")
}

fn timestamp_skew_ok(state: &Arc<AppState>, ts: &str, actor: &str) -> Option<u64> {
    let secs: u64 = match ts.parse() {
        Ok(s) => s,
        Err(_) => {
            deny_channel(state, actor, "unparseable webhook-timestamp");
            return None;
        }
    };
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if secs.abs_diff(now_secs) > WEBHOOK_TS_FUTURE_SKEW_SECS {
        deny_channel(state, actor, "timestamp outside replay window");
        return None;
    }
    Some(secs)
}

/// `POST /webhooks/channel/{kind}` — inbound envelope landing.
pub async fn receive_channel(
    State(state): State<Arc<AppState>>,
    Path(kind): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // 1–2. HMAC identity + signed-timestamp freshness BEFORE any parse.
    let Some(cfg) = verify_bridge(
        &state,
        &kind,
        &headers,
        &body,
        &format!("channel-webhook:{kind}"),
    ) else {
        return HandlerError::unauthorized("bridge signature verification failed").into_response();
    };
    let actor = format!("channel:{}:{}", cfg.bridge_id(), short_digest(&body));
    let Some(_ts_secs) =
        timestamp_skew_ok(&state, &header_str(&headers, "webhook-timestamp"), &actor)
    else {
        return HandlerError::unauthorized("timestamp check failed").into_response();
    };

    // 3. Bounds-checked envelope projection (pure; refuses loudly).
    let envelope = match InboundEnvelope::parse(&body) {
        Ok(e) => e,
        Err(code) => {
            deny_channel(&state, &actor, code);
            return HandlerError::bad_request("envelope_invalid", code).into_response();
        }
    };

    // 4. Replay claim keyed on (bridge, external_id): a replayed platform
    //    webhook can never double-post a note.
    let claim_id = format!("{}/{}:{}", cfg.kind, cfg.tenant, envelope.external_id);
    let queue = WebhookQueue::new(Arc::new(state.pool.clone()));
    let claim = claim_id.clone();
    let first_sight = tokio::task::spawn_blocking(move || queue.seen_claim(&claim))
        .await
        .unwrap_or_else(|e| Err(HandlerError::internal(format!("{e}"))));
    match first_sight {
        Ok(true) => {}
        Ok(false) => {
            return (StatusCode::OK, axum::Json(json!({ "status": "duplicate" }))).into_response();
        }
        Err(e) => return e.into_response(),
    }

    // 5. Land INSIDE one BEGIN IMMEDIATE tx: screen → thread/auto-open → note
    //    → audit rows commit atomically.
    let landing = tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        let cfg = cfg.clone();
        let envelope = envelope.clone();
        move || -> Result<channels::LandOutcome, String> {
            let mut conn = state.pool.get().map_err(|e| format!("{e}"))?;
            // Flood bound over the shared seen-window (bounds law).
            let recent: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM webhook_seen
                      WHERE seen_at >= datetime('now', '-1 hour')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| format!("{e}"))?;
            if recent >= CHANNEL_MAX_PER_HOUR {
                return Err("flood".into());
            }
            let now = chrono::Utc::now().timestamp();
            let mut tx =
                crate::workflow::tx::WorkflowTx::begin(&mut conn).map_err(|e| format!("{e}"))?;
            let outcome = channels::land_inbound_message(tx.tx(), &cfg, &envelope, now).map_err(
                |e| match e {
                    LandError::UnknownCase(id) => format!("unknown_case:{id}"),
                    LandError::UnknownThread => "unknown_thread".to_string(),
                    other => format!("land_refused:{other:?}"),
                },
            )?;
            tx.commit().map_err(|e| format!("{e}"))?;
            Ok(outcome)
        }
    })
    .await;

    match landing {
        Ok(Ok(outcome)) => {
            // Post-commit operator notifications (metadata only). The domain
            // layer NEVER touches AppState — it hands back alert payloads.
            let alerts = match &outcome.kind {
                channels::LandKind::Quality { alerts } => alerts.clone(),
                _ => Vec::new(),
            };
            for a in alerts {
                crate::alert::publish(&state, crate::alert::ALERT_KIND_WORKFLOW, a);
            }
            if let Ok(conn) = state.pool.get() {
                let detail = match &outcome.kind {
                    channels::LandKind::Note { opened_case, .. } if *opened_case => {
                        "channel/inbound opened-case"
                    }
                    channels::LandKind::Note { .. } => "channel/inbound note",
                    channels::LandKind::StatusLineage => "channel/status lineage",
                    channels::LandKind::Quality { .. } => "channel/quality observation",
                };
                crate::audit::record(
                    &conn,
                    crate::audit::AuditKind::Webhook,
                    &actor,
                    &format!("case:{}", outcome.case_run_id),
                    crate::audit::AuditStatus::Ok,
                    detail,
                );
            }
            let body = match &outcome.kind {
                channels::LandKind::Note {
                    note_id,
                    opened_case,
                } => json!({
                    "status": "note_recorded",
                    "case_run_id": outcome.case_run_id,
                    "note_id": note_id,
                    "opened_case": opened_case,
                }),
                channels::LandKind::StatusLineage => json!({
                    "status": "status_lineage_recorded",
                    "case_run_id": outcome.case_run_id,
                }),
                channels::LandKind::Quality { alerts } => json!({
                    "status": "quality_observed",
                    "alerts_published": alerts.len(),
                }),
            };
            (StatusCode::OK, axum::Json(body)).into_response()
        }
        Ok(Err(msg)) if msg.starts_with("unknown_case:") => {
            deny_channel(&state, &actor, &msg);
            HandlerError::conflict(format!(
                "[case] addressing refused: run {} is not in this bridge's domain",
                msg.trim_start_matches("unknown_case:")
            ))
            .into_response()
        }
        Ok(Err(msg)) if msg == "flood" => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({ "error": "channel_rate_limited" })),
        )
            .into_response(),
        Ok(Err(msg)) => {
            deny_channel(&state, &actor, &msg);
            HandlerError::bad_request("landing_refused", msg).into_response()
        }
        Err(e) => HandlerError::internal(format!("{e}")).into_response(),
    }
}

fn short_digest(body: &[u8]) -> String {
    crate::audit::hash(&String::from_utf8_lossy(body))[..12].to_string()
}

/// `POST /webhooks/channel/{kind}/drain` — pull-model outbound delivery to
/// the bridge crank. Same HMAC seam; returns ≤ [`CHANNEL_MAX_PER_HOUR`]-capped
/// batches of `channel/out` envelopes marked delivered ATOMICALLY (a crash
/// mid-send replays at-least-once; bridges dedupe on `event_id`).
pub async fn drain_channel(
    State(state): State<Arc<AppState>>,
    Path(kind): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(cfg) = verify_bridge(
        &state,
        &kind,
        &headers,
        &body,
        &format!("channel-drain:{kind}"),
    ) else {
        return HandlerError::unauthorized("bridge signature verification failed").into_response();
    };
    let actor = format!("channel-drain:{}", cfg.bridge_id());
    if timestamp_skew_ok(&state, &header_str(&headers, "webhook-timestamp"), &actor).is_none() {
        return HandlerError::unauthorized("timestamp check failed").into_response();
    }
    let batched = tokio::task::spawn_blocking({
        let kind = kind.clone();
        move || -> Result<Vec<serde_json::Value>, String> {
            let mut conn = state.pool.get().map_err(|e| format!("{e}"))?;
            channels::drain_out_batch(&mut conn, &kind, chrono::Utc::now().timestamp())
                .map_err(|e| format!("{e}"))
        }
    })
    .await;
    match batched {
        Ok(Ok(envelopes)) => (
            StatusCode::OK,
            axum::Json(json!({ "status": "ok", "count": envelopes.len(), "envelopes": envelopes })),
        )
            .into_response(),
        Ok(Err(e)) => HandlerError::internal(e).into_response(),
        Err(e) => HandlerError::internal(format!("{e}")).into_response(),
    }
}

// ── Mount-registration reuse (the ONE registration surface for bridges) ────

/// Validate a bridge registration payload: plugin `channel:{kind}`, action
/// mount, revision = the FULL sha256 of the bridge's config bytes (hex64).
/// Returns `(kind, domain)` — the server RECOMPUTES the digest itself from
/// its own copy of the config file (the Gateweld law adapted to edges:
/// evidence certifies bytes BOTH sides can hash).
pub(crate) fn validate_bridge_mount_body(
    v: &serde_json::Value,
) -> Result<(String, String, String), &'static str> {
    let plugin = v
        .get("plugin")
        .and_then(|x| x.as_str())
        .ok_or("missing plugin")?;
    let Some(rest) = plugin.strip_prefix("channel:") else {
        return Err("plugin must be channel:{kind}");
    };
    let kind = rest.to_string();
    if kind.is_empty()
        || kind.len() > 32
        || !kind
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("plugin kind invalid");
    }
    if v.get("action").and_then(|x| x.as_str()) != Some("mount") {
        return Err("action must be mount");
    }
    let bundle_sha256 = v
        .get("bundle_sha256")
        .and_then(|x| x.as_str())
        .ok_or("bundle_sha256 required")?
        .to_lowercase();
    if bundle_sha256.len() != 64 || !bundle_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("bundle_sha256 must be 64 hex chars");
    }
    let domain = v
        .get("domain")
        .and_then(|x| x.as_str())
        .ok_or("domain required")?
        .to_string();
    if domain.is_empty() || domain.len() > 63 {
        return Err("domain invalid");
    }
    Ok((kind, bundle_sha256, domain))
}

/// The `X-Bridge-Mount` HMAC-authenticated sibling used by
/// `post_plugin_mount` when the caller holds NO bearer: the bridge identity +
/// config-digest are verified here, then the evidence row lands through the
/// SAME audit path. Reused, never duplicated: one route, two authentications.
pub(crate) struct BridgeMountIdentity {
    pub kind: String,
    pub tenant: String,
    pub domain: String,
    pub config_sha256: String,
}

impl BridgeMountIdentity {
    pub(crate) fn bridge_label(&self) -> String {
        format!("{}/{}", self.kind, self.tenant)
    }
}

pub(crate) fn verify_bridge_mount(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    body: &Bytes,
) -> Option<BridgeMountIdentity> {
    // The identity KIND comes from the body plugin field; discover configs of
    // ALL kinds and let the signature decide (fail-closed).
    let id = header_str(headers, "webhook-id");
    let ts = header_str(headers, "webhook-timestamp");
    let sig = header_str(headers, "webhook-signature");
    if id.is_empty() || ts.is_empty() || sig.is_empty() {
        return None;
    }
    let dir = connector_config_dir();
    for cfg in channels::discover_bridge_configs(&dir) {
        if channels::verify_bridge_signature(&cfg.webhook_secret, &id, &ts, body, &sig) {
            // SHA-256 over the config FILE bytes (server-recomputed digest).
            use sha2::Digest;
            let config_sha256 = match std::fs::read(config_path(&dir, &cfg)) {
                Ok(bytes) => {
                    let digest = sha2::Sha256::digest(&bytes);
                    digest
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                }
                Err(_) => return None,
            };
            return Some(BridgeMountIdentity {
                kind: cfg.kind,
                tenant: cfg.tenant,
                domain: cfg.domain,
                config_sha256,
            });
        }
    }
    deny_channel(
        state,
        "channel-mount",
        "no candidate config verified the signature",
    );
    None
}

fn config_path(dir: &std::path::Path, cfg: &ChannelBridgeConfig) -> std::path::PathBuf {
    dir.join(format!("channel-{}-{}.json", cfg.kind, cfg.tenant))
}

#[allow(dead_code)] // re-export surface for wiring tests
fn _probe(_: Option<String>) {}
