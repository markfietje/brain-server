//! Herald: the multi-channel governed-edge bridge — one binary, three
//! edges selected by the config FILENAME kind segment
//! (`channel-{kind}-{tenant}.json`, kind ∈ whatsapp | slack | teams).
//!
//! Meta's own platform law IS the discipline — each adapter only ever MAPS
//! its platform law onto brain-server kernel law:
//!
//! 1. INBOUND (platform → bridge → kernel): the PUBLIC webhook endpoint
//!    lives HERE (whatsapp + teams; slack has NO inbound listener by
//!    construction — see `slack.rs`). Platform authentication is verified
//!    by THIS process before any parse (Meta hub signature / Bot Framework
//!    RS256 JWT); the subscription handshake (`hub.challenge`) is answered
//!    HERE, never by the kernel. Verified payloads are translated into
//!    normalized envelopes and forwarded to
//!    `POST /webhooks/channel/{kind}` signed Standard-Webhooks style. The
//!    kernel stays channel-blind: adapters map platform payloads to/from
//!    the envelope, never past this crate.
//! 2. OUTBOUND (kernel → bridge → platform): envelopes are DRAINED
//!    pull-style from the `channel/out` topic over the same HMAC seam and
//!    delivered per-kind (Cloud API / chat.postMessage / Bot Connector
//!    activities). The whatsapp edge additionally paces itself on the
//!    number's quality tier — a FRESH state file is the MOST RESTRICTIVE
//!    tier until a status webhook upgrades it (fail-closed throttle).
//! 3. MEDIA (whatsapp): attachments are downloaded by THIS process, SHA-256
//!    digested, the BYTES quarantined under the retention dir named by
//!    their digest, and only the digest rides the envelope onward.
//!
//! Credentials law (`bridge_holds_no_brain_credentials`, house-wide): the
//! bridge holds ONLY its own 0600 config + secret files + its quarantine/
//! state dirs. No brain token, no brain DB path, ever — the kernel seam
//! authenticates via the Standard-Webhooks HMAC secret alone.

mod config;
mod console;
mod hubsig;
mod outbound;
mod render;
mod slack;
mod teams;
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
    about = "Governed channel edges (WhatsApp/Slack/Teams) for brain-server"
)]
struct Args {
    /// Path to channel-{kind}-{tenant}.json (0600).
    #[arg(long)]
    config: String,

    /// Loopback bind port for the public-facing listener (TLS terminates at
    /// the operator's reverse proxy in front of this port). Unused by the
    /// slack edge, which opens NO listener by construction.
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

    /// Teams edge only: admin-gated, read-only enumeration of the joined
    /// teams + channels via Graph, printing `id<TAB>name` lines for
    /// `mapped_channels`. Never writes.
    #[arg(long)]
    list_channels: bool,
}

/// Structural loopback law: which edges run an inbound listener.
/// Slack → FALSE (Socket Mode dial-OUT only, pinned by
/// `socket_mode_never_opens_an_inbound_listener`); whatsapp/teams → true
/// (loopback listener, TLS at the operator proxy); ANY unknown kind → true
/// (fail-closed: assume a surface must be guarded).
pub(crate) fn needs_inbound_listener(kind: &str) -> bool {
    !matches!(kind, "slack")
}

#[derive(Clone)]
struct App {
    cfg: Arc<config::BridgeConfig>,
    http: reqwest::Client,
    brain_url: Arc<String>,
    retention_dir: Arc<String>,
    /// Whatsapp-only (the tier throttle). Absent on slack/teams edges —
    /// the fail-closed accessor refuses rather than silently skip pacing.
    state: Option<Arc<outbound::TierState>>,
}

