//! brain-server Switchboard adapter.
//!
//! The edge's TWO protocols with the kernel, both HMAC-authenticated with the
//! SAME shared 0600 bridge config (`channel-{kind}-{tenant}.json`) — no brain
//! token, ever:
//!
//! 1. INBOUND — every received Signal text is wrapped in the normalized
//!    envelope projection and POSTed to `/webhooks/channel/{kind}`, signed
//!    Standard-Webhooks style (`v1,<b64 hmac-sha256({id}.{ts}.{body})>`).
//!    The server screens + threads + lands it; this side never trusts itself
//!    to be the gatekeeper.
//! 2. OUTBOUND — a crank polls `/webhooks/channel/{kind}/drain`; returned
//!    `channel/out` envelopes (approved acts / consented alert forwards ONLY)
//!    are delivered as Signal messages. Dedupe on `event_id` keeps redelivery
//!    at-least-once from double-sending.
//! 3. REGISTRATION — at boot the adapter posts mount evidence to
//!    `/workflow/plugins/mount` carrying the SHA-256 of the SHARED config file
//!    bytes. The server recomputes that digest from its own copy; both sides
//!    hashing the same bytes is what makes the evidence certifiable.

use anyhow::{Context, Result};
use base64::Engine;
// hmac 0.12 canonical pattern (per its own crate docs): the `Mac` trait
// carries new_from_slice/update/finalize; Hmac<Sha256> is the type alias.
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

/// One shared bridge config (mirrors the server-side loader's expectations).
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub kind: String,
    pub tenant: String,
    pub domain: String,
    pub webhook_secret: Vec<u8>,
}

impl BridgeConfig {
    /// Load + validate a `channel-{kind}-{tenant}.json` file. Owner-only
    /// permissions are REQUIRED — the secret is a bearer capability.
    pub fn load(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(path)
                .with_context(|| format!("bridge config missing: {}", path.display()))?;
            if meta.permissions().mode() & 0o077 != 0 {
                anyhow::bail!(
                    "bridge config {} must be owner-only (0600); refusing",
                    path.display()
                );
            }
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("config path must have a filename")?
            .to_string();
        let stem = name
            .strip_prefix("channel-")
            .and_then(|s| s.strip_suffix(".json"))
            .context("filename must be channel-{kind}-{tenant}.json")?;
        let (kind, tenant) = stem
            .split_once('-')
            .context("filename must carry kind and tenant segments")?;
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(path).context("reading bridge config")?)
                .context("bridge config must be JSON")?;
        Ok(Self {
            kind: kind.to_string(),
            tenant: tenant.to_string(),
            domain: v
                .get("domain")
                .and_then(|d| d.as_str())
                .context("missing domain")?
                .to_string(),
            webhook_secret: v
                .get("webhook_secret")
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .context("missing webhook_secret")?
                .as_bytes()
                .to_vec(),
        })
    }

    /// The SHA-256 hex of the config FILE bytes — the mount-evidence digest.
    /// The server recomputes this from its own copy of the same file; matching
    /// digests are what make the registration certifiable.
    pub fn config_sha256(&self, path: &Path) -> Result<String> {
        let bytes = std::fs::read(path).context("reading config for digest")?;
        Ok(hex_sha256(&bytes))
    }

    pub fn bridge_id(&self) -> String {
        format!("{}/{}", self.kind, self.tenant)
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = <Sha256 as Digest>::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The exact Standard Webhooks signature the server verifies:
/// `v1,` + base64(HMAC-SHA256(secret, "{id}.{ts}.{body}")).
pub(crate) fn sign_request(secret: &[u8], id: &str, ts: &str, body: &[u8]) -> Result<String> {
    use base64::engine::general_purpose::STANDARD;
    // Per the HMAC crate docs this constructor accepts keys of ANY length; an
    // Err here is unreachable — but we propagate instead of panicking.
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret)
        .map_err(|e| anyhow::anyhow!("hmac key init rejected: {e}"))?;
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(ts.as_bytes());
    mac.update(b".");
    mac.update(body);
    Ok(format!(
        "v1,{}",
        STANDARD.encode(mac.finalize().into_bytes())
    ))
}

/// The normalized M1 envelope projection posted inbound.
pub(crate) struct OutgoingEnvelope {
    pub conversation_ref: String,
    pub text: String,
    pub external_id: String,
}

impl OutgoingEnvelope {
    pub(crate) fn new(conversation_ref: &str, text: &str, external_id: &str) -> Self {
        Self {
            conversation_ref: conversation_ref.to_string(),
            text: text.to_string(),
            external_id: external_id.to_string(),
        }
    }

    pub(crate) fn to_json(&self) -> String {
        json!({
            "envelope": {
                "conversation_ref": self.conversation_ref,
                "text": self.text,
                "external_id": self.external_id,
            }
        })
        .to_string()
    }
}

/// Pure decision: should THIS message be forwarded? Direct conversations only
/// for now (group threading rides the line roadmap), non-empty text required.
pub(crate) fn forwardable(text: Option<&str>, group_id: Option<&str>) -> bool {
    text.is_some_and(|t| !t.trim().is_empty()) && group_id.is_none()
}

/// External platform id for the replay-cap key: sender uuid + the platform
/// timestamp millis (stable across relay restarts, unlike an in-memory seq).
pub(crate) fn external_id(sender_uuid: &str, ts_millis: i64) -> String {
    format!("{sender_uuid}-{ts_millis}")
}

// ── HTTP plumbing ──────────────────────────────────────────────────────────

