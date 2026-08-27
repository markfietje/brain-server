//! The kernel-facing outbound seam and Cloud-API delivery: Standard-
//! Webhooks signing (drain + mount), the per-number tier state file, the
//! deterministic throttle table (MIRROR of the kernel's
//! `channels::min_send_interval_secs` — two halves of one law), media
//! quarantine, and the drain crank.

use crate::App;
use crate::hubsig::sha256_hex;
use anyhow::{Context, Result};
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

type HmacSha256 = Hmac<sha2::Sha256>;

/// Standard Webhooks signature for `{id}.{ts}.{body}`: `v1,<base64>`. The
/// EXACT scheme `channels::verify_bridge_signature` verifies kernel-side.
pub(crate) fn sw_sign(secret: &[u8], id: &str, ts: &str, body: &[u8]) -> String {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        // Unusable keys cannot sign; the kernel denies the constant tag.
        return "v1,".to_string();
    };
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(ts.as_bytes());
    mac.update(b".");
    mac.update(body);
    format!(
        "v1,{}",
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    )
}

/// ── Deterministic backoff table (plan M3) ─────────────────────────────────
/// The EDGE MIRROR of kernel `channels::min_send_interval_secs`. Both sides
/// are pinned: these tests pin these numbers and the downgrade-tightening
/// invariant; the kernel pin does the same for its copy. Change one, change
/// both, in one commit.
pub(crate) fn min_send_interval_secs(tier: Option<&str>) -> i64 {
    match tier {
        Some("green") => 0,
        Some("yellow") => 30,
        Some("orange") => 300,
        _ => 3_600, // red AND unobserved: fresh configs fail CLOSED
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct StateInner {
    #[serde(default)]
    tier: Option<String>,
    #[serde(default)]
    number_alias: Option<String>,
    #[serde(default)]
    last_send_attempt_at: Option<i64>,
    #[serde(default)]
    updated_at: Option<i64>,
}

/// The persisted per-number throttle state
/// (`state_dir/channel-whatsapp-{tenant}-state.json`, 0600). An ABSENT file
/// means UNOBSERVED, throttling at the MOST RESTRICTIVE interval until a
/// status webhook upgrades it — the fail-closed fresh-config law.
#[derive(Debug)]
pub(crate) struct TierState {
    path: PathBuf,
    inner: Mutex<StateInner>,
}

impl TierState {
    /// Open (lazily created on first write) the 0600 state file.
    pub(crate) fn open(state_dir: &Path, tenant: &str) -> Result<Self> {
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("state dir {}", state_dir.display()))?;
        let path = state_dir.join(format!("channel-whatsapp-{tenant}-state.json"));
        let inner = if path.exists() {
            #[cfg(unix)]
            check_perms(&path)?;
            let bytes = std::fs::read(&path).with_context(|| path.display().to_string())?;
            serde_json::from_slice(&bytes).unwrap_or_default()
        } else {
            StateInner::default()
        };
        Ok(Self {
            path,
            inner: Mutex::new(inner),
        })
    }

    fn persist(&self, inner: &StateInner) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(inner)?;
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::PermissionsExt;
            if !self.path.exists() {
                let mut f = std::fs::File::create(&self.path)
                    .with_context(|| self.path.display().to_string())?;
                f.write_all(&bytes)?;
                f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                return Ok(());
            }
        }
        std::fs::write(&self.path, &bytes).with_context(|| self.path.display().to_string())
    }

    /// Record a quality observation carried by a VERIFIED upstream payload.
    pub(crate) fn observe(&self, alias: &str, new_tier: &str) -> Result<()> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("tier state mutex poisoned"))?;
        g.tier = Some(new_tier.to_string());
        g.number_alias = Some(alias.to_string());
        g.updated_at = Some(chrono::Utc::now().timestamp());
        self.persist(&g)
    }

    pub(crate) fn current_tier(&self) -> Option<String> {
        self.inner.lock().ok().and_then(|g| g.tier.clone())
    }

    pub(crate) fn record_send_attempt(&self, now: i64) -> Result<()> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("tier state mutex poisoned"))?;
        g.last_send_attempt_at = Some(now);
        g.updated_at = Some(now);
        self.persist(&g)
    }

    /// True when enough time has passed under the CURRENT tier's interval.
    /// A poisoned mutex refuses sends (fail-closed, never fail-open).
    pub(crate) fn send_allowed_now(&self, now: i64) -> bool {
        let Ok(g) = self.inner.lock() else {
            return false;
        };
        let interval = min_send_interval_secs(g.tier.as_deref());
        match g.last_send_attempt_at {
            None => true,
            Some(last) => now.saturating_sub(last) >= interval,
        }
    }
}

