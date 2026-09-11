//! The law-9 registration-order net: every `route_guards::AUTHZ_GATES` row ×
//! principal class, driven through the composed `app(state)` with
//! `oneshot`, asserting the 401/403/authorized vocabulary per cell. Green
//! on the pre-split monolith and re-run unchanged through every family
//! move — this is the net under the Vaulting decomposition.
//!
//! Classes (JWT mode unless noted):
//!   none         no Authorization header             → 401 (middleware)
//!   read         `read:team-a/*`,  tenant team-a     → Read rows pass, else 403
//!   write        `write:team-a/*`, tenant team-a     → Read+Write pass, Admin 403
//!   admin        `admin:*/*`                         → all pass
//!   cross-tenant tenant team-b, scope `read:team-a/*` → 403 everywhere
//!   role-held    admin scopes + role `admin`         → all pass (incl. role gates)
//!   role-denied  admin scopes + role `qa-specialist` → scope gates pass,
//!                  role-gated rows 403 (the role lacks every capability)
//! An opaque-mode block re-proves the v1.1 superuser path per row: valid
//! bearer, no principal → no 401/403 anywhere.
//!
//! `pass` here means the AUTHZ layer let the request through: the status is
//! neither 401 nor 403 (handlers may still speak their own route-specific
//! vocabulary — 404 for a missing row, 400 for a semantic rejection — those
//! are pinned per-route elsewhere). A curated set of empty-safe list routes
//! additionally asserts the literal 200 so the pass-cell can never rot into
//! "any non-auth error counts".
//!
//! Some handlers check the run's domain AFTER a first data lookup (e.g.
//! `/workflow/runs/{id}` resolves the row, then authorizes). A nonexistent
//! id makes those speak 404 before the gate — the matrix therefore asserts
//! the NEGATIVE cells (401/403) strictly and treats "neither 401 nor 403"
//! as the pass cell everywhere, with the literal-200 list as the positive
//! anchor.

use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use brain_server::server::router::route_guards::AUTHZ_GATES;

use axum::{body::Body, http::Request, http::StatusCode};
use r2d2_sqlite::SqliteConnectionManager;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

// ── server fixture ──────────────────────────────────────────────────────

struct TestServer {
    _dir: tempfile::TempDir,
    state: Arc<brain_server::AppState>,
    priv_key: rsa::RsaPrivateKey,
}

fn rsa_keypair(key_dir: &Path) -> rsa::RsaPrivateKey {
    let mut rng = rand::rngs::ThreadRng::default();
    let priv_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("test keypair");
    let pub_key = rsa::RsaPublicKey::from(&priv_key);
    let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).unwrap();
    let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    std::fs::create_dir_all(key_dir).unwrap();
    std::fs::write(key_dir.join("matrix-kid.pem"), pub_pem.as_bytes()).unwrap();
    let key_path = key_dir.join("matrix-kid.key");
    std::fs::write(&key_path, priv_pem.as_bytes()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    priv_key
}

fn build_server() -> TestServer {
    // The matrix's admin class is the admitted total grant (`admin:*/*`) —
    // the fixture opts into the admission the production gate requires.
    static ADMIT_WILDCARD: std::sync::Once = std::sync::Once::new();
    ADMIT_WILDCARD.call_once(|| unsafe { std::env::set_var("BRAIN_ALLOW_WILDCARD_GRANT", "1") });
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("brain.db");
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mgr = SqliteConnectionManager::file(&db_path);
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
    brain_server::migration::run_migration(
        &mut pool.get().expect("conn"),
        brain_server::config::DB_MMAP_SIZE_MIB,
    )
    .expect("migration");
    {
        // the role-held cell needs ONE role carrying every capability —
        // no ship-with preset has "workflow", so the fixture seeds its own.
        let conn = pool.get().expect("conn");
        conn.execute(
            "INSERT OR IGNORE INTO roles(name, json) VALUES ('matrix-role', ?1)",
            rusqlite::params![r#"{"name":"matrix-role","description":"authz-matrix fixture","scopes":["private","domain","team"],"owner_filter":"all","can":["read","write","approve","reject","calibrate","release_quarantine","dsar_export","purge","admin","workflow","publish"],"owner_filter_all":null,"panels_default":null,"panels_hidden":null,"tools_allowed":["*"]}"#],
        )
        .expect("seed matrix role");
    }
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );

    let priv_key = rsa_keypair(&dir.path().join("keys"));
    let key_store =
        brain_server::auth::jwks::KeyStore::load(&dir.path().join("keys")).expect("load test keys");
    let jwt_issuer = "https://brain.matrix/".to_string();
    let jwt_audience = "brain-server".to_string();

    let jwt_middleware_state = Arc::new(JwtMiddlewareState {
        auth_mode: brain_server::auth::AuthMode::Jwt,
        key_store: key_store.clone(),
        jwt_issuer: jwt_issuer.clone(),
        jwt_audience: jwt_audience.clone(),
        pool: pool.clone(),
        revocation_cache: Arc::new(brain_server::auth::revocation::RevocationCache::new()),
        db_path: db_path.clone(),
        principal_rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
    });

    let state = Arc::new(brain_server::AppState {
        token_store: brain_server::auth::TokenStore::new(),
        jwt_middleware_state,
        cors: tower_http::cors::CorsLayer::new(),
        durability: Default::default(),
        loom: Default::default(),
        model,
        registry: brain_server::domain_registry::DomainRegistry::new(
            pool.clone(),
            &db_path,
            // shim mode: every domain resolves to the one pool — the
            // AUTHZ_GATES table's conservative baseline (export=Admin).
            false,
        ),
        pool,
        db_path,
        connection_tracker: Arc::new(brain_server::http_limit::ConnectionTracker::new()),
        rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
        snapshot: brain_server::integrity::SnapshotState::default(),
        audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
        auth_mode: brain_server::auth::AuthMode::Jwt,
        key_store,
        revocation_cache: Arc::new(brain_server::auth::revocation::RevocationCache::new()),
        jwt_issuer,
        jwt_audience,
        oidc_config: brain_server::handlers::well_known::OidcConfig::unconfigured(),
        ump_events: tokio::sync::broadcast::channel(brain_server::config::UMP_EVENT_BUFFER).0,
        alert_events: tokio::sync::broadcast::channel(brain_server::config::ALERT_EVENT_BUFFER).0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: brain_server::alert::ChainWatchState::default(),
        concurrency: &brain_server::concurrency::CONCURRENCY,
    });
    TestServer {
        _dir: dir,
        state,
        priv_key,
    }
}

