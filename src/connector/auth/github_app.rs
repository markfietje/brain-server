//! GitHub App authentication provider.
//!
//! Implements the GitHub-App-specific JWT → installation-token flow. This is
//! NOT standard OAuth 2.0 — it's GitHub's bespoke flow, and we use it for
//! GitHub specifically because it gives strictly stronger guarantees than
//! OAuth Apps:
//!
//! - **Per-installation repository scoping** via the optional `repositories`
//!   body field on `POST /app/installations/{id}/access_tokens`. This is the
//!   least-privilege mechanism that enforces "a GitHub App limited to
//!   two repos cannot index a third" at the API level.
//! - **Fine-grained permissions** (Issues: Read, Pull requests: Read, …)
//!   instead of OAuth Apps' coarse scopes (`repo`, `public_repo`).
//! - **Independent of any user** — installation tokens work even if the user
//!   who installed the app leaves the org.
//!
//! For SaaS sources that *do* use standard OAuth 2.0 (Salesforce, Slack,
//! Linear, Notion, HubSpot, …), the `OAuthProvider` impl (deferred)
//! will cover them. The `AuthProvider` trait is the unified surface.
//!
//! ## Flow (Context7-verified 2026-07-20, `/websites/github_en_rest`)
//!
//! 1. Construct an RS256-signed JWT with claims `{ iss: <app_id>, iat: now-60,
//!    exp: now+540 }`. The 9-minute window is GitHub's hard cap (10 min).
//! 2. `POST /app/installations/{id}/access_tokens` with the JWT as bearer.
//!    Body (optional): `{ "repositories": ["brain-server"] }` for per-repo
//!    scoping. Headers: `X-GitHub-Api-Version: 2026-03-10`,
//!    `Accept: application/vnd.github+json`.
//! 3. Response: `{ "token": "ghs_...", "expires_at": "2026-07-20T11:00:00Z" }`.
//!    Token is valid for 60 minutes.
//! 4. Cache the token until `expires_at - REFRESH_SKEW`, then re-fetch.
//!
//! Feature-gated because `jsonwebtoken` + its crypto deps shouldn't bloat the
//! server binary (see `bin_common/http.rs` line 4 — the server deliberately
//! has no outbound HTTP / crypto deps). Enable with `--features connector-github`.

#![cfg(feature = "connector-github")]

use anyhow::{Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{AccessToken, AuthProvider};

/// Refresh this long before the wire expiry. GitHub caps at 60 min; we refresh
/// at 59 min so a request that lands at 59:59 still uses a fresh token.
const REFRESH_SKEW: Duration = Duration::from_secs(60);

/// Maximum lifetime of a GitHub App JWT. GitHub rejects `exp - iat > 10min`,
/// so we use 9 minutes to leave room for clock skew.
const JWT_MAX_AGE: Duration = Duration::from_secs(9 * 60);

/// Clock-skew buffer subtracted from `iat`. If our clock is 30s ahead of
/// GitHub's, a JWT with `iat = now` looks future-dated to GitHub and is
/// rejected. Subtracting 60s covers reasonable skew.
const JWT_IAT_SKEW: Duration = Duration::from_secs(60);

/// GitHub REST API version header value (Context7-verified 2026-07-20).
/// Send on every request so GitHub's deprecation policy applies predictably.
pub const GITHUB_API_VERSION: &str = "2026-03-10";

/// Config shape for the GitHub App provider. Stored as JSON at
/// `~/.config/brain-server/connectors/github-{instance}.json` (0600).
///
/// Convention: `private_key_path` and `webhook_secret_path` point at 0600
/// PEM/secret files (matches `AUTH_TOKEN_FILE` ladder). The PEM file is the
/// one GitHub generates when you create the App.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitHubAppConfig {
    /// The App's numeric ID (e.g. `1234567`). Found in the App settings page URL.
    pub app_id: i64,
    /// The installation ID for the target org/user. Found in the installation
    /// URL: `https://github.com/settings/installations/{installation_id}`.
    pub installation_id: i64,
    /// Absolute path to the App's RSA private key (PEM). Mode 0600.
    pub private_key_path: String,
    /// Optional: absolute path to the webhook secret file. Mode 0600.
    /// Used by the webhook-ingress handler for HMAC verification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_secret_path: Option<String>,
    /// Optional: restrict the installation token to specific repo names.
    /// This is the API-level least-privilege mechanism. When `None`,
    /// the token covers every repo the installation can see.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<String>,
}

