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
use brain_server::pool::SqliteConnectionManager;
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
        // The broad matrix role is created through the same typed role
        // contract as the public API. It intentionally contains every
        // currently supported capability, but never the separate `publish`
        // gap that this round does not absorb.
        let conn = pool.get().expect("conn");
        let role = brain_server::role::Role {
            name: "matrix-role".to_string(),
            description: Some("authz-matrix fixture".to_string()),
            scopes: vec![
                "private".to_string(),
                "domain".to_string(),
                "team".to_string(),
            ],
            owner_filter: "all".to_string(),
            can: brain_server::role::CAN_ACTIONS
                .iter()
                .map(|capability| (*capability).to_string())
                .collect(),
            panels_default: None,
            panels_hidden: None,
            tools_allowed: Some(vec!["*".to_string()]),
        };
        brain_server::role::validate(&role).expect("matrix role must validate");
        brain_server::role::upsert(&conn, &role).expect("seed matrix role");
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
        jwt_azp: None,
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
        azp: None,
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
            .replace("{invite_id}", "1")
            .replace("{run_id}", "1")
            .replace("{model_ref}", "rules-reference@1.0.0");
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
            // `domain` is a required query param here too: the surface is
            // domain-scoped because a binding is one tenant's standing
            // authority, and the extractor refuses an absent one. Carrying a
            // real domain is what makes this row exercise the CROSS-TENANT
            // refusal (a team-b principal asking for another tenant) instead
            // of the param validation. `global` is the one name every
            // principal in this fixture's scope set admits.
            "/workflow/delivery/bindings" => {
                "/workflow/delivery/bindings?domain=global".to_string()
            }
            // Same required-query-param shape on the two shared-path census
            // reads (the scan drives the LAST registration, which is the GET):
            // carrying a real domain exercises the cross-tenant refusal
            // instead of the param validation.
            "/workflow/delivery/releases" => {
                "/workflow/delivery/releases?domain=global".to_string()
            }
            "/workflow/delivery/runs" => "/workflow/delivery/runs?domain=global".to_string(),
            // Same required-query-param shape on the derived read model: it is
            // domain-scoped (a release row resolves to one tenant), so carrying
            // a real domain exercises the cross-tenant refusal instead of the
            // param validation. `window` is OPTIONAL and bounded in the core,
            // so no arm is needed for it.
            "/workflow/delivery/outcomes" => {
                "/workflow/delivery/outcomes?domain=global".to_string()
            }
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
            // the launch body is complete-shaped but the run is absent:
            // the handler 404s BEFORE any provider contact (PRE_GATE_404).
            "/workflow/cases/{id}/gdl" => (
                "POST",
                r#"{"ticket":"m","base_url":"https://provider.invalid/v1/stream","model":"m","secret_file":"/nonexistent-secret"}"#,
            ),
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
            "/workflow/runs/{id}/handoff/decision" => {
                ("POST", r#"{"transition":"delivered","decision_ref":"m"}"#)
            }
            // The account surfaces: the listing row drives the DPO-gated
            // GET side (the gate row's stricter handler); the {id} routes
            // resolve the account FIRST, so id 1 (absent in this fixture)
            // answers the probe-blind 404 (PRE_GATE_404).
            "/accounts" => ("GET", ""),
            "/accounts/{id}" => ("GET", ""),
            "/accounts/{id}/pipeline" => ("POST", r#"{"stage":"qualified","decision_ref":"m"}"#),
            "/accounts/{id}/requests" => ("GET", ""),
            "/accounts/{id}/requests/{run_id}/link" => ("POST", ""),
            // The κ bench: complete-shaped label body so the Json
            // extractor clears and the gates decide; the digest matches
            // no seeded tuple, so the pass classes answer the handler's
            // probe-blind 404 (not an authz error).
            "/workflow/kappa/queue" => ("GET", ""),
            "/workflow/kappa/labels" => ("POST", r#"{"digest":"aa","label":"agree","run_id":1}"#),
            "/workflow/kappa/report" => ("GET", ""),
            // The agreement-labelling path: the same posture as the bench.
            // The label body is complete-shaped so the Json extractor clears
            // and the GATES decide — the subject matches no seeded trace row,
            // so a principal that clears every gate reaches the handler's own
            // probe-blind 404 rather than a decode failure. (A `{}` body here
            // would answer 422 on the extractor and the authz assertion would
            // never be reached: the refusal must come from the gate.)
            "/workflow/agreement/queue" => ("GET", ""),
            "/workflow/agreement/labels" => (
                "POST",
                r#"{"run_id":1,"subject_id":"trc_absent","verdict":"confirmed"}"#,
            ),
            "/workflow/agreement/report" => ("GET", ""),
            // The decision-runs listing row drives the DPO-gated GET side
            // (the gate row's stricter handler, the /accounts convention);
            // the execute POST is handler-source-pinned.
            "/workflow/decision-runs" => ("GET", ""),
            // The bodies only clear the typed extractor: the config loader
            // and the probe-blind run resolution refuse downstream, which
            // is a pass-path status for the classes that clear the gate.
            "/workflow/decision-runs/{id}" => ("GET", ""),
            "/workflow/decision-runs/{id}/replay-diff" => (
                "POST",
                r#"{"config":{},"rules_config":{},"mode":"deterministic","request_id":"m","question_ids":["m"],"query":"m"}"#,
            ),
            "/workflow/model-registry/register" => ("POST", r#"{"kind":"deterministic-rules"}"#),
            // The delivery loop's four run writes. The bodies only clear the
            // typed extractor so the AUTHORIZATION gate is what answers: an
            // empty body would 422 in the extractor and never reach the
            // scope or role check, which is the whole point of these rows.
            // The shared paths drive the READ side: the guard table's FIRST
            // row for each is the Read row (see route_guards.rs), so the
            // method must match it. The POST side is pinned by the handler
            // source scan and the round's own battery.
            "/workflow/delivery/runs" => ("GET", ""),
            "/workflow/delivery/runs/{id}/advance" => {
                ("POST", r#"{"expected_revision":0,"to_phase":"design"}"#)
            }
            "/workflow/delivery/runs/{id}/answer" => {
                ("POST", r#"{"expected_revision":0,"answer":"matrix"}"#)
            }
            "/workflow/delivery/runs/{id}/gates" => ("POST", r#"{"to_phase":"design"}"#),
            // the release family's writes. The bodies only clear the typed
            // extractor, as on the run writes above; the release/run resolution
            // refuses downstream for the classes that clear the gate.
            "/workflow/delivery/releases/{id}/approve" => ("POST", r#"{"scope":"promote"}"#),
            "/workflow/delivery/releases/{id}/promote" => ("POST", r#"{"confirm":false}"#),
            "/workflow/delivery/due" => ("POST", r#"{"domain":"global"}"#),
            // the derived read model drives the GET side explicitly (the
            // prompt's law): no body, the window defaults in the core.
            "/workflow/delivery/outcomes" => ("GET", ""),
            "/workflow/runs/{id}/back-referral/return" => (
                "POST",
                r#"{"contract_key":"m","report":{},"decision_ref":"m"}"#,
            ),
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
/// Rows whose role gate asks for an APPROVAL capability (`approve`/`reject`).
/// A principal that carries no role at all is refused on these even when its
/// scope grants Write: disposing of a proposal is a role act, and the matrix's
/// `write` class is deliberately role-less. Every entry here is checked
/// against `AUTHZ_GATES` by `approval_role_rows_are_real_write_rows`, so a
/// renamed route fails loudly instead of quietly losing its cell.
const APPROVAL_ROLE_ROWS: &[&str] = &[
    "/proposals/{id}/approve",
    "/proposals/{id}/reject",
    "/kcs/articles/{id}/approve",
    "/workflow/runs/{id}/answer",
    "/workflow/runs/{id}/steering",
    "/workflow/runs/{id}/status-ref",
    "/workflow/runs/{id}/rewind",
];
/// Row whose pre-gate body validation answers 400 before the gate runs
/// (the mount body must be a valid bridge bundle; the gate itself is
/// admin + audited).
const PRE_GATE_400: &[&str] = &["/workflow/plugins/mount"];
/// Rows whose TYPED body is deserialized by the extractor before the handler
/// body runs, so a request whose body does not match the type is rejected
/// before authorization is reached. The rejection is 422 from the extractor,
/// not 400 from the handler — which is why this is its own row rather than an
/// addition to the 400 list. The gate still runs for every well-formed body,
/// and the denied cell is asserted against both codes.
const PRE_GATE_BODY_REJECT: &[&str] = &["/workflow/claim-schemas", "/workflow/claims"];
/// Rows whose handler resolves the row FIRST and authorizes second: a
/// nonexistent id answers 404 before the gate — the gate still runs for
/// existing rows; the matrix accepts the pre-gate 404 in denied cells.
const PRE_GATE_404: &[&str] = &[
    "/workflow/runs/{id}",
    "/workflow/runs/{id}/steps",
    "/workflow/runs/{id}/report",
    "/workflow/runs/{id}/steering",
    "/workflow/runs/{id}/suggestions",
    "/workflow/runs/{id}/state",
    "/workflow/runs/{id}/events",
    "/workflow/runs/{id}/context",
    "/workflow/runs/{id}/answer",
    "/workflow/runs/{id}/status-ref",
    "/workflow/runs/{id}/handoff",
    "/workflow/cases/{id}/gdl",
    "/workflow/runs/{id}/rewind",
    "/workflow/runs/{id}/complaint/lifecycle",
    "/workflow/runs/{id}/complaint/remedy",
    "/workflow/runs/{id}/complaint/adr-packet",
    "/workflow/runs/{id}/complaint/ack",
    "/workflow/runs/{id}/outreach/followup",
    "/workflow/runs/{id}/handover/offer",
    "/workflow/runs/{id}/handover/{offer_id}/accept",
    "/workflow/runs/{id}/handover/{offer_id}/decline",
    "/workflow/runs/{id}/handoff/decision",
    "/workflow/runs/{id}/back-referral/return",
    "/workflow/runs/{id}/notes",
    "/workflow/runs/{id}/notes/{invite_id}/accept",
    "/workflow/runs/{id}/delegations",
    "/workflow/runs/{id}/delegations/{delegation_id}/result",
    "/kcs/articles/{id}/approve",
    "/kcs/articles/{id}/publish",
    // The delivery loop's three id-scoped writes. These resolve the RUN's
    // domain before any gate — which is the contract ("Write on the run's
    // domain"), not an ordering slip: the domain is unknowable without the
    // run, and authorizing against anything else would check the wrong
    // domain. So an absent run is the probe-blind 404 here, exactly as on
    // every other run-resolved route, and the class matrix accepts 404-or-403.
    // The 403-on-role proof is the seeded behavioural test below, which opens
    // a real run first — a role gate proven only against an absent row would
    // be a gate proven about nothing.
    "/workflow/delivery/runs/{id}/advance",
    "/workflow/delivery/runs/{id}/answer",
    "/workflow/delivery/runs/{id}/gates",
    // ...and the line's THREE id-scoped READS. They resolve the run before any
    // gate exactly as the writes do, so an absent run is the same probe-blind
    // 404 — and that is the whole point of listing them here: a read that
    // leaked existence (403 for one class, 404 for another) would be an oracle
    // telling a principal which runs exist.
    "/workflow/delivery/runs/{id}/attestations",
    "/workflow/delivery/runs/{id}/replay-verify",
    "/workflow/delivery/runs/{id}/trace",
    // ...and the release round's id-scoped surfaces, same contract: the two
    // release writes resolve the release -> run before any gate, and the two
    // id-scoped census reads resolve the run first.
    "/workflow/delivery/releases/{id}/approve",
    "/workflow/delivery/releases/{id}/promote",
    "/workflow/delivery/runs/{id}",
    "/workflow/delivery/runs/{id}/steps",
    // The account {id} routes resolve the account BEFORE any gate: an
    // absent id (and a non-account id — the same answer) is the probe-blind
    // 404.
    "/accounts/{id}",
    "/accounts/{id}/pipeline",
    "/accounts/{id}/requests",
    "/accounts/{id}/requests/{run_id}/link",
    // The decision-run surfaces resolve the run (or the stored trace's
    // run) BEFORE any gate: absent and foreign runs answer the SAME
    // probe-blind 404 (the decision-surface pin). The shared base path's
    // POST resolves the body's run first; its listing GET never 404s
    // pre-gate, so the shared row stays honest for both methods.
    "/workflow/decision-runs",
    "/workflow/decision-runs/{id}",
    "/workflow/decision-runs/{id}/replay-diff",
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
    // the DPO-gated listing on an empty corpus is a literal 200
    "/accounts",
    // the κ bench reads on an empty corpus are literal 200s
    "/workflow/kappa/queue",
    "/workflow/kappa/report",
    // the agreement-labelling path's reads are literal 200s on an empty corpus
    "/workflow/agreement/queue",
    "/workflow/agreement/report",
    // the evaluation listing has an empty-corpus 200 anchor
    "/workflow/decision-evals",
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
/// The approval-role list is hand-maintained, so it gets its own pin: every
/// entry must be a real Write row in the gate table (a renamed route fails
/// here rather than silently dropping its class cell), and the list must not
/// be empty — an empty list would make the write-cell branch unreachable and
/// the self-approval gap return with a green suite.
#[test]
fn approval_role_rows_are_real_write_rows() {
    assert!(
        !APPROVAL_ROLE_ROWS.is_empty(),
        "the approval-role row list must not empty out"
    );
    for template in APPROVAL_ROLE_ROWS {
        let action = AUTHZ_GATES
            .iter()
            .find(|(t, _)| t == template)
            .map(|(_, a)| *a)
            .unwrap_or_else(|| panic!("{template} is not an AUTHZ_GATES row"));
        assert_eq!(
            action, "Write",
            "{template} is pinned as an approval row but the table calls it {action}"
        );
    }
}

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
            } else if PRE_GATE_BODY_REJECT.contains(&template) {
                assert!(
                    st == StatusCode::UNPROCESSABLE_ENTITY || st == StatusCode::FORBIDDEN,
                    "{method} {template} ({label}) must 422 on a body the typed extractor \
                     rejects, or 403 once the body parses, got {st}"
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
        if template == "/workflow/cases/{id}/gdl" {
            assert_eq!(
                st,
                StatusCode::FORBIDDEN,
                "{method} {template} (write) is the GDL role-gated exception"
            );
        } else if can_write && APPROVAL_ROLE_ROWS.contains(&template) {
            // Disposing of a proposal is a role act, so the ROLE-LESS write
            // class is refused here even though its scope grants Write. This is
            // the row that proves the rule bites: without it a claim-less token
            // proposes and then approves its own memory.
            denied(st, "write (role-less approve row)");
        } else if can_write && LAYOUT_CONDITIONAL.contains(&template) {
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
        jwt_azp: None,
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

/// Create a fresh troubleshoot run through the real route for the GDL
/// authorization pins below. The fixture role carries `workflow`; the
/// principal under test is deliberately different.
async fn fresh_gdl_run(srv: &TestServer, admin: &str, domain: &str) -> i64 {
    let (status, body) = send_body(
        srv,
        Some(admin),
        "/workflow/runs",
        "POST",
        &serde_json::json!({
            "domain": domain,
            "kind": "troubleshoot",
            "state_json": "{}"
        })
        .to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "fresh run creation failed: {body}");
    serde_json::from_str::<serde_json::Value>(&body).expect("run JSON")["run_id"]
        .as_i64()
        .expect("run id")
}

#[tokio::test]
async fn gdl_workflow_role_is_grantable_through_role_api() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "gdl-workflow-admin",
        "user:gdl-workflow-admin",
        "team-a",
        &["admin:*/*"],
        &["admin"],
    );
    let (status, body) = send_body(
        &srv,
        Some(&admin),
        "/roles/workflow-operator",
        "POST",
        r#"{"description":"Workflow operator","scopes":[],"owner_filter":"self","can":["workflow"],"panels_default":null,"panels_hidden":null,"tools_allowed":[]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "role API response: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).expect("role JSON");
    assert_eq!(value["name"], "workflow-operator");
    assert_eq!(value["can"], serde_json::json!(["workflow"]));
    assert!(
        !value["can"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == "publish")
    );
}

#[tokio::test]
async fn gdl_workflow_role_reaches_provider_config_handling() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "gdl-workflow-config-admin",
        "user:gdl-workflow-config-admin",
        "team-a",
        &["admin:*/*"],
        &["admin", "workflow-operator"],
    );
    let (role_status, role_body) = send_body(
        &srv,
        Some(&admin),
        "/roles/workflow-operator",
        "POST",
        r#"{"description":"Workflow operator","scopes":[],"owner_filter":"self","can":["workflow"],"panels_default":null,"panels_hidden":null,"tools_allowed":[]}"#,
    )
    .await;
    assert_eq!(role_status, StatusCode::OK, "role setup: {role_body}");
    let run_id = fresh_gdl_run(&srv, &admin, "acme").await;
    let operator = mint(
        &srv,
        "gdl-workflow-config-operator",
        "user:gdl-workflow-config-operator",
        "team-a",
        &["write:team-a/acme"],
        &["workflow-operator"],
    );
    let (status, body) = send_body(
        &srv,
        Some(&operator),
        &format!("/workflow/cases/{run_id}/gdl"),
        "POST",
        r#"{"ticket":"workflow role reaches configuration"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).expect("error JSON");
    assert_eq!(
        value["error"]["code"], "provider_config_invalid",
        "body: {body}"
    );
}

#[tokio::test]
async fn gdl_agent_role_remains_denied() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "gdl-agent-admin",
        "user:gdl-agent-admin",
        "team-a",
        &["admin:*/*"],
        &["matrix-role"],
    );
    let run_id = fresh_gdl_run(&srv, &admin, "acme").await;
    let agent = mint(
        &srv,
        "gdl-agent-role",
        "user:gdl-agent-role",
        "team-a",
        &["write:team-a/*"],
        &["agent"],
    );
    let (status, body) = send_body(
        &srv,
        Some(&agent),
        &format!("/workflow/cases/{run_id}/gdl"),
        "POST",
        r#"{"ticket":"agent role remains denied"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert!(!body.contains("provider_config_invalid"), "body: {body}");
}

/// A JWT with the required Write scope but no role claim must not reach the
/// server-owned provider configuration or secret reader.
#[tokio::test]
async fn gdl_route_rejects_role_less_write_principal() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "gdl-admin-role-less",
        "user:gdl-admin-role-less",
        "team-a",
        &["admin:*/*"],
        &["matrix-role"],
    );
    let run_id = fresh_gdl_run(&srv, &admin, "acme").await;
    let roleless = mint(
        &srv,
        "gdl-roleless",
        "user:gdl-roleless",
        "team-a",
        &["write:team-a/*"],
        &[],
    );
    let (status, body) = send_body(
        &srv,
        Some(&roleless),
        &format!("/workflow/cases/{run_id}/gdl"),
        "POST",
        r#"{"ticket":"role gate before secret"}"#,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a role-less Write principal must be denied before profile/secret work: {body}"
    );
    assert!(
        !body.contains("secret_file"),
        "role refusal stays operator-safe"
    );
}

/// An asserted role name that is absent from the server role store contributes
/// no capability; it must fail closed rather than falling through to scopes.
#[tokio::test]
async fn gdl_route_rejects_unknown_role() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "gdl-admin-unknown-role",
        "user:gdl-admin-unknown-role",
        "team-a",
        &["admin:*/*"],
        &["matrix-role"],
    );
    let run_id = fresh_gdl_run(&srv, &admin, "acme").await;
    let unknown_role = mint(
        &srv,
        "gdl-unknown-role",
        "user:gdl-unknown-role",
        "team-a",
        &["write:team-a/*"],
        &["role-that-does-not-exist"],
    );
    let (status, body) = send_body(
        &srv,
        Some(&unknown_role),
        &format!("/workflow/cases/{run_id}/gdl"),
        "POST",
        r#"{"ticket":"unknown role before secret"}"#,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an unknown role must not authorize GDL: {body}"
    );
}

/// The run's actual domain, not a request-selected or global scope, is the
/// authorization input. A token scoped to a different domain is refused.
#[tokio::test]
async fn gdl_route_authorizes_the_actual_run_domain() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "gdl-admin-domain",
        "user:gdl-admin-domain",
        "team-a",
        &["admin:*/*"],
        &["matrix-role"],
    );
    let run_id = fresh_gdl_run(&srv, &admin, "forbidden-domain").await;
    let wrong_domain = mint(
        &srv,
        "gdl-wrong-domain",
        "user:gdl-wrong-domain",
        "team-a",
        &["write:team-a/allowed-domain"],
        &["matrix-role"],
    );
    let (status, body) = send_body(
        &srv,
        Some(&wrong_domain),
        &format!("/workflow/cases/{run_id}/gdl"),
        "POST",
        r#"{"ticket":"wrong domain before secret"}"#,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "GDL must authorize the run's actual domain: {body}"
    );
}

/// Authorization and profile refusal must both precede the secret reader. A
/// role-less principal is denied even when a profile is configured; an
/// authorized operator with no profile receives only the stable config code.
#[tokio::test]
async fn gdl_provider_never_reads_secret_before_authorization_and_config_refusal() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "gdl-admin-order",
        "user:gdl-admin-order",
        "team-a",
        &["admin:*/*"],
        &["matrix-role"],
    );
    let run_id = fresh_gdl_run(&srv, &admin, "acme").await;
    let roleless = mint(
        &srv,
        "gdl-order-roleless",
        "user:gdl-order-roleless",
        "team-a",
        &["write:team-a/*"],
        &[],
    );
    let (status, body) = send_body(
        &srv,
        Some(&roleless),
        &format!("/workflow/cases/{run_id}/gdl"),
        "POST",
        r#"{"ticket":"authorization before secret"}"#,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "role refusal came first: {body}"
    );
    assert!(!body.contains("secret"), "no secret diagnostic escaped");

    let (status, body) = send_body(
        &srv,
        Some(&admin),
        &format!("/workflow/cases/{run_id}/gdl"),
        "POST",
        r#"{"ticket":"configuration before secret"}"#,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "config refusal came first: {body}"
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("error JSON");
    assert_eq!(
        value["error"]["code"], "provider_config_invalid",
        "body: {body}"
    );
    assert!(
        !body.contains("secret_file"),
        "config refusal stays operator-safe"
    );
}

/// The case-launch boundary maps authority at the authenticated border:
/// the AGENT bearer is refused BY KIND (403 — agents do not self-launch
/// cases) even where a scope check would pass, while the OPERATOR on the
/// same absent run passes the gate and reaches the probe-blind 404 —
/// both BEFORE any provider contact (no provider config is even read).
#[tokio::test]
async fn gdl_route_rejects_agent_before_secret_read_and_dns() {
    let srv = twokey_server();
    let launch = r#"{"ticket":"m","base_url":"https://provider.invalid/v1/stream","model":"m","secret_file":"/nonexistent-secret"}"#;
    let (st, _) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        "/workflow/cases/1/gdl",
        "POST",
        launch,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "the agent bearer must be refused by kind, before the run lookup"
    );
    // The operator on the SAME absent run: gate passes → probe-blind 404.
    let (st, _) = send_body(
        &srv,
        Some(TWOKEY_OP),
        "/workflow/cases/1/gdl",
        "POST",
        launch,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "the operator passes the gate and reaches the run lookup (404 on a missing run)"
    );
}

/// A Blackout-revoked agent identity dies at the MIDDLEWARE on the launch
/// route too — `401 identity_revoked`, before any route logic (and so
/// before any provider contact). The route-level refusal (403) is the
/// live agent's posture; revocation removes the identity entirely.
#[tokio::test]
async fn gdl_route_rejects_revoked_principal() {
    let srv = twokey_server();
    revoke_via_route(&srv, TWOKEY_OP, "agent@loopback").await;
    let (st, text) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        "/workflow/cases/1/gdl",
        "POST",
        r#"{"ticket":"m","base_url":"https://provider.invalid/v1/stream","model":"m","secret_file":"/nonexistent-secret"}"#,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "a revoked agent dies at the middleware on the launch route"
    );
    assert_eq!(text, revoked_body("identity_revoked"));
}

/// The operator decision surface is role-gated end to end: the OPERATOR
/// opens a fixture run in the agent's writable domain and lands its
/// decision; the AGENT bearer (scopes fine, `workflow` role absent) is
/// refused 403 by the ROLE gate on the SAME run; on an ABSENT run the
/// agent reads the same probe-blind 404 the operator gets (no
/// disclosure); and a REVOKED agent identity dies at the middleware with
/// `401 identity_revoked` — the decision-ref law never even gets a body
/// to screen.
#[tokio::test]
async fn decision_routes_role_gated_agent_and_revoked_denied() {
    let srv = twokey_server();
    let (st, body) = send_body(
        &srv,
        Some(TWOKEY_OP),
        "/workflow/runs",
        "POST",
        r#"{"domain":"global","kind":"interview","state_json":"{}"}"#,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "the operator opens the fixture run");
    let run_id = serde_json::from_str::<serde_json::Value>(&body).expect("open reply")["run_id"]
        .as_i64()
        .expect("run id") as i64;
    let decision_path = format!("/workflow/runs/{run_id}/handoff/decision");
    let decision_body = r#"{"transition":"delivered","decision_ref":"m"}"#;

    // The agent's scope admits the domain, the ROLE gate refuses.
    let (st, _) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        &decision_path,
        "POST",
        decision_body,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "the agent bearer is refused by the workflow role gate"
    );

    // The operator's decision lands through the composed app.
    let (st, body) = send_body(&srv, Some(TWOKEY_OP), &decision_path, "POST", decision_body).await;
    assert_eq!(st, StatusCode::OK, "the operator's decision lands");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).expect("receipt")["audited"],
        serde_json::json!(true)
    );

    // Probe-blind: on an absent run the agent reads the operator's 404.
    let (st, _) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        "/workflow/runs/999999/handoff/decision",
        "POST",
        decision_body,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "absent runs probe-blind 404");

    // Revocation removes the identity entirely.
    revoke_via_route(&srv, TWOKEY_OP, "agent@loopback").await;
    let (st, text) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        &decision_path,
        "POST",
        decision_body,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "a revoked agent dies at the middleware on the decision route"
    );
    assert_eq!(text, revoked_body("identity_revoked"));
}