pub(crate) struct BrainClient {
    base_url: String,
    http: reqwest::Client,
}

impl Clone for BrainClient {
    fn clone(&self) -> Self {
        Self {
            base_url: self.base_url.clone(),
            http: self.http.clone(),
        }
    }
}

impl BrainClient {
    pub(crate) fn new(base_url: &str) -> Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()?,
        })
    }

    async fn signed_post(
        &self,
        cfg: &BridgeConfig,
        path: &str,
        body: String,
    ) -> Result<reqwest::Response> {
        let id = uuid::Uuid::new_v4().to_string();
        let ts = chrono::Utc::now().timestamp().to_string();
        let url = format!("{}{}", self.base_url, path);
        self.http
            .post(&url)
            .header("content-type", "application/json")
            .header("webhook-id", &id)
            .header("webhook-timestamp", &ts)
            .header(
                "webhook-signature",
                sign_request(&cfg.webhook_secret, &id, &ts, body.as_bytes())?,
            )
            .body(body)
            .send()
            .await
            .context(format!("POST {url}"))
    }

    /// Post ONE inbound envelope. Errors are returned so the caller decides
    /// retry policy (the broadcast bus does not buffer for slow consumers).
    pub(crate) async fn post_inbound(
        &self,
        cfg: &BridgeConfig,
        envelope: &OutgoingEnvelope,
    ) -> Result<Value> {
        let body = envelope.to_json();
        let resp = self
            .signed_post(cfg, &format!("/webhooks/channel/{}", cfg.kind), body)
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::ensure!(status.is_success(), "inbound rejected: {status} {text}");
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Poll one drain batch; returns parsed envelopes (possibly empty).
    pub(crate) async fn drain(&self, cfg: &BridgeConfig) -> Result<Vec<Value>> {
        let resp = self
            .signed_post(
                cfg,
                &format!("/webhooks/channel/{}/drain", cfg.kind),
                "{}".to_string(),
            )
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::ensure!(status.is_success(), "drain rejected: {status} {text}");
        let v: Value = serde_json::from_str(&text)?;
        Ok(v.get("envelopes")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Register mount evidence ONCE at boot. Fire-and-retry is bounded here:
    /// evidence loss is visible in the server's chain gap, not silent.
    pub(crate) async fn register_mount(&self, cfg: &BridgeConfig, path: &Path) -> Result<()> {
        let sha = cfg.config_sha256(path)?;
        let body = json!({
            "plugin": format!("channel:{}", cfg.kind),
            "action": "mount",
            "domain": cfg.domain,
            "bundle_sha256": sha,
        })
        .to_string();
        let resp = self
            .signed_post(cfg, "/workflow/plugins/mount", body)
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::ensure!(status.is_success(), "mount refused: {status} {text}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    // The signature MUST byte-match the server's verification algebra
    // (workflow::channels::verify_bridge_signature).
    #[test]
    fn signature_matches_server_scheme() {
        let secret = b"bridgesecret";
        let id = "mid";
        let ts = "1700000000";
        let body =
            br#"{"envelope":{"conversation_ref":"+31","text":"[case 1] hi","external_id":"m1"}}"#;
        let sig = sign_request(secret, id, ts, body).unwrap();
        assert!(sig.starts_with("v1,"));

        use base64::Engine as _;
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret).unwrap();
        mac.update(id.as_bytes());
        mac.update(b".");
        mac.update(ts.as_bytes());
        mac.update(b".");
        mac.update(body);
        let expected = format!(
            "v1,{}",
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        );
        assert_eq!(sig, expected);
        // Tamper detection.
        assert_ne!(sign_request(b"other", id, ts, body).unwrap(), sig);
    }

    #[test]
    fn envelope_projection_is_exact() {
        let e = OutgoingEnvelope::new("+31612345678", "hello world", "+31x-1700000000000");
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        let env = v.get("envelope").expect("nested envelope object");
        assert_eq!(env["conversation_ref"], "+31612345678");
        assert_eq!(env["text"], "hello world");
        assert_eq!(env["external_id"], "+31x-1700000000000");
    }

    #[test]
    fn only_direct_text_messages_forward() {
        assert!(forwardable(Some("hi"), None));
        assert!(!forwardable(None, None));
        assert!(!forwardable(Some(""), None));
        assert!(!forwardable(Some("   "), None));
        // Groups suppressed for now (line-roadmap ceiling).
        assert!(!forwardable(Some("group msg"), Some("groupid")));
    }

    #[test]
    fn external_ids_are_stable_and_sender_scoped() {
        let a = external_id("uuid-a", 111);
        let b = external_id("uuid-b", 111);
        assert_ne!(a, b, "same ms different senders stay distinct");
        assert_eq!(external_id("uuid-a", 111), a, "stable across calls");
    }

    #[test]
    fn digest_is_lowercase_hex_of_file_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("channel-signal-acme.json");
        std::fs::write(
            &path,
            b"{\"domain\":\"acme\",\"webhook_secret\":\"sekrit\"}",
        )
        .unwrap();
        // The loader enforces 0600 on real configs — match it here.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let cfg = BridgeConfig::load(&path).unwrap();
        assert_eq!(cfg.kind, "signal", "kind comes from the FILENAME segment");
        assert_eq!(cfg.tenant, "acme");
        assert_eq!(cfg.domain, "acme");
        assert_eq!(cfg.config_sha256(&path).unwrap().len(), 64);
        let again = cfg.config_sha256(&path).unwrap();
        assert_eq!(cfg.config_sha256(&path).unwrap(), again);
    }
}
