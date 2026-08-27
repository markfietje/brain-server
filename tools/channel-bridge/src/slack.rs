//! Slack Socket Mode adapter — the dial-OUT edge.
//!
//! LAW (`socket_mode_never_opens_an_inbound_listener`): this adapter owns
//! NO inbound socket of any kind. Nothing here listens or accepts: the
//! process DIALS Slack over WSS (Socket Mode) and reads events from that
//! stream only, alongside a drain-crank ticker. The loopback law holds
//! STRUCTURALLY and is pinned by a source-text test.
//!
//! Frame discipline: EVERY Socket Mode envelope is ACKed before any
//! processing (Slack kills sockets that stay silent ~3s); one malformed
//! frame never closes the connection. Frames are size-bounded; unmapped
//! channels NEVER cross the seam (drop + debug log).
//!
//! Approvals obey the DIGEST LAW: buttons carry the digest rendered with
//! the proposal; `console::digest_gate` refuses anything the render cache
//! has not seen, BEFORE any kernel relay. Slash commands are relays only —
//! role enforcement is KERNEL-side (the console seam maps the platform id
//! to a principal and role-checks there); the bridge surfaces the kernel's
//! error text verbatim on refusal.
//!
//! Reconnect law: on close/error/failed dial, exponential backoff with
//! jitter (capped), then a FRESH `apps.connections.open` — never a tight
//! loop, never a reused (single-use) socket URL.

use crate::App;
use crate::console;
use crate::render::{self, NoteDraft};
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type WsSink = futures_util::stream::SplitSink<WsStream, Message>;

/// Largest Socket Mode frame we will even look at (bounds law).
const MAX_FRAME_BYTES: usize = 1_048_576;
/// Reconnect cap: 30s total, exponential base + jitter.
const MAX_BACKOFF_MS: u64 = 30_000;

/// The Slack edge runtime: dial, serve, redial. Never returns; a dead
/// socket is a backoff-and-redial, never a process exit and never a bind.
pub(crate) async fn run(app: App, tick_secs: u64) -> Result<()> {
    let cache = console::RenderCache::new();
    let mut attempt: u32 = 0;
    loop {
        match dial(&app).await {
            Ok(ws) => {
                tracing::info!("socket mode connected (dial-out only; no inbound surface)");
                attempt = 0;
                serve(&app, &cache, ws, tick_secs).await;
                tracing::warn!("socket mode stream ended");
            }
            Err(e) => tracing::error!("socket mode dial failed: {e:#}"),
        }
        let delay = backoff_delay(attempt);
        tracing::info!(
            attempt,
            delay_ms = delay.as_millis() as u64,
            "backing off before redial"
        );
        tokio::time::sleep(delay).await;
        attempt = attempt.saturating_add(1);
    }
}

/// Fresh Socket Mode URL per connection: `apps.connections.open` mints a
/// single-use wss URL; the dial refuses anything that is not wss://.
async fn dial(app: &App) -> Result<WsStream> {
    let cfg = app.cfg.slack()?;
    let token = String::from_utf8_lossy(&cfg.app_token).to_string();
    let resp = app
        .http
        .post("https://slack.com/api/apps.connections.open")
        .bearer_auth(token)
        .send()
        .await
        .context("slack apps.connections.open unreachable")?;
    let v: Value = resp
        .json()
        .await
        .context("slack apps.connections.open returned non-json")?;
    if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        let err = v.get("error").and_then(|x| x.as_str()).unwrap_or("unknown");
        anyhow::bail!("slack refused the socket mode handshake: {err}");
    }
    let url = v
        .get("url")
        .and_then(|x| x.as_str())
        .context("socket mode response missing wss url")?;
    if !url.starts_with("wss://") {
        anyhow::bail!("socket mode url must be wss:// (transport law)");
    }
    let (ws, _handshake) = connect_async(url)
        .await
        .context("socket mode wss dial failed")?;
    Ok(ws)
}

/// `1s * 2^attempt + jitter`, capped at 30s. Jitter comes from wall-clock
/// nanos (no rng dep); the cap keeps the worst case bounded regardless.
fn backoff_delay(attempt: u32) -> std::time::Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()))
        .unwrap_or(0);
    let jitter = nanos % 500;
    let exp_ms = 1_000u64.saturating_mul(1u64 << attempt.min(16));
    let total = exp_ms.saturating_add(jitter).min(MAX_BACKOFF_MS);
    std::time::Duration::from_millis(total)
}

