//! The middleware stack: request-id propagation, security headers
//! (CSP), and the outermost rate limiter. The stack ORDER lives in
//! `app()` below — the pins that hold it
//! `rate_limit_layer_is_outside_auth_layers`,
//! `serve_wires_connect_info_with_socket_addr`) travel with the
//! composition, not with these definitions.

pub mod auth;
pub(crate) mod compliance;
pub mod core;
pub mod memory;
pub(crate) mod ump;
pub(crate) mod workflow;

use axum::{
    Json, Router,
    body::Body,
    extract::{Request, State},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::http_limit::RateLimiter;

use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer,
    request_id::PropagateRequestIdLayer, sensitive_headers::SetSensitiveHeadersLayer,
    set_header::SetResponseHeaderLayer, timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::config;
use crate::server::bootstrap::AppState;
use auth::{auth_middleware, jwt_auth_middleware};
use std::time::Duration as StdDuration;

/// CSP for API routes — the strictest possible (JSON-only, no content executes).
pub const API_CSP: &str = "default-src 'none'; frame-ancestors 'none'; form-action 'none'";

/// CSP for client routes — allows WASM compilation, same-origin API calls,
/// self-hosted fonts/CSS. No CDN, no inline scripts, NO eval.
/// The old `'unsafe-eval'` rung existed because wasm-bindgen emitted a
/// `new Function()` for module instantiation; since wasm-bindgen 0.2.109 the
/// glue uses `WebAssembly.instantiateStreaming`-shaped code that only needs
/// `'wasm-unsafe-eval'` — and this client pins 0.2.126. MANUAL GATE: boot the
/// built client once under the trimmed policy before shipping; if a glue path
/// still demands eval, restore `'unsafe-eval'` and re-document with evidence.
/// style-src 'unsafe-inline' covers Dioxus runtime <style> injection.
pub const CLIENT_CSP: &str = concat!(
    "default-src 'self'; ",
    "script-src 'self' 'wasm-unsafe-eval'; ",
    "style-src 'self' 'unsafe-inline'; ",
    "connect-src 'self'; ",
    "img-src 'self' data:; ",
    "font-src 'self' data:; ",
    "frame-ancestors 'none'; ",
    "form-action 'self'; ",
    "base-uri 'self'"
);

/// Request ID middleware - generates UUID v4 for tracing if not provided.
pub async fn request_id_middleware(mut req: Request<Body>, next: Next) -> Response {
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    req.headers_mut().insert(
        "x-request-id",
        request_id.parse().unwrap_or_else(|_| {
            axum::http::HeaderValue::from_str(&uuid::Uuid::new_v4().to_string())
                .expect("generated uuid is a valid header value")
        }),
    );
    next.run(req).await
}

/// Security headers middleware — applies standard hardening headers to every
/// response. Path-aware CSP (strict for API, WASM-friendly for client).
pub async fn security_headers_middleware(req: Request<Body>, next: Next) -> Response {
    // Read the path BEFORE next.run(req) consumes the request.
    let is_client = req.uri().path().starts_with("/app") || req.uri().path() == "/";
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        axum::http::header::STRICT_TRANSPORT_SECURITY,
        axum::http::HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
    );
    // Path-aware CSP: strict for API, WASM-friendly for client.
    let csp = if is_client { CLIENT_CSP } else { API_CSP };
    headers.insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        axum::http::HeaderValue::from_static(csp),
    );
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::header::X_FRAME_OPTIONS,
        axum::http::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "Permissions-Policy",
        axum::http::HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    res
}