/// Probe-blind 404: the OPERATOR (every gate passes) on a nonexistent run
/// id reaches the run lookup and answers the same 404 an absent row always
/// speaks — the decision routes disclose nothing about runs the caller
/// cannot see.
#[tokio::test]
async fn probe_blind_404_on_foreign_run() {
    let srv = twokey_server();
    for (path, body) in [
        (
            "/workflow/runs/999999/handoff/decision",
            r#"{"transition":"delivered","decision_ref":"m"}"#,
        ),
        (
            "/workflow/runs/999999/back-referral/return",
            r#"{"contract_key":"m","report":{},"decision_ref":"m"}"#,
        ),
    ] {
        let (st, _) = send_body(&srv, Some(TWOKEY_OP), path, "POST", body).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{path} answers probe-blind 404");
    }
}

/// THE NET, agent class. Every AUTHZ_GATES row × the AgentLoopback
/// principal: scope-passing rows go through UNLESS the route carries a
/// role gate the `agent` preset lacks (`workflow`/`approve`/`publish`/
/// `purge`/`dsar_export`/`reject`) or the DPO dual gate (the agent HAS
/// roles, so the empty-roles skip never fires) — those speak the denied
/// vocabulary. Admin rows 403 outright.
const ROLE_GATED_FOR_AGENT: &[&str] = &[
    // the `workflow` capability (19 gate sites). R47 CORRECTION: the reason
    // the agent is refused here is NOT that `workflow` is ungrantable —
    // CAN_ACTIONS does name it and the `workflow-operator` preset holds it
    // (the old comment here said otherwise, and was wrong). The agent is
    // refused because its own preset role holds can:["read","write","reject"],
    // which does not include `workflow`. Same verdict, correct reason.
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
    // the case-launch boundary refuses this class BY KIND at the
    // handler (agents do not self-launch cases) — before the run
    // lookup and any provider contact.
    "/workflow/cases/{id}/gdl",
    "/workflow/outreach/campaign/{id}",
    "/workflow/valet/due",
    "/workflow/valet/brief",
    "/workflow/valet/consent",
    "/workflow/runs/{id}/handover/offer",
    // The StewardOS account surfaces: writes and per-account reads demand
    // the `workflow` role; the listing carries the DPO dual gate (the
    // agent HAS roles, so the dual gate binds).
    "/accounts",
    "/accounts/{id}",
    "/accounts/{id}/pipeline",
    "/accounts/{id}/requests",
    "/accounts/{id}/requests/{run_id}/link",
    // The authority-bindings read. The sibling delivery reads are NOT listed
    // here because they pre-gate 404 on an absent run, so the agent cell never
    // reaches their role gate; this one is domain-scoped rather than
    // run-scoped, so it really is reached. Same `workflow` capability, same
    // reason: the role table has no agent-grantable `workflow` verb.
    "/workflow/delivery/bindings",
    // The derived read model. Domain-scoped rather than run-scoped, so the
    // agent cell really reaches it; same `workflow` capability, same reason
    // as the bindings read: the role table has no agent-grantable `workflow`
    // verb. NOT in PRE_GATE_404 — there is no id to resolve pre-gate; the
    // domain gate answers first, exactly like the bindings read.
    "/workflow/delivery/outcomes",
    // The κ bench: queue + capture demand the `calibrate` capability (the
    // agent preset carries read/write/reject only); the report carries
    // the DPO dual gate (the agent HAS roles, so the dual gate binds).
    "/workflow/kappa/queue",
    "/workflow/kappa/labels",
    "/workflow/kappa/report",
    // The agreement-labelling path: same posture as the bench — queue +
    // capture demand the `calibrate` capability, the report carries the DPO
    // dual gate, so the agent class is refused on all three paths.
    "/workflow/agreement/queue",
    "/workflow/agreement/labels",
    "/workflow/agreement/report",
    // The decision-run surfaces: execute/read/replay demand the
    // `workflow` role on top of the scope gate, and the listing carries
    // the DPO dual gate (the agent HAS roles, so the dual gate binds) —
    // the agent class is refused on all three paths.
    "/workflow/decision-runs",
    "/workflow/decision-runs/{id}",
    "/workflow/decision-runs/{id}/replay-diff",
    "/workflow/decision-evals",
    "/workflow/decision-evals/{id}",
    // The create loop, all five paths. Every surface here demands the
    // `workflow` role on top of the scope gate, so the agent class is refused
    // 403 on all of them: its own preset holds can:["read","write","reject"],
    // and that set does not include `workflow`.
    //
    // The reads are listed for a real reason rather than for coverage. The
    // screen read shows a claim that has NOT been ratified yet, and a model
    // that can read the review surface can learn what the human is about to
    // look at — which is a leak of the review process itself. The schema route
    // is the human-artifact act: the row an agent must never reach even if a
    // future preset handed it the capability.
    "/workflow/claims",
    "/workflow/claims/{id}",
    "/workflow/claims/{id}/verify",
    "/workflow/claims/{id}/promote",
    "/workflow/claim-schemas",
    // The operator decision surfaces: both handlers demand the `workflow`
    // role (the HITL law's gate shape) on top of the Write scope, so the
    // agent class is refused 403 exactly like the offer route.
    "/workflow/runs/{id}/handoff/decision",
    "/workflow/runs/{id}/back-referral/return",
    // The delivery loop's four run writes: each demands the `workflow` role on
    // top of Write on the run's own domain, so the agent class is refused 403
    // on all four. The gate-evaluation route is included deliberately — a
    // dispositions an agent could read would hand back the run's phase and
    // tier to the class that must not be able to drive them.
    "/workflow/delivery/runs",
    "/workflow/delivery/runs/{id}/advance",
    "/workflow/delivery/runs/{id}/answer",
    "/workflow/delivery/runs/{id}/gates",
    // The release family's three writes and the due crank are refused HARDER
    // than the role gate: the handler refuses the agent class BY KIND before
    // any work, because these are the writes whose consequences reach another
    // system. The row is here so the class cell stays asserted.
    "/workflow/delivery/releases",
    "/workflow/delivery/releases/{id}/approve",
    "/workflow/delivery/releases/{id}/promote",
    "/workflow/delivery/due",
    // ...and the three READS, which demand the same role. A read is not a
    // lesser surface: the attestation chain carries signed evidence, and the
    // replay report carries a verdict about a run's integrity. Either leaking
    // to a class that cannot drive the run is the same failure, so the reads
    // are refused here for the same reason the writes are. (The attestation
    // read shipped WITHOUT this row — an unasserted divergence where the matrix
    // expected the agent class to pass a route the handler refuses. Closed.)
    "/workflow/delivery/runs/{id}/attestations",
    "/workflow/delivery/runs/{id}/replay-verify",
    "/workflow/delivery/runs/{id}/trace",
    // ...and the census reads, same role law: a read is not a lesser surface.
    "/workflow/delivery/releases",
    "/workflow/delivery/runs",
    "/workflow/delivery/runs/{id}",
    "/workflow/delivery/runs/{id}/steps",
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
    "/workflow/reflection/corpus",
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
        } else if PRE_GATE_BODY_REJECT.contains(&template) {
            assert!(
                st == StatusCode::UNPROCESSABLE_ENTITY || st == StatusCode::FORBIDDEN,
                "{method} {template} (agent) must 422 on a body the typed extractor rejects, \
                 or 403 once it parses, got {st}"
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

/// R63 / D63.5 — the azp refusal counter is actually ON THE WIRE, not merely
/// defined. The value is pinned behaviorally in `tests/r63_azp_pins.rs`; this
/// pins the SCRAPE side, which is a separate link in the chain and can be lost
/// silently (a counter that increments but is never exported is invisible to
/// every operator watching a dashboard).
#[tokio::test]
async fn admin_scrape_exposes_the_azp_rejection_counter() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "m2-azp",
        "user:m2azp",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );
    let (st, body) = send_body(&srv, Some(&admin), "/metrics", "GET", "").await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        body.contains("# TYPE brain_jwt_azp_rejected_total counter"),
        "the azp counter must be exported as a counter: {body}"
    );
    assert!(
        body.contains("brain_jwt_azp_rejected_total "),
        "the azp counter must carry a value line: {body}"
    );
}