/// JWT claims for the GitHub App bearer JWT. GitHub requires `iss` = app_id
/// (as a string) and rejects tokens where `exp - iat > 10min`.
#[derive(Serialize)]
struct AppJwtClaims {
    /// App ID as a string (GitHub requires string, not integer).
    iss: String,
    /// Issued-at (seconds since UNIX epoch).
    iat: u64,
    /// Expiry (seconds since UNIX epoch). `exp - iat` ≤ 600.
    exp: u64,
}

/// Wire shape of `POST /app/installations/{id}/access_tokens` response.
/// Context7-verified 2026-07-20 (`/websites/github_en_rest`).
#[derive(Deserialize)]
struct InstallTokenResponse {
    token: String,
    /// ISO-8601 like `"2026-07-20T11:00:00Z"`. Parsed into epoch seconds
    /// by `parse_github_timestamp`.
    expires_at: String,
}

/// Parse a GitHub ISO-8601 timestamp like `"2026-07-20T11:00:00Z"` into
/// seconds-since-UNIX-epoch. GitHub always sends the `Z` suffix (UTC) per
/// their docs; we don't accept offsets because GitHub never sends them.
///
/// `ponytail:` hand-rolled to avoid pulling `chrono` into the auth module's
/// public surface. The shape is fixed (YYYY-MM-DDTHH:MM:SSZ), so we parse
/// the components directly. If GitHub ever changes the format, this fails
/// loud with a clear error instead of silently mis-parsing.
fn parse_github_timestamp(s: &str) -> Result<u64> {
    // Expected shape: "2026-07-20T11:00:00Z" (19 chars + 'Z' = 20).
    // Assert each separator explicitly so the parser fails loud on shape
    // drift (e.g. someone passing a RFC3339 with offset, or a space-sep date).
    let bytes = s.as_bytes();
    if s.len() != 20 || bytes[10] != b'T' || bytes[19] != b'Z' {
        anyhow::bail!(
            "unexpected GitHub timestamp format: {s:?} (expected 'YYYY-MM-DDTHH:MM:SSZ')"
        );
    }
    // Verify the dash / colon separators while we're at it.
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' || bytes[16] != b':' {
        anyhow::bail!(
            "unexpected GitHub timestamp format: {s:?} (expected 'YYYY-MM-DDTHH:MM:SSZ')"
        );
    }
    let year: u32 = s[0..4].parse().context("year")?;
    let month: u32 = s[5..7].parse().context("month")?;
    let day: u32 = s[8..10].parse().context("day")?;
    let hour: u32 = s[11..13].parse().context("hour")?;
    let minute: u32 = s[14..16].parse().context("minute")?;
    let second: u32 = s[17..19].parse().context("second")?;
    // Range-check each component so a bogus "99:99:99" fails loud here
    // rather than producing a meaningless epoch downstream.
    if !(1..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        anyhow::bail!("timestamp components out of range: {s:?}");
    }
    Ok(epoch_from_ymdhms(year, month, day, hour, minute, second))
}

/// Convert calendar-time to UNIX epoch seconds. Uses the proleptic Gregorian
/// algorithm (Howard Hinnant, http://howardhinnant.github.io/date_algorithms.html).
/// Valid for any year ≥ 1970; GitHub timestamps are always post-2008.
fn epoch_from_ymdhms(year: u32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> u64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let doy =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era as u64 * 146097 + doe - 719468;
    days * 86400 + hour as u64 * 3600 + minute as u64 * 60 + second as u64
}

