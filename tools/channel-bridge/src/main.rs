//! Caravel: the WhatsApp governed-edge bridge.
//!
//! Meta's own platform law IS the discipline — this adapter only ever MAPS
//! it onto brain-server kernel law:
//!
//! 1. INBOUND (Meta → bridge → kernel): the PUBLIC webhook endpoint lives
//!    HERE. The subscription handshake (`hub.challenge`) is answered by THIS
//!    process, never by the kernel; every POST is verified against
//!    `X-Hub-Signature-256` (raw-body HMAC-SHA256 with the app secret,
//!    length-checked, constant-time). Verified payloads are translated into
//!    normalized envelopes and forwarded to
//!    `POST /webhooks/channel/{kind}` signed Standard-Webhooks style. The
//!    kernel stays channel-blind: adapters map platform payloads to/from the
//!    envelope, never past this crate.
//! 2. OUTBOUND (kernel → bridge → Meta): envelopes are DRAINED pull-style
//!    from the `channel/out` topic over the same HMAC seam and delivered to
//!    the Cloud API. Outside-window sends arrive ONLY as approved
//!    `channel/template` acts (the kernel refuses anything else); the edge
//!    additionally paces itself on the number's quality tier — a FRESH state
//!    file is the MOST RESTRICTIVE tier until a status webhook upgrades it
//!    (fail-closed throttle), and downgrades are reported back upstream so
//!    the operator is alerted metadata-only.
//! 3. MEDIA: attachments are downloaded by THIS process (an operator-run
//!    act — the kernel never proxies media to a browser), SHA-256 digested,
//!    the BYTES quarantined under the retention dir named by their digest,
//!    and only the digest rides the envelope onward.
//!
//! Credentials law (`bridge_holds_no_brain_credentials`, house-wide): the
//! bridge holds ONLY its own 0600 config + secret files + its quarantine/
//! state dirs. No brain token, no brain DB path, ever.

mod config;
mod hubsig;
mod outbound;
mod translate;

use anyhow::{Context, Result};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use clap::Parser;
use std::collections::HashMap;
use std::sync::Arc;

/// CLI surface (mirrors signal-gateway posture: one config, few flags).
#[derive(Parser, Debug)]
#[command(
    name = "channel-bridge",
    about = "WhatsApp governed edge for brain-server"
)]
struct Args {
    /// Path to channel-whatsapp-{tenant}.json (0600).
    #[arg(long)]
    config: String,

    /// Loopback bind port for the public-facing listener (TLS terminates at
    /// the operator's reverse proxy in front of this port).
    #[arg(long, default_value_t = 8791)]
    port: u16,

    /// Kernel base URL (loopback; the bridge authenticates via HMAC only).
    #[arg(long, default_value = "http://127.0.0.1:8765")]
    brain_url: String,

    /// Directory for quarantined media bytes (named by SHA-256 digest).
    #[arg(long, default_value = "/var/lib/brain-server/channel-media")]
    retention_dir: String,

    /// Directory for the per-number throttle/tier state file (0600).
    #[arg(long, default_value = "/var/lib/brain-server/channel-bridge-state")]
    state_dir: String,

    /// Drain crank interval seconds (the pacing floor for sends; the tier
    /// table can only lengthen it, never shorten).
    #[arg(long, default_value_t = 5)]
    tick_secs: u64,
}

#[derive(Clone)]
struct App {
    cfg: Arc<config::BridgeConfig>,
    http: reqwest::Client,
    brain_url: Arc<String>,
    retention_dir: Arc<String>,
    graph_api_version: Arc<String>,
    state: Arc<outbound::TierState>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();

    let cfg = config::BridgeConfig::load(std::path::Path::new(&args.config))
        .with_context(|| format!("config {} refused at load (fail-closed)", args.config))?;
    tracing::info!(kind = %cfg.kind, tenant = %cfg.tenant, "bridge config loaded");