/// One connection's life: `tokio::select!` over the ws stream and the drain
/// crank ticker. Returns when the socket dies; `run` redials with backoff.
async fn serve(app: &App, cache: &console::RenderCache, ws: WsStream, tick_secs: u64) {
    let (mut sink, mut stream) = ws.split();
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(tick_secs.max(1)));
    loop {
        tokio::select! {
            item = stream.next() => match item {
                Some(Ok(Message::Text(txt))) => {
                    if txt.len() <= MAX_FRAME_BYTES {
                        handle_text(app, cache, &mut sink, &txt).await;
                    } else {
                        tracing::warn!("oversized socket-mode frame dropped (bounds law)");
                    }
                }
                Some(Ok(Message::Ping(p))) => {
                    if let Err(e) = sink.send(Message::Pong(p)).await {
                        tracing::error!("pong send failed; socket unusable: {e}");
                        break;
                    }
                }
                Some(Ok(Message::Close(c))) => {
                    tracing::info!(?c, "slack closed the socket mode stream");
                    break;
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    tracing::error!("socket mode read error: {e}");
                    break;
                }
                None => break,
            },
            _ = ticker.tick() => {
                if let Err(e) = crank(app).await {
                    tracing::error!("drain crank failed loudly: {e:#}");
                }
            }
        }
    }
}

async fn ack(sink: &mut WsSink, envelope_id: &str) -> Result<()> {
    sink.send(Message::text(
        json!({"envelope_id": envelope_id}).to_string(),
    ))
    .await
    .context("socket mode ack send failed")
}

async fn handle_text(app: &App, cache: &console::RenderCache, sink: &mut WsSink, txt: &str) {
    let Ok(frame) = serde_json::from_str::<Value>(txt) else {
        // One bad frame is never worth the socket: log + continue.
        tracing::warn!("malformed socket-mode frame ignored (socket stays up)");
        return;
    };
    let Some(envelope_id) = frame
        .get("envelope_id")
        .and_then(|x| x.as_str())
        .map(str::to_string)
    else {
        tracing::warn!("socket-mode frame without envelope_id ignored");
        return;
    };
    // ACK FIRST — before ANY processing.
    if let Err(e) = ack(sink, &envelope_id).await {
        tracing::error!("ack failed; socket unusable: {e:#}");
        return;
    }
    let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
    match frame.get("type").and_then(|x| x.as_str()).unwrap_or("") {
        "events_api" => handle_events_api(app, &payload).await,
        "interactivity" => handle_interactivity(app, cache, &payload).await,
        "slash_commands" => handle_slash(app, cache, &payload).await,
        other => tracing::debug!(frame_type = other, "socket-mode frame type not handled"),
    }
}

async fn handle_events_api(app: &App, payload: &Value) {
    if payload.get("type").and_then(|x| x.as_str()) != Some("event_callback") {
        return;
    }
    let Some(event) = payload.get("event") else {
        return;
    };
    let Ok(cfg) = app.cfg.slack() else {
        return;
    };
    let Some(draft) = project_message(&cfg.mapped_channels, event) else {
        return;
    };
    let envelope = crate::envelope_json(&draft);
    match crate::forward_envelope(app, &envelope).await {
        Ok(code) => tracing::debug!(code, "slack envelope forwarded to kernel"),
        Err(e) => tracing::error!("slack forward failed (loud): {e:#}"),
    }
}