/// R53a — the decision-class family is actually ON THE WIRE, not merely
/// defined. The values are pinned in `tests/r53a_decision_class_pins.rs` and
/// behaviourally in `src/agentloop/subagents.rs`; this pins the SCRAPE side,
/// which is a separate link in the chain and can be lost silently — a counter
/// that increments but is never exported is invisible to every operator
/// watching a dashboard.
///
/// It also pins the property `D53a.3` asks for: a value for every DECLARED
/// class, including the ones that have never fired in this process. A dashboard
/// must never have to distinguish "nothing happened" from "not instrumented".
#[tokio::test]
async fn admin_scrape_exposes_the_decision_class_family() {
    let srv = build_server();
    let admin = mint(
        &srv,
        "m2-r53a",
        "user:m2r53a",
        "team-a",
        &["admin:*/*"],
        &["admin", "matrix-role"],
    );
    let (st, body) = send_body(&srv, Some(&admin), "/metrics", "GET", "").await;
    assert_eq!(st, StatusCode::OK);
    for name in [
        "brain_model_calls_total",
        "brain_model_tokens_total",
        "brain_model_incomplete_total",
    ] {
        assert!(
            body.contains(&format!("# TYPE {name} counter")),
            "{name} must be exported as a counter: {body}"
        );
    }
    for class in brain_server::decision_class::DecisionClass::ALL {
        let row = format!("brain_model_calls_total{{class=\"{}\"}}", class.as_str());
        assert!(
            body.contains(&row),
            "the declared class {} emitted no row — every declared class must be \
             visible even at zero: {body}",
            class.as_str()
        );
    }
    // The family carries a class label and nothing else. A `domain` label here
    // would be a fourth cardinality to scope, and the first one an operator
    // could mine for tenant shape.
    assert!(
        !body.contains("brain_model_calls_total{class=\"open_generate\","),
        "the decision-class family must carry exactly one label: {body}"
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

/// The delivery loop is role-gated end to end, and the proof is seeded: the
/// OPERATOR opens a real delivery run; the AGENT bearer — whose scopes pass
/// and whose `workflow` role is absent — is refused 403 by the ROLE gate on
/// that SAME run; on an ABSENT run the agent reads the same probe-blind 404
/// the operator gets; and a REVOKED agent identity dies at the middleware with
/// `401 identity_revoked` before any body is read.
///
/// The seeding is the whole point. The class matrix drives an absent row, so it
/// can only assert 404-or-403 there; this test is what proves the gate fires
/// on a row that actually exists.
#[tokio::test]
async fn delivery_routes_role_gated_agent_and_revoked_denied() {
    let srv = twokey_server();
    let create = r#"{"domain":"global","goal":"gated","tier":"observe"}"#;

    // The operator opens the run — and the operator must clear the role gate,
    // or "403 for the agent" would prove nothing.
    let (st, body) = send_body(
        &srv,
        Some(TWOKEY_OP),
        "/workflow/delivery/runs",
        "POST",
        create,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "the operator opens a delivery run: {body}"
    );
    let run_id: i64 = serde_json::from_str::<serde_json::Value>(&body).unwrap()["run_id"]
        .as_i64()
        .expect("a run id in the admission receipt");
    assert!(run_id > 0);

    let advance = r#"{"expected_revision":0,"to_phase":"design","artifact_refs":[]}"#;
    let answer = r#"{"expected_revision":0,"answer":"yes"}"#;
    let gates = r#"{"to_phase":"design"}"#;
    // (label, path, method, body). The three reads join the three writes: a
    // read is not a lesser surface here, because the chain carries signed
    // evidence and the replay report carries a verdict about the run.
    let cases: Vec<(&str, String, &str, &str)> = vec![
        (
            "advance",
            format!("/workflow/delivery/runs/{run_id}/advance"),
            "POST",
            advance,
        ),
        (
            "answer",
            format!("/workflow/delivery/runs/{run_id}/answer"),
            "POST",
            answer,
        ),
        (
            "gates",
            format!("/workflow/delivery/runs/{run_id}/gates"),
            "POST",
            gates,
        ),
        (
            "attestations",
            format!("/workflow/delivery/runs/{run_id}/attestations"),
            "GET",
            "",
        ),
        (
            "replay-verify",
            format!("/workflow/delivery/runs/{run_id}/replay-verify"),
            "GET",
            "",
        ),
        (
            "trace",
            format!("/workflow/delivery/runs/{run_id}/trace"),
            "GET",
            "",
        ),
    ];

    // The agent's scopes pass; only the ROLE is missing.
    for (label, path, method, body) in &cases {
        let (st, _) = send_body(&srv, Some(TWOKEY_AGENT), path, method, body).await;
        assert_eq!(
            st,
            StatusCode::FORBIDDEN,
            "the agent bearer is refused by the workflow role gate on /{label} — a gate \
             proven only against an absent row is a gate proven about nothing"
        );
    }

    // ...and the OPERATOR clears all six on that same run. Without this arm the
    // 403 above would be satisfied by a route that refuses everybody, which is
    // not a gate.
    for (label, path, method, body) in &cases {
        let (st, text) = send_body(&srv, Some(TWOKEY_OP), path, method, body).await;
        assert!(
            st == StatusCode::OK || st == StatusCode::CONFLICT,
            "the operator — who HAS the workflow role — is not refused on /{label} (got {st}: \
             {text}). A 403-for-everybody is not a gate."
        );
    }

    // Probe-blind: an absent run reads the same 404 for both classes.
    for (label, path, method, body) in &cases {
        let absent = path.replacen(&run_id.to_string(), "999999", 1);
        let (st_op, _) = send_body(&srv, Some(TWOKEY_OP), &absent, method, body).await;
        let (st_agent, _) = send_body(&srv, Some(TWOKEY_AGENT), &absent, method, body).await;
        assert_eq!(
            st_op,
            StatusCode::NOT_FOUND,
            "absent run 404s for the operator on /{label}"
        );
        assert_eq!(
            st_agent, st_op,
            "absence must be indistinguishable between classes on /{label}"
        );
    }

    // Revocation kills the identity before the handler exists to gate it.
    revoke_via_route(&srv, TWOKEY_OP, "agent@loopback").await;
    let (st, text) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        &cases[0].1,
        cases[0].2,
        cases[0].3,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "a revoked agent dies at the middleware"
    );
    assert_eq!(text, revoked_body("identity_revoked"));

    // ...and on a READ too: revocation is not a write-scoped kill switch.
    let (st, text) = send_body(
        &srv,
        Some(TWOKEY_AGENT),
        "/workflow/delivery/runs/1/replay-verify",
        "GET",
        "",
    )
    .await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "a revoked agent dies at the middleware on the replay read too — a kill switch that \
         spares the read surfaces would leave the evidence readable by a dead identity"
    );
    assert_eq!(text, revoked_body("identity_revoked"));
}

