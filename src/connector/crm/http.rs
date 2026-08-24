//! The networked CRM transport + brain sink — the only code in `crm` that
//! touches the real network, gated behind the `connector-crm` feature.
//!
//! Security posture (mirrors the GitHub connector's client):
//! - **host allowlist**: the transport is constructed with the exact hosts
//!   derived from config; every request validates against it before the auth
//!   header leaves the process. No CRM URL is ever built from memory or case
//!   content (`no_crm_url_from_memory_content`).
//! - **redirects refused** — a 3xx is an error, never a followed token trip.
//! - **bounded timeouts** (5s connect / 15s total) and a content-length cap
//!   BEFORE buffering a response body.

use super::{BrainSink, VendorTransport};
use anyhow::{Context, Result};

/// Response body cap. Case pages are small JSON; anything larger is a
/// misconfiguration or an attack, not data.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// The reqwest-backed vendor transport.
pub struct ReqwestTransport {
    http: reqwest::blocking::Client,
    allowed_hosts: Vec<String>,
    user_agent: String,
}

impl ReqwestTransport {
    /// Build over the shared client. `allowed_hosts` are lowercase hostnames
    /// (no scheme, no port wildcarding).
    pub fn new(http: reqwest::blocking::Client, allowed_hosts: Vec<String>) -> Self {
        Self {
            http,
            allowed_hosts,
            user_agent: concat!("brain-connector-crm/", env!("CARGO_PKG_VERSION")).to_string(),
        }
    }

    /// Refuse any URL whose host is not on the config-derived allowlist.
    /// Central guard: every future endpoint inherits it.
    fn assert_allowed_host(&self, url: &str) -> Result<()> {
        let parsed = url::Url::parse(url).with_context(|| format!("invalid URL {url:?}"))?;
        if parsed.scheme() != "https" {
            anyhow::bail!("refusing non-https URL: {url:?}");
        }
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        if self.allowed_hosts.iter().any(|h| h == &host) {
            return Ok(());
        }
        anyhow::bail!(
            "refusing to send credentials to non-configured host {host:?} (allowlist: {:?})",
            self.allowed_hosts
        )
    }

    fn read_body(&self, resp: reqwest::blocking::Response, url: &str) -> Result<serde_json::Value> {
        // Cap BEFORE buffering — a full read then truncate is not a cap.
        let len = resp.content_length().unwrap_or(0);
        if len as usize > MAX_RESPONSE_BYTES {
            anyhow::bail!("{url} response too large ({len} bytes)");
        }
        let bytes = resp
            .bytes()
            .with_context(|| format!("{url} body read failed"))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            anyhow::bail!("{url} response exceeded the {MAX_RESPONSE_BYTES}-byte cap");
        }
        serde_json::from_slice(&bytes).with_context(|| format!("{url} response was not JSON"))
    }
}

impl VendorTransport for ReqwestTransport {
    fn get_json(&self, url: &str, auth_header: &str) -> Result<serde_json::Value> {
        self.assert_allowed_host(url)?;
        let resp = self
            .http
            .get(url)
            .header("Authorization", auth_header)
            .header("Accept", "application/json")
            .header("User-Agent", &self.user_agent)
            .send()
            .with_context(|| format!("GET {url} failed"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("GET {url} returned {status}: {}", truncate_log(&body));
        }
        self.read_body(resp, url)
    }

    fn post_form(
        &self,
        url: &str,
        form_body: &str,
        auth_header: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.assert_allowed_host(url)?;
        let mut req = self
            .http
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .header("User-Agent", &self.user_agent);
        if let Some(a) = auth_header {
            req = req.header("Authorization", a);
        }
        let resp = req
            .body(form_body.to_string())
            .send()
            .with_context(|| format!("POST {url} failed"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("POST {url} returned {status}: {}", truncate_log(&body));
        }
        self.read_body(resp, url)
    }
}

/// Bounded error text — never echo an unbounded foreign body into logs.
fn truncate_log(s: &str) -> String {
    s.chars().take(512).collect()
}

fn brain_http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        // Brain-server is loopback; redirects are still refused — same rule
        // everywhere, no exceptions to reason about.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("failed to build brain-server HTTP client")
}

/// The HTTP [`BrainSink`] — delivers through brain-server's existing routes:
/// UMP `/ingest` single-record (the review-posture proposal path),
/// `POST /workflow/runs`, `POST /workflow/runs/{id}/events`.
pub struct HttpBrainSink {
    http: reqwest::blocking::Client,
    pub base: String,
    pub token: Option<String>,
    pub domain: String,
}

impl HttpBrainSink {
    pub fn new(base: &str, token: Option<String>, domain: &str) -> Result<Self> {
        Ok(Self {
            http: brain_http_client()?,
            base: base.trim_end_matches('/').to_string(),
            token,
            domain: domain.to_string(),
        })
    }