    let state = outbound::TierState::open(std::path::Path::new(&args.state_dir), &cfg.tenant)?;
    let graph_api_version = Arc::new(cfg.graph_api_version.clone());
    let app = App {
        cfg: Arc::new(cfg),
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?,
        brain_url: Arc::new(args.brain_url.trim_end_matches('/').to_string()),
        retention_dir: Arc::new(args.retention_dir.clone()),
        graph_api_version,
        state: Arc::new(state),
    };

    // Registration evidence at boot: the mount records the FULL config-file
    // digest; the kernel recomputes it from ITS copy of the same 0600 file
    // (neither side self-certifies). Failure logs LOUD but does not kill the
    // edge — route discovery still governs every later request.
    match outbound::register_mount(&app).await {
        Ok(code) => tracing::info!(code, "mount evidence accepted by kernel"),
        Err(e) => tracing::error!("mount registration failed (loud): {e:#}"),
    }

    // The drain crank: kernel `channel/out` → Cloud API, tier-paced.
    let tick_secs = args.tick_secs;
    tokio::spawn({
        let app = app.clone();
        async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(tick_secs.max(1)));
            loop {
                ticker.tick().await;
                if let Err(e) = outbound::crank(&app).await {
                    tracing::error!("drain crank failed loudly: {e:#}");
                }
            }
        }
    });

    let listen = format!("127.0.0.1:{}", args.port);
    let router = axum::Router::new()
        .route(
            "/webhooks/channel/whatsapp",
            axum::routing::get(verify).post(receive),
        )
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("bind {listen}"))?;
    tracing::info!(%listen, "whatsapp edge listening");
    axum::serve(listener, router).await?;
    Ok(())
}

/// GET handshake — Meta calls this when the operator subscribes the webhook
/// URL. Answered HERE (the kernel never sees a challenge): token compared
/// constant-time, challenge echoed byte-exact, else 403.
async fn verify(
    State(app): State<App>,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let mode_ok = q.get("hub.mode").map(String::as_str) == Some("subscribe");
    let sent = q.get("hub.verify_token").map(String::as_str).unwrap_or("");
    if !mode_ok || !hubsig::constant_time_eq(sent.as_bytes(), app.cfg.verify_token.as_bytes()) {
        tracing::warn!("handshake refused (mode/token mismatch)");
        return (StatusCode::FORBIDDEN, "forbidden".to_string());
    }
    (
        StatusCode::OK,
        q.get("hub.challenge").cloned().unwrap_or_default(),
    )
}

fn deny(detail: &str) -> Response {
    tracing::warn!(detail, "inbound refused");
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({ "error": "hub_signature_invalid" })),
    )
        .into_response()
}

fn ok_json(v: serde_json::Value) -> Response {
    (StatusCode::OK, axum::Json(v)).into_response()
}

/// POST ingest — verify FIRST (raw-body hub HMAC), parse SECOND, then
/// forward each verified projection as a Standard-Webhooks-signed envelope.
/// A signature failure denies before ANY parse; benign projection gaps
/// return 2xx so Meta does not redeliver unrenderable payloads forever.
async fn receive(State(app): State<App>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let Some(sig) = headers
        .get("x-hub-signature-256")
        .and_then(|h| h.to_str().ok())
    else {
        return deny("missing x-hub-signature-256");
    };
    if !hubsig::verify_hub_signature(&app.cfg.app_secret, &body, sig) {
        return deny("hub signature verification failed");
    }

    let projections = match translate::project(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("payload projected nothing usable: {e}");
            return ok_json(serde_json::json!({ "received": 0 }));
        }
    };

    let mut sent = 0usize;
    for p in projections {
        match materialize(&app, &p).await {
            Ok(envelopes) => {
                for env in envelopes {
                    match forward_envelope(&app, &env).await {
                        Ok(code) => {
                            sent = sent.saturating_add(1);
                            tracing::debug!(code, "envelope forwarded to kernel");
                        }
                        Err(e) => {
                            // Kernel unreachable: Meta redelivers → replay cap
                            // server-side makes re-forwarding idempotent.
                            tracing::error!("forward failed (redeliverable): {e:#}")
                        }
                    }
                }
            }
            Err(e) => tracing::error!("media handling failed loudly: {e:#}"),
        }
    }
    ok_json(serde_json::json!({ "received": sent }))
}