/// PURE projection: a mapped human `message` event → the locked envelope
/// draft. Display-name-bearing fields (`user_profile`, user objects) are
/// NEVER read — actor_ref is the raw platform id and nothing else.
pub(crate) fn project_message(mapped_channels: &[String], event: &Value) -> Option<NoteDraft> {
    if event.get("type").and_then(|x| x.as_str()) != Some("message") {
        return None;
    }
    // Bot-authored events never become case notes.
    if event.get("bot_id").is_some() {
        return None;
    }
    let subtype = event.get("subtype").and_then(|x| x.as_str()).unwrap_or("");
    if !subtype.is_empty() && subtype != "message_replied" {
        return None;
    }
    let text = event
        .get("text")
        .and_then(|x| x.as_str())
        .filter(|t| !t.is_empty())?;
    let user = event
        .get("user")
        .and_then(|x| x.as_str())
        .filter(|u| render::bounded_ref(u))?;
    let channel = event.get("channel").and_then(|x| x.as_str())?;
    if !mapped_channels.iter().any(|m| m == channel) {
        tracing::debug!(channel, "unmapped slack channel dropped (never crosses)");
        return None;
    }
    let ts = event
        .get("ts")
        .and_then(|x| x.as_str())
        .filter(|t| render::bounded_ref(t))?;
    // Threaded replies carry their own stable message id under
    // `message.ts`; prefer it so kernel replay-capping stays per-reply.
    let external_id = match subtype {
        "message_replied" => event
            .pointer("/message/ts")
            .and_then(|x| x.as_str())
            .filter(|t| render::bounded_ref(t))
            .unwrap_or(ts),
        _ => ts,
    };
    Some(NoteDraft {
        conversation_ref: channel.to_string(),
        text: render::sanitize_text(text),
        external_id: external_id.to_string(),
        actor_ref: user.to_string(),
    })
}

async fn handle_interactivity(app: &App, cache: &console::RenderCache, payload: &Value) {
    if payload.get("type").and_then(|x| x.as_str()) != Some("block_actions") {
        return;
    }
    let Some(actor_ref) = payload
        .pointer("/user/id")
        .and_then(|x| x.as_str())
        .filter(|s| render::bounded_ref(s))
        .map(str::to_string)
    else {
        tracing::warn!("block action without a bounded user id; ignored");
        return;
    };
    let channel = payload
        .pointer("/channel/id")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    for action in payload
        .get("actions")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
        .take(8)
    {
        let Some(value) = action.get("value").and_then(|x| x.as_str()) else {
            continue;
        };
        let Some((approve, proposal_id, digest)) = render::parse_slack_action_value(value) else {
            tracing::warn!("button value failed to parse; ignored (never relayed)");
            continue;
        };
        let verdict = console::handle_button(
            cache,
            approve,
            proposal_id,
            &digest,
            &actor_ref,
            console_relay(app, &actor_ref),
        )
        .await;
        if !channel.is_empty()
            && render::bounded_ref(channel)
            && let Err(e) = post_message(app, channel, &verdict.reply_text).await
        {
            tracing::error!("button confirmation failed loudly: {e:#}");
        }
    }
}

async fn handle_slash(app: &App, cache: &console::RenderCache, payload: &Value) {
    let user_id = payload
        .get("user_id")
        .and_then(|x| x.as_str())
        .filter(|s| render::bounded_ref(s))
        .map(str::to_string);
    let channel_id = payload
        .get("channel_id")
        .and_then(|x| x.as_str())
        .filter(|s| render::bounded_ref(s))
        .map(str::to_string);
    let (Some(user_id), Some(channel_id)) = (user_id, channel_id) else {
        tracing::warn!("slash command missing bounded user/channel ids; ignored");
        return;
    };
    let text = payload
        .get("text")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let tokens: Vec<&str> = text.split_whitespace().take(3).collect();
    let reply = match tokens.as_slice() {
        ["due"] => match console::post_console(
            &app.http,
            &app.brain_url,
            &app.cfg.kind,
            &app.cfg.webhook_secret,
            &console::body_due(&user_id),
        )
        .await
        {
            Ok(resp) => format_due(&resp),
            Err(e) => format!(
                "kernel refused: {}",
                console::snippet_text(&format!("{e:#}"), 160)
            ),
        },
        ["crank", run] => match run.parse::<i64>() {
            Ok(run_id) => match console::post_console(
                &app.http,
                &app.brain_url,
                &app.cfg.kind,
                &app.cfg.webhook_secret,
                &console::body_crank(&user_id, run_id),
            )
            .await
            {
                Ok(resp) => format_crank(run_id, &resp),
                Err(e) => format!(
                    "kernel refused: {}",
                    console::snippet_text(&format!("{e:#}"), 160)
                ),
            },
            Err(_) => "usage: /brain crank <run id>".to_string(),
        },
        ["approve", id] => match id.parse::<i64>() {
            Ok(proposal_id) => match cache.digest_for(proposal_id) {
                None => format!(
                    "no rendered proposal #{proposal_id} in this session — approve via the proposal card buttons"
                ),
                Some(digest) => {
                    let verdict = console::handle_button(
                        cache,
                        true,
                        proposal_id,
                        &digest,
                        &user_id,
                        console_relay(app, &user_id),
                    )
                    .await;
                    verdict.reply_text
                }
            },
            Err(_) => "usage: /brain approve <proposal id>".to_string(),
        },
        ["pending", n] => {
            let limit = n.parse::<i64>().unwrap_or(console::MAX_PENDING_LIMIT);
            render_pending(app, cache, &channel_id, limit).await
        }
        ["pending"] => render_pending(app, cache, &channel_id, console::MAX_PENDING_LIMIT).await,
        _ => USAGE.to_string(),
    };
    if let Err(e) = post_message(app, &channel_id, &reply).await {
        tracing::error!("slash reply failed loudly: {e:#}");
    }
}

