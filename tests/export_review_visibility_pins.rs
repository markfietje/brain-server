//! R101 — export and the review listings disclose to reviewers and owners
//! only.
//!
//! Two related leaks, both on surfaces the read gate never reached:
//!
//! 1. **`GET /export`** redacted `knowledge[].content` and nothing else, so a
//!    foreign row still shipped its title, source, owner, origin and every
//!    other metadata field; and it shipped `bundle.proposals` VERBATIM — full
//!    body, every proposal, of every owner — because the bundle's proposal
//!    projection never read the `owner` column the table has carried since the
//!    QaQueue migration. Entities and relationships leaked the same way: a
//!    relationship names its `knowledge_id`, and the export shipped every
//!    edge regardless of whether the caller may see that memory.
//! 2. **`GET /proposals`** is the human review queue — unapproved memory
//!    bodies, owners and reviewer notes — and it was a plain Read row: every
//!    read-scoped principal read everyone's pending proposals. **`/quarantine`**
//!    and **`/decayed`** are the same shape: flagged and expired content
//!    across the corpus, readable by anyone with Read.
//!
//! The fix reuses what is already wired and adds no mechanism: the record
//! gate's `owner_in` becomes a predicate on the proposal listing (so an
//! owner-bound caller sees its own queue and a role-based reviewer still sees
//! the shared one), and the two operator review scans require the reviewer
//! posture `review_flags_allowed` already carries. In the export, a row the
//! caller may not see is replaced by a `{"redacted": true}` stub and counted,
//! and the graph is narrowed to the edges and entities the surviving rows
//! actually reference.
//!
//! Route-level pins through the composed `app(state)`, each with a distinct
//! authenticated subject, and every denial pins the database as unchanged.

use axum::{body::Body, http::Request, http::StatusCode};
use brain_server::pool::SqliteConnectionManager;
use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

// ── server fixture (the authz-matrix shape) ─────────────────────────────

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
    std::fs::write(key_dir.join("r101-kid.pem"), pub_pem.as_bytes()).unwrap();
    let key_path = key_dir.join("r101-kid.key");
    std::fs::write(&key_path, priv_pem.as_bytes()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    priv_key
}

