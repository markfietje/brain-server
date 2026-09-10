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
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
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
            let recent = crate::workflow::channels::recent_inbound_count(&conn)
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
        move || -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), String> {
            let mut conn = state.pool.get().map_err(|e| format!("{e}"))?;
            let now = chrono::Utc::now().timestamp();
            let envelopes =
                channels::drain_out_batch(&mut conn, &kind, now).map_err(|e| format!("{e}"))?;
            // Herald: Relay handover pings ride the SAME claim law, delivered
            // by the same HMAC crank. Additive key; older bridges ignore it.
            let pings =
                channels::drain_ping_batch(&mut conn, &kind, now).map_err(|e| format!("{e}"))?;
            Ok((envelopes, pings))
        }
    })
    .await;
    match batched {
        Ok(Ok((envelopes, pings))) => (
            StatusCode::OK,
            axum::Json(json!({
                "status": "ok",
                "count": envelopes.len(),
                "envelopes": envelopes,
                "pings": pings,
            })),
        )
            .into_response(),
        Ok(Err(e)) => HandlerError::internal(e).into_response(),
        Err(e) => HandlerError::internal(format!("{e}")).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct AckChannelRequest {
    /// Outbox event ids the bridge has delivered (`event_id` per envelope).
    #[serde(default)]
    event_ids: Vec<i64>,
}

/// `POST /webhooks/channel/{kind}/drain/ack` — the at-least-once close of
/// the pull-model drain: the bridge confirms delivered `event_id`s and those
/// rows mark delivered. Same HMAC seam as the drain; unacked rows stay
/// pending and redrill (bridges dedupe on `event_id`). A bridge cannot ack
/// another bridge's rows (thread-scoped). Bounded: 500 ids per call.
pub async fn ack_channel(
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
        &format!("channel-ack:{kind}"),
    ) else {
        return HandlerError::unauthorized("bridge signature verification failed").into_response();
    };
    let actor = format!("channel-ack:{}", cfg.bridge_id());
    if timestamp_skew_ok(&state, &header_str(&headers, "webhook-timestamp"), &actor).is_none() {
        return HandlerError::unauthorized("timestamp check failed").into_response();
    }
    if body.len() > MAX_CONSOLE_BODY {
        return HandlerError::bad_request("body_too_large", "ack body exceeds the bound")
            .into_response();
    }
    let req: AckChannelRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => {
            return HandlerError::bad_request("body_invalid", "expected {\"event_ids\":[...]}")
                .into_response();
        }
    };
    if req.event_ids.len() > 500 {
        return HandlerError::bad_request("too_many_ids", "at most 500 event_ids per ack")
            .into_response();
    }
    let batched = tokio::task::spawn_blocking({
        let kind = kind.clone();
        move || -> Result<usize, String> {
            let mut conn = state.pool.get().map_err(|e| format!("{e}"))?;
            let now = chrono::Utc::now().timestamp();
            channels::ack_out_batch(&mut conn, &kind, &req.event_ids, now)
                .map_err(|e| format!("{e}"))
        }
    })
    .await;
    match batched {
        Ok(Ok(n)) => (
            StatusCode::OK,
            axum::Json(json!({ "status": "ok", "acked": n })),
        )
            .into_response(),
        Ok(Err(e)) => HandlerError::internal(e).into_response(),
        Err(e) => HandlerError::internal(format!("{e}")).into_response(),
    }
}

// ── Herald: the operator-console annex (`POST /webhooks/channel/{kind}/console`)
// ──────────────────────────────────────────────────────────────────────────
//
// The bridge (holding NO brain token — the house-wide credentials law) dials
// the console INTO the kernel over the same HMAC seam as receive/drain. The
// kernel resolves every actor through `channel_user_map` (proposal-
// maintained; platform identity NEVER auto-trusts), role-checks the mapped
// principal against the role store, and then REUSES the existing handler
// machinery — approvals run the byte-identical `approve_proposal` path, so
// the digest binding is enforced TWICE: bridge-side against the rendered
// digest, and server-side inside the approve verb (`digest_required`).

use crate::auth::Scope;
use crate::handlers::auth::OptPrincipal;
use crate::handlers::gate::{ApproveQuery, approve_proposal, reject_proposal};

/// Console-request bounds (the bounds law: every input bounded).
const MAX_CONSOLE_BODY: usize = 4_096;
const MAX_CONSOLE_CRANK_STEPS: u32 = 10;
const CONSOLE_CRANK_TIMEOUT_SECS: u64 = 60;

