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

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::AppState;
use crate::auth::jwt::{TokenType, verify_access_token};
use crate::auth::revocation::{RefreshError, record_and_rotate, revoke};
use crate::auth::{AuthError, Claims};

/// Access-token lifetime. OWASP JWT Cheat Sheet: ≤15 min for access tokens.
const ACCESS_LIFETIME_SECS: u64 = 15 * 60;

/// Refresh-token lifetime. 24h — the cheat-sheet upper bound. Refresh tokens
/// rotate on every use (reuse detection revokes the chain).
const REFRESH_LIFETIME_SECS: u64 = 24 * 60 * 60;

/// Upper bound for a denylist row's lifetime. The row only needs to outlive
/// the token it denies; a longer cap changes nothing observable, and without
/// one a hostile or clock-wrong IdP token could pin rows to the bounded
/// table forever. `purge_expired`'s cadence is unchanged.
const DENYLIST_TTL_CAP_SECS: u64 = 24 * 60 * 60;

/// The denylist row's `expires_at` for a token whose (verified) `exp` claim
/// is `token_exp`, written at `now`. The row lives exactly as long as the
/// token it denies — the silent revocation lapse (a fixed 15-minute row
/// purged while an external-IdP token is still valid) is closed — clamped to
/// [`DENYLIST_TTL_CAP_SECS`]. `None` (no verified expiry in scope: the
/// operator-revoke route without an `expires_at`) keeps the old fixed-TTL
/// default. Server-minted 15-minute tokens: `exp ≤ now + 15 min`, so the
/// clamp never bites and the row dies with the token.
fn denylist_expires_at(token_exp: Option<u64>, now: u64) -> u64 {
    token_exp.map_or(now + ACCESS_LIFETIME_SECS, |exp| {
        exp.min(now + DENYLIST_TTL_CAP_SECS)
    })
}

