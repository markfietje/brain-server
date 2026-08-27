//! The kernel console seam — the wire contract that turns Slack buttons,
//! Adaptive Card submits, and slash commands into digest-bound decisions on
//! kernel proposals.
//!
//! WIRE LAW: every body below is Standard-Webhooks signed with the config
//! `webhook_secret` (same scheme as the drain; `outbound::sw_sign`) and
//! POSTed to `POST {brain_url}/webhooks/channel/{kind}/console`. The bridge
//! holds NO kernel credential beyond that HMAC secret — the self-credential
//! law holds on this seam like every other.
//!
//! THE DIGEST LAW (enforcement point #1; the kernel re-verifies against
//! stored bytes server-side — two independent gates): a proposal's digest
//! is remembered when the proposal is RENDERED (`RenderCache`); every
//! approve/reject action MUST carry a 64-hex digest that matches the cached
//! digest for that proposal id EXACTLY. Missing or mismatched → the action
//! is refused, logged at warn, and NEVER relayed.

use crate::outbound::sw_sign;
use crate::render;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;

/// Bridge-side ceiling mirroring the kernel's console limit.
pub(crate) const MAX_PENDING_LIMIT: i64 = 25;
/// Render-cache capacity; oldest entry evicted when full.
const RENDER_CACHE_CAP: usize = 256;
const REFUSAL_TEXT: &str =
    "❌ refused: digest mismatch/missing — proposal re-rendered or forged; reload the proposal";

/// Console action bodies (the locked wire contract).
pub(crate) fn body_pending(limit: i64) -> Value {
    json!({"action": "pending", "limit": limit.clamp(1, MAX_PENDING_LIMIT)})
}

pub(crate) fn body_decide(approve: bool, proposal_id: i64, digest: &str, actor_ref: &str) -> Value {
    json!({
        "action": "decide",
        "decision": if approve { "approve" } else { "reject" },
        "proposal_id": proposal_id,
        "digest": digest,
        "actor_ref": actor_ref,
    })
}

pub(crate) fn body_due(actor_ref: &str) -> Value {
    json!({"action": "due", "actor_ref": actor_ref})
}

pub(crate) fn body_crank(actor_ref: &str, run_id: i64) -> Value {
    json!({"action": "crank", "actor_ref": actor_ref, "run_id": run_id})
}

fn proposal_from(v: &Value) -> Option<render::Proposal> {
    let digest = v
        .get("digest")
        .and_then(|x| x.as_str())
        .filter(|d| render::is_hex64(d))?
        .to_ascii_lowercase();
    Some(render::Proposal {
        id: v.get("id").and_then(|x| x.as_i64())?,
        kind: v
            .get("kind")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .chars()
            .take(64)
            .collect(),
        content: v
            .get("content")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        digest,
    })
}

/// Relay `action:pending` and parse the proposals the kernel reports.
/// Proposals whose digest is not 64-hex are SKIPPED with a warn — they can
/// never be approved under the digest law, so they must not render.
pub(crate) async fn fetch_pending(
    http: &reqwest::Client,
    brain_url: &str,
    kind: &str,
    secret: &[u8],
    limit: i64,
) -> Result<Vec<render::Proposal>> {
    let resp = post_console(http, brain_url, kind, secret, &body_pending(limit)).await?;
    let mut out = Vec::new();
    for p in resp
        .get("proposals")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
        .take(MAX_PENDING_LIMIT as usize)
    {
        match proposal_from(p) {
            Some(pr) => out.push(pr),
            None => tracing::warn!(
                "pending proposal with unusable digest; skipped (never renderable for approval)"
            ),
        }
    }
    Ok(out)
}

/// A log-safe snippet: control chars stripped, char-bounded.
pub(crate) fn snippet_text(s: &str, max_chars: usize) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= max_chars {
        return clean;
    }
    let kept: String = clean.chars().take(max_chars).collect();
    format!("{kept}…")
}

/// Where a proposal render was seen, so a LATER click can bind to it.
/// Deliberately NOT `Debug`. Eviction order is INSERTION order (a
/// monotonic sequence, not the wall clock) so same-second renders evict
/// deterministically.
pub(crate) struct RenderCache {
    inner: Mutex<CacheInner>,
}

