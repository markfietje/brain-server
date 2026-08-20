//! Auth route handlers.
//!
//! - `POST /auth/refresh` — exchange a refresh token for a new access token
//!   + rotate the refresh token (reuse detection revokes the chain).
//! - `POST /auth/logout` — add the request's access-token `jti` to the denylist.
//! - `POST /auth/revoke` — operator/admin revokes a specific `jti` by id.
//!
//! Token minting: `/auth/refresh` signs new tokens with the server's current
//! signing key. Access tokens are 15min; refresh tokens are 24h. Both are JWS
//! (RS256 by default); the algorithm follows the signing key's algorithm.
//!
//! These routes are PUBLIC (no auth_middleware) for `/auth/refresh` and
//! `/auth/logout` — they verify the presented token themselves (a refresh
//! token is the credential). `/auth/revoke` requires admin auth.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::jwt::{verify_access_token, TokenType};
use crate::auth::revocation::{record_and_rotate, revoke, RefreshError};
use crate::auth::{AuthError, Claims};
use crate::AppState;

/// Access-token lifetime. OWASP JWT Cheat Sheet: ≤15 min for access tokens.
const ACCESS_LIFETIME_SECS: u64 = 15 * 60;

/// Refresh-token lifetime. 24h — the cheat-sheet upper bound. Refresh tokens
/// rotate on every use (reuse detection revokes the chain).
const REFRESH_LIFETIME_SECS: u64 = 24 * 60 * 60;

/// Request body for `/auth/refresh`. The refresh token is the credential.
#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Response body for `/auth/refresh` + `/auth/login` (future). Both tokens
/// are opaque strings the client treats as bearer credentials.
#[derive(Debug, Serialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
}

/// Mint a fresh access + refresh token pair from the verified claims of the
/// presented refresh token. The new refresh token's `jti` is random (UUIDv4);
/// the chain id is inherited so the family is traceable.
fn mint_pair(
    signing_kid: &str,
    encoding_key: &EncodingKey,
    alg: Algorithm,
    issuer: &str,
    audience: &str,
    source: &Claims,
    _chain_id: &str,
) -> Result<TokenPair, String> {
    let now = now_unix();
    let access_jti = uuid_v4();
    let refresh_jti = uuid_v4();

    let access_claims = Claims {
        iss: issuer.to_string(),
        aud: audience.to_string(),
        sub: source.sub.clone(),
        jti: access_jti.clone(),
        iat: now,
        nbf: now,
        exp: now + ACCESS_LIFETIME_SECS,
        tenant: source.tenant.clone(),
        scopes: source.scopes.clone(),
        roles: source.roles.clone(),
        manages: source.manages.clone(),
    };
    let mut access_header = Header::new(alg);
    access_header.kid = Some(signing_kid.to_string());
    let access_token =
        encode(&access_header, &access_claims, encoding_key).map_err(|e| e.to_string())?;

    let refresh_claims = Claims {
        iss: issuer.to_string(),
        aud: audience.to_string(),
        sub: source.sub.clone(),
        jti: refresh_jti.clone(),
        iat: now,
        nbf: now,
        exp: now + REFRESH_LIFETIME_SECS,
        tenant: source.tenant.clone(),
        scopes: Vec::new(), // refresh tokens carry no scopes
        roles: Vec::new(),  // nor roles — not presented to data routes
        manages: Vec::new(),
    };
    let mut refresh_header = Header::new(alg);
    refresh_header.kid = Some(signing_kid.to_string());
    refresh_header.typ = Some("refresh".to_string());
    let refresh_token =
        encode(&refresh_header, &refresh_claims, encoding_key).map_err(|e| e.to_string())?;

    Ok(TokenPair {
        access_token,
        refresh_token,
        token_type: "Bearer",
        expires_in: ACCESS_LIFETIME_SECS,
    })
}