/// Turn ONE platform projection into zero or more kernel-bound envelopes:
/// media projections download + digest + quarantine FIRST (bytes never
/// leave this crate), everything else maps directly.
async fn materialize(app: &App, p: &translate::Projection) -> Result<Vec<serde_json::Value>> {
    match p {
        translate::Projection::Message {
            conversation_ref,
            text,
            external_id,
            media_id,
            mime,
        } => {
            let mut digests: Vec<String> = Vec::new();
            if let Some(mid) = media_id {
                let quarantined = outbound::quarantine_media(app, mid).await?;
                digests.push(quarantined.sha256_hex);
            }
            let text = match text {
                Some(t) => t.clone(),
                None => format!("[attachment {}]", mime.as_deref().unwrap_or("file")),
            };
            Ok(vec![serde_json::json!({
                "envelope": {
                    "conversation_ref": conversation_ref,
                    "text": text,
                    "external_id": external_id,
                    "attachment_digests": digests,
                }
            })])
        }
        translate::Projection::Status {
            conversation_ref,
            state,
            message_ref,
        } => Ok(vec![serde_json::json!({
            "envelope": {
                "conversation_ref": conversation_ref,
                "text": "",
                "external_id": external_for_status(message_ref, state),
                "status": { "state": state, "ref": message_ref },
            }
        })]),
        translate::Projection::Quality {
            number_alias,
            old_tier,
            new_tier,
        } => {
            // Persist LOCALLY first (the throttle reads its own truth), then
            // report upstream so the KERNEL can alert the operator
            // metadata-only (the edge holds no alert-bus credentials).
            app.state.observe(number_alias, new_tier)?;
            Ok(vec![serde_json::json!({
                "envelope": {
                    "conversation_ref": "",
                    "text": "",
                    "external_id": external_for_quality(new_tier),
                    "quality": {
                        "number_alias": number_alias,
                        "old_tier": old_tier,
                        "new_tier": new_tier,
                    },
                }
            })])
        }
    }
}

fn external_for_status(message_ref: &str, state: &str) -> String {
    // Distinct per (ref,state): delivered AND read for one wamid must both
    // land (the kernel's lineage keys are idempotent on exactly this pair).
    format!("{state}:{message_ref}")
}

fn external_for_quality(new_tier: &str) -> String {
    format!("quality:{new_tier}:{}", chrono::Utc::now().timestamp())
}

/// Sign the envelope Standard-Webhooks style and POST it to the kernel's
/// channel seam — the ONLY way platform data crosses into the kernel.
async fn forward_envelope(app: &App, envelope: &serde_json::Value) -> Result<u16> {
    let body = serde_json::to_vec(envelope)?;
    let id = uuid::Uuid::new_v4().to_string();
    let ts = chrono::Utc::now().timestamp().to_string();
    let sig = outbound::sw_sign(&app.cfg.webhook_secret, &id, &ts, &body);
    let url = format!(
        "{}/webhooks/channel/whatsapp",
        app.brain_url.trim_end_matches('/')
    );
    let resp = app
        .http
        .post(&url)
        .header("webhook-id", id)
        .header("webhook-timestamp", ts)
        .header("webhook-signature", sig)
        .body(body)
        .send()
        .await
        .context("kernel channel seam unreachable")?;
    let code = resp.status().as_u16();
    if !(200..300).contains(&code) {
        anyhow::bail!("kernel refused forwarded envelope with {code}");
    }
    Ok(code)
}