/// THE RENDER PATH: relay `pending`, render each proposal as Blocks with
/// its digest, post it, and remember the digest so later button clicks can
/// bind. Without this path the render cache would never fill and the
/// digest law would refuse every approval (fail-closed, by design).
async fn render_pending(
    app: &App,
    cache: &console::RenderCache,
    channel_id: &str,
    limit: i64,
) -> String {
    match console::fetch_pending(
        &app.http,
        &app.brain_url,
        &app.cfg.kind,
        &app.cfg.webhook_secret,
        limit,
    )
    .await
    {
        Ok(proposals) if proposals.is_empty() => "nothing pending".to_string(),
        Ok(proposals) => {
            let mut rendered = 0usize;
            for p in &proposals {
                cache.remember(p.id, &p.digest);
                let payload = json!({
                    "channel": channel_id,
                    "text": format!("proposal #{} ({})", p.id, p.kind),
                    "blocks": render::slack_blocks(p),
                });
                if let Err(e) = post_chat(app, &payload).await {
                    tracing::error!(proposal_id = p.id, "blocks render failed loudly: {e:#}");
                } else {
                    rendered = rendered.saturating_add(1);
                }
            }
            format!("rendered {rendered} proposal(s) — approvals bind to the rendered digests")
        }
        Err(e) => format!(
            "kernel refused: {}",
            console::snippet_text(&format!("{e:#}"), 160)
        ),
    }
}

const USAGE: &str = "/brain usage: pending | due | crank <run id> | approve <proposal id> (approvals bind to rendered proposal digests)";