fn console_audit(
    state: &Arc<AppState>,
    actor: &str,
    target: &str,
    status: crate::audit::AuditStatus,
    detail: &str,
) {
    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Workflow,
            actor,
            target,
            status,
            detail,
        );
    }
}

/// Build the least-privilege principal a mapped operator acts as: tenant =
/// the bridge's configured domain, exactly ONE scope (Read or Write) on the
/// global pool, roles = the map row's role names. Empty roles NEVER get a
/// scope — the caller checks first (a channel-relayed act requires an
/// explicit grant; the JWT-era vacuous-role back-compat does not apply to
/// platform identities).
fn console_principal(
    cfg: &ChannelBridgeConfig,
    principal_label: &str,
    roles: Vec<String>,
    write: bool,
) -> crate::auth::Principal {
    crate::auth::Principal {
        sub: principal_label.to_string(),
        tenant: cfg.domain.clone(),
        scopes: vec![Scope {
            action: if write {
                crate::auth::Action::Write
            } else {
                crate::auth::Action::Read
            },
            team: cfg.domain.clone(),
            domain: "global".to_string(),
        }],
        jti: format!("channel-console:{}", cfg.bridge_id()),
        roles,
        manages: Vec::new(),
        kind: crate::auth::PrincipalKind::Jwt,
    }
}

/// Resolve + role-check a mapped actor inside one blocking step. Returns the
/// (principal, roles) pair or a named refusal (already audited by the
/// caller via the error string).
fn resolve_console_actor(
    conn: &rusqlite::Connection,
    cfg: &ChannelBridgeConfig,
    actor_ref: &str,
    capability: &str,
) -> Result<(String, Vec<String>), &'static str> {
    let Some((principal, roles)) =
        channels::lookup_mapped_actor(conn, &cfg.kind, &cfg.tenant, actor_ref)
            .map_err(|_| "map_unreadable")?
    else {
        return Err("actor_not_mapped");
    };
    // The kill-switch reaches the console: a mapped principal
    // sitting in `revoked_principals` must not list or decide through the
    // bridge HMAC alone — the bridge signature proves the MESSAGE, not the
    // actor's standing (pinned by `console_actor_revoked_refused`).
    if crate::workflow::mesh::is_revoked(conn, &principal).map_err(|_| "revocation_unreadable")? {
        return Err("actor_revoked");
    }
    // A channel-relayed act REQUIRES an explicit role grant — the map is the
    // only trust anchor and an empty grant grants nothing.
    if roles.is_empty() {
        return Err("actor_unroled");
    }
    let resolved = crate::role::resolve(conn, &roles).map_err(|_| "role_store_unreadable")?;
    if !resolved.iter().any(|r| r.can(capability)) {
        return Err("actor_lacks_capability");
    }
    Ok((principal, roles))
}

/// `POST /webhooks/channel/{kind}/console` — the bridge-relayed operator
/// console. Closed action vocabulary: `pending` (renderable proposals +
/// digest), `decide` (approve/reject with digest + actor), `due`, `crank`.
pub async fn post_console(
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
        &format!("channel-console:{kind}"),
    ) else {
        return HandlerError::unauthorized("bridge signature verification failed").into_response();
    };
    let actor = format!("channel-console:{}", cfg.bridge_id());
    if timestamp_skew_ok(&state, &header_str(&headers, "webhook-timestamp"), &actor).is_none() {
        return HandlerError::unauthorized("timestamp check failed").into_response();
    }
    if body.len() > MAX_CONSOLE_BODY {
        console_audit(
            &state,
            &actor,
            "console",
            crate::audit::AuditStatus::Denied,
            "body_oversized",
        );
        return HandlerError::bad_request("console_body_oversized", "request exceeds bounds")
            .into_response();
    }
    let v: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            console_audit(
                &state,
                &actor,
                "console",
                crate::audit::AuditStatus::Denied,
                "body_not_json",
            );
            return HandlerError::bad_request("console_body_invalid", "request must be json")
                .into_response();
        }
    };
    let action = v.get("action").and_then(|x| x.as_str()).unwrap_or("");
    match action {
        "pending" => console_pending_action(&state, &cfg, &actor, &v).await,
        "decide" => console_decide_action(&state, &cfg, &actor, &v).await,
        "due" => console_due_action(&state, &cfg, &actor, &v).await,
        "crank" => console_crank_action(&state, &cfg, &actor, &v).await,
        other => {
            console_audit(
                &state,
                &actor,
                "console",
                crate::audit::AuditStatus::Denied,
                &format!("unknown_action:{other}"),
            );
            HandlerError::bad_request(
                "console_action_unknown",
                "action must be pending|decide|due|crank",
            )
            .into_response()
        }
    }
}