/// `POST /auth/refresh`. Verifies the presented refresh token, detects reuse,
/// rotates the chain, returns a fresh access + refresh token pair.
pub async fn refresh(
    State(s): State<Arc<AppState>>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<TokenPair>, AuthHandlerError> {
    let mode = s.auth_mode;
    if !mode.is_jwt() {
        return Err(AuthHandlerError::jwt_unavailable());
    }
    let issuer = s.jwt_issuer.clone();
    let audience = s.jwt_audience.clone();
    let keys = s.key_store.verifying_keys();

    // Phase 1: verify the refresh token cryptographically.
    let (claims, _) = verify_access_token(
        &req.refresh_token,
        &keys,
        &issuer,
        &audience,
        TokenType::Refresh,
    )
    .map_err(AuthHandlerError::from_auth)?;

    // Phase 2: check + rotate the chain. This is where reuse is detected.
    let chain_id = derive_chain_id(&claims);
    let pool = s.pool.clone();
    let key_store = s.key_store.clone();
    let rev_cache = s.revocation_cache.clone();
    let issuer_clone = issuer.clone();
    let audience_clone = audience.clone();
    let refresh_token = req.refresh_token.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<TokenPair, AuthHandlerError> {
        let conn = pool.get().map_err(|_| AuthHandlerError::internal())?;
        let signing = key_store
            .signing_key()
            .ok_or_else(AuthHandlerError::no_signing_key)?;
        let encoding_key = build_encoding_key(signing)?;
        let new_pair = mint_pair(
            &signing.kid,
            &encoding_key,
            signing.verifying.alg,
            &issuer_clone,
            &audience_clone,
            &claims,
            &chain_id,
        )
        .map_err(AuthHandlerError::internal_msg)?;
        // `record_and_rotate` runs the reuse check + chain
        // rotation under `BEGIN IMMEDIATE` so two concurrent presentations of
        // the same refresh token cannot both pass (the prior check-then-act
        // race). On reuse it burns the chain exactly once and returns the
        // error after the burn is committed.
        match record_and_rotate(
            &conn,
            &chain_id,
            &claims.iss,
            &extract_jti(&new_pair.refresh_token).unwrap_or_default(),
            &claims.jti,
            claims.exp,
        ) {
            Ok(()) => {
                rev_cache.invalidate(&claims.jti, &claims.iss);
                Ok(new_pair)
            }
            Err(RefreshError::ReuseDetected) | Err(RefreshError::ChainBurned) => {
                Err(AuthHandlerError::reuse_detected())
            }
            Err(_) => Err(AuthHandlerError::internal()),
        }
    })
    .await
    .map_err(|_| AuthHandlerError::internal())?;
    let _ = refresh_token; // consumed by verification above
    result.map(Json)
}