/// The locked Slack/Teams envelope projection: platform ids only, bounded
/// text, a stable per-event external id (the kernel replay-caps on it).
pub(crate) fn envelope_json(d: &render::NoteDraft) -> serde_json::Value {
    serde_json::json!({
        "envelope": {
            "conversation_ref": d.conversation_ref,
            "text": d.text,
            "external_id": d.external_id,
            "actor_ref": d.actor_ref,
        }
    })
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

    if args.list_channels {
        if cfg.kind != "teams" {
            anyhow::bail!(
                "--list-channels applies only to the teams edge (config kind {})",
                cfg.kind
            );
        }
        let app = build_app(cfg, &args, None)?;
        return teams::list_channels(&app).await;
    }

    let state = if cfg.kind == "whatsapp" {
        Some(Arc::new(outbound::TierState::open(
            std::path::Path::new(&args.state_dir),
            &cfg.tenant,
        )?))
    } else {
        None
    };
    let app = build_app(cfg, &args, state)?;

    // Registration evidence at boot: the mount records the FULL config-file
    // digest; the kernel recomputes it from ITS copy of the same 0600 file
    // (neither side self-certifies). Failure logs LOUD but does not kill the
    // edge — route discovery still governs every later request.
    match outbound::register_mount(&app).await {
        Ok(code) => tracing::info!(code, "mount evidence accepted by kernel"),
        Err(e) => tracing::error!("mount registration failed (loud): {e:#}"),
    }

    if !needs_inbound_listener(&app.cfg.kind) {
        // Slack: dial-out only — the process runs the Socket Mode loop and
        // the drain-crank ticker and NOTHING else. No socket is opened for
        // inbound traffic, ever.
        return slack::run(app, args.tick_secs).await;
    }
    match app.cfg.kind.as_str() {
        "whatsapp" => serve_whatsapp(app, &args).await,
        "teams" => serve_teams(app, &args).await,
        other => anyhow::bail!("unsupported channel kind {other:?}"),
    }
}

fn build_app(
    cfg: config::BridgeConfig,
    args: &Args,
    state: Option<Arc<outbound::TierState>>,
) -> Result<App> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("http client build failed")?;
    Ok(App {
        cfg: Arc::new(cfg),
        http,
        brain_url: Arc::new(args.brain_url.trim_end_matches('/').to_string()),
        retention_dir: Arc::new(args.retention_dir.clone()),
        state,
    })
}

