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

use axum::routing::{delete, get, post, put};
use axum::{Router, middleware};
use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer, limit::RequestBodyLimitLayer,
    request_id::PropagateRequestIdLayer, sensitive_headers::SetSensitiveHeadersLayer,
    set_header::SetResponseHeaderLayer, timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::config;
use crate::handlers;
use crate::http_limit::RateLimiter;
use std::time::Duration as StdDuration;
// main.rs-resident handlers the chain routes to (they move to their family
// files in the next commits; the paths update then).
use crate::{AppState, alert};
use crate::{
    add_chunk, delete_quarantine, embeddings, get_chunk, get_edge_history, get_entity,
    get_relations, health, health_db, ingest_markdown, ingest_memory, list_audit, list_quarantined,
    metrics, multi_get, openapi, ready, reindex, release_quarantine, search, stats, traverse_graph,
    verify_audit_chain, version,
};
use auth::{auth_middleware, jwt_auth_middleware};

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
pub(crate) fn app(state: Arc<AppState>) -> Router {
    let compliance_router: Router<Arc<AppState>> = {
        #[cfg(feature = "compliance-pack")]
        {
            Router::new()
                .route("/audit/export", get(handlers::compliance::export_audit))
                .route(
                    "/compliance/evaluation-record",
                    post(handlers::compliance::post_evaluation_record),
                )
                .route(
                    "/compliance/inventory",
                    get(handlers::compliance::inventory),
                )
                .route(
                    "/ropa",
                    get(handlers::compliance::list_ropa).post(handlers::compliance::create_ropa),
                )
                .route("/ropa/{id}", post(handlers::compliance::upsert_ropa))
        }
        #[cfg(not(feature = "compliance-pack"))]
        {
            Router::new()
        }
    };

    let import_router = Router::new()
        .route(
            "/domains/{name}/import",
            post(handlers::domains::import_domain),
        )
        // F-49a: this dial is DELIBERATE and scoped to exactly this route
        // group — bulk markdown imports are Admin-gated (handler re-checks
        // before reading a byte) and stream-parsed one file at a time, so
        // the 1 GiB ceiling is an operator-scale allowance, not an anonymous
        // amplification surface. Every other route stays at the 1 MiB
        // default layered after this router.
        .layer(RequestBodyLimitLayer::new(1024 * 1024 * 1024));

    Router::new()
        // Static SPA seat (host-frontend-static semantics).
        // Serves the built client dist; absent dist degrades to 404 so an
        // API-only deployment is unaffected. Public surface (no auth) — the
        // bundle is static, data flows only through the gated API routes.
        .route("/app/", get(handlers::frontend::spa_index))
        .route("/app/{*path}", get(handlers::frontend::spa_static))
        .route("/app/boot.json", get(handlers::frontend::boot_json))
        .route("/app/boot.js", get(handlers::frontend::boot_js))
        .route("/app/boot.pub", get(handlers::frontend::boot_pub))
        .route("/app/sw.js", get(handlers::frontend::sw_js))
        .route(
            "/app/sw-register.js",
            get(handlers::frontend::sw_register_js),
        )
        .route("/health", get(health))
        .route("/health/db", get(health_db))
        .route("/ready", get(ready))
        .route("/openapi.yaml", get(openapi))
        .route("/stats", get(stats))
        .route("/version", get(version))
        .route("/add", post(add_chunk))
        .route("/ingest/memory", post(ingest_memory))
        .route("/search", get(search))
        // Legacy contract markers: `/add` and GET `/search` are superseded by
        // `/ingest/memory` + `/recall`. The `Deprecation` header (RFC 8594)
        // signals clients to migrate; both still function.
        .route_layer(SetResponseHeaderLayer::overriding(
            axum::http::HeaderName::from_static("deprecation"),
            axum::http::HeaderValue::from_static("version=\"0.9.5\""),
        ))
        .route("/v1/embeddings", post(embeddings))
        .route("/ingest/markdown", post(ingest_markdown))
        .route("/reindex", post(reindex))
        .route("/get/{id}", get(get_chunk))
        .route("/multi-get", post(multi_get))
        // quarantine operator surface. `GET /quarantine` lists
        // flagged chunks; release clears the flag; delete purges the chunk.
        .route("/quarantine", get(list_quarantined))
        .route("/quarantine/{id}/release", post(release_quarantine))
        .route("/quarantine/{id}/delete", post(delete_quarantine))
        .route("/graph/entity/{name}", get(get_entity))
        .route("/graph/relations", get(get_relations))
        .route("/graph/traverse", get(traverse_graph))
        .route("/graph/relationships/{id}/history", get(get_edge_history))
        // Plugin API (contract: API_CONTRACT.md). Wire is locked.
        .route("/recall", post(handlers::recall::recall))
        .route("/ingest", post(handlers::ingest::ingest))
        // the UMP 1.0 HTTP ops binding. Capabilities +
        // `/.well-known/ump.json` are PUBLIC (negotiation handshake); the
        // rest are authz-gated per the §3.3 matrix.
        .route("/ump/capabilities", get(handlers::ump_ops::capabilities))
        .route("/ump/remember", post(handlers::ump_ops::remember))
        .route("/ump/memory/{id}", get(handlers::ump_ops::get_memory))
        .route("/ump/recall", post(handlers::ump_ops::recall))
        .route("/ump/revise", post(handlers::ump_ops::revise))
        .route("/ump/forget", post(handlers::ump_ops::forget))
        .route("/ump/feedback", post(handlers::ump_ops::feedback))
        .route("/ump/subscribe", get(handlers::ump_ops::subscribe))
        .route("/events", get(alert::events))
        .route("/ump/audit", post(handlers::ump_ops::audit))
        .route("/ump/audit/verify", get(handlers::ump_ops::audit_verify))
        .route(
            "/.well-known/ump.json",
            get(handlers::ump_ops::capabilities),
        )
        .route("/memory/{id}", delete(handlers::forget::forget))
        .route(
            "/domains",
            get(handlers::domains::domains).post(handlers::domains::create_domain),
        )
        .route("/domains/{name}", delete(handlers::domains::delete_domain))
        // bulk relabel of chunks across domains (the non-re-ingest
        // fix for the 99%-in-global corpus). A POST on a distinct path, so it
        // cannot collide with the `/domains/{name}` DELETE above.
        .route("/domains/move", post(handlers::domains::move_domains))
        // one-shot recompute sweep over every domain's centroid.
        .route(
            "/domains/recompute",
            post(handlers::domains::recompute_domains),
        )
        // per-domain lifecycle. Vacuum reclaims free pages; export
        // streams a consistent snapshot; import restores a snapshot into a new
        // domain name. `name` is validated inside each handler.
        .route(
            "/domains/{name}/vacuum",
            post(handlers::domains::vacuum_domain),
        )
        .route(
            "/domains/{name}/export",
            get(handlers::domains::export_domain),
        )
        // the preset API. Reads are Read-gated; writes
        // (profile upsert + domain binding) are Admin + audited. Dual-method
        // paths register GET first then POST (the /retention precedent) so the
        // authz source-scan lands on the Admin POST as the conservative check.
        .route("/profiles", get(handlers::profiles::list_profiles))
        .route("/profiles/{name}", get(handlers::profiles::get_profile))
        .route("/profiles/{name}", post(handlers::profiles::upsert_profile))
        .route(
            "/domains/{name}/profile",
            get(handlers::profiles::domain_profile_get),
        )
        .route(
            "/domains/{name}/profile",
            post(handlers::profiles::domain_profile_bind),
        )
        // the role API. Reads are Read-gated; writes
        // (role upsert) are Admin + audited. Dual-method on {name}: GET then
        // POST, so the authz source-scan lands on the Admin POST as the
        // conservative check (the /retention + /profiles precedent).
        .route("/roles", get(handlers::roles::list_roles))
        .route("/roles/{name}", get(handlers::roles::get_role))
        .route("/roles/{name}", post(handlers::roles::upsert_role))
        // legal hold — place/release/list holds that
        // freeze ids against erasure (decay, /purge, DSAR).
        .route("/legal-hold", post(handlers::holds::post_legal_hold))
        .route(
            "/legal-hold/{id}/release",
            post(handlers::holds::release_legal_hold),
        )
        .route("/legal-holds", get(handlers::holds::list_legal_holds))
        // the breach-notification workflow. Human-
        // opened by the DPO role; every event is hash-chained into the audit.
        .route("/breach", post(handlers::breaches::post_breach))
        .route(
            "/breach/{id}/event",
            post(handlers::breaches::post_breach_event),
        )
        .route("/breach/{id}/close", post(handlers::breaches::close_breach))
        .route("/breaches", get(handlers::breaches::list_breaches))
        .route("/breaches/{id}", get(handlers::breaches::get_breach))
        // the cross-border transfer register +
        // the TIA/DPA evidence artifacts. Writes are Admin + audited; the
        // register + templates are the Art 30/46 + Schrems II evidence a
        // client's regulator asks for (a human DPO/legal reviews + signs them).
        .route("/transfers", post(handlers::transfers::register_transfer))
        .route("/transfers", get(handlers::transfers::list_transfers))
        .route("/transfers/{id}/tia", get(handlers::transfers::get_tia))
        .route("/transfers/{id}/dpa", get(handlers::transfers::get_dpa))
        // the BPO operating register — the spine every later
        // BPO release (onboard/dpa/dsar/holds/termination) reads. Writes are
        // Admin + audited (AuditKind::Client); the identity/evidence surface
        // only (no enforcement gate).
        .route("/clients", post(handlers::clients::register_client))
        .route("/clients", get(handlers::clients::list_clients))
        .route("/clients/{name}", get(handlers::clients::get_client))
        .route(
            "/clients/{name}/dpa",
            post(handlers::clients::set_client_dpa),
        )
        .route(
            "/clients/{name}/dpa",
            get(handlers::clients::get_client_dpa),
        )
        .route("/clients/{name}/dsar", post(handlers::clients::client_dsar))
        .route("/clients/{name}/hold", post(handlers::clients::client_hold))
        .route("/clients/{name}/end", post(handlers::clients::client_end))
        // the supervisor QA surface — owner-scoped queue
        // list + audited coaching (read + write are Admin like every client op).
        .route(
            "/clients/{name}/proposals",
            get(handlers::clients::client_proposals),
        )
        .route(
            "/clients/{name}/proposals/{id}/coach",
            post(handlers::clients::coach_proposal),
        )
        // source lifecycle. `reconcile` retires active sources
        // of a kind whose URI is no longer in the live set (a vault delete or
        // rename); `delete /sources/{id}` retires a single source explicitly.
        .route("/sources/reconcile", post(handlers::sources::reconcile))
        .route("/sources/{id}", delete(handlers::sources::delete_source))
        // connector registry. `GET /connectors` lists every
        // registered connector instance across all kinds.
        .route("/connectors", get(handlers::connectors::list))
        // register a connector instance, gated by the
        // domain's bound profile `connectors_allowed` (Admin, audited).
        .route("/connectors/register", post(handlers::connectors::register))
        // deterministic span verification. Given a
        // claim + chunk_id, returns whether the claim is supported by the
        // chunk's text. Pure lexical match — no embeddings, no LLM.
        .route("/verify", post(handlers::verify::verify))
        // opt-in, non-interrupting anticipation. `/suggest`
        // is an explicit pull (caller asks "what else might be relevant?");
        // `/suggest/feedback` records accept/dismiss; `/suggest/metrics` is
        // the false-positive rate (roadmap exit criterion). All three are
        // gated by BRAIN_SUGGEST_ENABLED and return 501 when disabled — the
        // roadmap's "otherwise the feature is removed" kill switch.
        .route("/suggest", post(handlers::suggest::suggest))
        .route("/suggest/feedback", post(handlers::suggest::feedback))
        .route("/suggest/metrics", get(handlers::suggest::metrics))
        // procedural memory + deterministic categorization
        // + decision evaluation. `POST /procedure` ingests an ordered runbook;
        // `GET /procedure/{id}/steps` returns the ordered chain; `POST /classify`
        // categorizes text deterministically (Mem0's premium, free); `POST
        // /decision/{id}/evaluate` runs a stored decision rule against input vars.
        // All deterministic — no LLM, no cloud, no tokens.
        .route("/procedure", post(handlers::procedure::create))
        .route("/procedure/{id}/steps", get(handlers::procedure::steps))
        .route("/classify", post(handlers::procedure::classify))
        .route(
            "/decision/{id}/evaluate",
            post(handlers::procedure::evaluate),
        )
        // reviewable consolidation. `propose` is pure
        // detection (no mutation); `apply` records operator-chosen typed links.
        .route("/consolidate/propose", post(handlers::consolidate::propose))
        .route("/consolidate/apply", post(handlers::consolidate::apply))
        // reverse prior supersession resolutions. The undo
        // arm of the roadmap exit criterion ("reject or undo them without
        // retrieval regression"). Clears valid_to + removes the supersedes link.
        .route("/consolidate/undo", post(handlers::consolidate::undo))
        // write-back gate — proposals queue + human review.
        // No auto-promote: a candidate becomes memory only by explicit approval.
        .route("/ingest/proposal", post(handlers::gate::ingest_proposal))
        .route("/proposals", get(handlers::gate::list_proposals))
        .route(
            "/proposals/{id}/approve",
            post(handlers::gate::approve_proposal),
        )
        .route(
            "/proposals/{id}/reject",
            post(handlers::gate::reject_proposal),
        )
        .route("/proposals/{id}/edit", post(handlers::gate::edit_proposal))
        // decay + GDPR lifecycle. `/export` is portable JSON
        // (interchange); `/purge` is hard, explicit, audited deletion; `/decayed`
        // is the operator review list. Nothing is deleted autonomously.
        .route("/decayed", get(handlers::gate::list_decayed))
        .route("/export", get(handlers::gate::export))
        .route("/purge", post(handlers::gate::purge))
        // per-kind retention policy, the Art 30
        // records-of-processing register, and the snapshot self-check
        // panel. GET /retention reads; POST /retention overrides
        // (Admin + audited); /art30 and /snapshot/status are Admin read-only.
        .route("/retention", get(handlers::govern::retention_get))
        .route("/retention", post(handlers::govern::retention_post))
        .route("/retention/report", get(handlers::govern::retention_report))
        .route("/art30", get(handlers::govern::art30))
        .route("/snapshot/status", get(handlers::govern::snapshot_status))
        // read-event trace + DSAR workflow. `/recall/{id}/
        // trace` replays a recorded recall decision path; `/dsar` is the GDPR
        // Art 15/17 workflow (locate → export → purge → certificate);
        // `/tombstones` is the queryable deletion registry; `/dsar/{id}/
        // certificate` re-fetches a past deletion certificate.
        .route(
            "/recall/{trace_id}/trace",
            get(handlers::observe::get_trace),
        )
        .route("/dsar", post(handlers::observe::post_dsar))
        // the DSAR ledger list (Admin) — past requests
        // + the Art 17 window the client countdown renders.
        .route("/dsar", get(handlers::observe::list_dsar))
        .route("/tombstones", get(handlers::observe::list_tombstones))
        .route(
            "/dsar/{id}/certificate",
            get(handlers::observe::get_dsar_certificate),
        )
        // verified webhook ingestion. The handler only verifies
        // the HMAC + enqueues; the drain worker (spawned in main) does the rest.
        .route("/webhooks/{kind}", post(handlers::webhooks::receive))
        .route(
            "/webhooks/channel/{kind}",
            post(handlers::channel_webhook::receive_channel),
        )
        .route(
            "/webhooks/channel/{kind}/drain",
            post(handlers::channel_webhook::drain_channel),
        )
        .route(
            "/webhooks/channel/{kind}/console",
            post(handlers::channel_webhook::post_console),
        )
        // The console annex is HMAC self-authenticating like its sibling
        // channel seams: the bridge holds no bearer, ever.
        // OIDC discovery + JWKS + auth endpoints. These are
        // PUBLIC routes (no auth_middleware) except `/auth/revoke` (admin)
        // and `/auth/logout` (the
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
        .route("/audit", get(list_audit))
        .route("/workflow/runs/{id}", get(handlers::workflow::get_run))
        .route(
            "/workflow/runs/{id}/state",
            get(handlers::workflow::get_run_state),
        )
        .route(
            "/workflow/runs/{id}/state",
            put(handlers::workflow::put_run_state),
        )
        .route("/workflow/runs", post(handlers::workflow::post_run))
        .route(
            "/workflow/runs/{id}/events",
            get(handlers::workflow_lineage::get_run_events),
        )
        .route(
            "/workflow/runs/{id}/events",
            post(handlers::workflow::post_event),
        )
        .route(
            "/workflow/runs/{id}/rewind",
            post(handlers::workflow_lineage::post_rewind),
        )
        .route(
            "/workflow/runs/{id}/handoff",
            get(handlers::workflow_lineage::get_handoff),
        )
        .route(
            "/workflow/runs/{id}/handover/offer",
            post(handlers::relay::post_handover_offer),
        )
        .route(
            "/workflow/runs/{id}/handover/{offer_id}/accept",
            post(handlers::relay::post_handover_accept),
        )
        .route(
            "/workflow/runs/{id}/handover/{offer_id}/decline",
            post(handlers::relay::post_handover_decline),
        )
        .route(
            "/workflow/runs/{id}/notes",
            post(handlers::channel::post_notes),
        )
        .route(
            "/workflow/runs/{id}/notes",
            get(handlers::channel::get_notes),
        )
        .route(
            "/workflow/runs/{id}/notes/{invite_id}/accept",
            post(handlers::channel::post_invite_accept),
        )
        .route(
            "/workflow/channel/user-map",
            post(handlers::channel::post_user_map_proposal),
        )
        // Mesh: agents as named colleagues — signed cards + delegation.
        .route("/ops/agents/cards", post(handlers::mesh::post_card))
        .route("/ops/agents/cards", get(handlers::mesh::get_cards))
        .route(
            "/workflow/runs/{id}/delegations",
            post(handlers::mesh::post_delegation),
        )
        .route(
            "/workflow/runs/{id}/delegations",
            get(handlers::mesh::get_delegations),
        )
        .route(
            "/workflow/runs/{id}/delegations/{delegation_id}/result",
            post(handlers::mesh::post_delegation_result),
        )
        // Parcels: signed site-to-site knowledge — export, import, ledger.
        .route("/parcels/export", post(handlers::parcels::post_export))
        .route("/parcels/import", post(handlers::parcels::post_import))
        .route("/parcels", get(handlers::parcels::get_ledger))
        .route("/ops/handovers", get(handlers::relay::get_ops_handovers))
        .route(
            "/workflow/runs/{id}/context",
            get(handlers::workflow_lineage::get_run_context),
        )
        .route(
            "/workflow/runs/{id}/answer",
            post(handlers::workflow::post_answer),
        )
        .route(
            "/workflow/runs/{id}/steering",
            get(handlers::workflow::get_steering),
        )
        .route(
            "/workflow/runs/{id}/steps",
            get(handlers::workflow::list_steps),
        )
        .route(
            "/workflow/runs/{id}/steering",
            post(handlers::workflow::post_steering),
        )
        .route(
            "/workflow/runs/{id}/suggestions",
            get(handlers::workflow::get_suggestions),
        )
        // The personal assistant's cranks + views.
        // due is the cron-cranked scheduler (no daemon); brief is today's
        // derived context; consent is the one-subject Outreach-lite registry.
        .route("/workflow/valet/due", post(handlers::valet::post_due))
        .route("/workflow/valet/brief", get(handlers::valet::get_brief))
        .route("/workflow/valet/consent", put(handlers::valet::put_consent))
        .route(
            "/workflow/runs/{id}/complaint/lifecycle",
            post(handlers::workflow::post_complaint_lifecycle),
        )
        .route(
            "/workflow/runs/{id}/complaint/remedy",
            post(handlers::workflow::post_complaint_remedy),
        )
        .route(
            "/workflow/runs/{id}/complaint/adr-packet",
            get(handlers::workflow::get_complaint_adr_packet),
        )
        .route(
            "/workflow/runs/{id}/complaint/ack",
            post(handlers::workflow::post_complaint_ack),
        )
        .route(
            "/workflow/complaints/ack-sweep",
            post(handlers::workflow::post_complaint_ack_sweep),
        )
        .route(
            "/workflow/outreach/campaign",
            post(handlers::workflow::post_outreach_campaign),
        )
        .route(
            "/workflow/outreach/campaign/{id}",
            get(handlers::workflow::get_outreach_campaign),
        )
        .route(
            "/workflow/outreach/consent",
            get(handlers::workflow::get_outreach_consent),
        )
        .route(
            "/workflow/runs/{id}/outreach/followup",
            post(handlers::workflow::post_outreach_followup),
        )
        .route(
            "/workflow/runs/{id}/status-ref",
            post(handlers::workflow::post_status_ref),
        )
        .route(
            "/workflow/scoreboard",
            get(handlers::workflow::get_scoreboard),
        )
        .route(
            "/workflow/calibration/sign",
            post(handlers::workflow::post_calibration_sign),
        )
        .route(
            "/workflow/plugins/mount",
            post(handlers::workflow::post_plugin_mount),
        )
        .route(
            "/kcs/articles/{id}/approve",
            post(handlers::kcs::post_kcs_article_approve),
        )
        .route("/kcs/articles", get(handlers::kcs::get_kcs_articles))
        .route("/kcs/translate", post(handlers::kcs::post_kcs_translate))
        .route(
            "/kcs/articles/{id}/publish",
            post(handlers::kcs::post_kcs_article_publish),
        )
        .route(
            "/kcs/articles/{id}/preview",
            get(handlers::kcs::get_kcs_article_preview),
        )
        .route("/ops/shifts", get(handlers::shifts::get_ops_shifts))
        .route("/ops/shifts", post(handlers::shifts::post_ops_shift))
        .route("/ops/crew", get(handlers::crew::get_ops_crew))
        .route("/ops/skills", get(handlers::crew::get_ops_skills))
        .route("/ops/skills", post(handlers::crew::post_ops_skills))
        .route(
            "/ops/crew/config",
            post(handlers::crew::post_ops_crew_config),
        )
        // Workload visibility: lineage-only
        // reads; fatigue alerts the scheduling human, never reassigns.
        .route("/ops/workload", get(handlers::workload::get_ops_workload))
        .route("/ops/coverage", get(handlers::workload::get_ops_coverage))
        .route("/audit/verify", get(verify_audit_chain))
        .route("/metrics", get(metrics))
        // Static SPA seat is registered ABOVE (`/app/` + `/app/{*path}` →
        // `handlers::frontend`): MIME + path-traversal prevention + SPA
        // deep-link fallback to index.html live there. The historical
        // `nest_service("/app", ServeDir)` registration is GONE — axum 0.8
        // panics at boot on the conflicting internal wildcard
        // (/app/{*__private__axum_nest_tail_param} vs /app/{*path}), a
        // latent boot-blocker the 1.28.4 line never exercised end-to-end.
        // Root → the client shell (a 301 so browsers + the client's fetch base
        // both see a canonical `/app/`).
        .route(
            "/",
            get(|| async { axum::response::Redirect::permanent("/app/") }),
        )
        // Inner layers (closest to handler)
        .layer(RequestBodyLimitLayer::new(config::MAX_REQUEST_SIZE))
        // merge the 1 GiB import router AFTER the
        // shared 1 MiB limit so the shared cap never wraps the import route
        // (see the import_router comment above). All shared layers below
        // (auth, JWT, rate limit, timeout, trace) still cover it.
        .merge(import_router)
        .merge(compliance_router)
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
        // JWT verification. Runs before `auth_middleware`.
        // In opaque mode (default) it's a no-op pass-through.
        // In JWT mode it verifies the JWS, checks revocation, and injects a
        // Principal into extensions (which `auth_middleware` then sees + passes).
        .layer(middleware::from_fn_with_state(
            state.jwt_middleware_state.clone(),
            jwt_auth_middleware,
        ))
        // Rate limiting — OUTERMOST of the security stack.
        // Previously it sat *inside* both auth layers, so an
        // unauthenticated flood was 401-rejected before ever consuming a
        // bucket: the limiter never bounded the very traffic shape it exists
        // for, and every free 401 performed a synchronous audit write (fresh
        // Connection::open + INSERT) — an unthrottled DB-write-per-request
        // DoS amplification. Outside authN, a flood trips 429 before any
        // token work or audit write happens.
        .layer(middleware::from_fn_with_state(
            state.rate_limiter.clone(),
            rate_limit_middleware,
        ))
        // Security headers — OUTERMOST of the security stack (axum: the last
        // `.layer()` wraps everything before it) so 401/403/429/404 responses
        // carry CSP/nosniff/HSTS too; previously they sat inside auth +
        // rate-limit and pre-auth rejections went out bare.
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