#[cfg(unix)]
fn check_perms(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(p).with_context(|| p.display().to_string())?;
    if meta.permissions().mode() & 0o077 != 0 {
        anyhow::bail!("state file must be owner-only (0600): {}", p.display());
    }
    Ok(())
}

async fn post_kernel(
    http: &reqwest::Client,
    url: &str,
    secret: &[u8],
    body: Vec<u8>,
) -> Result<reqwest::Response> {
    let id = uuid::Uuid::new_v4().to_string();
    let ts = chrono::Utc::now().timestamp().to_string();
    let sig = sw_sign(secret, &id, &ts, &body);
    let resp = http
        .post(url)
        .header("webhook-id", id)
        .header("webhook-timestamp", ts)
        .header("webhook-signature", sig)
        .body(body)
        .send()
        .await
        .context("kernel endpoint unreachable")?
        .error_for_status()
        .context("kernel refused the request")?;
    Ok(resp)
}

/// Boot-time registration evidence over `/workflow/plugins/mount`: plugin
/// `channel:{kind}`, revision = FULL config-file digest — which the kernel
/// RECOMPUTES from its own copy of the same 0600 file (Gateweld law for
/// edges: evidence certifies bytes BOTH sides can hash).
pub(crate) async fn register_mount(app: &App) -> Result<u16> {
    let body = serde_json::json!({
        "plugin": "channel:whatsapp",
        "action": "mount",
        "bundle_sha256": app.cfg.config_sha256,
        "domain": app.cfg.domain,
    })
    .to_string()
    .into_bytes();
    let url = format!(
        "{}/workflow/plugins/mount",
        app.brain_url.trim_end_matches('/')
    );
    let resp = post_kernel(&app.http, &url, &app.cfg.webhook_secret, body).await?;
    Ok(resp.status().as_u16())
}

pub(crate) struct Quarantined {
    pub(crate) sha256_hex: String,
}

/// Download ONE media item via the Graph API and QUARANTINE its bytes under
/// the retention dir named by digest. Written once per unique digest, never
/// served, never proxied onward — ONLY the hash crosses the envelope seam.
pub(crate) async fn quarantine_media(app: &App, media_id: &str) -> Result<Quarantined> {
    let token = String::from_utf8_lossy(&app.cfg.access_token).to_string();
    let meta_resp = app
        .http
        .get(format!(
            "https://graph.facebook.com/{}/{}",
            app.graph_api_version, media_id
        ))
        .bearer_auth(token.clone())
        .send()
        .await
        .context("graph media lookup failed")?
        .error_for_status()
        .context("graph media lookup refused")?;
    let meta: Value = meta_resp.json().await.context("graph media json invalid")?;
    let download_url = meta
        .get("url")
        .and_then(|x| x.as_str())
        .context("graph media metadata missing url")?
        .to_string();

    let bytes = app
        .http
        .get(download_url)
        .bearer_auth(token)
        .send()
        .await
        .context("media download failed")?
        .error_for_status()
        .context("media download refused")?
        .bytes()
        .await
        .context("media read failed")?;

    let digest = sha256_hex(bytes.as_ref());
    let dir = PathBuf::from(app.retention_dir.as_ref());
    std::fs::create_dir_all(&dir).context("retention dir create failed")?;
    let path = dir.join(format!("{digest}.bin"));
    if !path.exists() {
        // Same-digest dedupe: identical bytes are identical quarantine rows.
        std::fs::write(&path, bytes.as_ref())
            .with_context(|| format!("quarantine write {}", path.display()))?;
    }
    Ok(Quarantined { sha256_hex: digest })
}

/// One drain crank: claim pending `channel/out` envelopes for THIS kind via
/// the signed seam and deliver each to the Cloud API — approved template
/// acts go out as TEMPLATES (the kernel enqueues nothing else outside the
/// window), windowed replies as TEXT. Every send is paced by the tier
/// table; a throttled crank defers remaining rows to a later tick (the
/// kernel marks claim batches delivered at CLAIM time — at-least-once,
/// loud logs are the visibility story).
pub(crate) async fn crank(app: &App) -> Result<()> {
    let url = format!(
        "{}/webhooks/channel/whatsapp/drain",
        app.brain_url.trim_end_matches('/')
    );
    let resp = post_kernel(&app.http, &url, &app.cfg.webhook_secret, Vec::new()).await?;
    let parsed: Value = resp.json().await.context("drain payload not json")?;
    let envelopes = parsed
        .get("envelopes")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    for env in envelopes {
        let now = chrono::Utc::now().timestamp();
        if !app.state.send_allowed_now(now) {
            tracing::warn!(
                interval_secs = min_send_interval_secs(app.state.current_tier().as_deref()),
                "tier throttle active: deferring send(s) to a later tick"
            );
            break;
        }
        if let Err(e) = deliver_one(app, &env).await {
            tracing::error!("deliver failed loudly: {e:#}");
        }
        app.state
            .record_send_attempt(chrono::Utc::now().timestamp())?;
    }
    Ok(())
}

