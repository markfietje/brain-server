//! The auth middlewares: JWT verification (JWT mode), the opaque
//! bearer gate, and the UMP capability-token fallback. Layered by
//! `app()` in `super` — JWT first, then opaque, with the rate limiter
//! OUTSIDE both (the 429-before-authN posture is pinned, not prose).

use axum::{
    Json,
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::path::PathBuf;
use std::sync::Arc;

use crate::Pool;
use crate::audit;
use crate::auth::{self, AccessTokenExp, TokenStore};
use crate::config;
use crate::handlers;
use crate::http_limit::RateLimiter;
use crate::server::bootstrap::ct_eq;

/// Auth middleware. When
/// `AUTH_TOKEN`/`AUTH_TOKEN_FILE` is set, every non-public route requires a
/// matching `Authorization: Bearer <token>` header. When unset the server is
/// unauthenticated (safe only behind a loopback/proxy). Public read-only routes
/// (`/health`, `/ready`, `/version`, `/openapi.yaml`) are always exempt so a
/// load balancer can probe without credentials and third parties can discover
/// the contract without a token. CORS preflight (`OPTIONS`) is also exempt:
/// browsers send it without credentials and it must reach the CORS layer intact
/// to attach preflight headers; the following real request authenticates normally.
///
/// tokens come from the cached, mtime-refreshed `TokenStore` rather
/// than a per-request disk read. Fail-safe: if the file was deleted, the store
/// keeps the last-good set so auth can never silently clear.
/// state for the JWT auth middleware. A subset of AppState
/// containing only what the middleware needs. Kept separate so the middleware
/// can be layered with `from_fn_with_state` without the full AppState (which
/// is constructed at the very end of router setup).
#[derive(Clone)]
pub struct JwtMiddlewareState {
    pub auth_mode: auth::AuthMode,
    pub key_store: auth::jwks::KeyStore,
    pub jwt_issuer: String,
    pub jwt_audience: String,
    pub pool: Pool,
    pub revocation_cache: Arc<auth::revocation::RevocationCache>,
    pub db_path: PathBuf,
    /// Second rate-limit dimension keyed on the verified principal (the
    /// per-IP limiter cannot distinguish agents behind one address).
    pub principal_rate_limiter: Arc<RateLimiter>,
}

impl JwtMiddlewareState {
    /// The opaque-mode bundle for composed-app test states: empty
    /// issuer/audience, fresh revocation cache + principal limiter.
    /// Opaque-mode bundle for composed-app test states (integration
    /// fixtures + the law-9 matrix).
    pub fn opaque_for_tests(pool: Pool, db_path: PathBuf) -> Self {
        Self {
            auth_mode: auth::AuthMode::Opaque,
            key_store: auth::jwks::KeyStore::load(std::path::Path::new("/nonexistent"))
                .expect("empty key store"),
            jwt_issuer: String::new(),
            jwt_audience: String::new(),
            pool,
            revocation_cache: Arc::new(auth::revocation::RevocationCache::new()),
            db_path,
            principal_rate_limiter: Arc::new(RateLimiter::new()),
        }
    }
}

/// JWT verification middleware. Runs ONLY when JWT mode is
/// on (BRAIN_JWT_ISSUER + keys configured). In opaque mode it's a no-op pass-
/// through. On success, injects a `Principal` into request extensions; the
/// opaque `auth_middleware` sees the Principal already set and short-circuits.
///
/// This is layered BEFORE `auth_middleware` so the opaque path becomes the
/// fallback for non-JWT deployments (zero behavior change for v1.1 installs).
pub async fn jwt_auth_middleware(
    State(s): State<Arc<JwtMiddlewareState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    if !s.auth_mode.is_jwt() {
        return next.run(req).await;
    }
    let path = req.uri().path();
    // The one public-path list (router::PUBLIC_PATHS + rules) — both
    // middlewares consume the same decision; there is no second copy.
    if crate::server::router::route_guards::is_public_path(path)
        || req.method() == axum::http::Method::OPTIONS
    {
        return next.run(req).await;
    }
    // Extract the bearer token.
    let raw = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string());
    let raw = match raw {
        Some(r) if !r.is_empty() => r,
        _ => {
            // No token presented. Audit + 401.
            audit_auth_failure(&s.db_path, path, "missing_token").await;
            return unauthorized_response("missing_token");
        }
    };
    // Verify + check revocation in a blocking task (sqlite + crypto).
    let keys = s.key_store.verifying_keys();
    let issuer = s.jwt_issuer.clone();
    let audience = s.jwt_audience.clone();
    let pool = s.pool.clone();
    let rev_cache = s.revocation_cache.clone();
    let path_owned = path.to_string();
    // The capability fallback needs the raw bearer; clone before the move.
    let raw_for_fallback = raw.clone();
    let result = tokio::task::spawn_blocking(
        move || -> Result<(auth::Principal, AccessTokenExp), String> {
            let (claims, _) = auth::jwt::verify_access_token(
                &raw,
                &keys,
                &issuer,
                &audience,
                auth::jwt::TokenType::Access,
            )
            .map_err(|e| e.code().to_string())?;
            // Revocation check. Denial on ANY
            // store failure — the old `if let Ok(conn)` + `unwrap_or(false)` let a
            // pool/SQL error skip the check entirely, precisely during incident
            // response (fail-open on the one path that must fail closed).
            let conn = pool
                .get()
                .map_err(|e| format!("revocation store unavailable: {e}"))?;
            if rev_cache
                .is_revoked(&conn, &claims.jti, &claims.iss)
                .map_err(|e| format!("revocation store error: {e}"))?
            {
                return Err("revoked".to_string());
            }
            // The principal kill-switch at the authentication layer: a revoked
            // IDENTITY dies here, before any route logic — not only at the mesh
            // surfaces (verify_card / delegation dispatch / result submission)
            // that re-check it at their decision points. A dead identity answers
            // 401 `identity_revoked` (the identity is gone, not unauthorized for
            // this route), on every route, ahead of authorize. The check is the
            // same straight keyed read the mesh uses: one indexed SELECT, no
            // provisioning lookup, so it adds no existence oracle — a live
            // principal is untouched, a revoked one is denied identically
            // whether it holds rows anywhere or not. Store failure denies
            // (fail-closed), same posture as the jti check above.
            if crate::workflow::mesh::is_revoked(&conn, &claims.sub)
                .map_err(|e| format!("revocation store error: {e}"))?
            {
                return Err("identity_revoked".to_string());
            }
            // Build the principal from claims.
            let scopes: Vec<auth::Scope> = claims
                .scopes
                .iter()
                .filter_map(|s| auth::Scope::parse(s))
                .collect();
            let token_exp = claims.exp;
            Ok(auth::Principal {
                sub: claims.sub,
                tenant: claims.tenant,
                scopes,
                jti: claims.jti,
                roles: claims.roles,
                manages: claims.manages,
                kind: auth::PrincipalKind::Jwt,
            })
            .map(|principal| (principal, AccessTokenExp(token_exp)))
        },
    )
    .await;
    let result = match result {
        Ok(inner) => inner,
        Err(_) => {
            audit_auth_failure(&s.db_path, &path_owned, "internal").await;
            return unauthorized_response("internal");
        }
    };
    match result {
        Ok((principal, token_exp)) => {
            // Second rate-limit dimension keyed on the verified principal:
            // agents sharing one egress IP each get their own budget, so one
            // agent's flood cannot exhaust (or hide behind) its neighbors.
            if !s
                .principal_rate_limiter
                .is_allowed(&format!("p:{}", handlers::mask_sub(&principal.sub)))
            {
                return (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({ "error": "rate_limited", "code": "rate_limited" })),
                )
                    .into_response();
            }
            // Inject the principal + its access-token expiry (the denylist
            // row's TTL source at logout) + pass through. The opaque
            // auth_middleware will see the Principal set and short-circuit
            // to `next.run(req)`.
            req.extensions_mut().insert(principal);
            req.extensions_mut().insert(token_exp);
            next.run(req).await
        }
        Err(code) => {
            // on the UMP surface the bearer may be an operator-
            // signed capability token rather than a JWS. Try it before
            // rejecting (the handler's cap_gate enforces verbs × scope).
            if capability_pass_through(&mut req, &raw_for_fallback, &path_owned) {
                // The capability seam of the kill-switch: the token's `iss`
                // is its identity anchor, so a revoked issuer's capabilities
                // die with the identity. The JWT revocation above checked a
                // JWS sub; this checks the credential that just passed.
                if let Some(cap) = req
                    .extensions()
                    .get::<crate::ump_integrity::CapabilityToken>()
                    && let Err(deny) = ensure_cap_principal_alive(&s.pool, &cap.iss.clone()).await
                {
                    audit_auth_failure(&s.db_path, &path_owned, &deny).await;
                    return unauthorized_response(&deny);
                }
                return next.run(req).await;
            }
            audit_auth_failure(&s.db_path, &path_owned, &code).await;
            unauthorized_response(&code)
        }
    }
}

