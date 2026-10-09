//! R100: the four read surfaces R85's record gate never reached — legacy
//! `GET /search`, the graph trio (`/graph/entity/{name}`, `/graph/relations`,
//! `/graph/traverse`), and `POST /decision/{id}/evaluate`. A no-role read
//! JWT must see its own private rows and never another subject's, on every
//! one of them; the role shared pool, admin, and loopback postures are
//! unchanged. Red-first: each denial pin fails against the ungated handler.

use axum::{body::Body, http::Request, http::StatusCode};
use brain_server::pool::SqliteConnectionManager;
use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;
use zerocopy::IntoBytes;

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
    std::fs::write(key_dir.join("r100-kid.pem"), pub_pem.as_bytes()).unwrap();
    let key_path = key_dir.join("r100-kid.key");
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
        let conn = pool.get().expect("conn");
        for (name, json) in brain_server::role::PRESETS_RAW {
            conn.execute(
                "INSERT OR IGNORE INTO roles(name, json) VALUES (?1, ?2)",
                rusqlite::params![name, json],
            )
            .unwrap();
        }
    }
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );
    let priv_key = rsa_keypair(&dir.path().join("keys"));
    let key_store =
        brain_server::auth::jwks::KeyStore::load(&dir.path().join("keys")).expect("load test keys");
    let jwt_issuer = "https://brain.r100/".to_string();
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
        iss: "https://brain.r100/".to_string(),
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
    header.kid = Some("r100-kid".to_string());
    let pem = srv.priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    encode(&header, &claims, &encoding).unwrap()
}

async fn get(srv: &TestServer, token: &str, uri: &str) -> (StatusCode, serde_json::Value, String) {
    let req = Request::builder()
        .method(axum::http::Method::GET)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app(srv.state.clone()).oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body");
    let raw = String::from_utf8_lossy(&bytes).to_string();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json, raw)
}

async fn post_json(
    srv: &TestServer,
    token: &str,
    uri: &str,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(axum::http::Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_owned()))
        .unwrap();
    let resp = app(srv.state.clone()).oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body");
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ── seeding ─────────────────────────────────────────────────────────────

fn seed_private_row(pool: &brain_server::Pool, owner: &str, content: &str) -> i64 {
    let v = vec![0.5f32; 512];
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO knowledge (title, content, content_hash, source, domain, owner, access_scope)
         VALUES (?1, ?2, ?3, 'structured', 'global', ?4, 'private')",
        rusqlite::params![content, content, format!("h-{content}"), owner],
    )
    .unwrap();
    let kid = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO vec_knowledge (knowledge_id, embedding_int8, embedding_bit, source, created_at)
         VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'structured', datetime('now'))",
        rusqlite::params![kid, v.as_bytes()],
    )
    .unwrap();
    kid
}

fn seed_decision_row(pool: &brain_server::Pool, owner: &str) -> i64 {
    let rule = serde_json::json!({
        "description": "r100 tier rule",
        "branches": [{ "condition": "x >= 5", "result": "big" }],
        "default_branch": "small"
    });
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO knowledge (title, content, content_hash, source, domain, owner, access_scope, node_kind)
         VALUES ('tier rule', ?1, 'h-decision', 'structured', 'global', ?2, 'private', 'decision')",
        rusqlite::params![rule.to_string(), owner],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn entity_id(pool: &brain_server::Pool, name: &str) -> i64 {
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO entities (name) VALUES (?1)",
        rusqlite::params![name],
    )
    .unwrap();
    conn.query_row(
        "SELECT id FROM entities WHERE name = ?1",
        rusqlite::params![name],
        |r| r.get(0),
    )
    .unwrap()
}

fn seed_edge(pool: &brain_server::Pool, from: &str, to: &str, rel: &str, knowledge_id: i64) {
    let f = entity_id(pool, from);
    let t = entity_id(pool, to);
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO relationships (from_entity_id, to_entity_id, relation_type, knowledge_id)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![f, t, rel, knowledge_id],
        )
        .unwrap();
}

// ── the pins ────────────────────────────────────────────────────────────

/// Legacy `/search` is owner-bound: a no-role read JWT finds its own private
/// row and never another subject's; admin sees both; an empty subject sees
/// neither (the empty permit, not a dropped filter).
#[tokio::test]
async fn legacy_search_is_owner_bound() {
    let srv = build_server();
    let a = mint(&srv, "r100-sa", "user:a", &["read:team-a/*"], &[]);
    let b = mint(&srv, "r100-sb", "user:b", &["read:team-a/*"], &[]);
    let admin = mint(&srv, "r100-ad", "user:admin", &["admin:team-a/*"], &[]);
    let blank = mint(&srv, "r100-bl", "   ", &["read:team-a/*"], &[]);
    let own = seed_private_row(&srv.state.pool, "user:a", "kite notes own private");
    let foreign = seed_private_row(&srv.state.pool, "user:b", "kite notes foreign private");

    let ids = |v: &serde_json::Value| -> Vec<i64> {
        v["results"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r["id"].as_i64())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let (st, v, _) = get(&srv, &a, "/search?q=kite+notes").await;
    assert_eq!(st, StatusCode::OK);
    let got = ids(&v);
    assert!(
        got.contains(&own),
        "own private row surfaces for its owner: {got:?}"
    );
    assert!(
        !got.contains(&foreign),
        "a foreign private row must not surface in legacy search: {got:?}"
    );

    let (st, v, _) = get(&srv, &b, "/search?q=kite+notes").await;
    assert_eq!(st, StatusCode::OK);
    assert!(ids(&v).contains(&foreign), "the owner sees their own row");

    let (st, v, _) = get(&srv, &admin, "/search?q=kite+notes").await;
    assert_eq!(st, StatusCode::OK);
    let got = ids(&v);
    assert!(
        got.contains(&own) && got.contains(&foreign),
        "admin reads stay unrestricted: {got:?}"
    );

    let (st, v, _) = get(&srv, &blank, "/search?q=kite+notes").await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        ids(&v).is_empty(),
        "an empty subject reads no private rows (fail closed, filter not dropped): {v}"
    );
}