async fn deliver_one(app: &App, env: &Value) -> Result<()> {
    let conversation_ref = env
        .get("conversation_ref")
        .and_then(|x| x.as_str())
        .context("envelope missing conversation_ref")?;
    let text = env.get("text").and_then(|x| x.as_str()).unwrap_or("");
    let source = env.get("source_payload");
    let kind = source
        .and_then(|s| s.get("kind"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let template_name = source
        .and_then(|s| s.get("template"))
        .and_then(|x| x.as_str());

    let use_template = kind == "channel/template";
    let payload = if use_template {
        serde_json::json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": conversation_ref,
            "type": "template",
            // CEILING: parameterized components ship later — this sends the
            // named registered template verbatim (operators keep bodies
            // plain).
            "template": {
                "name": template_name.unwrap_or(text),
                "language": {"code": "en_US"},
            },
        })
    } else {
        serde_json::json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": conversation_ref,
            "type": "text",
            "text": { "preview_url": false, "body": text },
        })
    };

    let url = format!(
        "https://graph.facebook.com/{}/{}/messages",
        app.graph_api_version, app.cfg.phone_number_id
    );
    let resp = app
        .http
        .post(url)
        .bearer_auth(String::from_utf8_lossy(&app.cfg.access_token).to_string())
        .json(&payload)
        .send()
        .await
        .context("cloud api unreachable")?;
    let code = resp.status();
    if !code.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        anyhow::bail!("cloud api refused send ({code}): {body_text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::panic,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;

    // ── CARAVEL PIN (edge mirror): tier_downgrade_throttles_and_alerts —
    //    the throttle table matches the KERNEL constants exactly, downgrades
    //    only tighten, and FRESH state fails closed.
    #[test]
    fn tier_downgrade_throttles_and_alerts() {
        assert_eq!(min_send_interval_secs(Some("green")), 0);
        assert_eq!(min_send_interval_secs(Some("yellow")), 30);
        assert_eq!(min_send_interval_secs(Some("orange")), 300);
        assert_eq!(min_send_interval_secs(Some("red")), 3_600);

        // Fresh / unknown == most restrictive (fail-closed throttle).
        assert_eq!(
            min_send_interval_secs(None),
            min_send_interval_secs(Some("red"))
        );

        // Downgrade monotonicity across the whole ladder.
        let ladder = ["green", "yellow", "orange", "red"];
        for (i, older) in ladder.iter().enumerate() {
            for newer in &ladder[i + 1..] {
                assert!(
                    min_send_interval_secs(Some(newer)) >= min_send_interval_secs(Some(older)),
                    "downgrade {older}→{newer} must tighten-or-hold"
                );
            }
        }
    }

    #[test]
    fn tier_state_fresh_is_restrictive_then_times_open() {
        let tmp = tempfile::tempdir().unwrap();
        let st = TierState::open(tmp.path(), "acme").unwrap();
        assert_eq!(st.current_tier(), None, "fresh state is UNOBSERVED");
        assert!(st.send_allowed_now(1_000));

        st.record_send_attempt(1_000).unwrap();
        // Inside the restricted window → refused…
        assert!(!st.send_allowed_now(1_001));
        // …exactly AT the boundary → allowed again (inclusive-bound law).
        assert!(st.send_allowed_now(1_000 + min_send_interval_secs(None)));
    }

    #[test]
    fn observed_upgrade_relaxes_the_throttle() {
        let tmp = tempfile::tempdir().unwrap();
        let st = TierState::open(tmp.path(), "beta").unwrap();
        st.observe("biz_number", "green").unwrap();
        st.record_send_attempt(10_000).unwrap();
        // green interval == 0: immediately eligible again.
        assert_eq!(st.current_tier().as_deref(), Some("green"));
        assert!(st.send_allowed_now(10_001));
        // And the persisted file stays owner-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(tmp.path().join("channel-whatsapp-beta-state.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "state file stays owner-only");
        }
    }

    #[test]
    fn sw_sign_shape_is_kernel_compatible() {
        let sig = sw_sign(b"whsec", "mid", "1700000000", b"hello");
        assert!(sig.starts_with("v1,"));
        assert_eq!(
            sig,
            sw_sign(b"whsec", "mid", "1700000000", b"hello"),
            "deterministic"
        );
        // Different bodies → different tags.
        assert_ne!(sig, sw_sign(b"whsec", "mid", "1700000000", b"hellO"));
        // Empty keys still construct a tag, but config load refuses empty
        // secrets long before signing can ever run with one (defense in
        // depth lives at BOTH ends).
        assert!(sw_sign(b"", "mid", "1", b"x").starts_with("v1,"));
    }
}