/// §5.2: try the bearer as an operator-signed capability token
/// on the UMP surface (`/ump/*` + `/export`). A valid token is injected into
/// request extensions and the request passes — the handler's `cap_gate` then
/// enforces verbs × scope (expiry is enforced here at parse). Returns true
/// only when the request may continue on the strength of the capability.
/// `ponytail:` reads the operator key from disk per failing request on the
/// UMP surface — a rare, failing path, so the cost is acceptable; a cache
/// would be the upgrade if capability auth ever becomes hot.
pub(crate) fn capability_pass_through(req: &mut Request<Body>, raw: &str, path: &str) -> bool {
    let Some((_, sk)) = handlers::ump::operator_signing_key() else {
        return false;
    };
    if !capability_accepted(raw, path, &sk.verifying_key().to_bytes()) {
        return false;
    }
    let pk = sk.verifying_key().to_bytes();
    if let Ok(cap) = crate::ump_integrity::parse_capability_token(raw, &pk) {
        // Replay defense: a jti-bearing token is accepted once per
        // (jti, method, path) — capability tokens are per-request bearers,
        // so keying on jti alone burned the use on the first call; keyed this
        // way retries on the SAME endpoint stay valid while reuse on any
        // other method/path is refused as a replay.
        if !crate::ump_integrity::cap_replay_check(&cap, req.method().as_str(), path) {
            return false;
        }
        req.extensions_mut().insert(cap);
        true
    } else {
        false
    }
}

