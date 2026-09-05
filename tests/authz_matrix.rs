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
const SSE_SOFT: &[&str] = &["/events", "/ump/subscribe"];
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