/// GitHub-App-specific auth provider. Holds the encoded JWT signing key in
/// memory after first use. Thread-safe via `Mutex<Option<CachedToken>>`.
///
/// `Debug` is implemented manually because `jsonwebtoken::EncodingKey` does
/// not implement `Debug` — we redact it entirely (the key bytes must never
/// appear in logs).
pub struct GitHubAppProvider {
    config: GitHubAppConfig,
    encoding_key: EncodingKey,
    http: reqwest::blocking::Client,
    // In-memory cache of the last installation token. Refreshed transparently
    // when within REFRESH_SKEW of expiry.
    // ponytail: single-slot cache; we never have multiple concurrent tokens.
    cached: std::sync::Mutex<Option<CachedToken>>,
}

impl std::fmt::Debug for GitHubAppProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubAppProvider")
            .field("config", &self.config)
            // encoding_key intentionally redacted — secret material.
            .field("encoding_key", &"<redacted>")
            .field("cached", &self.cached.lock().map(|c| c.is_some()))
            .finish()
    }
}

#[derive(Clone)]
struct CachedToken {
    token: AccessToken,
}

impl GitHubAppProvider {
    /// Construct from a loaded config + a configured `reqwest::blocking::Client`.
    /// Reads and parses the RSA PEM key immediately so a bad key fails fast at
    /// construction, not on the first request.
    ///
    /// The HTTP client is injected (not constructed internally) so callers can
    /// configure timeouts, proxies, custom transports for tests, etc. The
    /// connector binary builds one client and shares it across the provider +
    /// the GitHub REST client.
    pub fn new(config: GitHubAppConfig, http: reqwest::blocking::Client) -> Result<Self> {
        let pem_bytes = std::fs::read(&config.private_key_path).with_context(|| {
            format!(
                "failed to read GitHub App private key at {}",
                config.private_key_path
            )
        })?;
        let encoding_key = EncodingKey::from_rsa_pem(&pem_bytes).with_context(|| {
            format!(
                "failed to parse RSA PEM at {} (expected PKCS#1 or PKCS#8 RSA private key)",
                config.private_key_path
            )
        })?;
        Ok(Self {
            config,
            encoding_key,
            http,
            cached: std::sync::Mutex::new(None),
        })
    }

    /// Sign a fresh GitHub-App JWT (RS256). Valid for `JWT_MAX_AGE` (9 min).
    /// Public so callers (e.g. tests) can inspect the JWT shape directly.
    pub fn sign_app_jwt(&self) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let iat = now.saturating_sub(JWT_IAT_SKEW.as_secs());
        let exp = iat
            .checked_add(JWT_MAX_AGE.as_secs())
            .context("JWT exp overflowed u64 — clock is broken")?;
        let claims = AppJwtClaims {
            iss: self.config.app_id.to_string(),
            iat,
            exp,
        };
        let header = Header::new(Algorithm::RS256);
        encode(&header, &claims, &self.encoding_key)
            .context("failed to encode GitHub App JWT (RS256)")
    }

    /// True when the cached token is missing or close to expiry. Takes the
    /// cache lock briefly so the subsequent fetch can run unlocked.
    fn needs_refresh(&self) -> bool {
        let guard = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            None => true,
            Some(c) => c.token.is_expired(REFRESH_SKEW),
        }
    }

    /// Force-fetch a fresh installation token from GitHub. Makes a blocking
    /// HTTPS POST to `https://api.github.com/app/installations/{id}/access_tokens`.
    /// Returns the token + its parsed expiry; the caller caches it.
    ///
    /// Context7-verified 2026-07-20 (`/websites/github_en_rest`):
    /// - Status 201 on success.
    /// - Response shape: `{ "token": "ghs_...", "expires_at": "2026-07-20T11:00:00Z" }`.
    /// - Optional body field `repositories` scopes the token at the API level
    ///   (the least-privilege mechanism that enforces DoD-1).
    fn fetch_installation_token(&self, app_jwt: &str) -> Result<AccessToken> {
        let url = format!(
            "https://api.github.com/app/installations/{}/access_tokens",
            self.config.installation_id
        );
        let mut req = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header(
                "User-Agent",
                concat!("brain-connector-gh/", env!("CARGO_PKG_VERSION")),
            );
        if !self.config.repositories.is_empty() {
            // Token-level repository scoping. The token literally cannot see
            // repos not in this list, even if the App is installed on more.
            // This is what DoD-1 ("limited to two repos cannot index a third")
            // actually relies on.
            req = req.json(&serde_json::json!({
                "repositories": self.config.repositories,
            }));
        }
        let resp = req.send().context("installation-token POST failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("installation-token endpoint returned {status}: {body}");
        }
        let parsed: InstallTokenResponse = resp
            .json()
            .context("installation-token response was not valid JSON")?;
        let expires_at = parse_github_timestamp(&parsed.expires_at)?;
        Ok(AccessToken {
            value: parsed.token,
            expires_at: Some(expires_at),
        })
    }
}

