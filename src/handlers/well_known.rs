//! OIDC discovery + JWKS endpoints (v1.2.0 "AuthN" M4).
//!
//! - `GET /.well-known/openid-configuration` — OIDC Discovery metadata (RFC 8414).
//! - `GET /.well-known/jwks.json` — JWK Set (RFC 7517) for key distribution.
//!
//! Both routes are PUBLIC (no auth) — this is how external clients discover
//! the server's signing keys + endpoints. The issuer is pinned to
//! `BRAIN_PUBLIC_BASE_URL` (never inferred from the `Host` header — OWASP
//! A02:2025 Security Misconfiguration: an attacker who can set the Host
//! header could otherwise redirect discovery to a malicious endpoint).

use std::sync::Arc;

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::auth::jwks::{JwkSet, KeyStore};
use crate::AppState;

/// `GET /.well-known/openid-configuration`. Returns the OIDC Discovery
/// metadata. The `jwks_uri` + `issuer` are derived from `BRAIN_PUBLIC_BASE_URL`.
pub async fn openid_configuration(State(s): State<Arc<AppState>>) -> Json<OidcConfig> {
    Json(s.oidc_config.clone())
}

/// `GET /.well-known/jwks.json`. Returns the current signing key set as a
/// RFC 7517 JWK Set JSON document. Cached in the KeyStore; rebuilt on rotation.
pub async fn jwks(State(s): State<Arc<AppState>>) -> Response {
    match build_jwks_response(&s.key_store) {
        Ok(resp) => resp,
        Err(_) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "code": "jwks_unavailable",
                "error": "no RSA signing keys configured for JWKS emission"
            })),
        )
            .into_response(),
    }
}

/// `GET /.well-known/security.txt` (RFC 9116). Public/no-auth — the disclosure
/// endpoint procurement and the EU Cyber Resilience Act look for. `Contact`
/// defaults to the project's private-vuln-reporting URL; override with
/// `BRAIN_SECURITY_CONTACT`. `Expires` is computed (now + 1 year) so it never
/// goes stale. `Canonical` is included when `BRAIN_PUBLIC_BASE_URL` is set.
pub async fn security_txt() -> Response {
    // Default matches SECURITY.md's disclosure address; override with
    // BRAIN_SECURITY_CONTACT (mailto: or https:// to a private-vuln-report URL).
    let contact = std::env::var("BRAIN_SECURITY_CONTACT")
        .unwrap_or_else(|_| "mailto:security@openclaw.dev".to_string());
    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::TimeDelta::days(365))
        .map(|d| d.to_rfc3339())
        .unwrap_or_default();
    let canonical = std::env::var("BRAIN_PUBLIC_BASE_URL").ok().map(|b| {
        let base = b.trim_end_matches('/');
        format!("{base}/.well-known/security.txt")
    });
    let body = build_security_txt(&contact, &expires, canonical.as_deref());
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

/// Pure builder for the RFC 9116 `security.txt` body. Split out so the field
/// layout is unit-tested without env/time.
fn build_security_txt(contact: &str, expires: &str, canonical: Option<&str>) -> String {
    let mut s = format!("Contact: {contact}\nExpires: {expires}\nPreferred-Languages: en\n");
    if let Some(c) = canonical {
        s.push_str(&format!("Canonical: {c}\n"));
    }
    s
}

/// Build the JWKS HTTP response once, with a long cache header. Clients cache
/// the key set; the cache header tells them how long. During rotation, the
/// old key stays in the set until every cached client's token has expired,
/// so a stale cache can't break verification.
fn build_jwks_response(store: &KeyStore) -> Result<Response, ()> {
    let jwks: JwkSet = store.to_jwks().map_err(|_| ())?;
    let body = serde_json::to_string(&jwks).map_err(|_| ())?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/jwk-set+json"),
            // 1h cache. RFC 8414 §3.5: clients SHOULD cache discovery; the
            // same applies to JWKS. Rotation keeps the old key live > cache TTL.
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response())
}

/// The OIDC Discovery metadata. Serialized to JSON as-is. Fields per RFC 8414
/// + the OIDC Core spec; only the ones brain-server actually supports.
#[derive(Debug, Clone, Serialize)]
pub struct OidcConfig {
    pub issuer: String,
    pub jwks_uri: String,
    pub token_endpoint: String,
    pub revocation_endpoint: String,
    pub id_token_signing_alg_values_supported: Vec<&'static str>,
    pub scopes_supported: Vec<&'static str>,
    pub response_types_supported: Vec<&'static str>,
    pub subject_types_supported: Vec<&'static str>,
    pub claims_supported: Vec<&'static str>,
}

impl OidcConfig {
    /// Build the discovery doc from the public base URL. The base URL must
    /// be configured explicitly via `BRAIN_PUBLIC_BASE_URL` — never inferred
    /// from the request (Host header spoofing).
    pub fn build(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        OidcConfig {
            issuer: format!("{base}/"),
            jwks_uri: format!("{base}/.well-known/jwks.json"),
            token_endpoint: format!("{base}/auth/refresh"),
            revocation_endpoint: format!("{base}/auth/revoke"),
            id_token_signing_alg_values_supported: vec![
                "RS256", "RS384", "RS512", "ES256", "ES384", "EdDSA",
            ],
            scopes_supported: vec!["openid", "read", "write", "admin"],
            response_types_supported: vec!["token"],
            subject_types_supported: vec!["public"],
            claims_supported: vec![
                "iss", "aud", "sub", "jti", "iat", "nbf", "exp", "tenant", "scopes",
            ],
        }
    }

    /// Placeholder config when JWT is not configured. The discovery endpoint
    /// still serves something (clients probing for OIDC support get a clear
    /// "not configured" signal rather than a 404 that looks like a routing bug).
    pub fn unconfigured() -> Self {
        OidcConfig {
            issuer: String::new(),
            jwks_uri: String::new(),
            token_endpoint: String::new(),
            revocation_endpoint: String::new(),
            id_token_signing_alg_values_supported: vec![],
            scopes_supported: vec![],
            response_types_supported: vec![],
            subject_types_supported: vec![],
            claims_supported: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oidc_config_derives_endpoints_from_base() {
        let c = OidcConfig::build("https://brain.example.com");
        assert_eq!(c.issuer, "https://brain.example.com/");
        assert_eq!(
            c.jwks_uri,
            "https://brain.example.com/.well-known/jwks.json"
        );
        assert_eq!(c.token_endpoint, "https://brain.example.com/auth/refresh");
    }

    #[test]
    fn oidc_config_trims_trailing_slash() {
        let c = OidcConfig::build("https://brain.example.com/");
        assert_eq!(c.issuer, "https://brain.example.com/");
    }

    #[test]
    fn security_txt_has_rfc9116_fields_and_optional_canonical() {
        let without = build_security_txt("mailto:sec@example.com", "2030-01-01T00:00:00Z", None);
        assert!(without.contains("Contact: mailto:sec@example.com"));
        assert!(without.contains("Expires: 2030-01-01T00:00:00Z"));
        assert!(without.contains("Preferred-Languages: en"));
        assert!(!without.contains("Canonical"));

        let with = build_security_txt(
            "mailto:sec@example.com",
            "2030-01-01T00:00:00Z",
            Some("https://brain.example.com/.well-known/security.txt"),
        );
        assert!(with.contains("Canonical: https://brain.example.com/.well-known/security.txt"));
    }
}
