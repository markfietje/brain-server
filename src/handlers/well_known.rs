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
/// endpoint procurement and the EU Cyber Resilience Act look for. `Contact` is
/// read from `BRAIN_SECURITY_CONTACT` (mailto: or https:// to a private-vuln-
/// report URL) and omitted when unset, so the operator owns the address.
/// `Expires` is computed (now + 1 year) so it never goes stale. `Canonical` is
/// included when `BRAIN_PUBLIC_BASE_URL` is set.
pub async fn security_txt() -> Response {
    let contact = std::env::var("BRAIN_SECURITY_CONTACT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::TimeDelta::days(365))
        .map(|d| d.to_rfc3339())
        .unwrap_or_default();
    let canonical = std::env::var("BRAIN_PUBLIC_BASE_URL").ok().map(|b| {
        let base = b.trim_end_matches('/');
        format!("{base}/.well-known/security.txt")
    });
    let body = build_security_txt(contact.as_deref(), &expires, canonical.as_deref());
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

/// `GET /.well-known/ai-notice` (EU AI Act Art 50 transparency). Public/no-auth.
/// Machine-readable disclosure that this service stores, retrieves, and may
/// return AI-generated or AI-processed content, so consumers can mark it as
/// such — the Art 50 model-origin transparency obligation. Version-tagged so
/// the notice can evolve without breaking consumers.
pub async fn ai_notice() -> Response {
    let body = build_ai_notice();
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

/// `GET /.well-known/ai-literacy` (EU AI Act Art 4). Public/no-auth.
/// Machine-readable pointer to the operator's AI-literacy playbook
/// (`docs/AI_LITERACY.md`), stating which controls make the component's
/// decisions inspectable — the operational substance of Art 4 literacy for a
/// memory component. The doc file stays the artifact; this route makes it
/// discoverable next to the Art 50 notice.
pub async fn ai_literacy() -> Response {
    let body = build_ai_literacy();
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

/// Pure builder for the Art 4 literacy disclosure. Returns a JSON document
/// pointing at the playbook and enumerating the inspectable controls.
fn build_ai_literacy() -> String {
    serde_json::json!({
        "schema_version": "1.0",
        "service": "brain-server",
        "art_4": true,
        "disclosure": "This component is an inspectable memory store: it stores, retrieves, and proposes, but generates no content. Its decisions are inspectable via the recall trace, the write-approval queue, quarantine, the audit chain, and the DSAR console — the deployer's AI-literacy surface.",
        "playbook": "https://github.com/markfietje/brain-server/blob/main/docs/AI_LITERACY.md",
        "inspectable_controls": ["recall_trace", "proposal_gate", "quarantine", "audit_chain", "dsar_console"],
        "effective_date": "2026-08-08",
        "jurisdiction": "EU AI Act Article 4 (Regulation (EU) 2024/1689)"
    })
    .to_string()
}

/// Pure builder for the Art 50 transparency notice. Returns a JSON document
/// describing how this service handles AI-generated content.
fn build_ai_notice() -> String {
    serde_json::json!({
        "schema_version": "1.0",
        "service": "brain-server",
        "art_50": true,
        "disclosure": "This service stores, retrieves, and may return content that is AI-generated, AI-processed, or otherwise not of human origin. Consumers should treat retrieved content as AI-derived and mark any human-facing output accordingly.",
        "origin_metadata": ["source", "assertion_kind", "confidence"],
        "effective_date": "2026-08-02",
        "jurisdiction": "EU AI Act Article 50 (Regulation (EU) 2024/1689)"
    })
    .to_string()
}

/// Pure builder for the RFC 9116 `security.txt` body. Split out so the field
/// layout is unit-tested without env/time. `Contact` is emitted only when the
/// operator configured an address.
fn build_security_txt(contact: Option<&str>, expires: &str, canonical: Option<&str>) -> String {
    let mut s = String::from("Expires: ");
    s.push_str(expires);
    s.push_str("\nPreferred-Languages: en\n");
    if let Some(c) = contact {
        s.push_str(&format!("Contact: {c}\n"));
    }
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
        let without =
            build_security_txt(Some("mailto:sec@example.com"), "2030-01-01T00:00:00Z", None);
        assert!(without.contains("Contact: mailto:sec@example.com"));
        assert!(without.contains("Expires: 2030-01-01T00:00:00Z"));
        assert!(without.contains("Preferred-Languages: en"));
        assert!(!without.contains("Canonical"));

        let with = build_security_txt(
            Some("mailto:sec@example.com"),
            "2030-01-01T00:00:00Z",
            Some("https://brain.example.com/.well-known/security.txt"),
        );
        assert!(with.contains("Canonical: https://brain.example.com/.well-known/security.txt"));
    }

    #[test]
    fn security_txt_omits_contact_when_unconfigured() {
        let out = build_security_txt(None, "2030-01-01T00:00:00Z", None);
        assert!(!out.contains("Contact:"));
    }

    #[test]
    fn ai_literacy_discloses_controls_and_playbook() {
        let body = build_ai_literacy();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["art_4"], true);
        assert_eq!(v["schema_version"], "1.0");
        assert!(v["disclosure"].as_str().unwrap().contains("inspectable"));
        assert!(v["inspectable_controls"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("recall_trace")));
        assert!(v["playbook"].as_str().unwrap().contains("AI_LITERACY.md"));
    }

    #[test]
    fn ai_notice_discloses_ai_origin_and_version() {
        let notice = build_ai_notice();
        let v: serde_json::Value = serde_json::from_str(&notice).unwrap();
        assert_eq!(v["art_50"], true);
        assert_eq!(v["schema_version"], "1.0");
        assert!(v["disclosure"].as_str().unwrap().contains("AI-generated"));
        assert!(v["origin_metadata"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("source")));
    }
}