struct CacheInner {
    map: HashMap<i64, (String, u64)>,
    seq: u64,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderCache {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                map: HashMap::new(),
                seq: 0,
            }),
        }
    }

    /// Record the digest a rendered proposal carried. Oldest-inserted
    /// evicted at cap.
    pub(crate) fn remember(&self, proposal_id: i64, digest: &str) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                tracing::warn!("render cache mutex poisoned; refusing to record renders");
                return;
            }
        };
        if g.map.contains_key(&proposal_id) {
            // Re-render REPLACES the binding (same slot, fresh order).
            g.seq = g.seq.saturating_add(1);
            let seq = g.seq;
            g.map
                .insert(proposal_id, (digest.to_ascii_lowercase(), seq));
            return;
        }
        if g.map.len() >= RENDER_CACHE_CAP
            && let Some(oldest) = g
                .map
                .iter()
                .min_by_key(|(_, (_, seq))| *seq)
                .map(|(k, _)| *k)
        {
            g.map.remove(&oldest);
        }
        g.seq = g.seq.saturating_add(1);
        let seq = g.seq;
        g.map
            .insert(proposal_id, (digest.to_ascii_lowercase(), seq));
    }

    /// The digest this session rendered for a proposal, if any. A poisoned
    /// cache returns None → the digest law refuses relays (fail-closed).
    pub(crate) fn digest_for(&self, proposal_id: i64) -> Option<String> {
        let g = self.inner.lock().ok()?;
        g.map.get(&proposal_id).map(|(d, _)| d.clone())
    }
}