fn bounded_actor_ref(v: &serde_json::Value) -> Option<String> {
    let s = v.get("actor_ref").and_then(|x| x.as_str())?;
    if s.is_empty() || s.len() > channels::MAX_ACTOR_REF || s.chars().any(char::is_control) {
        return None;
    }
    Some(s.to_string())
}

async fn console_pending_action(
    state: &Arc<AppState>,
    cfg: &ChannelBridgeConfig,
    actor: &str,
    v: &serde_json::Value,
) -> Response {
    let limit = v
        .get("limit")
        .and_then(|x| x.as_u64())
        .map(|n| n.min(channels::MAX_CONSOLE_PENDING as u64) as usize)
        .unwrap_or(channels::MAX_CONSOLE_PENDING);
    // Deadbolt (X-M6): the pending listing carries proposal bodies + digests,
    // so it role-checks the mapped actor exactly like every other console
    // action — `read` capability via the same map (empty grants nothing).
    let Some(actor_ref) = bounded_actor_ref(v) else {
        console_audit(
            state,
            actor,
            "console",
            crate::audit::AuditStatus::Denied,
            "actor_ref_invalid",
        );
        return HandlerError::bad_request(
            "actor_ref_invalid",
            "actor_ref must be an opaque platform id",
        )
        .into_response();
    };
    let pool = state.pool.clone();
    let cfg2 = cfg.clone();
    let resolved =
        tokio::task::spawn_blocking(move || -> Result<(String, Vec<String>), &'static str> {
            let conn = pool.get().map_err(|_| "pool_unavailable")?;
            resolve_console_actor(&conn, &cfg2, &actor_ref, "read")
        })
        .await;
    let (principal_label, _roles) = match resolved {
        Ok(Ok(pair)) => pair,
        Ok(Err(reason)) => {
            console_audit(
                state,
                actor,
                "console:pending",
                crate::audit::AuditStatus::Denied,
                reason,
            );
            return HandlerError::forbidden(crate::auth::Action::Read, &cfg.domain, "global")
                .into_response();
        }
        Err(e) => return HandlerError::internal(format!("{e}")).into_response(),
    };
    let pool = state.pool.clone();
    let rows = tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        channels::console_pending(&conn, limit).map_err(|e| format!("{e}"))
    })
    .await;
    console_audit(
        state,
        actor,
        "console:pending",
        crate::audit::AuditStatus::Ok,
        &format!("principal:{principal_label}"),
    );
    match rows {
        Ok(Ok(proposals)) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "proposals": proposals })),
        )
            .into_response(),
        Ok(Err(e)) => HandlerError::internal(e).into_response(),
        Err(e) => HandlerError::internal(format!("{e}")).into_response(),
    }
}