/// D2: `?verify=1` is documented in three places and was pinned NOWHERE
/// behaviourally. The verdict is UNCONDITIONAL — the parameter is an explicit
/// request for the IDENTICAL payload, and no value of it can switch
/// verification off. This opens a real run and reads it four ways.
#[tokio::test]
async fn delivery_attestation_verify_parameter_returns_the_identical_payload() {
    let srv = twokey_server();
    let (st, body) = send_body(
        &srv,
        Some(TWOKEY_OP),
        "/workflow/delivery/runs",
        "POST",
        r#"{"domain":"global","goal":"verify-param","tier":"observe"}"#,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "the operator opens a run: {body}");
    let run_id: i64 = serde_json::from_str::<serde_json::Value>(&body).unwrap()["run_id"]
        .as_i64()
        .expect("a run id");
    let path = format!("/workflow/delivery/runs/{run_id}/attestations");

    let (st, bare) = send_body(&srv, Some(TWOKEY_OP), &path, "GET", "").await;
    assert_eq!(
        st,
        StatusCode::OK,
        "the read serves without the parameter: {bare}"
    );
    let (st, asked) = send_body(
        &srv,
        Some(TWOKEY_OP),
        &format!("{path}?verify=1"),
        "GET",
        "",
    )
    .await;
    assert_eq!(st, StatusCode::OK, "`?verify=1` is accepted: {asked}");
    let (st, wrong) = send_body(
        &srv,
        Some(TWOKEY_OP),
        &format!("{path}?verify=0"),
        "GET",
        "",
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "an unrecognized value is still the identical payload — the parameter selects nothing, \
         so it cannot 400: {wrong}"
    );

    // The verdict is the same in all three. Only the report's own `generated_at`
    // could legitimately differ, and it does not — the assembly takes `now` from
    // the handler, and the chain payload carries no clock field.
    let a: serde_json::Value = serde_json::from_str(&bare).expect("json");
    let b: serde_json::Value = serde_json::from_str(&asked).expect("json");
    let c: serde_json::Value = serde_json::from_str(&wrong).expect("json");
    assert_eq!(a, b, "`?verify=1` returns the IDENTICAL payload");
    assert_eq!(a, c, "an unrecognized value returns the IDENTICAL payload");
    // A non-verifying chain is 200 with a NAMED per-link refusal, never a
    // silent empty chain that reads as verified. This run has NO links, so the
    // honest shape is: verified over zero links, and the link list is empty.
    assert_eq!(
        a["verdict"]["link_count"], 0,
        "a run with no signed link has no link to refuse"
    );
    assert_eq!(
        a["chain"].as_array().map(Vec::len),
        Some(0),
        "the chain list is empty, and the verdict says so rather than implying verification"
    );
}

/// An RAII guard that points `BRAIN_UMP_KEY_DIR` at `dir` and restores the
/// previous value when it DROPS — not at the end of the function body.
///
/// The manual save/restore this replaces was the last statement of the test,
/// which restores on exactly one of its thirteen exit paths. A panic — and a
/// panic is precisely what this test exists to provoke — unwound straight past
/// the restore and left the var pointing at a `TempDir` that was itself being
/// deleted during the unwind: a permanently dead key path for every test that
/// ran after it. The repo already wrote this rule down in `src/lib.rs`
/// ("Every test that mutates a process env var takes THIS lock") and already has
/// the `Drop`-based idiom; this adopts it.
struct KeyDirGuard {
    previous: Option<std::ffi::OsString>,
}

impl KeyDirGuard {
    fn arm(dir: &std::path::Path) -> Self {
        let previous = std::env::var_os("BRAIN_UMP_KEY_DIR");
        // SAFETY: `BK_ENV_LOCK` is held by the caller for this guard's whole
        // life, and the guard restores before the lock is released.
        unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", dir) };
        Self { previous }
    }
}

impl Drop for KeyDirGuard {
    fn drop(&mut self) {
        // SAFETY: as above — the lock outlives this drop.
        unsafe {
            match &self.previous {
                Some(v) => std::env::set_var("BRAIN_UMP_KEY_DIR", v),
                None => std::env::remove_var("BRAIN_UMP_KEY_DIR"),
            }
        }
    }
}

/// D3: the keyless-host `409 delivery_attestation_refused` was proven at the
/// CORE (both arms, the real resolver) and at NO HTTP hop. A core-level proof
/// cannot see a handler that maps the refusal to the wrong status, or drops it
/// into a 500.
#[tokio::test]
async fn delivery_attestation_read_refuses_with_409_on_a_keyless_host() {
    // The keyless posture is ARMED, not assumed. A developer machine (and this
    // one) carries a real operator key, so a test that merely ran here would
    // pass vacuously on a keyed host and prove nothing about the refusal. The
    // env lock is this file's own `BK_ENV_LOCK` precedent: the var is process
    // global, so the guard is held for the whole test. The var itself is set
    // through a `Drop` guard, so a failing assert cannot leak it.
    let _env = BK_ENV_LOCK.lock().await;
    let keyless_dir = tempfile::TempDir::new().expect("an empty key dir");
    let _keyless = KeyDirGuard::arm(keyless_dir.path());

    // A single-token opaque server: the operator token exists, and there is NO
    // agent token — so no operator key is provisioned, which is the posture a
    // fresh host is in.
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("brain.db");
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mgr = SqliteConnectionManager::file(&db_path);
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(2).build(mgr).expect("pool");
    brain_server::migration::run_migration(
        &mut pool.get().expect("conn"),
        brain_server::config::DB_MMAP_SIZE_MIB,
    )
    .expect("migration");
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );
    let tok_file = dir.path().join("tokens");
    std::fs::write(&tok_file, "keyless-op-token\n").expect("token file");
    let token_store = brain_server::auth::TokenStore::from_file(Some(tok_file));
    token_store.reload_parts_from(vec!["keyless-op-token".to_string()], None);
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
    let srv = TestServer {
        _dir: dir,
        state,
        priv_key: {
            let mut rng = rand::rngs::ThreadRng::default();
            rsa::RsaPrivateKey::new(&mut rng, 2048).expect("keypair")
        },
    };

    let (st, body) = send_body(
        &srv,
        Some("keyless-op-token"),
        "/workflow/delivery/runs",
        "POST",
        r#"{"domain":"global","goal":"keyless","tier":"observe"}"#,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "admission does not need a key: {body}");
    let run_id: i64 = serde_json::from_str::<serde_json::Value>(&body).unwrap()["run_id"]
        .as_i64()
        .expect("a run id");

    // The phase pass is the thing that needs the key. On a keyless host it
    // refuses — and it must refuse with the NAMED code, not a 500 and not a
    // silent success with a missing link.
    let (st, text) = send_body(
        &srv,
        Some("keyless-op-token"),
        &format!("/workflow/delivery/runs/{run_id}/advance"),
        "POST",
        r#"{"expected_revision":0,"to_phase":"design","artifact_refs":[]}"#,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "a phase pass with no operator key refuses with 409, never 500: {text}"
    );
    assert!(
        text.contains("delivery_attestation_refused"),
        "the refusal is NAMED — a client learns WHICH law stopped the pass, rather than parsing \
         prose: {text}"
    );

    // The read still serves, and it serves the truth: the chain is empty
    // because nothing was signed.
    let (st, text) = send_body(
        &srv,
        Some("keyless-op-token"),
        &format!("/workflow/delivery/runs/{run_id}/attestations"),
        "GET",
        "",
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "the read serves on a keyless host: {text}"
    );
    let read: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(
        read["verdict"]["link_count"], 0,
        "no key means no link — and the read says zero rather than implying a chain"
    );
    // DISCLOSED CEILING, asserted as it SHIPS rather than as one might wish:
    // `verified` is `links.iter().all(|l| l.verified)`, and `all` over an
    // EMPTY iterator is vacuously true. So a run with no signed evidence at all
    // reports `verified: true`. This is the shipped R40 semantics and changing
    // it is a NEW decision (an empty chain is a fail-closed posture, not a
    // verified one — but that is a wire-contract change to a shipped route and
    // is NOT R41's to make). A reader must therefore read `verified` together
    // with `link_count`, which is why both are asserted here and why the
    // openapi route description carries the negation.
    assert_eq!(
        read["verdict"]["verified"], true,
        "an empty chain is vacuously `verified` under the shipped `all()` semantics. This is \
         R40's shipped verdict, not a claim that evidence exists: `link_count` is 0 and `head` \
         is null. Reading `verified` WITHOUT `link_count` would misread a keyless host as an \
         integrity claim. R41 asserts the shipped truth and discloses the ceiling rather than \
         re-opening a made decision on a shipped wire contract."
    );
    assert!(
        read["verdict"]["head"].is_null(),
        "an empty chain has no head, and says so — a head would be a fabricated address"
    );

    // The guard restores `BRAIN_UMP_KEY_DIR` on the way out, INCLUDING on a
    // panic — a leak here would silently keyless every later test in the
    // process.
}