/// Pure §5.2 acceptance decision (the middleware's env/state-free core): the
/// bearer verifies as a capability token signed by `pk` AND the path is on
/// the UMP surface. Split out so the security decision is unit-testable
/// without env mutation (the parallel-test lesson from Agent 24).
pub fn capability_accepted(raw: &str, path: &str, pk: &[u8; 32]) -> bool {
    (path.starts_with("/ump/") || path == "/export")
        && crate::ump_integrity::parse_capability_token(raw, pk).is_ok()
}

/// The kill-switch read for capability credentials: is the token's issuer
/// principal revoked? `Err(code)` denies the request with that code —
/// `identity_revoked` for a revoked issuer, a descriptive store-error code
/// for any revocation-store failure (fail-closed; a live issuer passes only
/// through a clean read). Runs on `spawn_blocking` (pooled SQLite read); the
/// cost profile matches `capability_pass_through`'s own per-request key read,
/// which the UMP surface already accepts.
async fn ensure_cap_principal_alive(pool: &Pool, iss: &str) -> Result<(), String> {
    let pool = pool.clone();
    let iss = iss.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = pool
            .get()
            .map_err(|e| format!("revocation store unavailable: {e}"))?;
        if crate::workflow::mesh::is_revoked(&conn, &iss)
            .map_err(|e| format!("revocation store error: {e}"))?
        {
            return Err("identity_revoked".to_string());
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("revocation store join error: {e}"))?
}

/// Write an audit row for a failed JWT verification. Best-effort (opens a
/// fresh connection — failures are rare, the cost is negligible). Records the
/// path + failure code; never the token.
/// The deny-path audit write runs on
/// `spawn_blocking` — it opens a fresh connection + INSERT, which must never
/// block the async runtime thread. Rate of these is bounded by the rate
/// limiter, which sits OUTSIDE authN (see build_app layer order).
pub(crate) async fn audit_auth_failure(db_path: &std::path::Path, path: &str, code: &str) {
    let db_path = db_path.to_path_buf();
    let path = path.to_string();
    let code = code.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(conn) = rusqlite::Connection::open(db_path) {
            audit::record(
                &conn,
                audit::AuditKind::Auth,
                "api",
                &path,
                audit::AuditStatus::Denied,
                &code,
            );
        }
    })
    .await;
}

pub(crate) fn unauthorized_response(code: &str) -> Response {
    (
        axum::http::StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "unauthorized", "code": code })),
    )
        .into_response()
}