pub(crate) enum DigestVerdict {
    /// Cached digest matches the presented one — the relay may proceed.
    Bound,
    Refused(&'static str),
}

/// THE DIGEST LAW gate. Every approve/reject action MUST (a) carry a
/// 64-hex digest and (b) match the cached digest for that proposal id
/// exactly. Anything else is refused BEFORE any kernel relay.
pub(crate) fn digest_gate(cache: &RenderCache, proposal_id: i64, digest: &str) -> DigestVerdict {
    if !render::is_hex64(digest) {
        return DigestVerdict::Refused("presented digest is not 64 hex chars");
    }
    match cache.digest_for(proposal_id) {
        None => DigestVerdict::Refused("no rendered proposal in this session"),
        Some(cached) if cached == digest.to_ascii_lowercase() => DigestVerdict::Bound,
        Some(_) => DigestVerdict::Refused("digest does not bind the rendered proposal"),
    }
}

/// The outcome of one button/card approval action, ready to post back into
/// the channel. `relayed` is true only when the kernel was actually called.
pub(crate) struct ButtonVerdict {
    pub(crate) relayed: bool,
    pub(crate) reply_text: String,
}

/// Shared approve/reject flow for BOTH adapters (Slack buttons, Adaptive
/// Card submits, and slash-command approvals): digest law first, relay
/// second, channel-facing reply text last. `relay` is injected so tests can
/// assert that a refused action NEVER reaches the kernel.
pub(crate) async fn handle_button<R, Fut>(
    cache: &RenderCache,
    approve: bool,
    proposal_id: i64,
    digest: &str,
    actor_ref: &str,
    relay: R,
) -> ButtonVerdict
where
    R: FnOnce(bool, i64, String) -> Fut,
    Fut: std::future::Future<Output = Result<Value>>,
{
    match digest_gate(cache, proposal_id, digest) {
        DigestVerdict::Refused(reason) => {
            tracing::warn!(
                proposal_id,
                actor_ref,
                reason,
                "DIGEST LAW refused an approval action before any relay (enforcement point #1)"
            );
            ButtonVerdict {
                relayed: false,
                reply_text: REFUSAL_TEXT.to_string(),
            }
        }
        DigestVerdict::Bound => {
            match relay(approve, proposal_id, digest.to_ascii_lowercase()).await {
                Ok(_receipt) => ButtonVerdict {
                    relayed: true,
                    reply_text: if approve {
                        format!("✅ proposal #{proposal_id} approved (digest-bound)")
                    } else {
                        "⛔ rejected".to_string()
                    },
                },
                Err(e) => {
                    // Kernel refusal surfaces to the operator — the ACTION was
                    // relayed (relayed=true) but the kernel declined it.
                    tracing::error!("console decide relay refused loudly: {e:#}");
                    ButtonVerdict {
                        relayed: true,
                        reply_text: format!(
                            "kernel refused: {}",
                            snippet_text(&format!("{e:#}"), 160)
                        ),
                    }
                }
            }
        }
    }
}

/// Sign + POST one console action; non-2xx is a LOUD refusal carrying a
/// bounded body snippet. Returns the parsed kernel JSON receipt.
pub(crate) async fn post_console(
    http: &reqwest::Client,
    brain_url: &str,
    kind: &str,
    secret: &[u8],
    body: &Value,
) -> Result<Value> {
    let payload = serde_json::to_vec(body)?;
    let id = uuid::Uuid::new_v4().to_string();
    let ts = chrono::Utc::now().timestamp().to_string();
    let sig = sw_sign(secret, &id, &ts, &payload);
    let url = format!(
        "{}/webhooks/channel/{kind}/console",
        brain_url.trim_end_matches('/')
    );
    let resp = http
        .post(&url)
        .header("webhook-id", id)
        .header("webhook-timestamp", ts)
        .header("webhook-signature", sig)
        .body(payload)
        .send()
        .await
        .context("kernel console seam unreachable")?;
    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .context("kernel console response unreadable")?;
    if !status.is_success() {
        anyhow::bail!(
            "console refused ({}): {}",
            status,
            snippet_text(&String::from_utf8_lossy(&bytes), 200)
        );
    }
    serde_json::from_slice(&bytes).context("kernel console response is not json")
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
    use axum::routing::post;
    use std::sync::Arc;

    const HEX: &str = "1234123412341234123412341234123412341234123412341234123412341234";

    #[test]
    fn digest_gate_accepts_only_bound_digests() {
        let cache = RenderCache::new();
        cache.remember(42, HEX);

        assert!(
            matches!(digest_gate(&cache, 42, HEX), DigestVerdict::Bound),
            "exact cached digest binds"
        );
        // Uppercase presentation normalizes to the cached lowercase.
        assert!(matches!(
            digest_gate(&cache, 42, &HEX.to_uppercase()),
            DigestVerdict::Bound
        ));
        // Foreign digest for a RENDERED proposal → refused.
        let foreign = format!("{}0", &HEX[..63]);
        assert!(matches!(
            digest_gate(&cache, 42, &foreign),
            DigestVerdict::Refused(_)
        ));
        // Unrendered proposal → refused even with a well-formed digest.
        assert!(matches!(
            digest_gate(&cache, 43, HEX),
            DigestVerdict::Refused(_)
        ));
        // Malformed digest → refused before the cache is even consulted.
        assert!(matches!(
            digest_gate(&cache, 42, "zz"),
            DigestVerdict::Refused(_)
        ));
    }

    #[test]
    fn render_cache_is_capped_with_oldest_evicted() {
        let cache = RenderCache::new();
        for id in 0..(256 + 8) {
            cache.remember(id, &format!("{id:064x}"));
        }
        // The earliest ids were evicted; the newest survive.
        assert_eq!(cache.digest_for(0), None);
        assert_eq!(cache.digest_for(263), Some(format!("{:064x}", 263)));
    }

    #[test]
    fn body_builders_match_the_locked_contract() {
        assert_eq!(
            body_pending(999),
            json!({"action": "pending", "limit": MAX_PENDING_LIMIT}),
            "limit ceiling enforced bridge-side too"
        );
        assert_eq!(
            body_decide(true, 42, HEX, "U0PING1"),
            json!({"action": "decide", "decision": "approve", "proposal_id": 42,
                   "digest": HEX, "actor_ref": "U0PING1"})
        );
        assert_eq!(body_due("U1")["action"], "due");
        assert_eq!(body_crank("U1", 7)["run_id"], 7);
    }

    // ── HERALD PIN: console_seam_relay_carries_actor_and_digest — the
    //    decide relay signs Standard-Webhooks style, carries actor_ref and
    //    the EXACT digest, and a 4xx kernel error propagates as refusal.
    #[tokio::test]
    async fn console_seam_relay_carries_actor_and_digest() {
        type Captured = Arc<std::sync::Mutex<Option<Value>>>;
        let captured: Captured = Arc::new(std::sync::Mutex::new(None));

        async fn accept(
            axum::extract::State(state): axum::extract::State<Captured>,
            headers: axum::http::HeaderMap,
            body: axum::body::Bytes,
        ) -> axum::Json<Value> {
            let mut got = serde_json::from_slice::<Value>(&body).unwrap();
            got["__signed"] = serde_json::Value::Bool(
                headers.contains_key("webhook-id")
                    && headers.contains_key("webhook-timestamp")
                    && headers
                        .get("webhook-signature")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.starts_with("v1,"))
                        .unwrap_or(false),
            );
            *state.lock().unwrap() = Some(got);
            axum::Json(serde_json::json!({"status": "approved"}))
        }

        let app = axum::Router::new().route(
            "/webhooks/channel/slack/console",
            post(accept).with_state(captured.clone()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let http = reqwest::Client::new();
        let receipt = post_console(
            &http,
            &format!("http://{addr}"),
            "slack",
            b"whsec-test",
            &body_decide(true, 42, HEX, "U0PING1"),
        )
        .await
        .unwrap();
        assert_eq!(receipt["status"], "approved");

        let sent = captured.lock().unwrap().clone().unwrap();
        assert_eq!(sent["action"], "decide");
        assert_eq!(sent["actor_ref"], "U0PING1");
        assert_eq!(sent["digest"], HEX, "EXACT digest, byte for byte");
        assert_eq!(sent["__signed"], true, "Standard-Webhooks headers present");

        // A refusal server: kernel 403 → the relay errors with the snippet.
        async fn refuse() -> (axum::http::StatusCode, axum::Json<Value>) {
            (
                axum::http::StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({"error": "not allowed"})),
            )
        }
        let deny_app = axum::Router::new().route("/webhooks/channel/slack/console", post(refuse));
        let listener2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr2 = listener2.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener2, deny_app).await;
        });
        let err = post_console(
            &http,
            &format!("http://{addr2}"),
            "slack",
            b"whsec-test",
            &body_due("U1"),
        )
        .await
        .expect_err("4xx must propagate as refusal");
        assert!(format!("{err:#}").contains("403"));
        assert!(format!("{err:#}").contains("not allowed"));
    }
}