async fn console_decide_action(
    state: &Arc<AppState>,
    cfg: &ChannelBridgeConfig,
    actor: &str,
    v: &serde_json::Value,
) -> Response {
    let decision = v.get("decision").and_then(|x| x.as_str()).unwrap_or("");
    if decision != "approve" && decision != "reject" {
        return HandlerError::bad_request(
            "console_decision_invalid",
            "decision must be approve|reject",
        )
        .into_response();
    }
    let proposal_id = v.get("proposal_id").and_then(|x| x.as_i64()).unwrap_or(0);
    if proposal_id <= 0 {
        return HandlerError::bad_request(
            "console_proposal_invalid",
            "proposal_id must be positive",
        )
        .into_response();
    }
    let digest = v
        .get("digest")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        // THE binding law, kernel-side gate #1: a decision without the exact
        // rendered digest never reaches the approve verb.
        console_audit(
            state,
            actor,
            &format!("proposal:{proposal_id}"),
            crate::audit::AuditStatus::Denied,
            "console_decide_digest_missing",
        );
        return HandlerError::bad_request(
            "digest_required",
            "decide must carry the content_digest it was displayed with",
        )
        .into_response();
    }
    let Some(actor_ref) = bounded_actor_ref(v) else {
        console_audit(
            state,
            actor,
            "console",
            crate::audit::AuditStatus::Denied,
            "actor_ref_invalid",
        );
        return HandlerError::bad_request(
            "actor_ref_invalid",
            "actor_ref must be an opaque platform id",
        )
        .into_response();
    };
    let capability = if decision == "approve" {
        "approve"
    } else {
        "reject"
    };
    let pool = state.pool.clone();
    let cfg2 = cfg.clone();
    let actor2 = actor_ref.clone();
    let resolved =
        tokio::task::spawn_blocking(move || -> Result<(String, Vec<String>), &'static str> {
            let conn = pool.get().map_err(|_| "pool_unavailable")?;
            resolve_console_actor(&conn, &cfg2, &actor2, capability)
        })
        .await;
    let (principal_label, roles) = match resolved {
        Ok(Ok(pair)) => pair,
        Ok(Err(reason)) => {
            console_audit(
                state,
                actor,
                &format!("proposal:{proposal_id}"),
                crate::audit::AuditStatus::Denied,
                &format!("console_decide_refused:{reason}"),
            );
            return HandlerError::forbidden(crate::auth::Action::Write, &cfg.domain, "global")
                .into_response();
        }
        Err(e) => return HandlerError::internal(format!("{e}")).into_response(),
    };

    // Reuse the HTTP console's OWN verbs — the same CAS, the same digest
    // check (kernel-side gate #2), the same audit chain. The mapped
    // principal acts exactly as a logged-in reviewer would.
    let principal = console_principal(cfg, &principal_label, roles, true);
    let decision_response = if decision == "approve" {
        let fut = approve_proposal(
            State(Arc::clone(state)),
            OptPrincipal(Some(principal)),
            axum::extract::Path(proposal_id),
            axum::extract::Query(ApproveQuery {
                supersedes: None,
                digest: Some(digest),
            }),
        );
        fut.await
            .map(|Json(body)| (StatusCode::OK, Json(body)).into_response())
    } else {
        let fut = reject_proposal(
            State(Arc::clone(state)),
            OptPrincipal(Some(principal)),
            axum::extract::Path(proposal_id),
        );
        fut.await
            .map(|Json(body)| (StatusCode::OK, Json(body)).into_response())
    };
    decision_response.unwrap_or_else(IntoResponse::into_response)
}

async fn console_due_action(
    state: &Arc<AppState>,
    cfg: &ChannelBridgeConfig,
    actor: &str,
    v: &serde_json::Value,
) -> Response {
    let Some(actor_ref) = bounded_actor_ref(v) else {
        console_audit(
            state,
            actor,
            "console",
            crate::audit::AuditStatus::Denied,
            "actor_ref_invalid",
        );
        return HandlerError::bad_request(
            "actor_ref_invalid",
            "actor_ref must be an opaque platform id",
        )
        .into_response();
    };
    let pool = state.pool.clone();
    let cfg2 = cfg.clone();
    let resolved =
        tokio::task::spawn_blocking(move || -> Result<(String, Vec<String>), &'static str> {
            let conn = pool.get().map_err(|_| "pool_unavailable")?;
            resolve_console_actor(&conn, &cfg2, &actor_ref, "read")
        })
        .await;
    let (principal_label, _roles) = match resolved {
        Ok(Ok(pair)) => pair,
        Ok(Err(reason)) => {
            console_audit(
                state,
                actor,
                "console:due",
                crate::audit::AuditStatus::Denied,
                reason,
            );
            return HandlerError::forbidden(crate::auth::Action::Read, &cfg.domain, "global")
                .into_response();
        }
        Err(e) => return HandlerError::internal(format!("{e}")).into_response(),
    };
    let now = chrono::Utc::now().timestamp();
    let pool = state.pool.clone();
    let shaped =
        tokio::task::spawn_blocking(move || -> Result<(Vec<serde_json::Value>, usize), String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            channels::console_due(&conn, now).map_err(|e| format!("{e}"))
        })
        .await;
    console_audit(
        state,
        actor,
        "console:due",
        crate::audit::AuditStatus::Ok,
        &format!("principal:{principal_label}"),
    );
    match shaped {
        Ok(Ok((due, total))) => (
            StatusCode::OK,
            Json(serde_json::json!({ "due": due, "count": due.len(), "total": total })),
        )
            .into_response(),
        Ok(Err(e)) => HandlerError::internal(e).into_response(),
        Err(e) => HandlerError::internal(format!("{e}")).into_response(),
    }
}