/// The opaque-mode bundle for `auth_middleware`: the rotating bearer store
/// plus the pool + DB path the middleware needs for the capability seam of
/// the kill-switch — a revoked capability issuer must deny in opaque mode
/// too (that is the live deployment posture) — and for its denial audit
/// rows.
#[derive(Clone)]
pub struct OpaqueAuthState {
    pub tokens: TokenStore,
    pub pool: Pool,
    pub db_path: PathBuf,
}

pub async fn auth_middleware(
    State(s): State<OpaqueAuthState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    // The one public-path list (router::PUBLIC_PATHS + rules) — both
    // middlewares consume the same decision; there is no second copy.
    // Webhook endpoints are authenticated by their own HMAC signature check
    // (GitHub cannot present a brain bearer token), so they bypass the bearer
    // middleware but are verified inside the handler.
    let public = crate::server::router::route_guards::is_public_path(&path);
    if public || req.method() == axum::http::Method::OPTIONS {
        return next.run(req).await;
    }
    // JWT path. When JWT mode is on, the bearer token is a JWS; we
    // verify it, build a Principal, and inject it into request extensions.
    // Handlers that read the Principal (via `OptPrincipal` or `Extension`)
    // get the typed claims; handlers that don't see `None` and run as before.
    //
    // The JWT state lives in the OpaqueAuthState bundle alongside the
    // bearer store: the opaque middleware shares the JWT layer's
    // kill-switch duty for capability credentials, which needs the pool.
    //
    // For now: if the request already has a Principal in extensions (set by
    // a prior middleware), pass through. Otherwise fall through to opaque.
    if req.extensions().get::<auth::Principal>().is_some() {
        return next.run(req).await;
    }
    // the token read now distinguishes
    // "never configured" from "read failed" — a poisoned token store is a 500
    // fail-closed and a configured-but-empty store denies (auth is ON with
    // no valid tokens). Only a truly unconfigured store keeps the loopback
    // pass-through.
    let accepted: std::collections::HashSet<String> = match s.tokens.tokens() {
        auth::TokenRead::NotConfigured => return next.run(req).await,
        auth::TokenRead::ReadFailed => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "auth_store_unavailable",
                    "code": "auth_store_unavailable"
                })),
            )
                .into_response();
        }
        auth::TokenRead::Active(s) if s.is_empty() => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "unauthorized", "code": "unauthorized" })),
            )
                .into_response();
        }
        auth::TokenRead::Active(s) => s,
    };
    let presented = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|t| t.trim());
    let ok = presented
        .map(|p| {
            accepted
                .iter()
                .any(|t| ct_eq(p.as_bytes(), t.trim().as_bytes()))
        })
        .unwrap_or(false);
    // Owned copy: the capability fallback needs `&mut req`, and `presented`
    // borrows `req`'s headers — the two would conflict.
    let presented_owned = presented.unwrap_or("").to_string();
    if ok {
        next.run(req).await
    } else if s
        .tokens
        .agent_token()
        .is_some_and(|agent| ct_eq(presented_owned.as_bytes(), agent.as_bytes()))
    {
        // Twokeys: the AGENT bearer authenticates as the typed
        // AgentLoopback principal — never as the None superuser. Blackout's
        // kill-switch runs FIRST (identity precedes authorization, the same
        // ordering the JWT middleware pins), the principal is injected for
        // the handler gates, and the agent's 403s are audited at this
        // boundary (the one trust-boundary seam that sees every route —
        // operator/None traffic is untouched).
        let pool = s.pool.clone();
        let verdict = tokio::task::spawn_blocking(move || -> Result<bool, String> {
            let conn = pool
                .get()
                .map_err(|e| format!("revocation store unavailable: {e}"))?;
            crate::workflow::mesh::is_revoked(&conn, auth::AGENT_LOOPBACK_SUB)
                .map_err(|e| format!("revocation store error: {e}"))
        })
        .await
        .unwrap_or_else(|e| Err(format!("revocation store unavailable: {e}")));
        match verdict {
            Ok(true) => {
                audit_auth_failure(&s.db_path, &path, "identity_revoked").await;
                return unauthorized_response("identity_revoked");
            }
            Ok(false) => {}
            Err(code) => {
                audit_auth_failure(&s.db_path, &path, &code).await;
                return unauthorized_response(&code);
            }
        }
        req.extensions_mut()
            .insert(auth::Principal::agent_loopback());
        let resp = next.run(req).await;
        if resp.status() == axum::http::StatusCode::FORBIDDEN {
            audit_auth_failure(&s.db_path, &path, "agent_forbidden").await;
        }
        resp
    } else if capability_pass_through(&mut req, &presented_owned, &path) {
        // the bearer verified as an operator-signed capability
        // token on the UMP surface; the handler's cap_gate enforces verbs.
        // Before it passes: the capability seam of the kill-switch. The
        // opaque-loopback bearers skip revocation entirely — there is no
        // principal id to revoke in a static-token world (the opaque
        // operator/agent split is Twokeys territory) — but a capability's
        // `iss` names an identity, and a revoked identity's capabilities
        // die with it, exactly as they do under JWT mode.
        if let Some(cap) = req
            .extensions()
            .get::<crate::ump_integrity::CapabilityToken>()
            && let Err(deny) = ensure_cap_principal_alive(&s.pool, &cap.iss.clone()).await
        {
            audit_auth_failure(&s.db_path, &path, &deny).await;
            return unauthorized_response(&deny);
        }
        next.run(req).await
    } else {
        // audit denied auth attempts at the trust boundary. The
        // middleware has no pool, so open a fresh connection on
        // `spawn_blocking` (never block the async runtime thread on a
        // sync DB write; the outer rate limiter bounds how often this runs).
        // Best-effort — audit must never fail the action. Pass the request
        // path, never the token.
        let path_owned = path.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(conn) = rusqlite::Connection::open(config::brain_db_path()) {
                audit::record(
                    &conn,
                    audit::AuditKind::Auth,
                    "api",
                    &path_owned,
                    audit::AuditStatus::Denied,
                    "unauthorized",
                );
            }
        })
        .await;
        (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized", "code": "unauthorized" })),
        )
            .into_response()
    }
}