/// `r101-reviewer` opens the shared pool (`owner_filter: all`) — the class a
/// review desk runs as. Presets ride along so role resolution sees the
/// production store shape.
fn seed_roles(pool: &brain_server::Pool) {
    let conn = pool.get().expect("conn");
    for (name, json) in brain_server::role::PRESETS_RAW {
        conn.execute(
            "INSERT OR IGNORE INTO roles(name, json) VALUES (?1, ?2)",
            rusqlite::params![name, json],
        )
        .unwrap();
    }
    let role = brain_server::role::Role {
        name: "r101-reviewer".to_string(),
        description: Some("r101 fixture".to_string()),
        scopes: vec![
            "private".to_string(),
            "domain".to_string(),
            "team".to_string(),
        ],
        owner_filter: "all".to_string(),
        can: vec![
            "read".to_string(),
            "write".to_string(),
            "approve".to_string(),
            "reject".to_string(),
        ],
        panels_default: None,
        panels_hidden: None,
        tools_allowed: Some(vec!["*".to_string()]),
    };
    brain_server::role::validate(&role).expect("fixture role must validate");
    brain_server::role::upsert(&conn, &role).expect("seed fixture role");
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
    let jwt_issuer = "https://brain.r101/".to_string();
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

/// The opaque operator bearer. In `AuthMode::Opaque` the middleware resolves
/// this to the `None` principal — the loopback/operator class the export and
/// the review scans must keep working byte-identically.
const OP_TOKEN: &str = "r101-operator-token";

/// An opaque-mode server over its own database: the operator path needs a
/// principal-less caller, and the middleware in Jwt mode 401s an unauthenticated
/// request rather than treating it as loopback (which is the honest shape — only
/// an operator-configured bearer is the operator).
fn build_opaque_server() -> TestServer {
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
    std::fs::write(&tok_file, format!("{OP_TOKEN}\n")).expect("token file");
    let token_store = brain_server::auth::TokenStore::from_file(Some(tok_file));
    token_store.reload_parts_from(vec![OP_TOKEN.to_string()], None);
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
    TestServer {
        _dir: dir,
        state,
        priv_key: rsa::RsaPrivateKey::new(&mut rand::rngs::ThreadRng::default(), 2048)
            .expect("key"),
    }
}

fn mint(srv: &TestServer, jti: &str, sub: &str, scopes: &[&str], roles: &[&str]) -> String {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = brain_server::auth::jwt::Claims {
        iss: "https://brain.r101/".to_string(),
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
    header.kid = Some("r101-kid".to_string());
    let pem = srv.priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    encode(&header, &claims, &encoding).unwrap()
}

// ── row seeding ─────────────────────────────────────────────────────────

fn seed_private_row(pool: &brain_server::Pool, owner: &str, title: &str, content: &str) -> i64 {
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO knowledge (title, content, content_hash, source, domain, owner, access_scope, origin)
         VALUES (?1, ?2, ?3, 'structured', 'global', ?4, 'private', 'operator')",
        rusqlite::params![title, content, format!("h-{content}"), owner],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn seed_pending_proposal(pool: &brain_server::Pool, owner: &str, content: &str) -> i64 {
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO proposals(kind, content, owner, created_at, novelty, status)
         VALUES ('fact', ?1, ?2, 1, 0.5, 'pending')",
        rusqlite::params![content, owner],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// An entity + an edge hanging off `chunk_id`, so the export's graph can be
/// checked for foreign edges.
fn seed_entity_edge(pool: &brain_server::Pool, name: &str, chunk_id: i64) -> (i64, i64, i64) {
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO entities(name, entity_type) VALUES (?1, 'concept')",
        rusqlite::params![name],
    )
    .unwrap();
    let from = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO entities(name, entity_type) VALUES (?1, 'concept')",
        rusqlite::params![format!("{name}-hub")],
    )
    .unwrap();
    let to = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO relationships(from_entity_id, to_entity_id, relation_type, knowledge_id)
         VALUES (?1, ?2, 'relates_to', ?3)",
        rusqlite::params![from, to, chunk_id],
    )
    .unwrap();
    (from, to, conn.last_insert_rowid())
}

// ── request helper ──────────────────────────────────────────────────────

async fn get(srv: &TestServer, token: Option<&str>, path: &str) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(axum::http::Method::GET).uri(path);
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let resp = app(srv.state.clone())
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// The serialized export as one string — a leak that hides in a field the
/// assertion forgot to name is still a leak, and this is how a pin sees it.
fn export_text(body: &serde_json::Value) -> String {
    serde_json::to_string(body).unwrap_or_default()
}

// ── the pins ────────────────────────────────────────────────────────────

/// The export is the portability path, so it must still carry everything the
/// CALLER owns — and nothing else. A role-less read token exports its own
/// private row in full (title, source, content, owner) while the foreign row
/// ships as a bare `{"redacted": true}` stub, with the withheld count
/// reported so the envelope never silently under-reports.
#[tokio::test]
async fn export_withholds_foreign_knowledge_metadata() {
    let srv = build_server();
    let own = seed_private_row(
        &srv.state.pool,
        "user:me",
        "My own pillar note",
        "own private content about the launch plan",
    );
    let foreign = seed_private_row(
        &srv.state.pool,
        "user:them",
        "Their private salary band",
        "foreign private content about the merger talks",
    );
    seed_entity_edge(&srv.state.pool, "merger-talks", foreign);

    let tok = mint(&srv, "r101-export", "user:me", &["read:team-a/*"], &[]);
    let (st, body) = get(&srv, Some(&tok), "/export").await;
    assert_eq!(st, StatusCode::OK, "{body}");

    let text = export_text(&body);
    assert!(
        text.contains("own private content about the launch plan"),
        "the caller's own row must export in full — portability is the point"
    );
    for leaked in [
        "merger talks",
        "Their private salary band",
        "foreign private content",
        "user:them",
        "merger-talks",
    ] {
        assert!(
            !text.contains(leaked),
            "the export leaked `{leaked}` to a principal that does not own it: {body}"
        );
    }

    let knowledge = body["knowledge"].as_array().expect("knowledge array");
    assert_eq!(
        knowledge.len(),
        2,
        "both rows are accounted for: one real, one stub — the count is not a licence to drop silently"
    );
    let owned = knowledge
        .iter()
        .find(|k| k["id"].as_i64() == Some(own))
        .expect("own row present");
    assert_eq!(owned["title"], serde_json::json!("My own pillar note"));
    assert_eq!(owned["owner"], serde_json::json!("user:me"));
    assert_eq!(
        owned["access_scope"],
        serde_json::json!("private"),
        "the owner's own row exports with its scope label intact"
    );
    let stubs: Vec<&serde_json::Value> = knowledge
        .iter()
        .filter(|k| k["redacted"] == serde_json::json!(true))
        .collect();
    assert_eq!(
        stubs.len(),
        1,
        "the foreign row ships as exactly one bare stub: {body}"
    );
    assert_eq!(
        stubs[0],
        &serde_json::json!({"redacted": true}),
        "the stub carries nothing — not even the row id, which is itself a \
         handle on a row the caller may not read"
    );
    assert!(
        !knowledge.iter().any(|k| k["id"].as_i64() == Some(foreign)),
        "the foreign row id must not survive anywhere in the export: {body}"
    );
    assert_eq!(
        body["withheld"]["knowledge"],
        serde_json::json!(1),
        "the envelope must report what it withheld"
    );
}

/// Proposals never passed through the redaction at all: the bundle's proposal
/// projection did not even read `owner`. A role-less caller now sees its own
/// pending body in full and everyone else's as a stub.
#[tokio::test]
async fn export_withholds_foreign_proposal_bodies() {
    let srv = build_server();
    seed_pending_proposal(&srv.state.pool, "user:me", "my own pending proposal body");
    seed_pending_proposal(
        &srv.state.pool,
        "user:them",
        "their pending proposal body about the layoff plan",
    );

    let tok = mint(&srv, "r101-prop", "user:me", &["read:team-a/*"], &[]);
    let (st, body) = get(&srv, Some(&tok), "/export").await;
    assert_eq!(st, StatusCode::OK, "{body}");

    let text = export_text(&body);
    assert!(
        text.contains("my own pending proposal body"),
        "the caller's own proposal exports in full"
    );
    assert!(
        !text.contains("layoff plan"),
        "a foreign proposal body reached the export verbatim: {body}"
    );
    assert!(
        !text.contains("user:them"),
        "a foreign proposal owner reached the export: {body}"
    );
    assert_eq!(body["withheld"]["proposals"], serde_json::json!(1));
}

/// The graph rides the same rule: a relationship names its `knowledge_id`, so
/// an edge whose memory the caller cannot see must not ship, and neither must
/// the entities that only that edge referenced.
#[tokio::test]
async fn export_narrows_the_graph_to_visible_rows() {
    let srv = build_server();
    let own = seed_private_row(&srv.state.pool, "user:me", "Mine", "my own row");
    let foreign = seed_private_row(&srv.state.pool, "user:them", "Theirs", "their row");
    seed_entity_edge(&srv.state.pool, "my-topic", own);
    seed_entity_edge(&srv.state.pool, "their-topic", foreign);

    let tok = mint(&srv, "r101-graph", "user:me", &["read:team-a/*"], &[]);
    let (st, body) = get(&srv, Some(&tok), "/export").await;
    assert_eq!(st, StatusCode::OK, "{body}");

    let text = export_text(&body);
    assert!(
        text.contains("my-topic"),
        "the visible row's graph survives"
    );
    assert!(
        !text.contains("their-topic"),
        "an entity that only a foreign edge referenced shipped anyway: {body}"
    );
    let edges = body["relationships"].as_array().expect("relationships");
    assert_eq!(
        edges.len(),
        1,
        "exactly the edge whose knowledge the caller may read: {body}"
    );
}

/// The neighbouring behaviour, and the reason this is a redaction change and
/// not a restriction: the operator export is byte-identical. Loopback/opaque
/// is not an owner-bound caller, so it exports everything it always did.
#[tokio::test]
async fn operator_export_is_unchanged() {
    let srv = build_opaque_server();
    seed_private_row(&srv.state.pool, "user:me", "Mine", "my own row");
    seed_private_row(&srv.state.pool, "user:them", "Theirs", "their row");
    seed_pending_proposal(&srv.state.pool, "user:them", "their pending proposal");

    let (st, body) = get(&srv, Some(OP_TOKEN), "/export").await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let text = export_text(&body);
    assert!(text.contains("my own row"));
    assert!(
        text.contains("their row"),
        "the operator export must not narrow: an owner-bound redaction that reached \
         the loopback path would silently break every backup"
    );
    assert!(text.contains("their pending proposal"));
    for collection in ["knowledge", "proposals", "entities", "relationships"] {
        assert_eq!(
            body["withheld"][collection],
            serde_json::json!(0),
            "nothing is withheld from the operator, so `{collection}` must read zero"
        );
    }
}

/// The review queue is a reviewer surface. An owner-bound read token sees its
/// OWN pending proposals — the ones it could approve nothing of, but can still
/// read — and never another subject's.
#[tokio::test]
async fn proposals_listing_is_owner_bound() {
    let srv = build_server();
    seed_pending_proposal(&srv.state.pool, "user:me", "my own pending proposal");
    seed_pending_proposal(
        &srv.state.pool,
        "user:them",
        "their pending proposal about the layoff plan",
    );

    let tok = mint(&srv, "r101-list", "user:me", &["read:team-a/*"], &[]);
    let (st, body) = get(&srv, Some(&tok), "/proposals?status=pending").await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let text = export_text(&body);
    assert!(
        text.contains("my own pending proposal"),
        "an owner sees its own queue: {body}"
    );
    assert!(
        !text.contains("layoff plan"),
        "the review queue leaked another subject's pending proposal: {body}"
    );
}

/// The neighbouring class: a role whose `owner_filter` opens the shared pool
/// still reviews the whole queue. Restricting the listing must not mean
/// "reviewers see their own queue only".
#[tokio::test]
async fn reviewer_role_still_sees_the_shared_queue() {
    let srv = build_server();
    seed_pending_proposal(&srv.state.pool, "user:me", "my own pending proposal");
    seed_pending_proposal(
        &srv.state.pool,
        "user:them",
        "their pending proposal about the layoff plan",
    );

    let tok = mint(
        &srv,
        "r101-reviewer",
        "user:reviewer",
        &["read:team-a/*", "write:team-a/*"],
        &["r101-reviewer"],
    );
    let (st, body) = get(&srv, Some(&tok), "/proposals?status=pending").await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let text = export_text(&body);
    assert!(text.contains("my own pending proposal"), "{body}");
    assert!(
        text.contains("their pending proposal about the layoff plan"),
        "a shared-pool reviewer role must still see the whole queue: {body}"
    );
}

/// The two operator review scans are review POSTURES, not reads. A read-scoped
/// token gets neither, and the operator keeps both.
#[tokio::test]
async fn review_scans_require_the_reviewer_posture() {
    let srv = build_server();
    let op = build_opaque_server();
    let tok = mint(&srv, "r101-scan", "user:me", &["read:team-a/*"], &[]);

    for path in ["/quarantine", "/decayed?limit=5"] {
        let (st, body) = get(&srv, Some(&tok), path).await;
        assert_eq!(
            st,
            StatusCode::FORBIDDEN,
            "{path} is a reviewer posture, not a read: {body}"
        );
        let (st, body) = get(&op, Some(OP_TOKEN), path).await;
        assert_eq!(
            st,
            StatusCode::OK,
            "the operator keeps the review scans: {path} answered {body}"
        );
    }
}

/// An admin-scoped principal is a reviewer by the posture's own definition
/// (`review_flags_allowed` authorizes Admin on its own tenant) — the scans must
/// not lock out the very operator class the helper was written for.
#[tokio::test]
async fn admin_scoped_principal_keeps_the_review_scans() {
    let srv = build_server();
    let tok = mint(&srv, "r101-admin", "user:admin", &["admin:team-a/*"], &[]);
    for path in ["/quarantine", "/decayed?limit=5"] {
        let (st, body) = get(&srv, Some(&tok), path).await;
        assert_eq!(
            st,
            StatusCode::OK,
            "{path} must answer an admin-scoped caller: {body}"
        );
    }
}
