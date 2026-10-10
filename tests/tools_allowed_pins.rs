//! R107 — `tools_allowed` binds at the UMP verb seam.
//!
//! The field has been stored and surfaced through the role API since it
//! shipped, with enforcement deferred to v1.24 by an explicit "store now,
//! enforce later" note (the same discipline `connectors_allowed` shipped with).
//! It never arrived, so every preset's carefully chosen tool set — `agent`:
//! `ump.recall|get|feedback`; `supervisor`: plus `ump.revise|remember`;
//! `exec`: `ump.recall` alone — described intent rather than policy: the UMP
//! entry points read nothing. A role that grants no `ump.forget` could still
//! erase a row through `/ump/forget`.
//!
//! `authorize_tool` is the principal-side twin of `cap_gate`: one predicate
//! beside the gates the entry points already run, no new mechanism. The pins
//! below are behavioural through the composed app, because the failure mode
//! this round closes is exactly "the field is decorative".

use axum::{body::Body, http::Request, http::StatusCode};
use brain_server::pool::SqliteConnectionManager;
use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

struct TestServer {
    _dir: tempfile::TempDir,
    state: Arc<brain_server::AppState>,
    priv_key: rsa::RsaPrivateKey,
}

fn rsa_keypair(key_dir: &Path) -> rsa::RsaPrivateKey {
    let mut rng = rand::rngs::ThreadRng::default();
    let priv_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("test keypair");
    let pub_key = rsa::RsaPublicKey::from(&priv_key);
    std::fs::create_dir_all(key_dir).unwrap();
    std::fs::write(
        key_dir.join("r107-kid.pem"),
        pub_key
            .to_public_key_pem(LineEnding::LF)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let key_path = key_dir.join("r107-kid.key");
    std::fs::write(
        &key_path,
        priv_key.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    priv_key
}

/// The two roles the pins need, both VALIDATED through the production writer:
/// `r107-narrow` mirrors the `agent` preset's tool set (no revise, no
/// remember, no forget) and `r107-wide` holds `*`. Presets are seeded too, so
/// a preset can be used as-is without this file re-declaring one.
fn seed_roles(pool: &brain_server::Pool) {
    let conn = pool.get().expect("conn");
    for (name, json) in brain_server::role::PRESETS_RAW {
        conn.execute(
            "INSERT OR IGNORE INTO roles(name, json) VALUES (?1, ?2)",
            rusqlite::params![name, json],
        )
        .unwrap();
    }
    let fixture = |name: &str, tools: Vec<&str>| {
        let role = brain_server::role::Role {
            name: name.to_string(),
            description: Some("r107 fixture".to_string()),
            scopes: vec![
                "private".to_string(),
                "domain".to_string(),
                "team".to_string(),
            ],
            owner_filter: "all".to_string(),
            can: vec!["read".to_string(), "write".to_string()],
            panels_default: None,
            panels_hidden: None,
            tools_allowed: Some(tools.into_iter().map(String::from).collect()),
        };
        brain_server::role::validate(&role).expect("fixture role must validate");
        brain_server::role::upsert(&conn, &role).expect("seed fixture role");
    };
    fixture("r107-narrow", vec!["ump.recall", "ump.get", "ump.feedback"]);
    fixture("r107-wide", vec!["*"]);
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
    seed_roles(&pool);
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );
    let priv_key = rsa_keypair(&dir.path().join("keys"));
    let key_store =
        brain_server::auth::jwks::KeyStore::load(&dir.path().join("keys")).expect("load test keys");
    let jwt_issuer = "https://brain.r107/".to_string();
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
        registry: brain_server::domain_registry::DomainRegistry::new(pool.clone(), &db_path, false),
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

fn mint(srv: &TestServer, jti: &str, sub: &str, scopes: &[&str], roles: &[&str]) -> String {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = brain_server::auth::jwt::Claims {
        iss: "https://brain.r107/".to_string(),
        aud: "brain-server".to_string(),
        sub: sub.to_string(),
        jti: jti.to_string(),
        iat: now,
        nbf: now,
        exp: now + 600,
        tenant: "team-a".to_string(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        roles: roles.iter().map(|s| s.to_string()).collect(),
        manages: Vec::new(),
        chain: None,
        azp: None,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("r107-kid".to_string());
    let pem = srv.priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap(),
    )
    .unwrap()
}

async fn send(
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

fn seed_private_row(srv: &TestServer, owner: &str, content: &str) -> i64 {
    let conn = srv.state.pool.get().unwrap();
    conn.execute(
        "INSERT INTO knowledge (title, content, content_hash, source, domain, owner, access_scope)
         VALUES (?1, ?2, ?3, 'structured', 'global', ?4, 'private')",
        rusqlite::params![content, content, format!("h-{content}"), owner],
    )
    .unwrap();
    conn.last_insert_rowid()
}

// ── the pins ────────────────────────────────────────────────────────────

/// The headline: a role whose `tools_allowed` omits `ump.revise` is refused at
/// `/ump/revise` — even though its `can` allowlist grants `write`, and even
/// though the token's scope grants Write. Three layers said yes; the tool grant
/// is what the operator actually configured.
#[tokio::test]
async fn a_role_without_the_tool_is_refused_at_the_ump_verb() {
    let srv = build_server();
    let id = seed_private_row(&srv, "user:tools", "tool-gate fixture row");
    let tok = mint(
        &srv,
        "r107-narrow",
        "user:tools",
        &["write:team-a/*"],
        &["r107-narrow"],
    );

    let (st, body) = send(
        &srv,
        &tok,
        "/ump/revise",
        &format!(r#"{{"id":"ump:memory:{id}","record":{{"content":"rewritten by a tool it may not call"}}}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "`tools_allowed` omits ump.revise, so the verb must refuse: {body}"
    );
}

/// `ump.forget` is the destructive one — the strongest form of the same rule.
#[tokio::test]
async fn a_role_without_the_tool_cannot_forget() {
    let srv = build_server();
    let id = seed_private_row(&srv, "user:tools", "tool-gate forget fixture");
    let before = srv
        .state
        .pool
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get::<_, i64>(0))
        .unwrap();
    let tok = mint(
        &srv,
        "r107-narrow2",
        "user:tools",
        &["write:team-a/*"],
        &["r107-narrow"],
    );

    let (st, body) = send(
        &srv,
        &tok,
        "/ump/forget",
        &format!(r#"{{"id":"ump:memory:{id}"}}"#),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{body}");
    let after = srv
        .state
        .pool
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get::<_, i64>(0))
        .unwrap();
    assert_eq!(after, before, "the refusal erased nothing");
}

/// The role that DOES hold the tool reaches the handler: the gate must not
/// become a blanket 403 that makes the field meaningless in the other
/// direction. The verb still answers its own vocabulary (a missing record is a
/// 404/409, not a 403), which is how this pin distinguishes "the tool gate let
/// it through" from "the tool gate blocked it".
#[tokio::test]
async fn a_role_holding_the_tool_reaches_the_handler() {
    let srv = build_server();
    let tok = mint(
        &srv,
        "r107-wide",
        "user:wide",
        &["write:team-a/*"],
        &["r107-wide"],
    );
    let (st, body) = send(&srv, &tok, "/ump/forget", r#"{"id":"ump:memory:999999"}"#).await;
    assert_ne!(
        st,
        StatusCode::FORBIDDEN,
        "`*` grants every tool, so this must not be a 403 from the tool gate: {body}"
    );
}

/// The three tools the narrow role DOES hold still work — recall and feedback
/// included. A gate that refused everything would pass the two pins above.
#[tokio::test]
async fn the_tools_a_role_does_hold_still_work() {
    let srv = build_server();
    let tok = mint(
        &srv,
        "r107-narrow3",
        "user:tools",
        &["write:team-a/*"],
        &["r107-narrow"],
    );

    let (st, body) = send(&srv, &tok, "/ump/recall", r#"{"query":"anything"}"#).await;
    assert_ne!(st, StatusCode::FORBIDDEN, "ump.recall is granted: {body}");

    let (st, body) = send(&srv, &tok, "/ump/feedback", r#"{"id":"ump:memory:1"}"#).await;
    assert_ne!(st, StatusCode::FORBIDDEN, "ump.feedback is granted: {body}");
}

/// A role-less principal is untouched: this gate governs roles, and the
/// action seam (`authorize_role`) governs claim-less callers. Two seams, two
/// populations — a role-less token must not start 403ing here.
#[tokio::test]
async fn a_roleless_principal_is_untouched_by_the_tool_gate() {
    let srv = build_server();
    let tok = mint(
        &srv,
        "r107-roleless",
        "user:nobody",
        &["write:team-a/*"],
        &[],
    );
    let (st, body) = send(&srv, &tok, "/ump/forget", r#"{"id":"ump:memory:999999"}"#).await;
    assert_ne!(
        st,
        StatusCode::FORBIDDEN,
        "the tool gate governs roles only; the action seam owns the role-less class: {body}"
    );
}

/// Several roles are the UNION of their tool sets, and a role that never
/// declares the field grants everything (the field is optional and was never
/// enforced, so an undeclared set must not silently mean "no tools").
#[test]
fn the_tool_gate_semantics_are_one_rule() {
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
    let conn = pool.get().expect("conn");
    let undeclared = brain_server::role::Role {
        name: "r107-undeclared".to_string(),
        description: None,
        scopes: vec!["private".to_string()],
        owner_filter: "all".to_string(),
        can: vec!["read".to_string(), "write".to_string()],
        panels_default: None,
        panels_hidden: None,
        tools_allowed: None,
    };
    brain_server::role::validate(&undeclared).expect("valid role");
    brain_server::role::upsert(&conn, &undeclared).expect("seed");
    brain_server::role::upsert(
        &conn,
        &brain_server::role::Role {
            name: "r107-one".to_string(),
            description: None,
            scopes: vec!["private".to_string()],
            owner_filter: "all".to_string(),
            can: vec!["read".to_string(), "write".to_string()],
            panels_default: None,
            panels_hidden: None,
            tools_allowed: Some(vec!["ump.recall".to_string()]),
        },
    )
    .expect("seed");

    let p = |roles: Vec<&str>| {
        Some(brain_server::auth::Principal {
            sub: "user:union".to_string(),
            tenant: "team-a".to_string(),
            scopes: vec![],
            jti: "j".to_string(),
            roles: roles.into_iter().map(String::from).collect(),
            manages: vec![],
            kind: brain_server::auth::PrincipalKind::Jwt,
        })
    };
    let gate = |roles: Vec<&str>, tool: &str| {
        brain_server::handlers::authorize_tool(&p(roles), &pool, tool).is_ok()
    };
    assert!(
        gate(vec!["r107-undeclared"], "ump.forget"),
        "a role that never declared tools_allowed is not narrowed by this field"
    );
    assert!(
        !gate(vec!["r107-one"], "ump.forget"),
        "declared and absent = refused"
    );
    assert!(
        gate(vec!["r107-one"], "ump.recall"),
        "declared and present = allowed"
    );
    assert!(
        gate(vec!["r107-one", "r107-undeclared"], "ump.forget"),
        "several roles are the UNION of their grants"
    );
}
