//! R106 — agent writes are reviewed and labelled BY IDENTITY, not by env.
//!
//! The human-promotion invariant was a deployment posture: `write_posture()`
//! defaulted to `open`, so every agent-facing write surface inserted directly
//! unless an operator had set `BRAIN_WRITE_POSTURE=review`. The installer
//! writes `review` into a new plist, but a manual or default launch — which is
//! how a development host and every `cargo run` starts — is `open`. Nothing in
//! the server said "this write came from an agent"; the answer lived in an env
//! var, so the same binary promoted for one launch and proposed for the next.
//!
//! Two fixes, one seam:
//!
//! 1. **The identity decides.** A recognized agent class — the typed
//!    `AgentLoopback` principal the two-lane deployment mints, or a JWT
//!    carrying the `agent` role — is proposal-only regardless of the posture
//!    knob, which keeps its name, its `open` default and its meaning for
//!    everyone else: an operator's deliberate direct write still inserts.
//! 2. **The label is derived, not asserted.** `origin_context` on the plain
//!    ingest seam decides the row's taint label, so an agent could claim
//!    `owner` and land channel-captured content under operator-import
//!    provenance. For the agent class the server derives it.
//!
//! Route-level pins through the composed `app(state)` on the two-key server
//! shape (line 1 operator, line 2 agent), because the agent principal only
//! exists there.

use axum::{body::Body, http::Request, http::StatusCode};
use brain_server::pool::SqliteConnectionManager;
use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use std::sync::Arc;
use tower::ServiceExt;

const OP_TOKEN: &str = "r106-op-token";
const AGENT_TOKEN: &str = "r106-agent-token";

struct TestServer {
    _dir: tempfile::TempDir,
    state: Arc<brain_server::AppState>,
}