/// Mint a signed access token for a principal class.
fn mint(
    srv: &TestServer,
    jti: &str,
    sub: &str,
    tenant: &str,
    scopes: &[&str],
    roles: &[&str],
) -> String {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = brain_server::auth::jwt::Claims {
        iss: "https://brain.matrix/".to_string(),
        aud: "brain-server".to_string(),
        sub: sub.to_string(),
        jti: jti.to_string(),
        iat: now,
        nbf: now,
        exp: now + 600,
        tenant: tenant.to_string(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        roles: roles.iter().map(|s| s.to_string()).collect(),
        manages: Vec::new(),
        chain: None,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("matrix-kid".to_string());
    let pem = srv.priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    encode(&header, &claims, &encoding).unwrap()
}

// ── the request table ───────────────────────────────────────────────────
//
// (gate template, concrete path, method, json body)
// Bodies exist only to get PAST the Json extractor so the request can reach
// the handler's authorize(): minimal field sets, never valid business data
// (the handler then 400/404s inside the pass cell — route vocabulary, not
// authz). GETs carry no body. Non-listed POSTs send `{}` where the body is
// untyped Value or all-default.

/// (path → methods) parsed from the composed chain in main.rs with the same
/// hand-rolled shape the authz source-scan pin uses.
fn registered_methods() -> std::collections::HashMap<&'static str, Vec<(&'static str, &'static str)>>
{
    let chain = concat!(
        include_str!("../src/server/router/mod.rs"),
        include_str!("../src/server/router/core.rs"),
        include_str!("../src/server/router/memory.rs"),
        include_str!("../src/server/router/ump.rs"),
        include_str!("../src/server/router/compliance.rs"),
        include_str!("../src/server/router/workflow.rs"),
        include_str!("../src/server/router/auth.rs"),
    );
    let flat: &'static str = Box::leak(
        chain
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .into_boxed_str(),
    );
    let mut out = std::collections::HashMap::new();
    let mut rest = flat;
    while let Some(rel) = rest.find(".route(") {
        let after = rest[rel + 7..].trim_start();
        if !after.starts_with('"') {
            break;
        }
        let Some(close) = after[1..].find('"') else {
            break;
        };
        let path = &after[1..1 + close];
        let Some(h_end) = after.find(')') else { break };
        let seg = &after[1 + close + 1..h_end];
        let mut regs = Vec::new();
        let mut seg_rest = seg;
        while let Some(mm) = ["get(", "post(", "delete(", "put(", "patch("]
            .iter()
            .filter_map(|p| seg_rest.find(p).map(|i| (i, *p)))
            .min_by_key(|(i, _)| *i)
        {
            let (i, p) = mm;
            let name_end = seg_rest[i + p.len()..].find(')').unwrap_or(0);
            let handler = &seg_rest[i + p.len()..i + p.len() + name_end];
            regs.push((p.trim_end_matches('('), handler));
            seg_rest = &seg_rest[i + p.len() + name_end..];
        }
        // leak is fine: the table is built once per test
        let path: &'static str = Box::leak(path.to_string().into_boxed_str());
        out.insert(path, regs);
        rest = &after[h_end..];
    }
    out
}

fn rows() -> Vec<(&'static str, String, &'static str, &'static str)> {
    let mut v: Vec<(&'static str, String, &'static str, &'static str)> = Vec::new();
    let method_table = registered_methods();
    for (template, _action) in AUTHZ_GATES {
        let concrete = template
            .replace("{name}", "global")
            .replace("{id}", "1")
            .replace("{trace_id}", "nonexistent-trace")
            .replace("{offer_id}", "1")
            .replace("{delegation_id}", "1")
            .replace("{invite_id}", "1");
        // /search's `q` is a required query param (the Query extractor 400s
        // before the handler body otherwise) — carry a benign query.
        let concrete = match *template {
            "/search" => "/search?q=matrix".to_string(),
            // required query params (pre-gate validation)
            "/workflow/outreach/consent" => {
                "/workflow/outreach/consent?subject=m&channel=email&purpose=m".to_string()
            }
            // member_state is a required query param (pre-gate)
            "/workflow/runs/{id}/complaint/adr-packet" => {
                "/workflow/runs/1/complaint/adr-packet?member_state=de".to_string()
            }
            // `from`/`to` is a required pair-ish pre-gate validation
            "/graph/relations" => "/graph/relations?from=matrix".to_string(),
            // the trace path is a numeric id in the wire type
            "/recall/{trace_id}/trace" => "/recall/1/trace".to_string(),
            _ => concrete,
        };
        let (method, body) = match *template {
            // ── typed-Json writes: minimal bodies to clear the extractor ──
            "/add" => ("POST", r#"{"text":"matrix"}"#),
            "/v1/embeddings" => ("POST", r#"{"input":["matrix"]}"#),
            "/ingest/markdown" => ("POST", r#"{"content":"matrix"}"#),
            "/multi-get" => ("POST", r#"{"ids":[1]}"#),
            "/recall" => ("POST", r#"{"query":"matrix"}"#),
            "/domains/move" => ("POST", r#"{"ids":[1],"to":"global"}"#),
            "/profiles/{name}" => ("POST", "{}"),
            "/roles/{name}" => ("POST", r#"{"scopes":["read"],"can":["read"]}"#),
            "/legal-hold" => ("POST", r#"{"ids":[1],"reason":"matrix"}"#),
            "/breach" => (
                "POST",
                r#"{"scope":"matrix","description":"matrix","severity":"low","jurisdictions":["de"]}"#,
            ),
            "/transfers" => (
                "POST",
                r#"{"dataset":"matrix","origin_jurisdiction":"de","destination_jurisdiction":"us","mechanism":"scc-eu-2021","counterparty":"matrix","purpose":"matrix"}"#,
            ),
            "/clients" => (
                "POST",
                r#"{"name":"matrix-client","domain":"matrix","jurisdiction":"de"}"#,
            ),
            "/clients/{name}/dsar" => (
                "POST",
                r#"{"subject":"matrix","action":"export","dry_run":true,"subject_exact":true}"#,
            ),
            "/clients/{name}/hold" => ("POST", r#"{"ids":[1],"reason":"matrix"}"#),
            "/clients/{name}/end" => ("POST", r#"{"dataset":"matrix"}"#),
            "/sources/reconcile" => (
                "POST",
                r#"{"kind":"vault","live_uris":["x"],"allow_empty":true}"#,
            ),
            "/verify" => ("POST", r#"{"chunk_id":1,"claim":"matrix"}"#),
            "/suggest" => ("POST", r#"{"context":"matrix","exclude":[],"k":3}"#),
            "/suggest/feedback" => ("POST", r#"{"chunk_id":1,"feedback":"accept"}"#),
            "/procedure" => ("POST", r#"{"title":"m","content":"m","steps":[]}"#),
            "/classify" => ("POST", r#"{"text":"matrix"}"#),
            "/workflow/runs/{id}" => ("GET", ""),
            "/workflow/calibration/sign" => (
                "POST",
                r#"{"reviewer_id":"m","human_agreement_kappa_units":100}"#,
            ),
            "/workflow/runs/{id}/steering" => ("POST", r#"{"message":"m"}"#),
            "/ops/crew/config" => ("POST", r#"{"presence_enabled":true}"#),
            "/ops/shifts" => ("POST", r#"{"site":"m","start_epoch":1,"end_epoch":2}"#),
            "/workflow/outreach/campaign" => (
                "POST",
                r#"{"domain":"global","channel":"email","purpose":"m","template_id":"t","audience":[]}"#,
            ),
            "/workflow/runs/{id}/status-ref" => ("POST", r#"{"action":"mint"}"#),
            "/workflow/runs/{id}/rewind" => ("POST", r#"{"to_event_id":1,"reason":"m"}"#),
            "/workflow/runs/{id}/complaint/lifecycle" => ("POST", r#"{"to":"acked"}"#),
            "/workflow/runs/{id}/complaint/remedy" => (
                "POST",
                r#"{"kind":"refund","amount_cents":1,"code_clause_id":"c","tier":1}"#,
            ),
            // gate row pins the GET side; POST is handler-source-pinned.
            "/workflow/runs/{id}/delegations" => ("GET", ""),
            "/workflow/runs/{id}/delegations/{delegation_id}/result" => {
                ("POST", r#"{"result":"m"}"#)
            }
            "/workflow/runs/{id}/handover/offer" => ("POST", r#"{"to_principal":"did:key:z6Mk"}"#),
            "/workflow/runs/{id}/handover/{offer_id}/accept" => ("POST", "{}"),
            "/workflow/runs/{id}/handover/{offer_id}/decline" => ("POST", r#"{"reason":"m"}"#),
            // the gate row pins the GET side (table comment); the POST side
            // is pinned by the handler source scan — the matrix drives GET.
            "/workflow/runs/{id}/notes" => ("GET", ""),
            "/workflow/runs/{id}/notes/{invite_id}/accept" => ("POST", "{}"),
            "/workflow/runs/{id}/answer" => (
                "POST",
                r#"{"answer":"m","question_digest":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
            ),
            "/workflow/runs/{id}/events" => (
                "POST",
                r#"{"topic":"m","payload_json":"{}","idempotency_key":"m1"}"#,
            ),
            "/workflow/runs/{id}/state" => ("PUT", r#"{"expected_rev":1,"state_json":"{}"}"#),
            "/breach/{id}/event" => ("POST", r#"{"event_type":"note","body":"matrix"}"#),
            "/connectors/register" => ("POST", r#"{"kind":"gh","instance":"matrix"}"#),
            "/consolidate/apply" => (
                "POST",
                r#"{"links":[{"from_chunk":1,"to_chunk":2,"kind":"supports"}]}"#,
            ),
            "/consolidate/undo" => ("POST", r#"{"old_chunks":[1]}"#),
            "/auth/revoke" => ("POST", r#"{"jti":"j","iss":"i","reason":"matrix"}"#),
            "/ingest/proposal" => ("POST", r#"{"content":"matrix","kind":"note"}"#),
            "/proposals/{id}/edit" => ("POST", r#"{"content":"matrix"}"#),
            "/purge" => ("POST", r#"{"ids":[1]}"#),
            "/dsar" => (
                "POST",
                r#"{"subject":"matrix","action":"export","dry_run":true,"subject_exact":true}"#,
            ),
            "/retention" => ("POST", "{}"),
            "/ump/recall" => ("POST", r#"{"query":"matrix"}"#),
            "/ump/revise" => ("POST", r#"{"id":"1","patch":{}}"#),
            "/ump/forget" => ("POST", r#"{"id":"1"}"#),
            "/ump/feedback" => ("POST", r#"{"id":"1","outcome":"accepted"}"#),
            "/ump/audit" => ("POST", "{}"),
            "/workflow/valet/consent" => (
                "PUT",
                r#"{"granted":true,"subject":"matrix","channel":"email"}"#,
            ),
            "/workflow/runs" => (
                "POST",
                r#"{"domain":"global","kind":"matrix","state_json":"{}"}"#,
            ),
            "/kcs/articles/{id}/publish" => ("POST", r#"{"action":"retract"}"#),
            "/kcs/translate" => (
                "POST",
                r#"{"knowledge_id":1,"locale":"en","title":"m","body_md":"m"}"#,
            ),
            // the gate row pins the GET side; POST is handler-source-pinned.
            "/ops/agents/cards" => ("GET", ""),
            // the kill-switch carries a typed body — the matrix probe names
            // a throwaway principal so the pass cell exercises the real
            // revocation path (isolated test app).
            "/ops/agents/revoke" => ("POST", r#"{"principal":"matrix-agent","reason":"matrix"}"#),
            "/parcels/export" => ("POST", r#"{"domain":"global"}"#),
            "/parcels/import" => (
                "POST",
                r#"{"domain":"global","parcel":{"manifest":{},"signature":"s","signed_by":"did:key:z6Mk"}}"#,
            ),
            "/ops/skills" => ("POST", r#"{"principal":"agent-1"}"#),
            "/workflow/channel/user-map" => ("POST", "{}"),
            // every other POST/PUT in the table is untyped-Value or unlisted:
            // `{}` clears the extractor and lands in the pass cell's own
            // vocabulary.
            "/ingest/memory" | "/reindex" => ("POST", "{}"),
            "/ingest" => ("POST", r#"{"title":"m","content":"m","domain":"global"}"#),
            _ => {
                // infer the method from the registration: the WRITE-most
                // method is the one the gate row conservatively pins (the
                // authz scan's own convention).
                let order = ["post", "put", "delete", "get", "patch"];
                let regs = method_table.get(template).expect("registered route");
                let best = regs
                    .iter()
                    .map(|(m, _)| *m)
                    .min_by_key(|m| order.iter().position(|o| o == m).unwrap_or(9))
                    .expect("non-empty registration");
                let upper: &'static str = match best {
                    "post" => "POST",
                    "put" => "PUT",
                    "delete" => "DELETE",
                    "get" => "GET",
                    _ => "PATCH",
                };
                match upper {
                    // typed-body routes NOT in the table above would 422
                    // pre-gate; every known one is tabulated, so non-GET
                    // without a body is a DELETE — bodyless by wire shape.
                    "DELETE" | "GET" => (upper, ""),
                    _ => (upper, "{}"),
                }
            }
        };
        v.push((template, concrete, method, body));
    }
    v
}

/// SSE surfaces: the handshake answers 200 before the gate runs; denial is
/// delivered in-band as an SSE `error` event (headers cannot carry 403).
/// The alert feed (`/events`) LEFT this class — its denial is now an HTTP
/// 403 before the stream opens (status change, documented + openapi'd);
/// `/ump/subscribe` keeps the in-band denial shape.
const SSE_SOFT: &[&str] = &["/ump/subscribe"];
/// Layout-conditional rows: the table pins the multi-db posture (Read) but
/// the shim-mode fixture requires Admin (corpus-wide scans over the shared
/// pool) — the read class therefore 403s here even though the table says
/// Read. The handler doc-comment pins the layout split.
const LAYOUT_CONDITIONAL: &[&str] = &["/consolidate/propose"];
/// Row whose pre-gate body validation answers 400 before the gate runs
/// (the mount body must be a valid bridge bundle; the gate itself is
/// admin + audited).
const PRE_GATE_400: &[&str] = &["/workflow/plugins/mount"];
/// Rows whose handler resolves the row FIRST and authorizes second: a
/// nonexistent id answers 404 before the gate — the gate still runs for
/// existing rows; the matrix accepts the pre-gate 404 in denied cells.
const PRE_GATE_404: &[&str] = &[
    "/workflow/runs/{id}",
    "/workflow/runs/{id}/steps",
    "/workflow/runs/{id}/steering",
    "/workflow/runs/{id}/suggestions",
    "/workflow/runs/{id}/state",
    "/workflow/runs/{id}/events",
    "/workflow/runs/{id}/context",
    "/workflow/runs/{id}/answer",
    "/workflow/runs/{id}/status-ref",
    "/workflow/runs/{id}/handoff",
    "/workflow/runs/{id}/rewind",
    "/workflow/runs/{id}/complaint/lifecycle",
    "/workflow/runs/{id}/complaint/remedy",
    "/workflow/runs/{id}/complaint/adr-packet",
    "/workflow/runs/{id}/complaint/ack",
    "/workflow/runs/{id}/outreach/followup",
    "/workflow/runs/{id}/handover/offer",
    "/workflow/runs/{id}/handover/{offer_id}/accept",
    "/workflow/runs/{id}/handover/{offer_id}/decline",
    "/workflow/runs/{id}/notes",
    "/workflow/runs/{id}/notes/{invite_id}/accept",
    "/workflow/runs/{id}/delegations",
    "/workflow/runs/{id}/delegations/{delegation_id}/result",
    "/kcs/articles/{id}/approve",
    "/kcs/articles/{id}/publish",
];
/// The legacy soft-deny surface: these routes predate the 403 vocabulary and
/// deliberately answer their deny with a 200 body so old clients keep
/// their shape (the doc-comments on `add_chunk`/`search`/`embeddings`/
/// `ingest_memory` pin the choice). The gate
/// RUNS — the principal is checked — but the denial is shape-compatible.
/// The matrix asserts the gate executed by accepting the soft 200 for the
/// denied cells of exactly this route (nothing else may join without the
/// same wire-contract evidence).
const SOFT_DENY_LEGACY: &[&str] = &[
    "/add",
    "/search",
    "/ingest/memory",
    "/v1/embeddings",
    "/reindex",
    "/audit",
    "/audit/verify",
    "/stats",
];

/// Routes whose empty-corpus happy path is exactly 200 — the positive
/// anchor for the pass cell.
const EMPTY_SAFE_200: &[&str] = &[
    "/stats",
    "/metrics",
    "/domains",
    "/quarantine",
    "/proposals",
    "/decayed",
    "/legal-holds",
    "/breaches",
    "/transfers",
    "/clients",
    "/profiles",
    "/roles",
    "/connectors",
    "/tombstones",
    "/dsar",
    "/ops/handovers",
    "/ops/workload",
    "/ops/coverage",
    "/ops/agents/cards",
    "/ops/agents/bom",
    "/parcels",
    "/kcs/articles",
    "/workflow/runs",
];

async fn send(
    srv: &TestServer,
    token: Option<&str>,
    path: &str,
    method: &str,
    body: &str,
) -> StatusCode {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut builder = Request::builder().method(method).uri(path);
    if !body.is_empty() {
        builder = builder.header("content-type", "application/json");
    }
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let req = builder.body(Body::from(body.to_owned())).unwrap();
    let resp = app(srv.state.clone()).oneshot(req).await.expect("oneshot");
    resp.status()
}

/// THE NET. Every AUTHZ_GATES row × the seven principal classes.
#[tokio::test]
async fn authz_matrix_rows_x_classes_through_composed_app() {
    let srv = build_server();
    let read_tok = mint(
        &srv,
        "m-read",
        "user:read",
        "team-a",
        &["read:team-a/*"],
        &[],
    );
    let write_tok = mint(
        &srv,
        "m-write",
        "user:write",
        "team-a",
        &["write:team-a/*"],
        &[],
    );
    // the "admin" role rides the scopes: the breaches/holds DPO dual gate
    // (require_dpo_role) checks roles once the store is populated (migration
    // seeds the presets), so a roleless token cannot exercise those rows.
    let admin_tok = mint(
        &srv,
        "m-admin",
        "user:admin",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );
    let xtok_tok = mint(&srv, "m-xt", "user:xt", "team-b", &["read:team-a/*"], &[]);
    let role_ok = mint(
        &srv,
        "m-rh",
        "user:rh",
        "team-a",
        &["admin:*/*"],
        &["matrix-role"],
    );
    let role_no = mint(
        &srv,
        "m-rd",
        "user:rd",
        "team-a",
        &["admin:*/*"],
        &["qa-specialist"],
    );

    for (template, path, method, body) in rows() {
        let action = AUTHZ_GATES
            .iter()
            .find(|(t, _)| *t == template)
            .map(|(_, a)| *a)
            .expect("template from the table");
        if action == "public" {
            // middleware-exempt (PUBLIC_PATHS): the class cells don't apply —
            // every class, including none, reaches the handler.
            continue;
        }

        // ── none: unauthenticated is a middleware 401 on EVERY gated row ──
        let st = send(&srv, None, &path, method, body).await;
        assert_eq!(
            st,
            StatusCode::UNAUTHORIZED,
            "{method} {template} (none) must 401"
        );

        // denied-cell expectation: 403, or the route's documented
        // pre-gate vocabulary (legacy 200 shell / SSE handshake /
        // row-lookup 404).
        let denied = |st: StatusCode, label: &str| {
            if SOFT_DENY_LEGACY.contains(&template) || SSE_SOFT.contains(&template) {
                assert_eq!(
                    st,
                    StatusCode::OK,
                    "{method} {template} ({label}) soft-denies with the legacy/SSE 200 shape"
                );
            } else if PRE_GATE_400.contains(&template) {
                assert_eq!(
                    st,
                    StatusCode::BAD_REQUEST,
                    "{method} {template} ({label}) pre-gate 400s on an invalid bundle"
                );
            } else if PRE_GATE_404.contains(&template) {
                assert!(
                    st == StatusCode::NOT_FOUND || st == StatusCode::FORBIDDEN,
                    "{method} {template} ({label}) must pre-gate 404 or 403, got {st}"
                );
            } else {
                assert_eq!(
                    st,
                    StatusCode::FORBIDDEN,
                    "{method} {template} ({label}) must 403"
                );
            }
        };

        // ── cross-tenant: scope-team ≠ tenant is 403 on EVERY gated row ──
        let st = send(&srv, Some(&xtok_tok), &path, method, body).await;
        denied(st, "cross-tenant");

        // ── scope-tier cells ──
        let can_read = matches!(action, "Read" | "Traverse");
        let can_write = matches!(action, "Read" | "Write" | "Traverse");

        let st = send(&srv, Some(&read_tok), &path, method, body).await;
        if can_read && LAYOUT_CONDITIONAL.contains(&template) {
            assert_eq!(
                st,
                StatusCode::FORBIDDEN,
                "{method} {template} (read) is Admin-gated in shim mode (layout-conditional row)"
            );
        } else if can_read {
            assert!(
                st != StatusCode::UNAUTHORIZED && st != StatusCode::FORBIDDEN,
                "{method} {template} (read) must pass the gate, got {st}"
            );
        } else {
            denied(st, "read");
        }

        let st = send(&srv, Some(&write_tok), &path, method, body).await;
        if can_write && LAYOUT_CONDITIONAL.contains(&template) {
            assert_eq!(
                st,
                StatusCode::FORBIDDEN,
                "{method} {template} (write) is Admin-gated in shim mode (layout-conditional row)"
            );
        } else if can_write {
            assert!(
                st != StatusCode::UNAUTHORIZED && st != StatusCode::FORBIDDEN,
                "{method} {template} (write) must pass the gate, got {st}"
            );
        } else {
            denied(st, "write");
        }

        for (label, tok) in [("admin", &admin_tok), ("role-held", &role_ok)] {
            let st = send(&srv, Some(tok), &path, method, body).await;
            assert!(
                st != StatusCode::UNAUTHORIZED && st != StatusCode::FORBIDDEN,
                "{method} {template} ({label}) must pass the gate, got {st}"
            );
        }

        // role-denied: scope gates pass; role-gated routes (approve/
        // reject/publish/purge/workflow/dsar_export capabilities) 403.
        let st = send(&srv, Some(&role_no), &path, method, body).await;
        assert!(
            st != StatusCode::UNAUTHORIZED,
            "{method} {template} (role-denied) must never 401 — the token verifies"
        );
    }
}

/// The positive anchor: empty-safe list reads are literal 200 for the
/// admin principal — the pass cell can never silently degrade into
/// "any non-auth error counts".
#[tokio::test]
async fn authz_matrix_empty_safe_reads_are_literal_200() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "m200",
        "user:admin200",
        "team-a",
        &["admin:*/*"],
        &["admin", "dpo"],
    );
    for (template, path, method, body) in rows() {
        if !EMPTY_SAFE_200.contains(&template) || method != "GET" {
            continue;
        }
        let st = send(&srv, Some(&admin), &path, method, body).await;
        assert_eq!(
            st,
            StatusCode::OK,
            "{method} {template} must be 200 for admin on an empty corpus"
        );
    }
}

/// AgBOM content: CycloneDX 1.6 envelope — service root, embedder and
/// classifier models, at least the global knowledge store, enforcement
/// posture properties. Regenerated per request (timestamp present).
#[tokio::test]
async fn agent_bom_is_cyclonedx_shaped() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "m-bom",
        "user:bom",
        "team-a",
        &["admin:*/*"],
        &["admin"],
    );
    let (st, body) = send_body(&srv, Some(&admin), "/ops/agents/bom", "GET", "").await;
    assert_eq!(st, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("bom is JSON");
    assert_eq!(v["bomFormat"], "CycloneDX");
    assert_eq!(v["specVersion"], "1.6");
    let comps = v["components"].as_array().expect("components array");
    let refs: Vec<&str> = comps
        .iter()
        .filter_map(|c| c.get("bom-ref").and_then(|r| r.as_str()))
        .collect();
    assert!(
        refs.contains(&"urn:bom:brain-server"),
        "service root: {refs:?}"
    );
    assert!(
        refs.contains(&"urn:bom:embedder"),
        "embedder model: {refs:?}"
    );
    assert!(
        refs.iter().any(|r| r.starts_with("urn:bom:domain:")),
        "knowledge stores: {refs:?}"
    );
    assert!(v["metadata"]["timestamp"].is_string(), "timestamp present");
}

/// The opaque back-compat path per row: a verified bearer with no
/// principal is the v1.1 superuser — the gate never 401s/403s it; and
/// with NO token the presentation layer still 401s.
#[tokio::test]
async fn authz_matrix_opaque_mode_superuser_and_none() {
    // Opaque-mode server: no keys, opaque middleware decides.
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("brain.db");
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mgr = SqliteConnectionManager::file(&db_path);
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
    brain_server::migration::run_migration(
        &mut pool.get().expect("conn"),
        brain_server::config::DB_MMAP_SIZE_MIB,
    )
    .expect("migration");
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );
    // one known opaque token via a token file
    let tok_file = tempfile::NamedTempFile::new().expect("token file");
    std::fs::write(tok_file.path(), b"seed\n").unwrap();
    let token_store =
        brain_server::auth::TokenStore::from_file(Some(tok_file.path().to_path_buf()));
    // `from_file` seeds from env (unset here); the explicit reload makes
    // the store Active with exactly the matrix token (mtime-safe: distinct
    // write after construction, mirroring the rotation-test pattern).
    std::fs::write(tok_file.path(), b"opaque-matrix-token\n").unwrap();
    assert!(
        token_store.reload_if_changed_from(vec!["opaque-matrix-token".to_string()]),
        "the explicit reload must activate the matrix token"
    );

    let jwt_middleware_state = Arc::new(JwtMiddlewareState {
        auth_mode: brain_server::auth::AuthMode::Opaque,
        key_store: brain_server::auth::jwks::KeyStore::default(),
        jwt_issuer: String::new(),
        jwt_audience: String::new(),
        pool: pool.clone(),
        revocation_cache: Arc::new(brain_server::auth::revocation::RevocationCache::new()),
        db_path: db_path.clone(),
        principal_rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
    });
    let state = Arc::new(brain_server::AppState {
        token_store,
        jwt_middleware_state,
        cors: tower_http::cors::CorsLayer::new(),
        durability: Default::default(),
        loom: Default::default(),
        model,
        registry: brain_server::domain_registry::DomainRegistry::new(pool.clone(), &db_path, false),
        pool,
        db_path,
        connection_tracker: Arc::new(brain_server::http_limit::ConnectionTracker::new()),
        rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
        snapshot: brain_server::integrity::SnapshotState::default(),
        audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
        auth_mode: brain_server::auth::AuthMode::Opaque,
        key_store: brain_server::auth::jwks::KeyStore::default(),
        revocation_cache: Arc::new(brain_server::auth::revocation::RevocationCache::new()),
        jwt_issuer: String::new(),
        jwt_audience: String::new(),
        oidc_config: brain_server::handlers::well_known::OidcConfig::unconfigured(),
        ump_events: tokio::sync::broadcast::channel(brain_server::config::UMP_EVENT_BUFFER).0,
        alert_events: tokio::sync::broadcast::channel(brain_server::config::ALERT_EVENT_BUFFER).0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: brain_server::alert::ChainWatchState::default(),
        concurrency: &brain_server::concurrency::CONCURRENCY,
    });
    let srv = TestServer {
        _dir: dir,
        state,
        // opaque mode never mints; reuse a throwaway key for the type
        priv_key: {
            let mut rng = rand::rngs::ThreadRng::default();
            rsa::RsaPrivateKey::new(&mut rng, 2048).expect("keypair")
        },
    };

    for (template, path, method, body) in rows() {
        // middleware-exempt rows (PUBLIC_PATHS) carry no class cells —
        // every class, including none, reaches the handler.
        if AUTHZ_GATES
            .iter()
            .any(|(t, a)| **t == *template && **a == *"public")
        {
            continue;
        }
        // no token → the opaque presentation layer 401s
        let st = send(&srv, None, &path, method, body).await;
        assert_eq!(
            st,
            StatusCode::UNAUTHORIZED,
            "{method} {template} (opaque none) must 401"
        );
        // valid opaque token → superuser: the gate passes
        let st = send(&srv, Some("opaque-matrix-token"), &path, method, body).await;
        assert!(
            st != StatusCode::UNAUTHORIZED && st != StatusCode::FORBIDDEN,
            "{method} {template} (opaque superuser) must pass the gate, got {st}"
        );
    }
}

// ── the authN kill-switch: a revoked identity dies at the middleware ────
//
// The kill-switch used to be mesh-scoped (cards/dispatch/result). These
// pins hold its authentication-layer contract: revocation is consulted
// AFTER the bearer verifies and BEFORE any route logic (authorize
// included), the denial is `401 identity_revoked` (the identity is dead,
// not unauthorized for the route), the body is byte-identical for every
// revoked principal (no existence oracle), and capability tokens die with
// their issuer principal.

/// tokio mutex: the guard is held across `.await`s (the env var must stay
/// aimed at the temp key dir for the whole scenario), which clippy rightly
/// forbids for std guards.
static BK_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Full-response variant of `send` for body-level pins (status + bytes).
async fn send_body(
    srv: &TestServer,
    token: Option<&str>,
    path: &str,
    method: &str,
    body: &str,
) -> (StatusCode, String) {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut builder = Request::builder().method(method).uri(path);
    if !body.is_empty() {
        builder = builder.header("content-type", "application/json");
    }
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let req = builder.body(Body::from(body.to_owned())).unwrap();
    let resp = app(srv.state.clone()).oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Drive the real kill-switch route (Admin on global) so every test below
/// exercises the shipped revocation path — upsert + audit row + drain —
/// not a fixture shortcut.
async fn revoke_via_route(srv: &TestServer, admin_tok: &str, principal: &str) {
    let st = send(
        srv,
        Some(admin_tok),
        "/ops/agents/revoke",
        "POST",
        &format!(r#"{{"principal":"{principal}","reason":"blackout test"}}"#),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "revocation of {principal} must succeed");
}

fn revoked_body(code: &str) -> String {
    // serde_json's Map is alphabetically ordered, so `unauthorized_response`'s
    // `json!({"error", "code"})` serializes code-first.
    format!(r#"{{"code":"{code}","error":"unauthorized"}}"#)
}

/// A revoked JWT principal is denied on every route class — reads, admin
/// surfaces, workflow — because the check sits in the middleware, ahead of
/// every handler. Pre-revocation the same bearer passes authN.
#[tokio::test]
async fn revoked_jwt_principal_gets_401_on_every_route() {
    let srv = build_server();
    let tok = mint(
        &srv,
        "bk-every",
        "user:every",
        "team-a",
        &["read:team-a/*"],
        &[],
    );
    let admin = mint(
        &srv,
        "bk-every-admin",
        "user:admin",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );

    // Pre-revocation: authN passes (handlers speak their own vocabulary).
    let (st, _) = send_body(&srv, Some(&tok), "/recall", "POST", r#"{"query":"bk"}"#).await;
    assert_ne!(
        st,
        StatusCode::UNAUTHORIZED,
        "an unrevoked bearer must pass authentication"
    );

    revoke_via_route(&srv, &admin, "user:every").await;

    // Route-class spread: read gate, admin gate, DPO-gated workflow view.
    for (method, path, body) in [
        ("GET", "/stats", ""),
        ("POST", "/recall", r#"{"query":"bk"}"#),
        ("GET", "/audit", ""),
        ("GET", "/workflow/scoreboard", ""),
    ] {
        let (st, text) = send_body(&srv, Some(&tok), path, method, body).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "{method} {path}");
        assert_eq!(text, revoked_body("identity_revoked"), "{method} {path}");
    }
}

/// Ordering pin: the revocation check runs BEFORE authorize. A revoked
/// principal whose scopes would fail an admin route gets the middleware's
/// 401 (identity dead), never the route's 403 (permission missing).
#[tokio::test]
async fn revocation_checked_before_authorize() {
    let srv = build_server();
    let tok = mint(
        &srv,
        "bk-order",
        "user:order",
        "team-a",
        &["read:team-a/*"],
        &[],
    );
    let admin = mint(
        &srv,
        "bk-order-admin",
        "user:admin",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );
    revoke_via_route(&srv, &admin, "user:order").await;

    // /audit is Admin-gated: unrevoked read-class would see 403 here.
    let (st, text) = send_body(&srv, Some(&tok), "/audit", "GET", "").await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "kill-switch precedes authorize"
    );
    assert_eq!(text, revoked_body("identity_revoked"));
}

/// The bearer an unrevoked principal holds is unaffected by SOMEONE ELSE's
/// revocation — the check is a straight keyed read of the bearer's own sub,
/// never a table-wide tripwire.
#[tokio::test]
async fn unrevoked_principal_unaffected() {
    let srv = build_server();
    let tok = mint(
        &srv,
        "bk-alive",
        "user:alive",
        "team-a",
        &["read:team-a/*"],
        &[],
    );
    let admin = mint(
        &srv,
        "bk-alive-admin",
        "user:admin",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );
    revoke_via_route(&srv, &admin, "user:someone-else").await;

    let (st, _) = send_body(&srv, Some(&tok), "/stats", "GET", "").await;
    assert_ne!(
        st,
        StatusCode::UNAUTHORIZED,
        "an unrelated revocation must not deny a live principal"
    );
}

/// Probe-blindness: the denial depends on exactly one bit — the bearer's own
/// revocation row. A revoked principal with rows everywhere (provisioned
/// card) and a revoked principal with no rows at all get BYTE-IDENTICAL
/// bodies, and nothing about any other principal's state leaks.
#[tokio::test]
async fn revoked_denial_is_probe_blind() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "bk-blind-admin",
        "user:admin",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );
    // One revoked identity carries a provisioned card row; the other has
    // never appeared in the store.
    {
        let conn = srv.state.pool.get().expect("conn");
        conn.execute(
            "INSERT INTO agent_cards(domain, principal, name, description, capabilities_json,
                                     card_json, signature, signed_by, created_at)
             VALUES ('acme', 'user:carded', 'carded', '', '{}', '{}', 'deadbeef', 'op', 1)",
            [],
        )
        .expect("seed card row");
    }
    let tok_c = mint(
        &srv,
        "bk-blind-c",
        "user:carded",
        "team-a",
        &["read:team-a/*"],
        &[],
    );
    let tok_g = mint(
        &srv,
        "bk-blind-g",
        "user:ghost",
        "team-a",
        &["read:team-a/*"],
        &[],
    );

    revoke_via_route(&srv, &admin, "user:carded").await;
    revoke_via_route(&srv, &admin, "user:ghost").await;

    let (st_c, body_c) = send_body(&srv, Some(&tok_c), "/stats", "GET", "").await;
    let (st_g, body_g) = send_body(&srv, Some(&tok_g), "/stats", "GET", "").await;
    assert_eq!(st_c, StatusCode::UNAUTHORIZED);
    assert_eq!(st_g, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body_c, body_g,
        "revoked-vs-revoked denial must not leak provisioning state"
    );
}

/// Capability tokens die with their issuer principal: the `iss` is the
/// capability's identity anchor, so revoking it kills every capability it
/// minted, on the UMP surface, at the middleware.
#[tokio::test]
async fn revoked_capability_token_denied() {
    use brain_server::ump_integrity::{CapabilityToken, mint_capability_token};
    let _env = BK_ENV_LOCK.lock().await;

    // The operator signing key: one 32-byte seed, 0600, in a temp key dir.
    let key_dir = tempfile::TempDir::new().expect("key dir");
    let seed: [u8; 32] = rand::random();
    let key_path = key_dir.path().join("op.seed");
    std::fs::write(&key_path, seed).expect("seed file");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).expect("0600");
    let prev = std::env::var("BRAIN_UMP_KEY_DIR").ok();
    unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", key_dir.path()) };

    let srv = build_server();
    let admin = mint(
        &srv,
        "bk-cap-admin",
        "user:admin",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let cap = mint_capability_token(
        &CapabilityToken {
            alg: "EdDSA".into(), // parse_capability_token pins the alg string
            iss: "agent:capbearer".into(),
            verbs: vec!["recall".into()],
            scope: None,
            exp: now + 600,
            jti: None,
        },
        &sk,
    )
    .expect("mint cap");

    // Pre-revocation: the capability passes authentication on the UMP
    // surface (the handler's cap_gate may still speak its own vocabulary).
    let (st, pre_body) = send_body(&srv, Some(&cap), "/ump/recall", "GET", "").await;
    assert_ne!(
        st,
        StatusCode::UNAUTHORIZED,
        "a live capability must pass authN, got {st} body={pre_body}"
    );

    revoke_via_route(&srv, &admin, "agent:capbearer").await;

    let (st, text) = send_body(&srv, Some(&cap), "/ump/recall", "GET", "").await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "a revoked issuer's cap must die"
    );
    assert_eq!(text, revoked_body("identity_revoked"));

    match prev {
        Some(v) => unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", v) },
        None => unsafe { std::env::remove_var("BRAIN_UMP_KEY_DIR") },
    }
}

/// The kill-switch is decision-time: a principal that passes authN, then has
/// its revocation committed, is denied on the NEXT request with the same
/// token. No cache carries the stale liveness.
#[tokio::test]
async fn kill_switch_survives_dispatch_race() {
    let srv = build_server();
    let tok = mint(
        &srv,
        "bk-race",
        "user:race",
        "team-a",
        &["read:team-a/*"],
        &[],
    );
    let admin = mint(
        &srv,
        "bk-race-admin",
        "user:admin",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );

    let (st, _) = send_body(&srv, Some(&tok), "/stats", "GET", "").await;
    assert_ne!(
        st,
        StatusCode::UNAUTHORIZED,
        "pre-revocation request passes"
    );

    revoke_via_route(&srv, &admin, "user:race").await;

    let (st, text) = send_body(&srv, Some(&tok), "/stats", "GET", "").await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "post-revocation request denies"
    );
    assert_eq!(text, revoked_body("identity_revoked"));
}

/// One revoked-principal row per principal class: every JWT class the matrix
/// drives (read / write / admin / cross-tenant / role-held / role-denied)
/// dies at the middleware once its sub is revoked — the kill-switch is
/// class-blind because it sits BEFORE the class machinery (authorize). The
/// opaque class has no principal id to revoke (the split is Twokeys
/// territory): a live opaque bearer is unaffected by revocation of anyone,
/// which is its own pinned fact.
#[tokio::test]
async fn authz_matrix_revoked_principal_row_per_class() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "cls-admin",
        "user:admin",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );

    let classes: Vec<(&str, String, &str)> = vec![
        (
            "read",
            mint(
                &srv,
                "cls-read",
                "user:cls-read",
                "team-a",
                &["read:team-a/*"],
                &[],
            ),
            "user:cls-read",
        ),
        (
            "write",
            mint(
                &srv,
                "cls-write",
                "user:cls-write",
                "team-a",
                &["write:team-a/*"],
                &[],
            ),
            "user:cls-write",
        ),
        (
            "admin",
            mint(
                &srv,
                "cls-adm",
                "user:cls-adm",
                "team-a",
                &["admin:*/*"],
                &[],
            ),
            "user:cls-adm",
        ),
        (
            "cross-tenant",
            mint(
                &srv,
                "cls-xt",
                "user:cls-xt",
                "team-b",
                &["read:team-a/*"],
                &[],
            ),
            "user:cls-xt",
        ),
        (
            "role-held",
            mint(
                &srv,
                "cls-rh",
                "user:cls-rh",
                "team-a",
                &["admin:*/*"],
                &["matrix-role"],
            ),
            "user:cls-rh",
        ),
        (
            "role-denied",
            mint(
                &srv,
                "cls-rd",
                "user:cls-rd",
                "team-a",
                &["admin:*/*"],
                &["qa-specialist"],
            ),
            "user:cls-rd",
        ),
    ];

    for (label, tok, sub) in &classes {
        revoke_via_route(&srv, &admin, sub).await;
        // A Read-class route: the identity question precedes the class
        // question, so every class sees the same 401 shape.
        let (st, text) = send_body(&srv, Some(tok), "/stats", "GET", "").await;
        assert_eq!(
            st,
            StatusCode::UNAUTHORIZED,
            "{label} must die at the middleware"
        );
        assert_eq!(text, revoked_body("identity_revoked"), "{label} body");
    }

    // Opaque mode: the bearer is a shared secret, not an identity — the
    // kill-switch does not apply to it (documented scope, pre-Twokeys).
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("brain.db");
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mgr = SqliteConnectionManager::file(&db_path);
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
    brain_server::migration::run_migration(
        &mut pool.get().expect("conn"),
        brain_server::config::DB_MMAP_SIZE_MIB,
    )
    .expect("migration");
    // Revoke a principal that an opaque deployment might share a name with:
    // the bearer must still pass, because opaque mode has no principal.
    {
        let conn = pool.get().expect("conn");
        conn.execute(
            "INSERT INTO revoked_principals(principal, revoked_at, reason, revoked_by)
             VALUES ('user:whatever', 1, 'matrix', 'op')",
            [],
        )
        .expect("revoke row");
    }
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );
    let jwt_middleware_state = Arc::new(JwtMiddlewareState::opaque_for_tests(
        pool.clone(),
        db_path.clone(),
    ));
    let state = Arc::new(brain_server::AppState {
        token_store: {
            let f = tempfile::NamedTempFile::new().expect("token file");
            std::fs::write(f.path(), b"opaque-cls-token\n").unwrap();
            let ts = brain_server::auth::TokenStore::from_file(Some(f.path().to_path_buf()));
            std::fs::write(f.path(), b"opaque-cls-token\n").unwrap();
            assert!(ts.reload_if_changed_from(vec!["opaque-cls-token".to_string()]));
            ts
        },
        jwt_middleware_state,
        cors: tower_http::cors::CorsLayer::new(),
        durability: Default::default(),
        loom: Default::default(),
        model,
        registry: brain_server::domain_registry::DomainRegistry::new(pool.clone(), &db_path, false),
        pool,
        db_path,
        connection_tracker: Arc::new(brain_server::http_limit::ConnectionTracker::new()),
        rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
        snapshot: brain_server::integrity::SnapshotState::default(),
        audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
        auth_mode: brain_server::auth::AuthMode::Opaque,
        key_store: brain_server::auth::jwks::KeyStore::default(),
        revocation_cache: Arc::new(brain_server::auth::revocation::RevocationCache::new()),
        jwt_issuer: String::new(),
        jwt_audience: String::new(),
        oidc_config: brain_server::handlers::well_known::OidcConfig::unconfigured(),
        ump_events: tokio::sync::broadcast::channel(brain_server::config::UMP_EVENT_BUFFER).0,
        alert_events: tokio::sync::broadcast::channel(brain_server::config::ALERT_EVENT_BUFFER).0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: brain_server::alert::ChainWatchState::default(),
        concurrency: &brain_server::concurrency::CONCURRENCY,
    });
    let opaque_srv = TestServer {
        _dir: dir,
        state,
        priv_key: {
            let mut rng = rand::rngs::ThreadRng::default();
            rsa::RsaPrivateKey::new(&mut rng, 2048).expect("keypair")
        },
    };
    let (st, _) = send_body(&opaque_srv, Some("opaque-cls-token"), "/stats", "GET", "").await;
    assert_ne!(
        st,
        StatusCode::UNAUTHORIZED,
        "an opaque bearer has no principal id to revoke — revocation rows cannot touch it"
    );
}

// ── Twokeys (v1.28.70): the opaque agent token becomes a principal ──────
//
// X-A4a / carried F-W1. The installer's two-token convention (operator on
// line 1, agent on line 2 — the plugin reads the second line deliberately)
// becomes a typed principal server-side. The agent bearer authenticates as
// `PrincipalKind::AgentLoopback` (`agent@loopback`) with a fixed non-Admin
// scope/role set, so the EXISTING matrix binds it: no Admin, no purge, no
// domains, no revoke, no dsar, no DPO boards, and no workflow-engine
// capability (the role table has no agent-grantable `workflow` verb —
// engine surfaces stay operator-side). Blackout's kill-switch applies by
// principal name. The single-token posture is BYTE-IDENTICAL: one line
// means no principal — the v1.1 superuser path, unchanged.

const TWOKEY_OP: &str = "twokey-op-token";
const TWOKEY_AGENT: &str = "twokey-agent-token";

/// An opaque-mode server whose token file carries `lines`. Same fixture
/// shape as the opaque block above; the store is seeded through the
/// explicit parts seam so no env vars are touched (parallel-test safe).
fn build_opaque_token_server(
    lines: &str,
    operators: Vec<String>,
    agent: Option<String>,
) -> TestServer {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("brain.db");
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mgr = SqliteConnectionManager::file(&db_path);
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
    brain_server::migration::run_migration(
        &mut pool.get().expect("conn"),
        brain_server::config::DB_MMAP_SIZE_MIB,
    )
    .expect("migration");
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );
    let tok_file = dir.path().join("tokens");
    std::fs::write(&tok_file, lines).expect("token file");
    let token_store = brain_server::auth::TokenStore::from_file(Some(tok_file));
    token_store.reload_parts_from(operators, agent);

    let jwt_middleware_state = Arc::new(JwtMiddlewareState::opaque_for_tests(
        pool.clone(),
        db_path.clone(),
    ));
    let state = Arc::new(brain_server::AppState {
        token_store,
        jwt_middleware_state,
        cors: tower_http::cors::CorsLayer::new(),
        durability: Default::default(),
        loom: Default::default(),
        model,
        registry: brain_server::domain_registry::DomainRegistry::new(pool.clone(), &db_path, false),
        pool,
        db_path,
        connection_tracker: Arc::new(brain_server::http_limit::ConnectionTracker::new()),
        rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
        snapshot: brain_server::integrity::SnapshotState::default(),
        audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
        auth_mode: brain_server::auth::AuthMode::Opaque,
        key_store: brain_server::auth::jwks::KeyStore::default(),
        revocation_cache: Arc::new(brain_server::auth::revocation::RevocationCache::new()),
        jwt_issuer: String::new(),
        jwt_audience: String::new(),
        oidc_config: brain_server::handlers::well_known::OidcConfig::unconfigured(),
        ump_events: tokio::sync::broadcast::channel(brain_server::config::UMP_EVENT_BUFFER).0,
        alert_events: tokio::sync::broadcast::channel(brain_server::config::ALERT_EVENT_BUFFER).0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: brain_server::alert::ChainWatchState::default(),
        concurrency: &brain_server::concurrency::CONCURRENCY,
    });
    TestServer {
        _dir: dir,
        state,
        priv_key: {
            let mut rng = rand::rngs::ThreadRng::default();
            rsa::RsaPrivateKey::new(&mut rng, 2048).expect("keypair")
        },
    }
}

fn twokey_server() -> TestServer {
    build_opaque_token_server(
        &format!("{TWOKEY_OP}\n{TWOKEY_AGENT}\n"),
        vec![TWOKEY_OP.to_string()],
        Some(TWOKEY_AGENT.to_string()),
    )
}

/// One line = today's posture, byte for byte: the bearer authenticates to
/// NO principal — the v1.1 superuser path. Admin routes pass, reads pass,
/// no-token still 401s.
#[tokio::test]
async fn single_token_legacy_posture_unchanged() {
    let srv =
        build_opaque_token_server(&format!("{TWOKEY_OP}\n"), vec![TWOKEY_OP.to_string()], None);
    for (method, path, body) in [
        ("GET", "/stats", ""),
        ("POST", "/recall", r#"{"query":"t"}"#),
        ("POST", "/reindex", "{}"),
        ("POST", "/purge", r#"{"ids":[1]}"#),
    ] {
        let (st, _) = send_body(&srv, Some(TWOKEY_OP), path, method, body).await;
        assert!(
            st != StatusCode::UNAUTHORIZED && st != StatusCode::FORBIDDEN,
            "{method} {path} (single-token operator) must keep the superuser path, got {st}"
        );
    }
    let (st, _) = send_body(&srv, None, "/stats", "GET", "").await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "no token still 401s");
}

/// The operator's per-route outcomes are IDENTICAL with and without the
/// agent line: adding line 2 must not move line 1 anywhere (status AND
/// body compared).
#[tokio::test]
async fn operator_token_behavior_byte_identical() {
    let one =
        build_opaque_token_server(&format!("{TWOKEY_OP}\n"), vec![TWOKEY_OP.to_string()], None);
    let two = twokey_server();
    for (method, path, body) in [
        ("GET", "/stats", ""),
        ("GET", "/audit", ""),
        ("POST", "/recall", r#"{"query":"t"}"#),
        ("POST", "/reindex", "{}"),
        ("POST", "/purge", r#"{"ids":[1]}"#),
        (
            "POST",
            "/dsar",
            r#"{"subject":"m","action":"export","dry_run":true,"subject_exact":true}"#,
        ),
    ] {
        let a = send_body(&one, Some(TWOKEY_OP), path, method, body).await;
        let b = send_body(&two, Some(TWOKEY_OP), path, method, body).await;
        assert_eq!(
            a, b,
            "{method} {path}: the operator's response must not move when the agent line exists"
        );
    }
}

/// The agent bearer authenticates as a SCOPED principal: reads pass, and an
/// Admin route that the None-superuser passes is 403 — the injection proof
/// (a bare middleware pass would carry no principal and inherit the
/// superuser path).
#[tokio::test]
async fn agent_token_authenticates_as_scoped_principal() {
    let srv = twokey_server();
    let (st, _) = send_body(&srv, Some(TWOKEY_AGENT), "/stats", "GET", "").await;
    assert_eq!(st, StatusCode::OK, "agent reads pass");
    // The injection proof: /purge is a HARD-403 Admin route (`/reindex`
    // soft-denies with the legacy 200 shell) — the None superuser passes
    // it; the scoped agent bearer must not.
    let (st, _) = send_body(&srv, Some(TWOKEY_AGENT), "/purge", "POST", r#"{"ids":[1]}"#).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "the agent bearer must be a scoped principal, not the None superuser"
    );
    let (st, _) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        "/recall",
        "POST",
        r#"{"query":"t"}"#,
    )
    .await;
    assert!(
        st != StatusCode::UNAUTHORIZED && st != StatusCode::FORBIDDEN,
        "agent recall passes, got {st}"
    );
}

/// The plan's denial sample: purge, domains, revoke, dsar all 403 — and the
/// purge denial leaves an auth audit row (agent denials are evidenced at
/// the middleware; operator/None denials are untouched).
#[tokio::test]
async fn agent_principal_denied_admin_routes() {
    let srv = twokey_server();
    for (method, path, body) in [
        ("POST", "/purge", r#"{"ids":[1]}"#),
        ("POST", "/domains/move", r#"{"ids":[1],"to":"global"}"#),
        (
            "POST",
            "/ops/agents/revoke",
            r#"{"principal":"someone","reason":"t"}"#,
        ),
        (
            "POST",
            "/dsar",
            r#"{"subject":"m","action":"export","dry_run":true,"subject_exact":true}"#,
        ),
    ] {
        let (st, _) = send_body(&srv, Some(TWOKEY_AGENT), path, method, body).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{method} {path} must 403");
    }
    // The denial evidence: an auth audit row exists for the agent's 403
    // (the middleware hashes the detail — match the digest).
    {
        let detail_hash = brain_server::audit::hash("agent_forbidden");
        let conn = srv.state.pool.get().expect("conn");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE status = 'denied' AND detail_hash = ?1",
                rusqlite::params![detail_hash],
                |r| r.get(0),
            )
            .expect("audit count");
        assert!(n >= 1, "the agent's denied attempts must leave audit rows");
    }
}

/// Review posture: the agent's ingest lands as a pending proposal (202) and
/// the agent CANNOT promote it — the approve gate 403s (the `agent` role
/// carries no `approve` capability).
#[tokio::test]
async fn agent_principal_can_propose_not_promote() {
    static POSTURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _env = POSTURE_LOCK.lock().await;
    let prev = std::env::var("BRAIN_WRITE_POSTURE").ok();
    unsafe { std::env::set_var("BRAIN_WRITE_POSTURE", "review") };

    let srv = twokey_server();
    let (st, body) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        "/ingest",
        "POST",
        r#"{"title":"m","content":"m","domain":"global"}"#,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::ACCEPTED,
        "ingest under review → 202: {body}"
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("202 body is JSON");
    assert_eq!(
        v["status"], "pending",
        "the write landed as a pending proposal"
    );
    let proposal_id = v["proposal_id"].as_i64().expect("proposal_id in body");

    // Promote attempt on the REAL proposal: the approve capability is not
    // the agent role's to spend (403 before any row work).
    let (st, _) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        &format!("/proposals/{proposal_id}/approve"),
        "POST",
        "",
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "the agent cannot promote");

    match prev {
        Some(v) => unsafe { std::env::set_var("BRAIN_WRITE_POSTURE", v) },
        None => unsafe { std::env::remove_var("BRAIN_WRITE_POSTURE") },
    }
}

/// Blackout's kill-switch binds the agent BY PRINCIPAL NAME: the operator
/// (None-superuser) revokes `agent@loopback` through the real route, and
/// the next agent bearer is `401 identity_revoked` at the middleware —
/// while the operator token is untouched.
#[tokio::test]
async fn revoked_agent_principal_denied_everywhere() {
    let srv = twokey_server();
    revoke_via_route(&srv, TWOKEY_OP, "agent@loopback").await;

    let (st, text) = send_body(&srv, Some(TWOKEY_AGENT), "/stats", "GET", "").await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "a revoked agent dies at the middleware"
    );
    assert_eq!(text, revoked_body("identity_revoked"));

    let (st, _) = send_body(&srv, Some(TWOKEY_OP), "/stats", "GET", "").await;
    assert_ne!(st, StatusCode::UNAUTHORIZED, "the operator is untouched");
}

/// THE NET, agent class. Every AUTHZ_GATES row × the AgentLoopback
/// principal: scope-passing rows go through UNLESS the route carries a
/// role gate the `agent` preset lacks (`workflow`/`approve`/`publish`/
/// `purge`/`dsar_export`/`reject`) or the DPO dual gate (the agent HAS
/// roles, so the empty-roles skip never fires) — those speak the denied
/// vocabulary. Admin rows 403 outright.
const ROLE_GATED_FOR_AGENT: &[&str] = &[
    // the `workflow` capability (19 gate sites; the role table has no
    // agent-grantable `workflow` verb — validate() restricts `can` to
    // CAN_ACTIONS, which does not name it)
    "/workflow/runs",
    "/workflow/runs/{id}",
    "/workflow/runs/{id}/state",
    "/workflow/runs/{id}/events",
    "/workflow/runs/{id}/answer",
    "/workflow/runs/{id}/steering",
    "/workflow/runs/{id}/complaint/lifecycle",
    "/workflow/runs/{id}/complaint/remedy",
    "/workflow/runs/{id}/complaint/adr-packet",
    "/workflow/runs/{id}/complaint/ack",
    "/workflow/complaints/ack-sweep",
    "/workflow/outreach/campaign",
    "/workflow/outreach/consent",
    "/workflow/outreach/followup",
    "/workflow/runs/{id}/status-ref",
    "/workflow/runs/{id}/rewind",
    "/workflow/outreach/campaign/{id}",
    "/workflow/valet/due",
    "/workflow/valet/brief",
    "/workflow/valet/consent",
    "/workflow/runs/{id}/handover/offer",
    // NOT workflow-gated (verified: the relay `workflow` sites both live in
    // post_handover_offer; accept/decline + mesh's post_delegation_result
    // carry only the scope gate — they pass for this class on Write)
    // approve / translate / purge / dsar_export — note `reject` is NOT
    // here: the `agent` preset role CAN reject (its own drafts), so the
    // reject route passes the role gate for this class; nor is
    // `/kcs/articles/{id}/publish` — its retract branch carries only the
    // Write scope (the 409 there is pass-path route vocabulary)
    "/proposals/{id}/approve",
    "/kcs/articles/{id}/approve",
    "/kcs/translate",
    "/purge",
    "/dsar",
    "/clients/{name}/dsar",
    // the DPO dual gate (require_dpo_role): binds because the agent
    // principal CARRIES roles — the empty-roles skip never fires
    "/legal-hold",
    "/legal-hold/{id}/release",
    "/legal-holds",
    "/breach",
    "/breach/{id}/event",
    "/breach/{id}/close",
    "/breaches",
    "/breaches/{id}",
    "/workflow/scoreboard",
    "/workflow/calibration/sign",
];

#[tokio::test]
async fn authz_matrix_agent_loopback_class() {
    let srv = twokey_server();
    for (template, path, method, body) in rows() {
        let Some((_, action)) = AUTHZ_GATES.iter().find(|(t, _)| *t == template) else {
            continue;
        };
        if *action == "public" {
            continue;
        }
        // status-only `send` (the sibling loops' pattern): the SSE rows
        // (`/events`, `/ump/subscribe`) answer the handshake 200 and stream
        // forever — `to_bytes` would wait on a body that never ends.
        let st = send(&srv, Some(TWOKEY_AGENT), &path, method, body).await;
        let scope_pass = matches!(*action, "Read" | "Write" | "Traverse");
        let role_denied = ROLE_GATED_FOR_AGENT.contains(&template);
        let pass = scope_pass && !role_denied && !LAYOUT_CONDITIONAL.contains(&template);
        if pass {
            assert!(
                st != StatusCode::UNAUTHORIZED && st != StatusCode::FORBIDDEN,
                "{method} {template} (agent) must pass the gate, got {st}"
            );
        } else if SOFT_DENY_LEGACY.contains(&template) {
            assert_eq!(
                st,
                StatusCode::OK,
                "{method} {template} (agent) soft-denies with the legacy 200 shape"
            );
        } else if PRE_GATE_400.contains(&template) {
            assert_eq!(
                st,
                StatusCode::BAD_REQUEST,
                "{method} {template} (agent) pre-gate 400s"
            );
        } else if PRE_GATE_404.contains(&template) {
            assert!(
                st == StatusCode::NOT_FOUND || st == StatusCode::FORBIDDEN,
                "{method} {template} (agent) must pre-gate 404 or 403, got {st}"
            );
        } else if LAYOUT_CONDITIONAL.contains(&template) {
            assert_eq!(
                st,
                StatusCode::FORBIDDEN,
                "{method} {template} (agent) is Admin-gated in shim mode"
            );
        } else {
            assert_eq!(
                st,
                StatusCode::FORBIDDEN,
                "{method} {template} (agent) must 403"
            );
        }
    }
}

// ── Twokeys M2: observability scoping (X-A5) ────────────────────────────

/// The full /health/db body is Admin-on-global; a Read principal gets the
/// reduced public shape ({status, version, db_ok}) — no model, no system,
/// no pool, no DPO/durability/WAL detail.
#[tokio::test]
async fn health_db_admin_full_read_reduced() {
    let srv = build_server();
    let reader = mint(
        &srv,
        "m2-hdb-r",
        "user:m2r",
        "team-a",
        &["read:team-a/*"],
        &[],
    );
    let admin = mint(
        &srv,
        "m2-hdb-a",
        "user:m2a",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );

    let (st, body) = send_body(&srv, Some(&admin), "/health/db", "GET", "").await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        body.contains("\"model\"") && body.contains("\"system\""),
        "admin sees the full body"
    );

    let (st, body) = send_body(&srv, Some(&reader), "/health/db", "GET", "").await;
    assert_eq!(
        st,
        StatusCode::OK,
        "a Read principal still gets the reduced probe"
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("reduced body is JSON");
    let obj = v.as_object().expect("object");
    assert_eq!(
        obj.keys().collect::<Vec<_>>(),
        vec!["db_ok", "status", "version"],
        "the reduced shape is exactly status/version/db_ok (serde sorts keys)"
    );
}

/// Public /health is untouched: no auth, minimal shape.
#[tokio::test]
async fn public_health_unchanged() {
    let srv = build_server();
    let (st, body) = send_body(&srv, None, "/health", "GET", "").await;
    assert_eq!(st, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v,
        serde_json::json!({"status": "ok", "version": v["version"]})
    );
}

/// The admin scrape still carries real per-domain labels (shim mode:
/// `global`), and never the collapse label.
#[tokio::test]
async fn admin_sees_domain_labels() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "m2-met-a",
        "user:m2ma",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );
    let (st, body) = send_body(&srv, Some(&admin), "/metrics", "GET", "").await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        body.contains("brain_pool_in_use{domain=\"global\"}"),
        "admin sees the domain name: {body}"
    );
    assert!(
        !body.contains("{domain=\"other\"}"),
        "no collapse series for admin"
    );
}

/// Pure pin over the scoping rule at the unit seam (shim-mode /metrics can
/// only ever enumerate `global`, which every /metrics reader is gated to
/// read — the cross-tenant collapse is witnessable where the rule lives).
/// A tenant reader (read:team-a/*) sees `global` named (its scope grants
/// the shared pool) but foreign named domains collapse to `other`; the
/// None superuser sees every name.
#[test]
fn tenant_reader_sees_other_not_domain_names() {
    use brain_server::server::router::core::scoped_domain_label;
    let reader = Some(brain_server::auth::Principal {
        sub: "user:r".into(),
        tenant: "team-a".into(),
        scopes: vec![brain_server::auth::Scope::parse("read:team-a/*").unwrap()],
        jti: String::new(),
        roles: vec![],
        manages: vec![],
        kind: brain_server::auth::PrincipalKind::Jwt,
    });
    assert_eq!(
        scoped_domain_label(&reader, "global"),
        "global",
        "the shared pool stays named for a wildcard-team reader"
    );
    assert_eq!(
        scoped_domain_label(&reader, "acme-us"),
        "other",
        "a foreign tenant domain collapses: count visible, name hidden"
    );
    assert_eq!(
        scoped_domain_label(&None, "acme-us"),
        "acme-us",
        "the None superuser sees every name"
    );
}