async fn serve_whatsapp(app: App, args: &Args) -> Result<()> {
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

async fn serve_teams(app: App, args: &Args) -> Result<()> {
    let teams_cfg = app.cfg.teams()?.clone();
    let st = teams::TeamsState {
        app: app.clone(),
        client: Arc::new(teams::BfClient::new(
            app.http.clone(),
            teams_cfg.bot_tenant_id.clone(),
            teams_cfg.bot_app_id.clone(),
            teams_cfg.bot_password.clone(),
        )),
        verifier: Arc::new(teams::BfVerifier::new(
            app.http.clone(),
            teams_cfg.bot_app_id.clone(),
        )),
        cache: Arc::new(console::RenderCache::new()),
    };

    let tick_secs = args.tick_secs;
    tokio::spawn({
        let st = st.clone();
        async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(tick_secs.max(1)));
            loop {
                ticker.tick().await;
                if let Err(e) = teams::crank(&st).await {
                    tracing::error!("drain crank failed loudly: {e:#}");
                }
            }
        }
    });

    let listen = format!("127.0.0.1:{}", args.port);
    let router = axum::Router::new()
        .route("/messaging", axum::routing::post(teams::messaging))
        .with_state(st);
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("bind {listen}"))?;
    tracing::info!(%listen, "teams edge listening (TLS terminates at the operator proxy)");
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
    let wa = match app.cfg.whatsapp() {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("handshake refused (non-whatsapp config): {e:#}");
            return (StatusCode::FORBIDDEN, "forbidden".to_string());
        }
    };
    let mode_ok = q.get("hub.mode").map(String::as_str) == Some("subscribe");
    let sent = q.get("hub.verify_token").map(String::as_str).unwrap_or("");
    if !mode_ok || !hubsig::constant_time_eq(sent.as_bytes(), wa.verify_token.as_bytes()) {
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
    let Ok(wa) = app.cfg.whatsapp() else {
        return deny("non-whatsapp config cannot serve the meta webhook");
    };
    if !hubsig::verify_hub_signature(&wa.app_secret, &body, sig) {
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
            let state = app
                .state
                .as_ref()
                .context("tier state absent on a whatsapp edge")?;
            state.observe(number_alias, new_tier)?;
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
pub(crate) async fn forward_envelope(app: &App, envelope: &serde_json::Value) -> Result<u16> {
    let body = serde_json::to_vec(envelope)?;
    let id = uuid::Uuid::new_v4().to_string();
    let ts = chrono::Utc::now().timestamp().to_string();
    let sig = outbound::sw_sign(&app.cfg.webhook_secret, &id, &ts, &body);
    let url = format!(
        "{}/webhooks/channel/{}",
        app.brain_url.trim_end_matches('/'),
        app.cfg.kind
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::panic,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;

    // ── HERALD PIN: socket_mode_never_opens_an_inbound_listener — the
    //    structural dispatch AND a source-text pin over the slack adapter:
    //    no listener types, no axum routing, no bind anywhere in it.
    #[test]
    fn socket_mode_never_opens_an_inbound_listener() {
        assert!(!needs_inbound_listener("slack"), "slack dials out only");
        assert!(needs_inbound_listener("whatsapp"));
        assert!(needs_inbound_listener("teams"));
        assert!(
            needs_inbound_listener("signal"),
            "unknown kinds fail closed"
        );
        assert!(needs_inbound_listener(""), "empty kind fails closed");

        let src = include_str!("../src/slack.rs");
        for forbidden in ["TcpListener", "axum::routing", "bind("] {
            assert!(
                !src.contains(forbidden),
                "slack adapter source must never contain {forbidden:?}"
            );
        }
    }

    // ── HERALD PIN: user_map_events_never_carry_display_names — BOTH
    //    projections use raw platform ids; display-name fields are never
    //    read into ANY envelope field, even when they carry injections.
    #[test]
    fn user_map_events_never_carry_display_names() {
        // Slack: user_profile/display-name junk present but never read.
        let mapped = vec!["C0123ABCD".to_string()];
        let event = serde_json::json!({
            "type": "message",
            "channel": "C0123ABCD",
            "user": "U0PING1",
            "ts": "1712345678.123456",
            "text": "hello kernel",
            "user_profile": {
                "display_name": "Robert'); DROP TABLE cases;--",
                "real_name": "<svg onload=alert(1)>"
            }
        });
        let draft = slack::project_message(&mapped, &event).expect("projects");
        let env = envelope_json(&draft).to_string();
        assert!(env.contains("U0PING1"), "raw platform id is the actor");
        assert!(!env.contains("Robert"), "display name never crosses");
        assert!(!env.contains("svg onload"), "injection never crosses");

        // Teams: from.name junk present but never read.
        let teams_mapped = vec!["19:abc@thread.tacv2".to_string()];
        let activity = serde_json::json!({
            "type": "message",
            "id": "act-9",
            "from": {
                "id": "29:xyz456",
                "name": "Robert'); DROP TABLE cases;--",
                "aadObjectId": "guid-ish"
            },
            "conversation": {"id": "19:abc@thread.tacv2"},
            "text": "ping"
        });
        let draft2 =
            teams::project_activity(&teams_mapped, "bot-app", &activity).expect("projects");
        assert_eq!(draft2.actor_ref, "29:xyz456");
        let env2 = envelope_json(&draft2).to_string();
        assert!(env2.contains("29:xyz456"));
        assert!(!env2.contains("Robert"), "from.name never crosses");
        assert!(
            !env2.contains("aadObjectId"),
            "extra identity fields never cross"
        );
    }

    #[test]
    fn envelope_shape_is_locked_for_slack_and_teams() {
        let draft = render::NoteDraft {
            conversation_ref: "C0123ABCD".to_string(),
            text: "hello".to_string(),
            external_id: "1712345678.123456".to_string(),
            actor_ref: "U0PING1".to_string(),
        };
        let env = envelope_json(&draft);
        let inner = env["envelope"].as_object().expect("envelope object");
        assert_eq!(inner.len(), 4, "exactly the locked four fields");
        assert_eq!(inner["conversation_ref"], "C0123ABCD");
        assert_eq!(inner["text"], "hello");
        assert_eq!(inner["external_id"], "1712345678.123456");
        assert_eq!(inner["actor_ref"], "U0PING1");
    }
}