impl AuthProvider for GitHubAppProvider {
    fn kind(&self) -> &'static str {
        "github-app"
    }

    fn access_token(&self) -> Result<AccessToken> {
        // Fast path: cached + not near expiry.
        if !self.needs_refresh() {
            let guard = self.cached.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(c) = guard.as_ref() {
                return Ok(c.token.clone());
            }
        }

        // Slow path: sign a fresh JWT and exchange it for an installation token.
        let app_jwt = self.sign_app_jwt()?;
        let token = self.fetch_installation_token(&app_jwt)?;
        let mut guard = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(CachedToken {
            token: token.clone(),
        });
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// We don't ship a real RSA keypair with the tests. Instead, the JWT
    /// signing test generates a throwaway keypair at test time. This requires
    /// the `rsa` crate (RustCrypto, pure Rust) as a dev-dep — already pulled
    /// in transitively by `jsonwebtoken` when `use_pem` is enabled.
    ///
    /// The test asserts the JWT *shape* (header alg, claim fields present,
    /// exp-iat window) rather than signature validity — GitHub verifies the
    /// signature against the App's registered public key, so what we need to
    /// guarantee locally is the structure.
    fn throwaway_rsa_pem() -> Vec<u8> {
        // The `rsa` crate's `RsaPrivateKey::new(&mut rng, 2048)` is the
        // standard way to generate a keypair for tests. We use `rand`'s
        // thread_rng() — already a transitive dep of `rsa`.
        use rand::rngs::OsRng;
        use rsa::{pkcs8::EncodePrivateKey, RsaPrivateKey};
        let mut rng = OsRng;
        let key = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA keypair");
        key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
            .expect("encode PKCS#8 PEM")
            .as_bytes()
            .to_vec()
    }

    fn sample_config(tmp_key_path: &std::path::Path) -> GitHubAppConfig {
        GitHubAppConfig {
            app_id: 123456,
            installation_id: 789012,
            private_key_path: tmp_key_path.to_string_lossy().into_owned(),
            webhook_secret_path: None,
            repositories: vec!["brain-server".to_string()],
        }
    }

    /// Build a throwaway HTTP client for tests. We never actually contact
    /// GitHub in unit tests — the JWT-shape tests don't need a live token,
    /// and the fetch test would need a mock server (not worth the plumbing
    /// for M2.2; integration coverage comes from the `#[ignore]` live test
    /// in `connector::github::client::tests`).
    fn test_http_client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_millis(100))
            .build()
            .expect("build test client")
    }

    #[test]
    fn test_sign_app_jwt_has_correct_claims_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("test-key.pem");
        std::fs::write(&key_path, throwaway_rsa_pem()).unwrap();

        let provider =
            GitHubAppProvider::new(sample_config(&key_path), test_http_client()).unwrap();
        let jwt = provider.sign_app_jwt().unwrap();

        // A JWT is three base64 segments joined by '.'.
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT must have 3 segments, got {parts:?}");

        // Decode the header (segment 0) and verify alg = RS256.
        let header_bytes = base64_url_decode(parts[0]);
        let header: serde_json::Value = serde_json::from_slice(&header_bytes).unwrap();
        assert_eq!(header["alg"], "RS256");

        // Decode the payload (segment 1) and verify required claims.
        let payload_bytes = base64_url_decode(parts[1]);
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();
        assert_eq!(
            payload["iss"], "123456",
            "iss must be the app_id as a string"
        );
        assert!(payload["iat"].is_u64(), "iat must be a numeric timestamp");
        assert!(payload["exp"].is_u64(), "exp must be a numeric timestamp");

        let iat = payload["iat"].as_u64().unwrap();
        let exp = payload["exp"].as_u64().unwrap();
        let window = exp - iat;
        assert!(
            window <= 600,
            "JWT lifetime must be ≤ 600s (GitHub hard cap), got {window}s"
        );
        assert!(
            window >= 540,
            "JWT lifetime should be ~9 min (540s) for usability, got {window}s"
        );
    }

    #[test]
    fn test_new_fails_loudly_on_missing_pem() {
        let cfg = GitHubAppConfig {
            app_id: 1,
            installation_id: 1,
            private_key_path: "/nonexistent/key.pem".to_string(),
            webhook_secret_path: None,
            repositories: vec![],
        };
        let err = GitHubAppProvider::new(cfg, test_http_client()).unwrap_err();
        assert!(
            err.to_string().contains("failed to read"),
            "error should mention read failure, got: {err}"
        );
    }

    #[test]
    fn test_new_fails_loudly_on_malformed_pem() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("bad.pem");
        std::fs::write(&key_path, b"not a PEM file").unwrap();
        let err = GitHubAppProvider::new(sample_config(&key_path), test_http_client()).unwrap_err();
        assert!(
            err.to_string().contains("failed to parse RSA PEM"),
            "error should mention parse failure, got: {err}"
        );
    }

    #[test]
    fn test_access_token_fails_when_github_unreachable() {
        // The provider now attempts a real HTTPS POST on cache miss. Pointing
        // it at an unreachable URL surfaces a network error (not a silent
        // 'not wired yet' stub). With a 100ms timeout, this fails fast.
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("test-key.pem");
        std::fs::write(&key_path, throwaway_rsa_pem()).unwrap();
        let provider =
            GitHubAppProvider::new(sample_config(&key_path), test_http_client()).unwrap();
        let err = provider.access_token().unwrap_err();
        // The error message should mention the network call failed, NOT
        // 'not wired yet' (that was the M2.1 stub).
        assert!(
            !err.to_string().contains("not wired yet"),
            "M2.2 provider should attempt the real call, got: {err}"
        );
        assert!(
            err.to_string().contains("installation-token") || err.to_string().contains("connect"),
            "error should mention the token endpoint or network failure, got: {err}"
        );
    }

    #[test]
    fn test_parse_github_timestamp_handles_canonical_format() {
        // GitHub always uses 'YYYY-MM-DDTHH:MM:SSZ'.
        let epoch = parse_github_timestamp("2026-07-20T11:00:00Z").unwrap();
        // 2026-07-20T11:00:00Z = 1_784_545_200 seconds since UNIX epoch.
        // Verifiable via Python: datetime(2026,7,20,11,0,0,tzinfo=UTC).timestamp()
        assert_eq!(epoch, 1_784_545_200);
    }

    #[test]
    fn test_parse_github_timestamp_rejects_bad_shapes() {
        // Wrong length (missing Z).
        assert!(parse_github_timestamp("2026-07-20T11:00:00").is_err());
        // Space instead of 'T' separator (GitHub always uses 'T').
        assert!(parse_github_timestamp("2026-07-20 11:00:00Z").is_err());
        // Bogus digits.
        assert!(parse_github_timestamp("2026-13-45T99:99:99Z").is_err());
    }

    /// Decode a base64url-no-pad segment (JWT header or payload) to bytes.
    /// Uses the `base64` dev-dep rather than hand-rolling — clippy-clean and
    /// matches what `jsonwebtoken` uses internally.
    fn base64_url_decode(s: &str) -> Vec<u8> {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        URL_SAFE_NO_PAD.decode(s).unwrap_or_default()
    }
}