/// The refresh-phase kill-switch: `/auth/refresh` is a PUBLIC
/// route, so the auth middleware's identity check never runs there — this
/// is the one door a revoked identity could keep walking through, rotating
/// its chain forever behind the revocation. Same store, same 401
/// `identity_revoked` code the middleware returns on the authenticated
/// routes. Pinned by `refresh_refuses_revoked_identity`.
fn refresh_principal_alive(conn: &rusqlite::Connection, sub: &str) -> Result<(), AuthHandlerError> {
    if crate::workflow::mesh::is_revoked(conn, sub).map_err(|_| AuthHandlerError::internal())? {
        return Err(AuthHandlerError::identity_revoked());
    }
    Ok(())
}

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
    chain_id: &str,
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
        chain: None, // access tokens never carry the refresh family
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
        chain: Some(chain_id.to_string()),
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
    // Prefer the presented token's own family id; legacy tokens without one
    // fall back to the derived per-(iss, sub) family (shared across that
    // user's concurrent sessions until they rotate into stamped chains).
    let chain_id = claims
        .chain
        .clone()
        .unwrap_or_else(|| derive_chain_id(&claims));
    let pool = s.pool.clone();
    let key_store = s.key_store.clone();
    let issuer_clone = issuer.clone();
    let audience_clone = audience.clone();
    let refresh_token = req.refresh_token.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<TokenPair, AuthHandlerError> {
        let conn = pool.get().map_err(|_| AuthHandlerError::internal())?;
        // Kill-switch FIRST: /auth/refresh is a PUBLIC route, so
        // the middleware's identity-revocation check never runs for it. A
        // revoked identity's refresh chain must die here too, or the family
        // keeps rotating forever behind the revocation (pinned by
        // `refresh_refuses_revoked_identity`).
        refresh_principal_alive(&conn, &claims.sub)?;
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
            Ok(()) => Ok(new_pair),
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
/// The row's `expires_at` is the token's REAL `exp` (injected by the JWT
/// middleware as [`crate::auth::AccessTokenExp`]), clamped — the row dies
/// when the token dies, not at a fixed guess after presentation.
pub async fn logout(
    State(s): State<Arc<AppState>>,
    principal: OptPrincipal,
    token_exp: Option<axum::extract::Extension<crate::auth::AccessTokenExp>>,
) -> Result<StatusCode, AuthHandlerError> {
    let Some(p) = principal.0 else {
        return Ok(StatusCode::UNAUTHORIZED);
    };
    let token_exp = token_exp.map(|ext| ext.0.0); // Option<u64>
    let pool = s.pool.clone();
    let issuer = s.jwt_issuer.clone();
    // a failed denylist write must surface. An
    // operator logging out believes the token is dead; if the denylist write failed
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
            denylist_expires_at(token_exp, now_unix()),
            Some(&p.sub),
            "logout",
        )?;
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
    // Bounds law (2026-09-11): the denylist table is size-bounded — cap the
    // caller-controlled key fields so each row stays a bounded record, not a
    // ~1 MiB blob riding the body cap (jti is a UUID ≤ 36; iss a URL).
    if req.jti.len() > 128 {
        return Err(AuthHandlerError {
            status: StatusCode::BAD_REQUEST,
            code: "jti_too_long",
            message: "jti exceeds 128 chars".to_string(),
        });
    }
    if req.iss.len() > 256 {
        return Err(AuthHandlerError {
            status: StatusCode::BAD_REQUEST,
            code: "iss_too_long",
            message: "iss exceeds 256 chars".to_string(),
        });
    }
    let pool = s.pool.clone();
    // The operator supplies the target token's real `exp` when it is known;
    // the clamp bounds the row either way (a hostile or clock-wrong value
    // cannot pin the bounded table).
    let exp = denylist_expires_at(req.expires_at, now_unix());
    let jti = req.jti.clone();
    let iss = req.iss.clone();
    let reason = req.reason.clone();
    // was 204-always — a failed denylist write told
    // the operator the token was dead when it wasn't. Now 500 `revoke_failed`.
    tokio::task::spawn_blocking(move || -> Result<(), rusqlite::Error> {
        let conn = pool.get().map_err(|e| {
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(e.to_string()))
        })?;
        revoke(&conn, &jti, &iss, None, exp, None, &reason)?;
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
pub struct OptCapability(pub Option<crate::ump_integrity::CapabilityToken>);

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
            .get::<crate::ump_integrity::CapabilityToken>()
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
            | AuthError::AlgMismatchForKid(_)
            | AuthError::BadSignature
            | AuthError::InvalidClaim(_)
            | AuthError::MissingJti
            | AuthError::TokenLifetimeExceeded
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

    /// The kill-switch reached the refresh seam: the same 401
    /// code the auth middleware returns on the authenticated routes — the
    /// identity is GONE, not merely unauthorized.
    pub fn identity_revoked() -> Self {
        AuthHandlerError {
            status: StatusCode::UNAUTHORIZED,
            code: "identity_revoked",
            message: "principal has been revoked".to_string(),
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

/// Stable chain id from a refresh token's `(iss, sub)` pair. LEGACY fallback:
/// tokens minted before per-login families carry no `chain` claim, so they
/// share one family per user per issuer (concurrent sessions rotate the same
/// chain — the OWASP "family" identifier in its original coarse form).
/// Current mints stamp a random per-login `chain` claim instead.
fn derive_chain_id(claims: &Claims) -> String {
    // SHA-256 of (iss, sub) → hex. Stable across rotations within a session.
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(claims.iss.as_bytes());
    h.update(b"|");
    h.update(claims.sub.as_bytes());
    h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
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

// ── the denylist TTL pins (the row dies when the token dies) ────────────
#[cfg(test)]
mod tests {
    use super::*;

    /// The refresh seam consults the kill-switch (v1.28.76): a revoked
    /// identity's `sub` refuses with the middleware's 401
    /// `identity_revoked` shape — the family cannot rotate behind the
    /// revocation; a clean sub passes. The revocation is written through
    /// the production core (the no-SQL-in-handlers law covers tests too).
    #[test]
    fn refresh_refuses_revoked_identity() {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migration::run_migration(&mut conn, 1).unwrap();
        assert!(refresh_principal_alive(&conn, "user@example").is_ok());
        crate::workflow::mesh::revoke_principal(
            &conn,
            "user@example",
            "second-pass drill",
            "op",
            1,
        )
        .unwrap();
        let err = refresh_principal_alive(&conn, "user@example").unwrap_err();
        assert_eq!(err.code, "identity_revoked");
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    /// A long-lived external-IdP token gets a denylist row that covers its
    /// real validity window (clamped), not the old fixed 15-minute guess —
    /// the silent revocation lapse is closed.
    #[test]
    fn logout_row_outlives_long_lived_idp_token() {
        let now = 1_800_000_000u64;
        let idp_exp = now + 30 * 24 * 3600; // an IdP token with a month of life
        let row = denylist_expires_at(Some(idp_exp), now);
        assert_eq!(row, now + DENYLIST_TTL_CAP_SECS);
        assert!(
            row > now + ACCESS_LIFETIME_SECS,
            "the row must outlive the old fixed TTL"
        );
    }

    /// The clamp: a hostile or clock-wrong `exp` cannot pin denylist rows to
    /// the bounded table forever; the purge cadence stays meaningful.
    #[test]
    fn denylist_row_capped_at_24h() {
        let now = 1_800_000_000u64;
        assert_eq!(
            denylist_expires_at(Some(now + 365 * 24 * 3600), now),
            now + 24 * 3600
        );
        assert_eq!(denylist_expires_at(Some(u64::MAX), now), now + 24 * 3600);
    }

    /// Server-minted 15-minute access tokens: the clamp never bites, so the
    /// row expires exactly at the token's real death — for a token presented
    /// at mint time that is the old `now + 15min` byte-for-byte. A missing
    /// expiry (the operator-revoke route without `expires_at`) keeps the old
    /// fixed-TTL default.
    #[test]
    fn server_minted_logout_unchanged() {
        let now = 1_800_000_000u64;
        let exp = now + ACCESS_LIFETIME_SECS;
        assert_eq!(denylist_expires_at(Some(exp), now), exp);
        assert_eq!(
            denylist_expires_at(Some(exp), now),
            now + ACCESS_LIFETIME_SECS
        );
        assert_eq!(denylist_expires_at(None, now), now + ACCESS_LIFETIME_SECS);
    }
    /// Per-login refresh families: the minted refresh token carries the
    /// passed chain id, and rotation prefers a presented chain over the
    /// derived per-(iss, sub) fallback (concurrent sessions stop sharing
    /// one family once they rotate).
    #[test]
    fn mint_stamps_chain_and_rotate_prefers_presented() {
        use rsa::pkcs8::EncodePrivateKey;
        let mut rng = rand::rngs::ThreadRng::default();
        let priv_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pem = priv_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
        let now = now_unix();
        let source = Claims {
            iss: "https://brain.test/".to_string(),
            aud: "brain-server".to_string(),
            sub: "user:test".to_string(),
            jti: "login-jti".to_string(),
            iat: now,
            nbf: now,
            exp: now + 600,
            tenant: "global".to_string(),
            scopes: vec![],
            roles: vec![],
            manages: vec![],
            chain: None,
        };
        let pair = mint_pair(
            "test-kid",
            &encoding,
            Algorithm::RS256,
            "https://brain.test/",
            "brain-server",
            &source,
            "sess-abc",
        )
        .expect("mint");
        let segs: Vec<&str> = pair.refresh_token.split('.').collect();
        assert_eq!(segs.len(), 3);
        use base64::Engine as _;
        let payload: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(segs[1])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["chain"], "sess-abc");
        assert!(
            !payload["jti"].as_str().unwrap().is_empty(),
            "minted refresh token carries a jti"
        );
    }
}