/// The graph trio is owner-bound at the knowledge JOIN: edges whose chunk is
/// a foreign private row are invisible on `/graph/entity`, `/graph/relations`,
/// and `/graph/traverse` (the walk must not route THROUGH them either).
#[tokio::test]
async fn graph_reads_are_owner_bound() {
    let srv = build_server();
    let a = mint(&srv, "r100-ga", "user:a", &["read:team-a/*"], &[]);
    let b = mint(&srv, "r100-gb", "user:b", &["read:team-a/*"], &[]);
    let blank = mint(&srv, "r100-gbl", "   ", &["read:team-a/*"], &[]);
    let own_chunk = seed_private_row(&srv.state.pool, "user:a", "graph own chunk");
    let foreign_chunk = seed_private_row(&srv.state.pool, "user:b", "graph foreign chunk");
    seed_edge(&srv.state.pool, "alpha", "beta", "ownlink", own_chunk);
    seed_edge(
        &srv.state.pool,
        "alpha",
        "gamma",
        "foreignlink",
        foreign_chunk,
    );

    // /graph/entity/{name}: own edges listed, foreign edges absent.
    let (st, v, _) = get(&srv, &a, "/graph/entity/alpha").await;
    assert_eq!(st, StatusCode::OK, "entity read itself succeeds");
    let names: Vec<&str> = v["relations"]
        .as_array()
        .map(|r| r.iter().filter_map(|x| x["to_entity"].as_str()).collect())
        .unwrap_or_default();
    assert!(names.contains(&"beta"), "own edge surfaces: {names:?}");
    assert!(
        !names.contains(&"gamma"),
        "foreign edge must not: {names:?}"
    );

    // /graph/relations?from=: same binding.
    let (st, v, _) = get(&srv, &a, "/graph/relations?from=alpha").await;
    assert_eq!(st, StatusCode::OK);
    let names: Vec<&str> = v["relations"]
        .as_array()
        .map(|r| r.iter().filter_map(|x| x["entity"].as_str()).collect())
        .unwrap_or_default();
    assert!(names.contains(&"beta"), "own edge surfaces: {names:?}");
    assert!(
        !names.contains(&"gamma"),
        "foreign edge must not: {names:?}"
    );

    // /graph/traverse: the walk may not route through a foreign private edge.
    let (st, v, raw) = get(&srv, &a, "/graph/traverse?start=alpha&max_depth=2").await;
    assert_eq!(st, StatusCode::OK);
    assert!(raw.contains("beta"), "own edge is walkable: {raw}");
    assert!(
        !raw.contains("gamma"),
        "a foreign private edge must yield neither the edge nor derived text: {raw}"
    );
    let _ = v;

    // The foreign row's owner sees their own edge on every surface.
    let (st, _, raw) = get(&srv, &b, "/graph/traverse?start=alpha&max_depth=2").await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        raw.contains("gamma"),
        "the owner walks their own edge: {raw}"
    );

    // Empty subject: the empty permit must not degrade to unrestricted.
    let (st, v, _) = get(&srv, &blank, "/graph/relations?from=alpha").await;
    assert_eq!(st, StatusCode::OK);
    let count = v["relations"].as_array().map(|a| a.len()).unwrap_or(0);
    assert_eq!(count, 0, "empty subject reads no private edges: {v}");
}

/// `/decision/{id}/evaluate` loads a stored rule — the same belt-and-braces
/// row gate as `/procedure/{id}/steps`: a foreign private decision rule is
/// probe-blind 404; its owner evaluates it.
#[tokio::test]
async fn decision_evaluate_is_owner_bound() {
    let srv = build_server();
    let a = mint(&srv, "r100-da", "user:a", &["read:team-a/*"], &[]);
    let b = mint(&srv, "r100-db", "user:b", &["read:team-a/*"], &[]);
    let id = seed_decision_row(&srv.state.pool, "user:b");

    let (st, body) = post_json(
        &srv,
        &a,
        &format!("/decision/{id}/evaluate"),
        r#"{"variables": {"x": 10}}"#,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "a foreign private decision rule must be probe-blind: {body}"
    );

    let (st, body) = post_json(
        &srv,
        &b,
        &format!("/decision/{id}/evaluate"),
        r#"{"variables": {"x": 10}}"#,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "the owner evaluates their rule: {body}");
    assert_eq!(body["result"], "big");
}
