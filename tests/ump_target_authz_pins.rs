//! R93: the UMP any-id mutations (`/ump/revise`, `/ump/forget`,
//! `/ump/feedback`) are target-authorized — a principal may only mutate rows
//! its record gate admits (the same `(owner, access_scope)` pair the read
//! seam enforces), and hard forget additionally requires the `/purge`
//! destructive authority (Admin + the `purge` role capability; a capability
//! bearer can never reach it — no admin verb exists in the §5.2 vocabulary).
//!
//! Route-level pins through the composed `app(state)` with distinct
//! authenticated subjects and rows, per the round plan: the first foreign-row
//! test fails against the vulnerable handler (red-first), and every denial
//! pins the database state as unchanged.

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
    std::fs::write(key_dir.join("r93-kid.pem"), pub_pem.as_bytes()).unwrap();
    let key_path = key_dir.join("r93-kid.key");
    std::fs::write(&key_path, priv_pem.as_bytes()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    priv_key
}

/// The custom roles the pins need: `r93-purger` (carries the destructive
/// `purge` capability), `r93-nopurge` (deliberately without it), and
/// `self-keeper` (the `self` owner filter for the role-reach pin). Presets
/// ride along so role resolution sees the production store shape.
fn seed_roles(pool: &brain_server::Pool) {
    let conn = pool.get().expect("conn");
    for (name, json) in brain_server::role::PRESETS_RAW {
        conn.execute(
            "INSERT OR IGNORE INTO roles(name, json) VALUES (?1, ?2)",
            rusqlite::params![name, json],
        )
        .unwrap();
    }
    let fixture = |name: &str, owner_filter: &str, can: Vec<&str>| -> brain_server::role::Role {
        let role = brain_server::role::Role {
            name: name.to_string(),
            description: Some("r93 fixture".to_string()),
            scopes: vec![
                "private".to_string(),
                "domain".to_string(),
                "team".to_string(),
            ],
            owner_filter: owner_filter.to_string(),
            can: can.into_iter().map(String::from).collect(),
            panels_default: None,
            panels_hidden: None,
            tools_allowed: Some(vec!["*".to_string()]),
        };
        brain_server::role::validate(&role).expect("fixture role must validate");
        brain_server::role::upsert(&conn, &role).expect("seed fixture role");
        role
    };
    fixture(
        "r93-purger",
        "all",
        vec!["read", "write", "reject", "purge"],
    );
    fixture("r93-nopurge", "all", vec!["read", "write", "reject"]);
    fixture("self-keeper", "self", vec!["read", "write"]);
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
    let jwt_issuer = "https://brain.r93/".to_string();
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

/// Mint a signed access token (the authz-matrix shape).
fn mint(srv: &TestServer, jti: &str, sub: &str, scopes: &[&str], roles: &[&str]) -> String {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = brain_server::auth::jwt::Claims {
        iss: "https://brain.r93/".to_string(),
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
    header.kid = Some("r93-kid".to_string());
    let pem = srv.priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    encode(&header, &claims, &encoding).unwrap()
}

/// An operator signing key installed at `BRAIN_UMP_KEY_DIR` for the whole
/// binary (the dir is `keep()`d, never dropped — the env is process-global
/// and only the capability tests read it; no other test in this binary
/// moves it, so no lock is needed).
fn operator_key_env() -> &'static Path {
    static KEY_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    KEY_DIR.get_or_init(|| {
        let dir = tempfile::TempDir::new().expect("temp key dir");
        let path = dir.path().join("operator.key");
        std::fs::write(&path, [7u8; 32]).expect("operator seed");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let kept = dir.keep();
        // SAFETY: set once, before any capability test runs; never removed.
        unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", &kept) };
        kept
    })
}

/// A §5.2 write-verb capability token (no jti — legacy expiry-only form, so
/// the process replay cache does not pin it to one endpoint across tests).
fn capability_token(verbs: &[&str]) -> String {
    operator_key_env();
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    let cap = brain_server::ump_integrity::CapabilityToken::new(
        "EdDSA",
        "did:key:r93",
        verbs,
        Some("global"),
        exp,
    );
    brain_server::ump_integrity::mint_capability_token(&cap, &sk).expect("mint capability")
}

// ── row seeding + state probes ──────────────────────────────────────────

fn seed_private_row(pool: &brain_server::Pool, owner: &str, content: &str) -> i64 {
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO knowledge (title, content, content_hash, source, domain, owner, access_scope)
         VALUES (?1, ?2, ?3, 'structured', 'global', ?4, 'private')",
        rusqlite::params![content, content, format!("h-{content}"), owner],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn knowledge_count(pool: &brain_server::Pool) -> i64 {
    pool.get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
        .unwrap()
}

fn row_state(pool: &brain_server::Pool, id: i64) -> (i64, String) {
    // (exists, content)
    pool.get()
        .unwrap()
        .query_row(
            "SELECT content FROM knowledge WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get::<_, String>(0),
        )
        .map(|c| (1, c))
        .unwrap_or((0, String::new()))
}

fn flagged(pool: &brain_server::Pool, id: i64) -> bool {
    pool.get()
        .unwrap()
        .query_row(
            "SELECT flagged FROM knowledge WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get::<_, i64>(0),
        )
        .map(|f| f != 0)
        .unwrap_or(false)
}

fn tombstone_count(pool: &brain_server::Pool, id: i64) -> i64 {
    pool.get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM tombstones WHERE knowledge_id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
}

fn feedback_count(pool: &brain_server::Pool, id: i64) -> i64 {
    pool.get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM suggest_feedback WHERE chunk_id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
}

fn hold(pool: &brain_server::Pool, ids: &[i64]) {
    let mut conn = pool.get().unwrap();
    let tx = conn.transaction().unwrap();
    brain_server::legal_hold::insert_holds(&tx, ids, "litigation 2026-118", Some("dpo"), 60)
        .expect("seed hold");
    tx.commit().unwrap();
}

// ── request helper ──────────────────────────────────────────────────────

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
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ── the pins ────────────────────────────────────────────────────────────

/// A write-scoped principal revises its own row and CANNOT revise, supersede,
/// or even probe a foreign private row — the mutation surface is bound by the
/// same record gate the read seam enforces, probe-blind (404).
#[tokio::test]
async fn revise_is_target_authorized() {
    let srv = build_server();
    let a_tok = mint(&srv, "r93-a", "user:a", &["write:team-a/*"], &[]);
    let a2_tok = mint(&srv, "r93-a2", "user:a", &["write:team-a/*"], &[]);
    let own = seed_private_row(&srv.state.pool, "user:a", "own private content alpha");
    let foreign = seed_private_row(&srv.state.pool, "user:b", "foreign private content beta");

    // Own row revises: a new revision chunk lands, the old one stays.
    let (st, body) = send(
        &srv,
        &a_tok,
        "/ump/revise",
        &format!(r#"{{"id": "{own}", "patch": {{"body": {{"text": "revised by owner a"}}}}}}"#),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "own-row revise succeeds: {body}");
    assert!(
        body["supersedes"].is_array(),
        "the revise response names its supersession: {body}"
    );
    let (exists, content) = row_state(&srv.state.pool, own);
    assert_eq!((exists, content.as_str()), (1, "own private content alpha"));
    assert!(
        knowledge_count(&srv.state.pool) >= 3,
        "the revision created a new chunk"
    );

    // Foreign row: refused probe-blind, nothing mutated anywhere.
    let before = knowledge_count(&srv.state.pool);
    let (st, body) = send(
        &srv,
        &a2_tok,
        "/ump/revise",
        &format!(r#"{{"id": "{foreign}", "patch": {{"body": {{"text": "laundered revision"}}}}}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "foreign-row revise must be denied probe-blind: {body}"
    );
    assert_eq!(
        row_state(&srv.state.pool, foreign),
        (1, "foreign private content beta".to_string()),
        "the foreign row is byte-identical after the refused revise"
    );
    assert_eq!(
        knowledge_count(&srv.state.pool),
        before,
        "no revision chunk, no supersession — denial left the database unchanged"
    );
}

/// Soft forget is target-authorized: own row tombstones, a foreign private
/// row refuses probe-blind with zero side effects (no flag, no tombstone).
#[tokio::test]
async fn forget_soft_is_target_authorized() {
    let srv = build_server();
    let a_tok = mint(&srv, "r93-sa", "user:a", &["write:team-a/*"], &[]);
    let a2_tok = mint(&srv, "r93-sa2", "user:a", &["write:team-a/*"], &[]);
    let own = seed_private_row(&srv.state.pool, "user:a", "soft forget own content");
    let foreign = seed_private_row(&srv.state.pool, "user:b", "soft forget foreign content");

    let (st, body) = send(
        &srv,
        &a_tok,
        "/ump/forget",
        &format!(r#"{{"id": "{own}", "reason": "owner cleanup"}}"#),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "own-row soft forget succeeds: {body}");
    assert_eq!(body["result"], "tombstoned");
    assert!(flagged(&srv.state.pool, own), "own row is flagged");
    assert_eq!(tombstone_count(&srv.state.pool, own), 1);

    let (st, body) = send(
        &srv,
        &a2_tok,
        "/ump/forget",
        &format!(r#"{{"id": "{foreign}", "reason": "illicit"}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "foreign-row soft forget must be denied probe-blind: {body}"
    );
    assert!(
        !flagged(&srv.state.pool, foreign),
        "no quarantine flag lands on the foreign row"
    );
    assert_eq!(
        tombstone_count(&srv.state.pool, foreign),
        0,
        "no tombstone lands for the refused forget"
    );
}

/// Hard forget is a destructive act: Write scope alone never reaches it —
/// not a write-scoped principal, not an admin-scoped principal whose role
/// lacks `purge`, and never a capability bearer. The documented erase path
/// (admin scopes + a role carrying `purge`) still erases; a legal hold still
/// refuses with the existing 409 vocabulary.
#[tokio::test]
async fn forget_hard_requires_destructive_authority() {
    let srv = build_server();
    let w_tok = mint(&srv, "r93-hw", "user:a", &["write:team-a/*"], &[]);
    let nopurge_tok = mint(
        &srv,
        "r93-hnp",
        "user:a",
        &["admin:team-a/*"],
        &["r93-nopurge"],
    );
    let purge_tok = mint(
        &srv,
        "r93-hp",
        "user:a",
        &["admin:team-a/*"],
        &["r93-purger"],
    );

    // Write scope, own row: refused, row untouched.
    let own = seed_private_row(&srv.state.pool, "user:a", "hard forget target one");
    let (st, body) = send(
        &srv,
        &w_tok,
        "/ump/forget",
        &format!(r#"{{"id": "{own}", "hard": true, "reason": "self erase"}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "a write-only principal cannot hard-erase, even its own row: {body}"
    );
    assert_eq!(row_state(&srv.state.pool, own).0, 1, "row survives");
    assert_eq!(tombstone_count(&srv.state.pool, own), 0);

    // Admin scopes but the role lacks `purge`: still refused (the /purge
    // authority is Admin AND the capability — one without the other is 403).
    let (st, body) = send(
        &srv,
        &nopurge_tok,
        "/ump/forget",
        &format!(r#"{{"id": "{own}", "hard": true, "reason": "role lacks purge"}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "admin scopes without the purge capability must not hard-erase: {body}"
    );
    assert_eq!(row_state(&srv.state.pool, own).0, 1, "row survives");

    // A capability token can never reach hard erase (no admin verb in §5.2).
    let cap = capability_token(&["write", "derive"]);
    let (st, body) = send(
        &srv,
        &cap,
        "/ump/forget",
        &format!(r#"{{"id": "{own}", "hard": true, "reason": "cap erase"}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "a capability bearer cannot reach hard erase: {body}"
    );
    assert_eq!(row_state(&srv.state.pool, own).0, 1, "row survives");

    // The documented erase path: admin scopes + a role carrying `purge`.
    let (st, body) = send(
        &srv,
        &purge_tok,
        "/ump/forget",
        &format!(r#"{{"id": "{own}", "hard": true, "reason": "operator erase"}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "the authorized erase path works: {body}"
    );
    assert_eq!(body["result"], "erased");
    assert_eq!(row_state(&srv.state.pool, own).0, 0, "row is erased");

    // The legal-hold fence still refuses the fully authorized chain (409).
    let held = seed_private_row(&srv.state.pool, "user:a", "litigation evidence row");
    hold(&srv.state.pool, &[held]);
    let (st, body) = send(
        &srv,
        &purge_tok,
        "/ump/forget",
        &format!(r#"{{"id": "{held}", "hard": true, "reason": "held erase"}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "the legal-hold fence still refuses hard erase: {body}"
    );
    assert_eq!(
        body["error"]["code"], "legal_hold_active",
        "the 409 vocabulary"
    );
    assert_eq!(row_state(&srv.state.pool, held).0, 1, "held row survives");
}

/// A capability token keeps its explicitly-allowed soft-forget lane (the
/// operator-signed §5.2 bearer is the documented writable agent) — the
/// destructive refusal above is about `hard`, not about the soft path.
#[tokio::test]
async fn capability_soft_forget_lane_is_unchanged() {
    let srv = build_server();
    let cap = capability_token(&["write"]);
    let row = seed_private_row(&srv.state.pool, "user:b", "capability lane target");
    let (st, body) = send(
        &srv,
        &cap,
        "/ump/forget",
        &format!(r#"{{"id": "{row}", "reason": "agent tombstone"}}"#),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "soft forget stays in the lane: {body}");
    assert_eq!(body["result"], "tombstoned");
}

/// Feedback is an any-id mutation too: it lands on a row the principal's
/// gate admits, and a foreign private row refuses probe-blind with zero
/// rows written (no ranking pollution, no id-oracle).
#[tokio::test]
async fn feedback_is_target_authorized() {
    let srv = build_server();
    let a_tok = mint(&srv, "r93-fa", "user:a", &["write:team-a/*"], &[]);
    let a2_tok = mint(&srv, "r93-fa2", "user:a", &["write:team-a/*"], &[]);
    let own = seed_private_row(&srv.state.pool, "user:a", "feedback own content");
    let foreign = seed_private_row(&srv.state.pool, "user:b", "feedback foreign content");

    let (st, body) = send(
        &srv,
        &a_tok,
        "/ump/feedback",
        &format!(r#"{{"id": "{own}", "outcome": "followed"}}"#),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "own-row feedback succeeds: {body}");
    assert_eq!(feedback_count(&srv.state.pool, own), 1);

    let (st, body) = send(
        &srv,
        &a2_tok,
        "/ump/feedback",
        &format!(r#"{{"id": "{foreign}", "outcome": "followed"}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "foreign-row feedback must be denied probe-blind: {body}"
    );
    assert_eq!(
        feedback_count(&srv.state.pool, foreign),
        0,
        "no feedback row lands for the refused id"
    );
}

/// An absent/empty subject fails closed on every mutation surface (the
/// record gate's empty permit matches nothing — private data is not
/// reachable by a subject-less principal).
#[tokio::test]
async fn empty_subject_fails_closed() {
    let srv = build_server();
    let blank_tok = mint(&srv, "r93-blank", "   ", &["write:team-a/*"], &[]);
    let row = seed_private_row(&srv.state.pool, "user:a", "empty subject target");
    let before = knowledge_count(&srv.state.pool);

    for (path, body) in [
        (
            "/ump/revise",
            format!(r#"{{"id": "{row}", "patch": {{"body": {{"text": "x"}}}}}}"#),
        ),
        (
            "/ump/forget",
            format!(r#"{{"id": "{row}", "reason": "x"}}"#),
        ),
        (
            "/ump/feedback",
            format!(r#"{{"id": "{row}", "outcome": "followed"}}"#),
        ),
    ] {
        let (st, body_out) = send(&srv, &blank_tok, path, &body).await;
        assert_eq!(
            st,
            StatusCode::NOT_FOUND,
            "empty subject must fail closed on {path}: {body_out}"
        );
    }
    assert_eq!(
        knowledge_count(&srv.state.pool),
        before,
        "nothing was written by the subject-less principal"
    );
    assert_eq!(feedback_count(&srv.state.pool, row), 0);
    assert_eq!(tombstone_count(&srv.state.pool, row), 0);
}

/// A role-bearing principal's target reach follows its role policy (the
/// `self` owner filter): own row mutates, foreign row refuses.
#[tokio::test]
async fn role_self_filter_binds_the_mutation_target() {
    let srv = build_server();
    let self_tok = mint(
        &srv,
        "r93-self",
        "user:a",
        &["write:team-a/*"],
        &["self-keeper"],
    );
    let own = seed_private_row(&srv.state.pool, "user:a", "self role own target");
    let foreign = seed_private_row(&srv.state.pool, "user:b", "self role foreign target");

    let (st, _body) = send(
        &srv,
        &self_tok,
        "/ump/forget",
        &format!(r#"{{"id": "{own}", "reason": "own cleanup"}}"#),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "the self role reaches its own row");

    let (st, body) = send(
        &srv,
        &self_tok,
        "/ump/forget",
        &format!(r#"{{"id": "{foreign}", "reason": "overreach"}}"#),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "the self role must not reach a foreign row: {body}"
    );
    assert!(!flagged(&srv.state.pool, foreign));
}