/// Rate limiter middleware — per-IP sliding window (10 000 req/min default,
/// bounded key set via `RATE_LIMIT_MAX_KEYS`).
/// The peer `SocketAddr` extension (injected by
/// `into_make_service_with_connect_info`) is now guaranteed present, so each
/// remote address gets its own bucket. `X-Forwarded-For` is still honored
/// only under `BRAIN_TRUST_PROXY=1`.
pub async fn rate_limit_middleware(
    State(rate_limiter): State<Arc<RateLimiter>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    // only trust `X-Forwarded-For` when the operator has explicitly
    // opted in via `BRAIN_TRUST_PROXY=1`. Default uses the socket address — a
    // direct-connection attacker cannot spoof it, so the per-IP limiter actually
    // bounds them. When behind a reversing proxy that overwrites client XFF,
    // operators set the flag and the proxy-provided value is trusted instead.
    let ip = if config::brain_trust_proxy() {
        req.headers()
            .get("x-forwarded-for")
            .and_then(|h| h.to_str().ok())
            // Take the RIGHTMOST entry — the one the
            // trusted proxy APPENDED. The leftmost is client-controlled (an
            // attacker pre-seeds `X-Forwarded-For: 1.2.3.4` and the appending
            // proxy preserves it), so leftmost-trust allowed bucket evasion
            // and targeted cross-victim 429s under `BRAIN_TRUST_PROXY=1`.
            .and_then(|s| s.split(',').next_back())
            .map(|s| s.trim().to_string())
    } else {
        None
    }
    .or_else(|| {
        req.extensions()
            .get::<SocketAddr>()
            .map(|a| a.ip().to_string())
    })
    .unwrap_or_else(|| "unknown".to_string());

    if !rate_limiter.is_allowed(&ip) {
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": "rate_limited", "code": "rate_limited" })),
        )
            .into_response();
    }
    next.run(req).await
}

// ── the composed application (moved from main.rs verbatim at C3a) ──────

/// The composed application: every route family, the shared middleware
/// stack, and the state binding. THE WIRE: paths, methods, status
/// vocabulary, and the layer order below are frozen (openapi.yaml +
/// the route-authz table + the law-9 matrix pin them); the layer ORDER
/// is load-bearing — the rate limiter sits OUTSIDE both auth layers
/// (429 before token work), security headers outermost of all, and the
/// 1 MiB body limit is applied BEFORE the 1 GiB import-router merge
/// (tower-http eager-application pitfall — see the import_router
/// comment below).
pub fn app(state: Arc<AppState>) -> Router {
    let base = core::router()
        .merge(memory::legacy_router())
        // Legacy contract markers: `/add` and GET `/search` are superseded by
        // `/ingest/memory` + `/recall`. The `Deprecation` header (RFC 8594)
        // signals clients to migrate; both still function. The layer sits
        // HERE so exactly the routes above it (the SPA seat + the health
        // family + the three legacy writes/reads) carry the header — the
        // original chain's application set, preserved byte-for-byte.
        .route_layer(SetResponseHeaderLayer::overriding(
            axum::http::HeaderName::from_static("deprecation"),
            axum::http::HeaderValue::from_static("version=\"0.9.5\""),
        ))
        .merge(memory::router())
        .merge(ump::router())
        .merge(workflow::router())
        .merge(compliance::router())
        .merge(auth::router());
    // the shared 1 MiB body limit FIRST, then the 1 GiB import dial merged
    // AFTER it (tower-http eager-application pitfall: an outer limit can
    // never be raised by an inner one — see memory::import_router).
    base.layer(tower_http::limit::RequestBodyLimitLayer::new(
        config::MAX_REQUEST_SIZE,
    ))
    .merge(memory::import_router())
    // the compliance pack merges after the shared cap like the chain did
    .merge(compliance::pack_router())
    // Inner layers (closest to handler)
    .layer(TimeoutLayer::with_status_code(
        axum::http::StatusCode::REQUEST_TIMEOUT,
        StdDuration::from_secs(30),
    ))
    .layer(CatchPanicLayer::new())
    .layer(SetSensitiveHeadersLayer::new([
        axum::http::header::AUTHORIZATION,
        axum::http::header::COOKIE,
        axum::http::header::SET_COOKIE,
    ]))
    .layer(CompressionLayer::new())
    .layer(middleware::from_fn(request_id_middleware))
    .layer(PropagateRequestIdLayer::new(
        axum::http::HeaderName::from_static("x-request-id"),
    ))
    .layer(TraceLayer::new_for_http())
    // Security layers
    .layer(state.cors.clone())
    .layer(middleware::from_fn_with_state(
        state.token_store.clone(),
        auth_middleware,
    ))
    .layer(middleware::from_fn_with_state(
        state.jwt_middleware_state.clone(),
        jwt_auth_middleware,
    ))
    .layer(middleware::from_fn_with_state(
        state.rate_limiter.clone(),
        rate_limit_middleware,
    ))
    .layer(middleware::from_fn(security_headers_middleware))
    // Response headers
    .layer(SetResponseHeaderLayer::overriding(
        axum::http::header::SERVER,
        axum::http::HeaderValue::from_static("brain-server"),
    ))
    .layer(SetResponseHeaderLayer::overriding(
        axum::http::HeaderName::from_static("x-api-version"),
        axum::http::HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
    ))
    .with_state(state.clone())
}