// ── the auth route family (moved from the chain at C3b) ─────────────────

/// The auth + well-known routes; merges into `app()` like every family.
pub(crate) fn router() -> axum::Router<Arc<crate::AppState>> {
    axum::Router::new()
        // middleware verifies the presented access token, so the handler can
        // revoke its `jti`; an unauthenticated logout would revoke nothing).
        // `/auth/refresh` verifies the presented refresh token itself.
        .route(
            "/.well-known/openid-configuration",
            get(handlers::well_known::openid_configuration),
        )
        .route("/.well-known/jwks.json", get(handlers::well_known::jwks))
        .route(
            "/.well-known/security.txt",
            get(handlers::well_known::security_txt),
        )
        .route(
            "/.well-known/ai-notice",
            get(handlers::well_known::ai_notice),
        )
        .route(
            "/.well-known/ai-literacy",
            get(handlers::well_known::ai_literacy),
        )
        .route(
            "/.well-known/cop-notice",
            get(handlers::well_known::cop_notice),
        )
        .route("/auth/refresh", post(handlers::auth::refresh))
        .route("/auth/logout", post(handlers::auth::logout))
        .route("/auth/revoke", post(handlers::auth::revoke_handler))
}

// ── middleware pins (moved with their subjects from main.rs) ────────────
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::http_limit;
    use crate::server::router::rate_limit_middleware;
    use axum::middleware;
    use std::net::SocketAddr;

    /// The opaque middleware's state bundle for middleware-level tests: the
    /// store under test + a memory pool (the kill-switch's capability seam
    /// is never exercised here, so the pool is never read) + a throwaway DB
    /// path for the best-effort denial audit. Returns the TempDir so the
    /// audit path outlives the state.
    fn opaque_state(store: TokenStore) -> (OpaqueAuthState, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(r2d2_sqlite::SqliteConnectionManager::memory())
            .expect("memory pool");
        (
            OpaqueAuthState {
                tokens: store,
                pool,
                db_path: dir.path().join("audit.db"),
            },
            dir,
        )
    }

    /// auth presentation at the middleware layer. Non-public
    /// routes 401 without a token; public + webhook prefixes bypass; a valid
    /// opaque token passes. The per-handler action gates are pinned separately
    /// by `authz_gates_cover_every_non_public_route`.
    #[tokio::test]
    async fn auth_middleware_enforces_presentation_and_public_bypass() {
        use axum::routing::{get, post};
        use tower::ServiceExt;

        async fn stub() -> &'static str {
            "ok"
        }

        // Inject a known token via the file-reload path (no env races under
        // parallel tests); mirror the auth module's own rotation-test pattern
        // (sleep so the second write advances the 1s mtime resolution).
        let f = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(f.path(), "test-tok-1\n").unwrap();
        let store = TokenStore::from_file(Some(f.path().to_path_buf()));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(f.path(), "test-tok-1\n").unwrap();
        assert!(store.reload_if_changed_from(vec!["test-tok-1".to_string()]));

        let (state, _dir) = opaque_state(store);
        let app = axum::Router::new()
            .route("/health", get(stub))
            .route("/webhooks/gh", post(stub))
            .route("/private", get(stub))
            .with_state(state.clone())
            .layer(middleware::from_fn_with_state(state, auth_middleware));

        // No token on a non-public route -> 401.
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/private")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);

        // Wrong token on a non-public route -> 401.
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/private")
                    .header("authorization", "Bearer wrong-token")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);

        // Public route bypasses without a token.
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // Webhook prefix bypasses (HMAC is verified inside the handler).
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/webhooks/gh")
                    .method("POST")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // Valid opaque token on the non-public route passes.
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/private")
                    .header("authorization", "Bearer test-tok-1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// the rate limiter keys buckets by the
    /// peer `SocketAddr` extension — the gap the audit flagged (pre-v1.27.16
    /// the extension was missing, so EVERY request shared one bucket). One
    /// remote address exhausting its budget must never throttle another.
    #[tokio::test]
    async fn rate_limit_buckets_per_socket_addr_and_does_not_share() {
        use axum::routing::get;
        use tower::ServiceExt;

        async fn stub() -> &'static str {
            "ok"
        }

        let limiter = Arc::new(RateLimiter::new());
        let app = axum::Router::new()
            .route("/", get(stub))
            .with_state(limiter.clone())
            .layer(middleware::from_fn_with_state(
                limiter,
                rate_limit_middleware,
            ));

        let addr_a: SocketAddr = "10.0.0.1:1111".parse().unwrap();
        let addr_b: SocketAddr = "10.0.0.2:2222".parse().unwrap();

        fn req(addr: Option<SocketAddr>) -> axum::http::Request<Body> {
            let mut r = axum::http::Request::builder()
                .uri("/")
                .body(Body::empty())
                .unwrap();
            if let Some(a) = addr {
                r.extensions_mut().insert(a);
            }
            r
        }

        // A exhausts its own 60s window budget (10 000 req/min default).
        for _ in 0..http_limit::RateLimiter::WINDOW_BUDGET_PROBE {
            let resp = app.clone().oneshot(req(Some(addr_a))).await.unwrap();
            assert_eq!(
                resp.status(),
                axum::http::StatusCode::OK,
                "within budget → served"
            );
        }
        let resp = app.clone().oneshot(req(Some(addr_a))).await.unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "exhausted budget → 429"
        );

        // B shares nothing with A: its own bucket, still served.
        for _ in 0..3 {
            let resp = app.clone().oneshot(req(Some(addr_b))).await.unwrap();
            assert_eq!(
                resp.status(),
                axum::http::StatusCode::OK,
                "a second address is unaffected"
            );
        }

        // No extension → the "unknown" bucket (the real wiring always injects
        // one via into_make_service_with_connect_info; a request without it
        // simply shares the fallback bucket).
        let resp = app.clone().oneshot(req(None)).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    /// a configured-but-EMPTY token store
    /// must deny (401), never read as "auth disabled" (the pre-Drawbridge
    /// allow-all collapse). Middleware-level pin: file exists, zero tokens.
    #[tokio::test]
    async fn configured_but_empty_token_store_denies_not_opens() {
        use axum::routing::get;
        use tower::ServiceExt;

        async fn stub() -> &'static str {
            "ok"
        }

        let f = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(f.path(), b"").unwrap();
        let store = TokenStore::from_file(Some(f.path().to_path_buf()));

        let (state, _dir) = opaque_state(store);
        let app = axum::Router::new()
            .route("/private", get(stub))
            .with_state(state.clone())
            .layer(middleware::from_fn_with_state(state, auth_middleware));

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/private")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "configured-but-empty must deny"
        );
    }
}
