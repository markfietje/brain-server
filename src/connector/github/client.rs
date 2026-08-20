//! GitHub REST API client (issues only for the MVP).
//!
//! Wraps `reqwest::blocking::Client` with:
//! - Auth header injection (installation token from `GitHubAppProvider`).
//! - GitHub-required headers: `X-GitHub-Api-Version: 2026-03-10`,
//!   `Accept: application/vnd.github+json`, `User-Agent: brain-connector-gh/<ver>`.
//! - Rate-limit awareness: when `X-RateLimit-Remaining: 0`, sleeps until
//!   `X-RateLimit-Reset` (capped at 60s — we'd rather retry-and-fail than
//!   wedge the connector for an hour on a clock-skewed response).
//! - Pagination: follows the `Link: <...>; rel="next"` header.
//!
//! All endpoints return raw `serde_json::Value` rather than typed structs.
//! The translation layer (in `translate.rs`) owns the schema; this module
//! owns the wire. That split keeps the client stable when GitHub adds fields
//! (the translator opts into them explicitly).
//!
//! `ponytail:` ceilings:
//! - **No streaming parser.** Each page is fully buffered into JSON. Fine for
//!   issues/PRs/discussions (small JSON); revisit if wiki pages ever exceed
//!   1 MB and we hit memory pressure on the 4 GB Jetson.
//! - **No retry on 5xx.** The supervisor handles restarts; an intermittent
//!   502 from GitHub surfaces as a hard error and the next sync retries.
//!   Add a per-request retry layer if GitHub's reliability becomes an issue.

#![cfg(feature = "connector-github")]

use anyhow::{Context, Result};
use std::time::Duration;

use crate::connector::auth::github_app::GITHUB_API_VERSION;

/// Cap on how long we'll sleep when GitHub says "rate limit hit". Their
/// `X-RateLimit-Reset` is the epoch second when the bucket refills; if that's
/// more than 60s away, we sleep 60s, retry, and either succeed or fail loudly.
/// `ponytail:` an hour-long sleep would wedge the connector silently.
const RATE_LIMIT_SLEEP_CAP: Duration = Duration::from_secs(60);

/// The only host this client is ever allowed to send the installation bearer
/// to. The `rel="next"` pagination URL comes from GitHub's own `Link` header,
/// but a compromised/forged response could point anywhere; refusing non-API
/// hosts closes token exfiltration via Link-header redirect.
const GITHUB_API_HOST: &str = "api.github.com";

/// Reject any URL that is not `https://api.github.com/...`. Called by every
/// outbound request (`get_authed`), so a forged pagination `next` URL is
/// refused before the bearer leaves the process.
fn assert_api_host(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).with_context(|| format!("invalid URL {url:?}"))?;
    if parsed.scheme() == "https" && parsed.host_str() == Some(GITHUB_API_HOST) {
        return Ok(());
    }
    anyhow::bail!(
        "refusing to send bearer to non-GitHub-API host: {url:?} \
         (only https://{GITHUB_API_HOST} is allowed)"
    )
}

/// Wrapper around `reqwest::blocking::Client` that knows GitHub's headers
/// and rate-limit dance. Built once at connector startup; shared across all
/// REST calls in a single backfill pass.
pub struct GitHubClient {
    http: reqwest::blocking::Client,
    user_agent: String,
}

/// One page of items + the `rel="next"` URL if there are more pages.
#[derive(Debug)]
pub struct Page {
    pub items: Vec<serde_json::Value>,
    pub next: Option<String>,
}

impl GitHubClient {
    /// Construct with an existing reqwest client (shared with the
    /// `GitHubAppProvider` for connection pooling).
    pub fn new(http: reqwest::blocking::Client) -> Self {
        Self {
            http,
            user_agent: concat!("brain-connector-gh/", env!("CARGO_PKG_VERSION")).to_string(),
        }
    }