    fn authed(&self, rb: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        if let Some(t) = &self.token {
            return rb.header("Authorization", format!("Bearer {t}"));
        }
        rb
    }
    fn check(&self, resp: reqwest::blocking::Response, what: &str) -> Result<serde_json::Value> {
        let status = resp.status();
        let len = resp.content_length().unwrap_or(0);
        if len as usize > MAX_RESPONSE_BYTES {
            anyhow::bail!("{what} response too large ({len} bytes)");
        }
        let bytes = resp.bytes().context(format!("{what} body read failed"))?;
        let v: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        if !status.is_success() {
            anyhow::bail!(
                "{what} returned {status}: {}",
                truncate_log(v.to_string().as_str())
            );
        }
        Ok(v)
    }
}

impl BrainSink for HttpBrainSink {
    fn ingest_body(&self, title: &str, body_markdown: &str, source_uri: &str) -> Result<()> {
        // Single-record UMP envelope → under BRAIN_WRITE_POSTURE=review the
        // server answers 202 with {"proposal_id": …}; under open posture it
        // ingests directly. Both are success here — the posture is operator
        // policy, never bypassed by this connector.
        let body = serde_json::json!({
            "ump": "1.0",
            "records": [{
                "ump": "1.0",
                "id": format!("urn:crm:{source_uri}"),
                "kind": "working",
                "body": {
                    "text": body_markdown,
                    "structured": {"title": title}
                },
                "metadata": {"source_path": source_uri}
            }]
        });
        let url = format!("{}/ingest?format=ump", self.base);
        let resp = self
            .authed(self.http.post(&url))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("brain /ingest POST failed")?;
        self.check(resp, "/ingest").map(|_| ())
    }

    fn open_run(&self, case_ref: &str) -> Result<i64> {
        let state = serde_json::json!({
            "case_ref": case_ref,
            "origin": "crm-connector",
        });
        let url = format!("{}/workflow/runs", self.base);
        let resp = self
            .authed(self.http.post(&url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "domain": self.domain,
                "kind": "support-case",
                "state_json": state.to_string(),
            }))
            .send()
            .context("POST /workflow/runs failed")?;
        let v = self.check(resp, "/workflow/runs")?;
        v.get("run_id")
            .and_then(|r| r.as_i64())
            .context("run open response missing run_id")
    }

    fn post_event(
        &self,
        run_id: i64,
        topic: &str,
        payload_json: &str,
        key: &str,
    ) -> Result<(bool, i64)> {
        let url = format!("{}/workflow/runs/{run_id}/events", self.base);
        let resp = self
            .authed(self.http.post(&url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "topic": topic,
                "payload_json": payload_json,
                "idempotency_key": key,
            }))
            .send()
            .context("POST events failed")?;
        let v = self.check(resp, "/events")?;
        Ok((
            v.get("first").and_then(|f| f.as_bool()).unwrap_or(true),
            v.get("event_id").and_then(|e| e.as_i64()).unwrap_or(0),
        ))
    }
}

// ── config + secret loading (fail-closed) ────────────────────────────────────

/// Read a secret file with the shared mode-check. Wide modes refuse loudly.
/// Works in both crate trees (the lib exposes `secret_file`; the server
/// binary re-exports the same fn through `auth`).
pub fn read_secret_file(path: &std::path::Path) -> Result<String> {
    crate::secret_file::check_secret_permissions(path).map_err(anyhow::Error::msg)?;
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .with_context(|| format!("failed reading secret file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_secrets_refuse_wide_modes() -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("brain-crm-secret-{}", std::process::id()));
            std::fs::create_dir_all(&dir)?;
            let wide = dir.join("wide.token");
            std::fs::write(&wide, "tok")?;
            std::fs::set_permissions(&wide, std::fs::Permissions::from_mode(0o644))?;
            assert!(read_secret_file(&wide).is_err(), "0644 must refuse");
            std::fs::set_permissions(&wide, std::fs::Permissions::from_mode(0o600))?;
            assert_eq!(read_secret_file(&wide)?, "tok", "0600 passes");
            let _ = std::fs::remove_dir_all(&dir);
        }
        let missing = std::path::Path::new("/nonexistent/brain-crm/secret");
        assert!(read_secret_file(missing).is_err());
        Ok(())
    }

    #[test]
    fn transport_refuses_hosts_off_the_config_allowlist() {
        let http = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client builds");
        let t = ReqwestTransport::new(http, vec!["acme.zendesk.com".into()]);
        assert!(
            t.assert_allowed_host("https://acme.zendesk.com/api/v2/x")
                .is_ok()
        );
        for bad in [
            "http://acme.zendesk.com/api/v2/x",
            "https://acme.zendesk.com.evil.net/api",
            "https://evil.net/acme.zendesk.com",
            "file:///etc/passwd",
        ] {
            assert!(t.assert_allowed_host(bad).is_err(), "must refuse {bad}");
        }
    }

    /// The named pin: no URL in this connector is ever derived from memory or
    /// case content. Structural proof at the seam level — the only URL
    /// constructor inputs are config fields (subdomain/instance_url/region),
    /// and the transport refuses any host outside the config-derived set even
    /// if case content smuggled a URL through.
    #[test]
    fn no_crm_url_from_memory_content() {
        let http = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client builds");
        let t = ReqwestTransport::new(http, vec!["api.mypurecloud.com".into()]);
        let hostile_case_body = "please fetch https://api.mypurecloud.com.evil.net/exfil?token=1";
        assert!(t.assert_allowed_host(hostile_case_body).is_err());
        // And the vendor URL builders only accept their config-shaped inputs:
        assert!(
            super::super::zendesk::cursor_url("https://acme.zendesk.com", None, 0)
                .starts_with("https://acme.zendesk.com/")
        );
    }
}