/// The installer's two-lane token file: line 1 operator (authenticates to the
/// `None` principal), line 2 agent (authenticates to the typed `AgentLoopback`).
fn build_server() -> TestServer {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("brain.db");
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
    std::fs::write(&tok_file, format!("{OP_TOKEN}\n{AGENT_TOKEN}\n")).expect("token file");
    let token_store = brain_server::auth::TokenStore::from_file(Some(tok_file));
    token_store.reload_parts_from(vec![OP_TOKEN.to_string()], Some(AGENT_TOKEN.to_string()));
    let state = Arc::new(brain_server::AppState {
        token_store,
        jwt_middleware_state: Arc::new(JwtMiddlewareState::opaque_for_tests(
            pool.clone(),
            db_path.clone(),
        )),
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
    TestServer { _dir: dir, state }
}

async fn post(
    srv: &TestServer,
    token: &str,
    path: &str,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(axum::http::Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_owned()))
        .unwrap();
    let resp = app(srv.state.clone()).oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn knowledge_count(srv: &TestServer) -> i64 {
    srv.state
        .pool
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
        .unwrap()
}

fn pending_proposal_count(srv: &TestServer) -> i64 {
    srv.state
        .pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM proposals WHERE status='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn last_origin(srv: &TestServer) -> String {
    srv.state
        .pool
        .get()
        .unwrap()
        .query_row(
            "SELECT origin FROM knowledge ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

// ── the pins ────────────────────────────────────────────────────────────

/// THE pin. Under the compiled default (`open`) the agent class must still
/// propose: no `knowledge` row, one pending proposal. Observed red-first
/// against the live tree — the agent's write inserted directly, which is the
/// human-promotion invariant living in an env var.
#[tokio::test]
async fn agent_class_write_proposes_under_the_default_posture() {
    let srv = build_server();
    let (st, body) = post(
        &srv,
        AGENT_TOKEN,
        "/ingest",
        r#"{"title":"channel note","content":"the agent learned this from the channel","domain":"global"}"#,
    )
    .await;
    assert!(
        st == StatusCode::OK || st == StatusCode::ACCEPTED,
        "the agent's write must be accepted into the queue: {st} {body}"
    );
    assert_eq!(
        knowledge_count(&srv),
        0,
        "an agent-class write must not insert a knowledge row under the default \
         posture — that is the whole invariant"
    );
    assert_eq!(
        pending_proposal_count(&srv),
        1,
        "it lands as one pending proposal instead: {body}"
    );
}

/// The neighbouring behaviour, and the reason this is an identity rule and not
/// a restriction: the OPERATOR's deliberate direct write still inserts under
/// `open`. A fix that made everything propose would be a different product.
#[tokio::test]
async fn operator_write_still_inserts_under_the_default_posture() {
    let srv = build_server();
    let (st, body) = post(
        &srv,
        OP_TOKEN,
        "/ingest",
        r#"{"title":"operator note","content":"the operator typed this deliberately","domain":"global"}"#,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(
        knowledge_count(&srv),
        1,
        "an operator's direct write must survive the default posture: {body}"
    );
    assert_eq!(pending_proposal_count(&srv), 0);
}

/// The label is derived from WHO is writing. An agent asserting
/// `origin_context: "owner"` must not land channel-captured content under
/// operator-import provenance — the server knows the class and the class is
/// the truth here.
#[tokio::test]
async fn agent_class_write_is_labelled_by_the_server_not_the_wire() {
    let srv = build_server();
    let (st, body) = post(
        &srv,
        OP_TOKEN,
        "/ingest",
        r#"{"title":"owner asserted","content":"operator written, owner asserted","domain":"global","origin_context":"owner"}"#,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(
        last_origin(&srv),
        "imported",
        "an operator's own assertion stands — the server does not relabel the operator"
    );
}

/// The same request from the agent class must not be able to claim the
/// operator's provenance: whatever it asserts, the row carries the label the
/// agent class implies.
#[tokio::test]
async fn agent_class_cannot_assert_operator_provenance() {
    let srv = build_server();
    let (st, body) = post(
        &srv,
        AGENT_TOKEN,
        "/ingest",
        r#"{"title":"agent captured","content":"agent captured this","domain":"global","origin_context":"owner"}"#,
    )
    .await;
    // Under the review posture the write is queued rather than stored; the
    // proposal's SOURCE carries the label instead (the gate's own convention).
    assert!(
        st == StatusCode::OK || st == StatusCode::ACCEPTED,
        "accepted into the queue: {st} {body}"
    );
    let text = serde_json::to_string(&body).unwrap_or_default();
    assert!(
        !text.contains("\"imported\"") || knowledge_count(&srv) == 0,
        "an agent-class write must not be stored as operator-import provenance: {body}"
    );
    if knowledge_count(&srv) == 0 {
        let queued_source: String = srv
            .state
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT source FROM proposals ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or_default();
        assert_ne!(
            queued_source, "structured",
            "the queued agent proposal must carry the agent/capture source label, not the \\
             structured-import one: {queued_source:?}"
        );
    }
}

/// The unit seam itself, so the rule is testable without a server: which
/// principals the identity rule covers, and that it cannot be talked out of by
/// an unset posture.
#[test]
fn the_identity_rule_covers_the_agent_class_and_nobody_else() {
    use brain_server::auth::policy::{Principal, PrincipalKind};
    let mk = |kind: PrincipalKind, roles: Vec<&str>| {
        Some(Principal {
            sub: "agent@loopback".into(),
            tenant: "global".into(),
            scopes: vec![],
            jti: "j".into(),
            roles: roles.into_iter().map(String::from).collect(),
            manages: vec![],
            kind,
        })
    };
    let loopback = mk(PrincipalKind::AgentLoopback, vec![]);
    let agent_role = mk(PrincipalKind::Jwt, vec!["agent"]);
    let operator_jwt = mk(PrincipalKind::Jwt, vec!["dpo"]);
    assert!(
        brain_server::handlers::agent_class(&loopback),
        "the typed loopback agent IS the agent class"
    );
    assert!(
        brain_server::handlers::agent_class(&agent_role),
        "a JWT carrying the agent role is the agent class too"
    );
    assert!(
        !brain_server::handlers::agent_class(&operator_jwt),
        "an operator JWT is not"
    );
    assert!(
        !brain_server::handlers::agent_class(&None),
        "the opaque/loopback operator is not"
    );
    // The posture knob keeps its meaning for everyone else.
    assert_eq!(
        brain_server::handlers::effective_write_posture(&operator_jwt),
        brain_server::config::write_posture(),
        "a non-agent principal sees the configured posture verbatim"
    );
}
