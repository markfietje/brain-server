//! The middleware stack: request-id propagation, security headers
//! (CSP), and the outermost rate limiter. The stack ORDER lives in
//! `app()` below — the pins that hold it
//! `rate_limit_layer_is_outside_auth_layers`,
//! `serve_wires_connect_info_with_socket_addr`) travel with the
//! composition, not with these definitions.

pub(crate) mod auth;

use axum::{
    Json,
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::config;
use crate::http_limit::RateLimiter;

/// CSP for API routes — the strictest possible (JSON-only, no content executes).
pub(crate) const API_CSP: &str = "default-src 'none'; frame-ancestors 'none'; form-action 'none'";

/// CSP for client routes — allows WASM compilation, same-origin API calls,
/// self-hosted fonts/CSS. No CDN, no inline scripts, NO eval.
/// The old `'unsafe-eval'` rung existed because wasm-bindgen emitted a
/// `new Function()` for module instantiation; since wasm-bindgen 0.2.109 the
/// glue uses `WebAssembly.instantiateStreaming`-shaped code that only needs
/// `'wasm-unsafe-eval'` — and this client pins 0.2.126. MANUAL GATE: boot the
/// built client once under the trimmed policy before shipping; if a glue path
/// still demands eval, restore `'unsafe-eval'` and re-document with evidence.
/// style-src 'unsafe-inline' covers Dioxus runtime <style> injection.
pub(crate) const CLIENT_CSP: &str = concat!(
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
pub(crate) async fn request_id_middleware(mut req: Request<Body>, next: Next) -> Response {
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
pub(crate) async fn security_headers_middleware(req: Request<Body>, next: Next) -> Response {
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
pub(crate) async fn rate_limit_middleware(
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