async fn console_crank_action(
    state: &Arc<AppState>,
    cfg: &ChannelBridgeConfig,
    actor: &str,
    v: &serde_json::Value,
) -> Response {
    let Some(actor_ref) = bounded_actor_ref(v) else {
        console_audit(
            state,
            actor,
            "console",
            crate::audit::AuditStatus::Denied,
            "actor_ref_invalid",
        );
        return HandlerError::bad_request(
            "actor_ref_invalid",
            "actor_ref must be an opaque platform id",
        )
        .into_response();
    };
    let run_id = v.get("run_id").and_then(|x| x.as_i64()).unwrap_or(0);
    if run_id <= 0 {
        return HandlerError::bad_request("crank_run_invalid", "run_id must be positive")
            .into_response();
    }
    let max_steps = v
        .get("max_steps")
        .and_then(|x| x.as_u64())
        .map(|n| n.min(MAX_CONSOLE_CRANK_STEPS as u64) as u32)
        .unwrap_or(MAX_CONSOLE_CRANK_STEPS);
    let pool = state.pool.clone();
    let cfg2 = cfg.clone();
    let resolved =
        tokio::task::spawn_blocking(move || -> Result<(String, Vec<String>), &'static str> {
            let conn = pool.get().map_err(|_| "pool_unavailable")?;
            resolve_console_actor(&conn, &cfg2, &actor_ref, "write")
        })
        .await;
    let (principal_label, _roles) = match resolved {
        Ok(Ok(pair)) => pair,
        Ok(Err(reason)) => {
            console_audit(
                state,
                actor,
                "console:crank",
                crate::audit::AuditStatus::Denied,
                reason,
            );
            return HandlerError::forbidden(crate::auth::Action::Write, &cfg.domain, "global")
                .into_response();
        }
        Err(e) => return HandlerError::internal(format!("{e}")).into_response(),
    };

    // The crank is the steward-harness act the `brain workflow crank` CLI
    // performs — same binary resolution, bounded steps, ONE timeout window.
    // It runs on the kernel host because THAT is where the engine lives; the
    // channel seam merely relays an operator's role-checked command.
    let harness = match resolve_harness_bin() {
        Ok(p) => p,
        Err(HarnessBinRefusal::RelativeOverride(v)) => {
            console_audit(
                state,
                actor,
                "console:crank",
                crate::audit::AuditStatus::Denied,
                "harness_relative_override_refused",
            );
            return HandlerError::internal(format!(
                "BRAIN_STEWARD_BIN='{v}' must be an ABSOLUTE path — PATH is never consulted \
                 (install steward-harness beside the kernel binary, or name it absolutely)"
            ))
            .into_response();
        }
        Err(HarnessBinRefusal::NotFound) => {
            console_audit(
                state,
                actor,
                "console:crank",
                crate::audit::AuditStatus::Denied,
                "harness_missing",
            );
            return HandlerError::internal("steward-harness binary not found beside the kernel")
                .into_response();
        }
    };
    let cmd = serde_json::json!({ "cmd": "crank", "run_id": run_id, "max_steps": max_steps });
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(CONSOLE_CRANK_TIMEOUT_SECS),
        run_harness_crank(harness, cmd),
    )
    .await;
    let report = match outcome {
        Ok(Ok(value)) => value,
        Ok(Err(e)) => {
            console_audit(
                state,
                actor,
                "console:crank",
                crate::audit::AuditStatus::Denied,
                &format!("crank_failed:{e}"),
            );
            return HandlerError::internal(format!("crank failed: {e}")).into_response();
        }
        Err(_) => {
            console_audit(
                state,
                actor,
                "console:crank",
                crate::audit::AuditStatus::Denied,
                "crank_timeout",
            );
            return HandlerError::internal("crank exceeded its timeout window").into_response();
        }
    };
    console_audit(
        state,
        actor,
        "console:crank",
        crate::audit::AuditStatus::Ok,
        &format!(
            "principal:{principal_label} run:{run_id} steps:{}",
            report["steps_executed"]
        ),
    );
    (StatusCode::OK, Json(report)).into_response()
}

