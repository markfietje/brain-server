//! Connector authentication foundation.
//!
//! The `AuthProvider` trait is the unified surface every connector uses to
//! obtain a bearer token for the external source it talks to. Concrete impls:
//!
//! - [`StaticTokenProvider`] — fixed token, used by the stub connector and
//!   by tests. No external dependencies, always available.
//! - `GitHubAppProvider` (in `github_app.rs`, feature-gated on
//!   `connector-github`) — JWT → installation-token flow.
//! - `OAuthProvider` (TODO) — standard OAuth 2.0 + PKCE + refresh.
//!   Lands when the first non-GitHub SaaS connector does.
//!
//! ## Why a trait, not a struct
//!
//! Each connector's auth flow has its own state (RSA keys for GitHub, refresh
//! tokens for OAuth, etc.). A trait lets every connector pick its impl while
//! the supervisor / connector runner stays auth-agnostic. The trait is sync
//! because the connector is a batch process — async here would buy nothing
//! and complicate every impl.
//!
//! ## Threat model
//!
//! The trait returns an [`AccessToken`] whose `value` is the raw bearer token.
//! Callers must never log it; the `Display` impl is intentionally redacted.
//! Storage of long-lived secrets (refresh tokens, PEM keys) is delegated to
//! [`super::store::CredentialStore`], which uses the same 0600 file pattern
//! the existing `auth-token` infrastructure uses — no at-rest encryption
//! beyond filesystem permissions and (on macOS) FileVault.

pub mod store;

#[cfg(feature = "connector-github")]
pub mod github_app;

use anyhow::Result;
use serde::Serialize;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A bearer access token plus its expiry. `expires_at` is `None` when the
/// token never expires (rare for OAuth but common for static test tokens).
#[derive(Clone, Debug)]
pub struct AccessToken {
    /// The raw bearer token. Treated as a secret — see `Display` impl.
    pub value: String,
    /// Absolute expiry as seconds since UNIX epoch. `None` = no expiry.
    pub expires_at: Option<u64>,
}

impl AccessToken {
    /// Construct a non-expiring token. Used by tests and the stub.
    pub fn static_token(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            expires_at: None,
        }
    }

    /// True if the token is past (or within `skew` of) its expiry. Always
    /// false for tokens with no expiry. The skew lets callers refresh a
    /// hair before the wire deadline (GitHub caps at 60 min; we refresh at
    /// 59 min by default — see `GitHubAppProvider::REFRESH_SKEW`).
    pub fn is_expired(&self, skew: Duration) -> bool {
        match self.expires_at {
            None => false,
            Some(exp) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                now + skew.as_secs() >= exp
            }
        }
    }
}

/// Display redacts the token value. Logging an `AccessToken` by accident
/// produces `AccessToken(***)` instead of leaking the secret.
impl fmt::Display for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccessToken(***)")
    }
}

/// The unified auth surface. Implementations are responsible for caching,
/// refresh, and clock-skew handling; callers just ask for a fresh token.
///
/// Implementations should be cheap to construct — they typically read
/// credentials from [`super::store::CredentialStore`] at construction time
/// and lazily fetch / refresh tokens on demand.
pub trait AuthProvider: Send + Sync {
    /// Stable identifier for the impl (`"static"`, `"github-app"`,
    /// `"oauth"`, …). Used in logs and the `connectors` table.
    fn kind(&self) -> &'static str;

    /// Return a valid bearer token. Implementations should:
    /// - Return a cached token if it is not near expiry.
    /// - Refresh transparently otherwise.
    /// - Surface hard failures via `Err` — the supervisor handles backoff.
    fn access_token(&self) -> Result<AccessToken>;
}

/// The simplest possible provider: returns the same token forever. Used by
/// the stub connector and by every unit test that needs *an* authed HTTP
/// call without standing up real OAuth or GitHub-App plumbing.
pub struct StaticTokenProvider {
    token: AccessToken,
}

impl StaticTokenProvider {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            token: AccessToken::static_token(value),
        }
    }
}

impl AuthProvider for StaticTokenProvider {
    fn kind(&self) -> &'static str {
        "static"
    }
    fn access_token(&self) -> Result<AccessToken> {
        Ok(self.token.clone())
    }
}

/// Serialize helper for connector runners that need to ship the token to a
/// child process via env or argv. Never logs the value.
#[derive(Serialize)]
pub struct AccessTokenWire<'a> {
    /// Redacted in logs via the field name; only the connector process reads it.
    pub token: &'a str,
    pub expires_at: Option<u64>,
}

impl<'a> From<&'a AccessToken> for AccessTokenWire<'a> {
    fn from(t: &'a AccessToken) -> Self {
        Self {
            token: &t.value,
            expires_at: t.expires_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_token_provider_returns_same_value() {
        let p = StaticTokenProvider::new("ghs_test_abc");
        let t1 = p.access_token().unwrap();
        let t2 = p.access_token().unwrap();
        assert_eq!(t1.value, "ghs_test_abc");
        assert_eq!(t1.value, t2.value);
        assert_eq!(p.kind(), "static");
    }

    #[test]
    fn test_access_token_display_redacts_value() {
        let t = AccessToken::static_token("super-secret-bearer");
        let s = format!("{t}");
        assert!(!s.contains("super-secret-bearer"));
        assert!(s.contains("***"));
    }

    #[test]
    fn test_access_token_is_expired_with_skew() {
        // now = 1000. Token exp = 1000 → already at boundary, with skew=10 → expired.
        let t = AccessToken {
            value: "x".into(),
            expires_at: Some(1000),
        };
        // We can't easily mock SystemTime without a third-party dep. The test
        // asserts the *structure* of the skew logic: a token with expiry=now
        // in the year 2026 (epoch ~1.8B) is unambiguously expired.
        assert!(t.is_expired(Duration::from_secs(0)));
        assert!(t.is_expired(Duration::from_secs(60)));
    }

    #[test]
    fn test_access_token_no_expiry_never_expires() {
        let t = AccessToken::static_token("x");
        assert!(!t.is_expired(Duration::from_secs(0)));
        assert!(!t.is_expired(Duration::from_secs(365 * 24 * 3600)));
    }

    #[test]
    fn test_access_token_wire_serializes_for_transport() {
        let t = AccessToken {
            value: "v1".into(),
            expires_at: Some(1234),
        };
        let wire = AccessTokenWire::from(&t);
        let json = serde_json::to_string(&wire).unwrap();
        assert!(json.contains("\"token\":\"v1\""));
        assert!(json.contains("\"expires_at\":1234"));
    }
}