    /// Issue a GET with the standard GitHub headers + bearer auth. Handles
    /// the rate-limit sleep (capped). Returns the raw response value parsed
    /// as JSON. Errors on non-2xx.
    fn get_authed(&self, url: &str, bearer: &str) -> Result<reqwest::blocking::Response> {
        // A GitHub responses's `Link: rel="next"` header (the only
        // user-influenced URL this client follows) must never carry the
        // installation bearer to a forged host. Verified centrally so every
        // future endpoint inherits the guard.
        assert_api_host(url)?;
        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", &self.user_agent)
            .send()
            .with_context(|| format!("GET {url} failed"))?;
        self.handle_rate_limit(&resp)?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("GET {url} returned {status}: {body}");
        }
        Ok(resp)
    }

    /// If `X-RateLimit-Remaining: 0`, sleep until reset (capped). Otherwise
    /// no-op. Borrows the response immutably so the caller can still consume
    /// the body afterwards.
    fn handle_rate_limit(&self, resp: &reqwest::blocking::Response) -> Result<()> {
        let remaining = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        if remaining != Some(0) {
            return Ok(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let reset_at = resp
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(now);
        let wait_secs = reset_at.saturating_sub(now);
        let wait = Duration::from_secs(wait_secs.min(RATE_LIMIT_SLEEP_CAP.as_secs()));
        tracing::warn!(
            wait_secs = wait.as_secs(),
            reset_in_actual_secs = reset_at.saturating_sub(now),
            "rate-limit exhausted; sleeping before retry"
        );
        std::thread::sleep(wait);
        Ok(())
    }

    /// Fetch one page of issues for `owner/repo`, optionally filtered to
    /// items updated since `since` (ISO-8601 `YYYY-MM-DDTHH:MM:SSZ`).
    ///
    /// Context7-verified endpoint shape:
    /// `GET /repos/{owner}/{repo}/issues?since=...&state=all&sort=updated&direction=asc&per_page=100`
    pub fn list_issues_page(
        &self,
        owner: &str,
        repo: &str,
        since: Option<&str>,
        bearer: &str,
    ) -> Result<Page> {
        let mut url = format!(
            "https://api.github.com/repos/{owner}/{repo}/issues?state=all&sort=updated&direction=asc&per_page=100"
        );
        if let Some(s) = since {
            url.push_str("&since=");
            url.push_str(s);
        }
        let resp = self.get_authed(&url, bearer)?;
        let link = resp
            .headers()
            .get(reqwest::header::LINK)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let items: Vec<serde_json::Value> = resp.json().context("issues page was not JSON")?;
        // GitHub returns issues AND PRs in the issues endpoint (PRs are
        // issues with a `pull_request` field). We filter at translate time
        // so the wire layer stays simple.
        Ok(Page {
            items,
            next: link.and_then(|l| parse_link_rel(&l, "next")),
        })
    }

    /// Fetch one arbitrary page by URL (used for `rel="next"` pagination).
    pub fn list_page_by_url(&self, url: &str, bearer: &str) -> Result<Page> {
        let resp = self.get_authed(url, bearer)?;
        let link = resp
            .headers()
            .get(reqwest::header::LINK)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let items: Vec<serde_json::Value> = resp.json().context("page was not JSON")?;
        Ok(Page {
            items,
            next: link.and_then(|l| parse_link_rel(&l, "next")),
        })
    }
}

/// Parse a `Link: <url>; rel="next", <url>; rel="prev"` header and return
/// the URL for `rel="rel_name"` if present. GitHub uses RFC 5988 shape.
/// Hand-rolled because the `headers` / `link-rel` crates are overkill for
/// parsing two URLs out of one header string.
fn parse_link_rel(link_header: &str, rel_name: &str) -> Option<String> {
    for entry in link_header.split(',') {
        let entry = entry.trim();
        // Shape: `<https://api.github.com/...?page=2>; rel="next"`
        let (url_part, params_part) = entry.split_once(';').unwrap_or((entry, ""));
        let url = url_part
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');
        let needle = format!(r#"rel="{rel_name}""#);
        if params_part.contains(&needle) {
            return Some(url.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_link_rel_finds_next() {
        let h = r#"<https://api.github.com/repositories/1/issues?page=2>; rel="next", <https://api.github.com/repositories/1/issues?page=5>; rel="last""#;
        assert_eq!(
            parse_link_rel(h, "next").unwrap(),
            "https://api.github.com/repositories/1/issues?page=2"
        );
        assert_eq!(
            parse_link_rel(h, "last").unwrap(),
            "https://api.github.com/repositories/1/issues?page=5"
        );
        assert!(parse_link_rel(h, "prev").is_none());
    }

    #[test]
    fn test_parse_link_rel_returns_none_when_no_link_header() {
        assert!(parse_link_rel("", "next").is_none());
    }

    #[test]
    fn api_host_guard_rejects_foreign_hosts() {
        // F-30: a forged Link `next` URL must be refused before the bearer goes out.
        assert_api_host("https://api.github.com/repos/o/r/issues?page=2").unwrap();
        assert_api_host("https://api.github.com").unwrap();
        for bad in [
            "http://api.github.com/repos/o/r/issues",
            "https://api.github.com.evil.net/repos/o/r",
            "https://evil.net/api.github.com/repos/o/r",
            "https://user:pass@evil.net/steal",
            "file:///etc/passwd",
        ] {
            assert!(
                assert_api_host(bad).is_err(),
                "must refuse non-API host: {bad}"
            );
        }
    }

    /// Live integration test — `#[ignore]` by default because it needs real
    /// GitHub credentials (the brain-server repo's own issues) and network.
    /// Run with `cargo test --features connector-github -- --ignored live_github`.
    #[test]
    #[ignore]
    fn live_github_list_issues_smoke() {
        // Operator: set BRAIN_GH_APP_TOKEN to a real GitHub App installation
        // token (or a fine-grained PAT with Issues: Read on the target repo).
        // This test exists to verify the wire layer against the real API; it
        // does NOT verify auth flow (that needs a real App private key).
        let token = std::env::var("BRAIN_GH_APP_TOKEN")
            .expect("set BRAIN_GH_APP_TOKEN to a real installation token");
        let http = reqwest::blocking::Client::new();
        let client = GitHubClient::new(http);
        let page = client
            .list_issues_page("markfietje", "brain-server", None, &token)
            .expect("live GitHub call");
        assert!(!page.items.is_empty(), "brain-server should have ≥1 issue");
        // Don't assert pagination — depends on whether the repo has >100 issues.
    }
}