/// Resolve the crank's binary (X-M4): the ABSOLUTE `BRAIN_STEWARD_BIN`
/// override, then beside-the-kernel-binary. The PATH scan is DELETED — a
/// writable PATH entry in the service context must never become arbitrary
/// code execution as the service user. A RELATIVE override is refused with
/// the requirement named (the operator asked for a specific binary; silently
/// falling to the exe-dir twin would be a lie about what ran).
#[derive(Debug, PartialEq)]
pub(crate) enum HarnessBinRefusal {
    /// `BRAIN_STEWARD_BIN` was set to a relative path — never resolved.
    RelativeOverride(String),
    /// No usable override and nothing beside the kernel binary.
    NotFound,
}

pub(crate) fn resolve_harness_bin() -> Result<std::path::PathBuf, HarnessBinRefusal> {
    if let Ok(override_bin) = std::env::var("BRAIN_STEWARD_BIN")
        && !override_bin.trim().is_empty()
    {
        let p = std::path::PathBuf::from(&override_bin);
        if !p.is_absolute() {
            return Err(HarnessBinRefusal::RelativeOverride(override_bin));
        }
        if p.exists() {
            return Ok(p);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let p = dir.join("steward-harness");
        if p.exists() {
            return Ok(p);
        }
    }
    Err(HarnessBinRefusal::NotFound)
}

async fn run_harness_crank(
    harness: std::path::PathBuf,
    cmd: serde_json::Value,
) -> Result<serde_json::Value, String> {
    run_harness_crank_bounded(
        harness,
        cmd,
        std::time::Duration::from_secs(CONSOLE_CRANK_TIMEOUT_SECS),
    )
    .await
}

/// The bounded crank — the timeout is a parameter so tests can scale the
/// 60 s window down without shipping a 60-second test (X-M5: the child dies
/// with its budget — `kill_on_drop` reaps it when the timeout drops the
/// future; cap/timeout semantics otherwise unchanged).
async fn run_harness_crank_bounded(
    harness: std::path::PathBuf,
    cmd: serde_json::Value,
    budget: std::time::Duration,
) -> Result<serde_json::Value, String> {
    use tokio::io::AsyncWriteExt;
    let outcome = tokio::time::timeout(budget, async move {
        let mut child = tokio::process::Command::new(&harness)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", harness.display()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(format!("{cmd}\n").as_bytes())
                .await
                .map_err(|e| format!("write stdin: {e}"))?;
            stdin
                .shutdown()
                .await
                .map_err(|e| format!("close stdin: {e}"))?;
        }
        child
            .wait_with_output()
            .await
            .map_err(|e| format!("wait harness: {e}"))
    })
    .await;
    let out = match outcome {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(e),
        // The timeout dropped the spawn future — `kill_on_drop(true)` sent
        // SIGKILL and reaped the child: nothing outlives its budget.
        Err(_) => return Err("crank exceeded its timeout window".to_string()),
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let snippet: String = stderr.chars().take(200).collect();
        return Err(format!("harness exited with {}: {snippet}", out.status));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|_| "harness stdout was not json".to_string())?;
    // Refs only: stopped_at + steps_executed ride; step detail stays in the
    // harness/kernel lineage, never in the channel.
    Ok(serde_json::json!({
        "stopped_at": parsed.get("stopped_at").cloned().unwrap_or(serde_json::Value::Null),
        "steps_executed": parsed.get("steps_executed").cloned().unwrap_or(serde_json::Value::Null),
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The kill-switch reaches the console (v1.28.76): a mapped, role-
    /// holding actor whose principal sits in `revoked_principals` is
    /// refused (`actor_revoked`) BEFORE the capability check — the bridge
    /// HMAC proves the message, not the actor's standing. Fixtures ride the
    /// production cores (role::upsert, apply_user_map_change,
    /// revoke_principal) — the no-SQL-in-handlers law covers tests too.
    #[test]
    fn console_actor_revoked_refused() {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migration::run_migration(&mut conn, 1).unwrap();
        crate::role::upsert(
            &conn,
            &crate::role::Role {
                name: "supervisor".to_string(),
                description: None,
                scopes: vec!["private".to_string()],
                owner_filter: "all".to_string(),
                can: vec!["read".to_string(), "approve".to_string()],
                panels_default: None,
                panels_hidden: None,
                tools_allowed: None,
            },
        )
        .unwrap();
        crate::workflow::channels::apply_user_map_change(
            &conn,
            &crate::workflow::channels::UserMapChange {
                action: "add".to_string(),
                channel: "whatsapp".to_string(),
                tenant: "acme".to_string(),
                platform_user_id: "UOPERATOR".to_string(),
                principal: "ops@acme".to_string(),
                roles: vec!["supervisor".to_string()],
            },
            "seed",
            100,
        )
        .unwrap();
        let cfg = ChannelBridgeConfig {
            kind: "whatsapp".to_string(),
            tenant: "acme".to_string(),
            domain: "acme".to_string(),
            webhook_secret: b"secret".to_vec(),
        };
        let (principal, roles) = resolve_console_actor(&conn, &cfg, "UOPERATOR", "approve")
            .expect("mapped + roled actor resolves");
        assert_eq!(principal, "ops@acme");
        assert_eq!(roles, vec!["supervisor".to_string()]);
        crate::workflow::mesh::revoke_principal(&conn, "ops@acme", "second-pass drill", "op", 200)
            .unwrap();
        assert_eq!(
            resolve_console_actor(&conn, &cfg, "UOPERATOR", "approve"),
            Err("actor_revoked"),
            "a revoked actor must not list or decide through the bridge"
        );
    }

    /// Env-mutating tests serialize on this lock (the hostcalls posture:
    /// env reads are process-global). Poison-tolerant so a panicking sibling
    /// cannot cascade.
    static CRANK_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn crank_env_lock() -> std::sync::MutexGuard<'static, ()> {
        CRANK_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Set + restore one env var (SAFETY: single-threaded under the lock).
    struct EnvGuard(&'static str);
    impl EnvGuard {
        fn set(name: &'static str, value: &str) -> EnvGuard {
            // SAFETY: single-threaded under CRANK_ENV_LOCK.
            unsafe { std::env::set_var(name, value) };
            EnvGuard(name)
        }
        fn remove(name: &'static str) -> EnvGuard {
            // SAFETY: single-threaded under CRANK_ENV_LOCK.
            unsafe { std::env::remove_var(name) };
            EnvGuard(name)
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: single-threaded under CRANK_ENV_LOCK.
            unsafe { std::env::remove_var(self.0) };
        }
    }

    /// macOS Sonoma+ gives freshly-written executables a
    /// `com.apple.provenance` xattr and Gatekeeper SIGKILLs the FIRST exec
    /// (exit 137) — the install-service.sh lesson. Test stubs strip it
    /// after writing; on Linux there is no xattr binary and this is a
    /// silent no-op.
    fn write_executable(path: &std::path::Path, content: &str) {
        std::fs::write(path, content).expect("write stub");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.provenance"])
            .arg(path)
            .status();
    }

    /// `relative_steward_bin_refuses` — a RELATIVE `BRAIN_STEWARD_BIN` is
    /// refused with the requirement named; the exe-dir twin is never
    /// silently consulted for an override that cannot be honored.
    #[test]
    fn relative_steward_bin_refuses() {
        let _g = crank_env_lock();
        let _bin = EnvGuard::set(
            "BRAIN_STEWARD_BIN",
            "tools/steward-harness/target/debug/harness",
        );
        let refusal = resolve_harness_bin().expect_err("relative override must refuse");
        match refusal {
            HarnessBinRefusal::RelativeOverride(v) => {
                assert_eq!(v, "tools/steward-harness/target/debug/harness");
            }
            other => panic!("wrong refusal: {other:?}"),
        }
    }

    /// `path_lookup_never_consulted` — a shadowing `steward-harness` planted
    /// in a PATH directory is NEVER returned: the PATH scan is deleted, so
    /// with no override and nothing beside the kernel the resolution is a
    /// named refusal, not whatever a writable PATH entry offers.
    #[test]
    fn path_lookup_never_consulted() {
        let _g = crank_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let planted = dir.path().join("steward-harness");
        std::fs::write(&planted, b"#!/bin/sh\ntouch shadow-ran\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _bin = EnvGuard::remove("BRAIN_STEWARD_BIN");
        let _path = EnvGuard::set("PATH", dir.path().to_str().unwrap());
        let refusal = resolve_harness_bin().expect_err("PATH must never be consulted");
        assert_eq!(refusal, HarnessBinRefusal::NotFound, "{refusal:?}");
        assert!(planted.exists(), "the shadow binary sits unexecuted");
    }

    /// `exe_dir_fallback_still_works` — with no override, a steward-harness
    /// installed beside the (test) binary is the resolution (the as-rooted
    /// fallback the plan keeps).
    #[test]
    fn exe_dir_fallback_still_works() {
        let _g = crank_env_lock();
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().unwrap().to_path_buf();
        let planted = dir.join("steward-harness");
        // Defense: if a stray file is already there, adopt it for the test
        // and restore rather than clobber.
        let original = std::fs::read(&planted).ok();
        std::fs::write(&planted, b"#!/bin/sh\nexit 0\n").unwrap();
        let _bin = EnvGuard::remove("BRAIN_STEWARD_BIN");
        let resolved = resolve_harness_bin();
        match &original {
            Some(bytes) => {
                let _ = std::fs::write(&planted, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&planted);
            }
        }
        let resolved = resolved.expect("the beside-the-binary install must resolve");
        assert_eq!(resolved, planted);
    }

    /// `crank_success_path_unchanged` — the success path is byte-identical
    /// to the pre-Deadbolt shape: valid JSON stdin/stdout, the refs-only
    /// projection (stopped_at + steps_executed, nothing else rides).
    #[tokio::test]
    async fn crank_success_path_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let stub = dir.path().join("steward-harness");
        write_executable(
            &stub,
            "#!/bin/sh\nread -r line\necho '{\"stopped_at\":\"s3\",\"steps_executed\":3,\"secret_detail\":\"DROP\"}'\n",
        );
        let report = run_harness_crank_bounded(
            stub,
            serde_json::json!({"cmd": "crank", "run_id": 7}),
            std::time::Duration::from_secs(5),
        )
        .await
        .expect("the success path is unchanged");
        assert_eq!(report["stopped_at"], "s3", "{report}");
        assert_eq!(report["steps_executed"], 3, "{report}");
        assert!(
            report.get("secret_detail").is_none(),
            "refs-only projection holds: {report}"
        );
    }

    /// `crank_timeout_kills_child` (X-M5, scaled) — a sleep-harness records
    /// its pid, then sleeps far past the (300 ms) budget. The timeout drops
    /// the future; `kill_on_drop(true)` reaps the child: within a short
    /// poll the recorded pid is gone (`kill -0` fails).
    ///
    /// The warmup leg absorbs a real-machinery quirk: the FIRST exec of a
    /// freshly-written binary on macOS can be delayed seconds by a
    /// Gatekeeper/syspolicyd scan (measured 0.3–2 s on the dev host —
    /// sibling of the install-service.sh provenance-xattr lesson). Warm
    /// first, measure second: the scaled window then tests only kill
    /// semantics.
    #[tokio::test]
    async fn crank_timeout_kills_child() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let stub = dir.path().join("steward-harness");
        let pid_path = pid_file.display().to_string();
        write_executable(
            &stub,
            &format!("#!/bin/sh\necho $$ > \"{pid_path}\"\nexec sleep 30\n"),
        );

        // Warmup: run the stub once (stdin closed), wait until it actually
        // ran (the pid file appears), kill + reap it, clear the pid file.
        let mut warm = tokio::process::Command::new(&stub)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .spawn()
            .expect("warmup spawn");
        let warmup_deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !pid_file.exists() {
            assert!(
                std::time::Instant::now() < warmup_deadline,
                "warmup child never ran (first-exec scan exceeded 15 s)"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        warm.kill().await.expect("warmup kill");
        warm.wait().await.expect("warmup reap");
        std::fs::remove_file(&pid_file).expect("clear warmup pid file");

        // Measured window (300 ms): a warm exec records its pid within
        // milliseconds, so the file is present when the timeout fires.
        let started = std::time::Instant::now();
        let err = run_harness_crank_bounded(
            stub,
            serde_json::json!({"cmd": "crank"}),
            std::time::Duration::from_millis(300),
        )
        .await
        .expect_err("the sleep-harness must hit the budget");
        assert!(err.contains("timeout"), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(10));

        // The child was killed + reaped: `kill -0 <pid>` must fail shortly
        // after the timeout (poll: tokio's orphan reaper is asynchronous).
        let pid = std::fs::read_to_string(&pid_file)
            .expect("the measured child recorded its pid")
            .trim()
            .to_string();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let alive = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("kill -0 {pid} 2>/dev/null"))
                .status()
                .expect("sh")
                .success();
            if !alive {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child pid {pid} outlived its budget — kill_on_drop did not reap"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}