fn format_due(resp: &Value) -> String {
    let count = resp.get("count").and_then(|x| x.as_i64()).unwrap_or(0);
    let items: Vec<String> = resp
        .get("due")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .take(console::MAX_PENDING_LIMIT as usize)
                .map(|d| {
                    let run = d.get("run_id").and_then(|x| x.as_i64()).unwrap_or(0);
                    let what = d.get("what").and_then(|x| x.as_str()).unwrap_or("");
                    let overdue = d.get("overdue_secs").and_then(|x| x.as_i64()).unwrap_or(0);
                    format!(
                        "• run #{run}: {} (overdue {overdue}s)",
                        console::snippet_text(what, 120)
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if items.is_empty() {
        return "nothing due".to_string();
    }
    format!("{count} due:\n{}", items.join("\n"))
}

fn format_crank(run_id: i64, resp: &Value) -> String {
    let steps = resp
        .get("steps_executed")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let stopped = resp
        .get("stopped_at")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown");
    format!(
        "cranked run #{run_id}: {steps} steps; stopped at {}",
        console::snippet_text(stopped, 80)
    )
}

/// Build the injected decide relay for `console::handle_button`. The
/// returned future is 'static (it owns an `App` clone + actor), so tests
/// can hold it without lifetime gymnastics.
type DecideFut = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>;

fn console_relay(app: &App, actor_ref: &str) -> impl FnOnce(bool, i64, String) -> DecideFut {
    let app = app.clone();
    let actor_ref = actor_ref.to_string();
    move |approve: bool, proposal_id: i64, digest: String| {
        let app = app.clone();
        let actor_ref = actor_ref.clone();
        Box::pin(async move {
            console::post_console(
                &app.http,
                &app.brain_url,
                &app.cfg.kind,
                &app.cfg.webhook_secret,
                &console::body_decide(approve, proposal_id, &digest, &actor_ref),
            )
            .await
        })
    }
}

/// Slack Web API `chat.postMessage` — the ONLY write surface this adapter
/// has toward Slack (least privilege: one bot token, text + blocks only).
pub(crate) async fn post_chat(app: &App, payload: &Value) -> Result<()> {
    let cfg = app.cfg.slack()?;
    let token = String::from_utf8_lossy(&cfg.bot_token).to_string();
    let resp = app
        .http
        .post("https://slack.com/api/chat.postMessage")
        .bearer_auth(token)
        .json(payload)
        .send()
        .await
        .context("slack web api unreachable")?;
    let v: Value = resp
        .json()
        .await
        .context("slack web api returned non-json")?;
    if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        let err = v.get("error").and_then(|x| x.as_str()).unwrap_or("unknown");
        anyhow::bail!("slack chat.postMessage refused: {err}");
    }
    Ok(())
}

pub(crate) async fn post_message(app: &App, channel: &str, text: &str) -> Result<()> {
    post_chat(
        app,
        &json!({"channel": channel, "text": text, "link_names": false}),
    )
    .await
}

/// One slack drain crank: pull the kind's outbox over the HMAC seam and
/// deliver each envelope to its mapped channel; Relay handover pings go to
/// the case channel (or the configured handover channel for roomless
/// cases). Delivery failures log loud; the crank never crashes the loop.
pub(crate) async fn crank(app: &App) -> Result<()> {
    let batch = crate::outbound::drain_batch(app).await?;
    let cfg = app.cfg.slack()?;
    for env in batch.envelopes.iter().take(64) {
        let Some(conversation_ref) = env
            .get("conversation_ref")
            .and_then(|x| x.as_str())
            .filter(|c| render::bounded_ref(c))
        else {
            tracing::warn!("drained envelope without a bounded conversation_ref; dropped");
            continue;
        };
        if !cfg.mapped_channels.iter().any(|m| m == conversation_ref) {
            tracing::warn!(
                conversation_ref,
                "drained envelope for an unmapped channel; dropped"
            );
            continue;
        }
        let text = env
            .get("text")
            .and_then(|x| x.as_str())
            .map(render::sanitize_text)
            .unwrap_or_default();
        if let Err(e) = post_message(app, conversation_ref, &text).await {
            tracing::error!(conversation_ref, "slack delivery failed loudly: {e:#}");
        }
    }
    for ping in batch.pings.iter().take(32) {
        let Some((case_target, text)) = render::build_ping_message_slack(ping) else {
            continue;
        };
        let Some(target) = render::ping_target(
            case_target.as_deref().unwrap_or(""),
            cfg.handover_channel.as_deref(),
        ) else {
            tracing::warn!("handover ping with no case channel and no handover_channel; dropped");
            continue;
        };
        if let Err(e) = post_message(app, &target, &text).await {
            tracing::error!(target, "handover ping delivery failed loudly: {e:#}");
        }
    }
    Ok(())
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

    const HEX: &str = "abcdefababcdefababcdefababcdefababcdefababcdefababcdefababcdefab";

    fn proposal() -> render::Proposal {
        render::Proposal {
            id: 42,
            kind: "draft".to_string(),
            content: "quarterly draft".to_string(),
            digest: HEX.to_string(),
        }
    }

    // ── HERALD PIN: mapped_channel_messages_become_notes_with_threading
    //    (bridge half) — mapped human messages project to the locked
    //    envelope shape; unmapped channels and bot messages project to
    //    NOTHING.
    #[test]
    fn mapped_channel_messages_become_notes_with_threading() {
        let mapped = vec!["C0123ABCD".to_string()];
        let event = json!({
            "type": "message",
            "channel": "C0123ABCD",
            "user": "U0PING1",
            "ts": "1712345678.123456",
            "text": "hello kernel"
        });
        let draft = project_message(&mapped, &event).expect("mapped message projects");
        assert_eq!(draft.conversation_ref, "C0123ABCD");
        assert_eq!(draft.actor_ref, "U0PING1");
        assert_eq!(draft.external_id, "1712345678.123456");
        assert_eq!(draft.text, "hello kernel");

        // The locked envelope contract: exactly these four fields.
        let env = crate::envelope_json(&draft);
        let inner = env["envelope"].as_object().unwrap();
        assert_eq!(inner.len(), 4);
        assert_eq!(inner["conversation_ref"], "C0123ABCD");
        assert_eq!(inner["text"], "hello kernel");
        assert_eq!(inner["external_id"], "1712345678.123456");
        assert_eq!(inner["actor_ref"], "U0PING1");

        // Unmapped channel → NOTHING crosses.
        let unmapped = json!({
            "type": "message", "channel": "C999ZZZZ", "user": "U0PING1",
            "ts": "1712345678.123456", "text": "sneak"
        });
        assert!(project_message(&mapped, &unmapped).is_none());

        // Bot-authored message → skipped.
        let bot = json!({
            "type": "message", "channel": "C0123ABCD", "user": "UBOT",
            "bot_id": "B0001", "ts": "1712345678.123457", "text": "beep"
        });
        assert!(project_message(&mapped, &bot).is_none());

        // Non-message subtypes → skipped; message_replied threads through
        // with the REPLY's own stable ts as external_id.
        let file_share = json!({
            "type": "message", "channel": "C0123ABCD", "user": "U0PING1",
            "ts": "1712345678.123458", "subtype": "file_share", "text": "file"
        });
        assert!(project_message(&mapped, &file_share).is_none());
        let reply = json!({
            "type": "message", "channel": "C0123ABCD", "user": "U0PING1",
            "ts": "1712345678.000001", "subtype": "message_replied",
            "text": "a reply",
            "message": {"ts": "1712345699.777777"}
        });
        let draft_reply = project_message(&mapped, &reply).expect("message_replied projects");
        assert_eq!(draft_reply.external_id, "1712345699.777777");

        // Empty text / missing user → nothing.
        assert!(project_message(
            &mapped,
            &json!({"type": "message", "channel": "C0123ABCD", "user": "U0PING1", "ts": "1.1", "text": ""})
        )
        .is_none());
        assert!(
            project_message(
                &mapped,
                &json!({"type": "message", "channel": "C0123ABCD", "ts": "1.1", "text": "anon"})
            )
            .is_none()
        );
    }

    // ── HERALD PIN: slack_button_approval_carries_digest_and_binds — the
    //    rendered button parses back to id+digest, a cache hit binds, and a
    //    tampered digest is refused with NO relay call (counter closure).
    #[tokio::test]
    async fn slack_button_approval_carries_digest_and_binds() {
        let blocks = render::slack_blocks(&proposal());
        let value = blocks[2]["elements"][0]["value"].as_str().unwrap();
        let (approve, id, digest) = render::parse_slack_action_value(value).unwrap();
        assert!(approve && id == 42 && digest == HEX);

        let cache = console::RenderCache::new();
        cache.remember(id, &digest);

        // Bound digest → relayed exactly once.
        let calls = std::cell::Cell::new(0usize);
        let relay = |_: bool, _: i64, _: String| {
            calls.set(calls.get() + 1);
            async { Ok::<Value, anyhow::Error>(json!({"status": "approved"})) }
        };
        let verdict = console::handle_button(&cache, true, id, &digest, "U0PING1", relay).await;
        assert!(verdict.relayed);
        assert_eq!(calls.get(), 1);
        assert!(verdict.reply_text.contains("approved (digest-bound)"));

        // Tampered digest → REFUSED, the kernel is NEVER called.
        let tampered = format!(
            "{}{}",
            if digest.starts_with('a') { "b" } else { "a" },
            &digest[1..]
        );
        let never = std::cell::Cell::new(0usize);
        let refused_relay = |_: bool, _: i64, _: String| {
            never.set(never.get() + 1);
            async { Ok::<Value, anyhow::Error>(Value::Null) }
        };
        let verdict2 =
            console::handle_button(&cache, true, id, &tampered, "U0PING1", refused_relay).await;
        assert!(
            !verdict2.relayed,
            "tampered digest never reaches the kernel"
        );
        assert_eq!(never.get(), 0, "no relay call happened");
        assert!(verdict2.reply_text.contains("refused"));
    }

    #[test]
    fn backoff_is_exponential_capped_and_jittered() {
        for attempt in 0..12u32 {
            let d = backoff_delay(attempt);
            assert!(d.as_millis() <= 30_000, "cap holds at attempt {attempt}");
        }
        let first = backoff_delay(0);
        assert!(first.as_millis() >= 1_000, "base is at least 1s");
        // Monotone-ish growth until the cap (jitter is sub-second).
        assert!(backoff_delay(3).as_millis() >= backoff_delay(1).as_millis());
    }
}