/// `POST /auth/logout`. Adds the request's access-token `jti` to the denylist.
/// The access token comes from the `Authorization: Bearer` header (verified by
/// the middleware before this handler runs, so the principal is authenticated).
pub async fn logout(
    State(s): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<StatusCode, AuthHandlerError> {
    let Some(p) = principal.0 else {
        return Ok(StatusCode::UNAUTHORIZED);
    };
    let pool = s.pool.clone();
    let cache = s.revocation_cache.clone();
    let issuer = s.jwt_issuer.clone();
    // a failed denylist write must surface. An
    // operator logging out believes the token is dead; if the INSERT failed
    // the token would live its full 15 min with that lie in the client.
    tokio::task::spawn_blocking(move || -> Result<(), rusqlite::Error> {
        let conn = pool.get().map_err(|e| {
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(e.to_string()))
        })?;
        revoke(
            &conn,
            &p.jti,
            &issuer,
            Some(&p.sub),
            now_unix() + ACCESS_LIFETIME_SECS,
            Some(&p.sub),
            "logout",
        )?;
        cache.invalidate(&p.jti, &issuer);
        Ok(())
    })
    .await
    .map_err(|_| AuthHandlerError::internal())?
    .map_err(|_| AuthHandlerError::revoke_failed("logout denylist write failed"))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /auth/revoke` (operator). Body: `{ jti, iss, reason }`. Requires
/// admin auth. Used to revoke a specific token without the holder presenting it.
#[derive(Debug, Deserialize)]
pub struct RevokeRequest {
    pub jti: String,
    pub iss: String,
    #[serde(default = "default_revoke_reason")]
    pub reason: String,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

fn default_revoke_reason() -> String {
    "operator_revoked".to_string()
}

pub async fn revoke_handler(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Json(req): Json<RevokeRequest>,
) -> Result<StatusCode, AuthHandlerError> {
    // the route comment says "requires admin auth" — enforce
    // it. `None` (no JWT) = superuser (v1.1 opaque back-compat).
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")
        .map_err(|e| AuthHandlerError::forbidden(e.inner.message))?;
    let pool = s.pool.clone();
    let cache = s.revocation_cache.clone();
    let exp = req
        .expires_at
        .unwrap_or_else(|| now_unix() + ACCESS_LIFETIME_SECS);
    let jti = req.jti.clone();
    let iss = req.iss.clone();
    let reason = req.reason.clone();
    // was 204-always — a failed denylist INSERT told
    // the operator the token was dead when it wasn't. Now 500 `revoke_failed`.
    tokio::task::spawn_blocking(move || -> Result<(), rusqlite::Error> {
        let conn = pool.get().map_err(|e| {
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(e.to_string()))
        })?;
        revoke(&conn, &jti, &iss, None, exp, None, &reason)?;
        cache.invalidate(&jti, &iss);
        Ok(())
    })
    .await
    .map_err(|_| AuthHandlerError::internal())?
    .map_err(|_| AuthHandlerError::revoke_failed("revocation denylist write failed"))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Extractor that pulls the `Principal` from request extensions (set by the
/// auth middleware). `None` when not authenticated (opaque/no-auth mode).
pub struct OptPrincipal(pub Option<crate::auth::Principal>);

impl<S> axum::extract::FromRequestParts<S> for OptPrincipal
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let p = parts.extensions.get::<crate::auth::Principal>().cloned();
        Ok(OptPrincipal(p))
    }
}

/// §5.2: the capability token the auth middleware injected when
/// a bearer verified as an operator-signed UMP capability token on the UMP
/// surface (`/ump/*` + `/export`). `None` for every other auth path — the
/// handler's `cap_gate` is then a no-op.
pub struct OptCapability(pub Option<brain_server::ump_integrity::CapabilityToken>);

impl<S> axum::extract::FromRequestParts<S> for OptCapability
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let c = parts
            .extensions
            .get::<brain_server::ump_integrity::CapabilityToken>()
            .cloned();
        Ok(OptCapability(c))
    }
}

/// Error envelope for auth handlers. Maps cleanly to HTTP statuses.
#[derive(Debug)]
pub struct AuthHandlerError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl AuthHandlerError {
    pub fn from_auth(e: AuthError) -> Self {
        let status = match &e {
            AuthError::WeakAlgorithm(_)
            | AuthError::HmacForbidden
            | AuthError::Malformed
            | AuthError::MissingKeyId
            | AuthError::UnknownKeyId(_)
            | AuthError::BadSignature
            | AuthError::InvalidClaim(_)
            | AuthError::MissingJti
            | AuthError::WrongType
            | AuthError::Other(_) => StatusCode::UNAUTHORIZED,
        };
        AuthHandlerError {
            status,
            code: e.code(),
            message: e.to_string(),
        }
    }

    pub fn reuse_detected() -> Self {
        AuthHandlerError {
            status: StatusCode::FORBIDDEN,
            code: "refresh_reuse_detected",
            message: "refresh token reuse detected; chain revoked".to_string(),
        }
    }

    pub fn jwt_unavailable() -> Self {
        AuthHandlerError {
            status: StatusCode::NOT_FOUND,
            code: "jwt_unavailable",
            message: "JWT auth not configured (set BRAIN_JWT_ISSUER)".to_string(),
        }
    }

    pub fn no_signing_key() -> Self {
        AuthHandlerError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "no_signing_key",
            message: "no signing key configured; cannot mint tokens".to_string(),
        }
    }

    pub fn internal() -> Self {
        AuthHandlerError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal",
            message: "internal error".to_string(),
        }
    }

    pub fn internal_msg(msg: String) -> Self {
        AuthHandlerError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal",
            message: msg,
        }
    }

    pub fn forbidden(msg: String) -> Self {
        AuthHandlerError {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: msg,
        }
    }

    /// the revocation denylist write failed — an
    /// operator must never believe a token dead when it isn't.
    pub fn revoke_failed(msg: &str) -> Self {
        AuthHandlerError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "revoke_failed",
            message: msg.to_string(),
        }
    }
}

impl IntoResponse for AuthHandlerError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({
                "code": self.code,
                "error": self.message,
            })),
        )
            .into_response()
    }
}

// ── helpers ──────────────────────────────────────────────────────────

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Stable chain id from a refresh token's `(iss, sub)` pair. The chain is
/// per-user per-issuer; a new login session (different sub or iss) gets a
/// new chain. This is the OWASP "family" identifier.
fn derive_chain_id(claims: &Claims) -> String {
    // SHA-256 of (iss, sub) → hex. Stable across rotations within a session.
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(claims.iss.as_bytes());
    h.update(b"|");
    h.update(claims.sub.as_bytes());
    format!("{:x}", h.finalize())
}

/// Parse the `jti` out of a just-minted token (without verifying — we just
/// signed it). Used to feed rotate_chain. Returns None if the token is
/// malformed (shouldn't happen since we just minted it).
fn extract_jti(raw: &str) -> Option<String> {
    // Cheap: the payload is the middle segment. Decode + read `jti`.
    use base64::Engine as _;
    let segs: Vec<&str> = raw.split('.').collect();
    let payload = segs.get(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("jti")?.as_str().map(|s| s.to_string())
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Build an `EncodingKey` from a private PEM. The algorithm comes from the
/// managed key; we try RSA, EC, Ed in order (mirrors `parse_public_pem`).
fn build_encoding_key(mk: &crate::auth::jwks::ManagedKey) -> Result<EncodingKey, AuthHandlerError> {
    let pem = mk
        .private_pem
        .as_ref()
        .ok_or_else(AuthHandlerError::no_signing_key)?;
    let alg = mk.verifying.alg;
    match alg {
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 => {
            EncodingKey::from_rsa_pem(pem.as_bytes())
                .map_err(|e| AuthHandlerError::internal_msg(e.to_string()))
        }
        Algorithm::ES256 | Algorithm::ES384 => EncodingKey::from_ec_pem(pem.as_bytes())
            .map_err(|e| AuthHandlerError::internal_msg(e.to_string())),
        Algorithm::EdDSA => EncodingKey::from_ed_pem(pem.as_bytes())
            .map_err(|e| AuthHandlerError::internal_msg(e.to_string())),
        _ => Err(AuthHandlerError::internal_msg(format!(
            "unsupported signing alg {alg:?}"
        ))),
    }
}
