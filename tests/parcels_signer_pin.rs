//! v1.28.67 "Pin" (X-C1) — the parcel import seam names its counterparty:
//!
//! * `expected_signer` is REQUIRED — a missing publisher is the named 400
//!   `signer_required`, never a silent `None` that skips the identity gate.
//! * with a local operator key configured, an `expected_signer` aliasing
//!   OUR did for a FOREIGN-produced parcel refuses 409 `signer_alias`
//!   (anti-aliasing: nobody imports parcels "from us" that we did not
//!   produce).
//!
//! Driven through the composed `app(state)` — the same harness as the
//! authz matrix — because both behaviors are wire-visible error codes.

use axum::{body::Body, http::Request, http::StatusCode};
use r2d2_sqlite::SqliteConnectionManager;
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
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );

    let priv_key = rsa_keypair(&dir.path().join("keys"));
    let key_store =
        brain_server::auth::jwks::KeyStore::load(&dir.path().join("keys")).expect("load test keys");
    let jwt_issuer = "https://brain.matrix/".to_string();
    let jwt_audience = "brain-server".to_string();

    let jwt_middleware_state = Arc::new(brain_server::server::router::auth::JwtMiddlewareState {
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

fn mint(srv: &TestServer) -> String {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = brain_server::auth::jwt::Claims {
        iss: "https://brain.matrix/".to_string(),
        aud: "brain-server".to_string(),
        sub: "matrix-agent".to_string(),
        jti: "parcels-pin-jti".to_string(),
        iat: now,
        nbf: now,
        exp: now + 600,
        tenant: "team-a".to_string(),
        scopes: vec!["admin:*/*".to_string()],
        roles: vec!["admin".to_string()],
        manages: Vec::new(),
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("matrix-kid".to_string());
    let pem = srv.priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    encode(&header, &claims, &encoding).unwrap()
}

async fn post_import(srv: &TestServer, token: &str, body: &str) -> (StatusCode, String) {
    let app = brain_server::server::router::app(srv.state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/parcels/import")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// The env these tests touch (BRAIN_UMP_KEY_DIR) is process-global — the
/// tests serialize on one ASYNC mutex (a std guard would hold across the
/// oneshot awaits), the documented posture made await-safe.
async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// A 0600 operator seed dir + the did it resolves to.
struct OperatorKey(tempfile::TempDir);
impl OperatorKey {
    fn new(seed: [u8; 32]) -> OperatorKey {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("operator.key"), seed).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.path().join("operator.key"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        // SAFETY: single-threaded under env_lock().
        unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", dir.path()) };
        OperatorKey(dir)
    }
    fn did(&self) -> String {
        let (_, sk) = brain_server::handlers::ump::operator_signing_key().unwrap();
        brain_server::ump_integrity::did_key_from_ed25519(&sk.verifying_key().to_bytes())
    }
}
impl Drop for OperatorKey {
    fn drop(&mut self) {
        // SAFETY: single-threaded under env_lock().
        unsafe { std::env::remove_var("BRAIN_UMP_KEY_DIR") };
    }
}

/// parcel_import_requires_signer — a body WITHOUT `expected_signer` is the
/// named 400 `signer_required` (the migration note: name your counterparty);
/// the same body WITH one passes the new gate and reaches the parcel
/// verification vocabulary.
#[tokio::test]
async fn parcel_import_requires_signer() {
    let _g = env_lock().await;
    // empty key dir: the alias check must be OUT of the way for the control
    let _empty = OperatorKey::new([1u8; 32]);
    std::fs::remove_dir_all(_empty.0.path()).unwrap(); // truly keyless

    let srv = build_server();
    let token = mint(&srv);

    // Missing expected_signer → 400 signer_required.
    let (status, body) = post_import(
        &srv,
        &token,
        r#"{"domain":"global","parcel":{"manifest":{},"signature":"s","signed_by":"did:key:z6Mk"}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("signer_required"), "{body}");

    // Control: WITH expected_signer the request reaches the parcel core and
    // fails LATER, in its own vocabulary (expected ≠ claimed → signer_mismatch).
    let (status, body) = post_import(
        &srv,
        &token,
        r#"{"domain":"global","parcel":{"manifest":{},"signature":"s","signed_by":"did:key:z6Mk"},"expected_signer":"did:key:zOther"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("signer_mismatch"), "{body}");
}

/// signer_alias_refused — with a local operator key configured, an
/// `expected_signer` equal to the LOCAL did on a parcel produced by a
/// DIFFERENT did refuses 409 `signer_alias` before any byte is trusted.
#[tokio::test]
async fn signer_alias_refused() {
    let _g = env_lock().await;
    let key = OperatorKey::new([7u8; 32]);
    let local_did = key.did();

    let srv = build_server();
    let token = mint(&srv);

    let payload = format!(
        r#"{{"domain":"global","parcel":{{"manifest":{{}},"signature":"s","signed_by":"did:key:zForeign"}},"expected_signer":"{local_did}"}}"#
    );
    let (status, body) = post_import(&srv, &token, &payload).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("signer_alias"), "{body}");

    // A parcel genuinely produced by the local did (roundtrip import) is NOT
    // aliased — it proceeds past the alias gate into verification vocabulary
    // (the bogus signature fails there, which is the point: the gate let it through).
    let payload = format!(
        r#"{{"domain":"global","parcel":{{"manifest":{{}},"signature":"s","signed_by":"{local_did}"}},"expected_signer":"{local_did}"}}"#
    );
    let (status, body) = post_import(&srv, &token, &payload).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.contains("parcel_unsigned")
            || body.contains("signer_mismatch")
            || body.contains("parcel_tampered"),
        "a self-parcel must reach the verification core, not the alias gate: {body}"
    );
}
