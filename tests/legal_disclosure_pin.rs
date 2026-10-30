//! The legal-DB refusal's disclosure law: driver text stays in the log.
//!
//! `legal_refusal`'s io/sqlite arm used to pass `LegalDbError::Io/Sqlite`
//! Display verbatim into the 503 body — and those strings embed filesystem
//! paths (`unable to open database file: /home/…/legal.db`). The refusal is
//! still NAMED (`legal_db_unavailable`, unchanged code, unchanged status);
//! what changed is which half carries the diagnosis: the operator's log line
//! holds the driver text, the body holds the named posture and the pointer
//! to `BRAIN_LEGAL_DB_PATH`.
//!
//! Two arms, one route:
//! * the leak arm — an unopenable `BRAIN_LEGAL_DB_PATH` renders a body with
//!   no path-like text, while the captured log line carries the driver text
//!   (the diagnosis is preserved for the operator, never dropped);
//! * the anti-vacuity arm — with the env unset, the curated
//!   `legal_db_unconfigured` refusal still renders its full operator message
//!   verbatim. A fix that silenced named refusals would be the wrong fix.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

/// A `MakeWriter` over a shared buffer, so the pin can assert what the
/// operator's log line carried without touching global subscriber state.
#[derive(Clone)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().expect("buf lock").clone()).expect("log buffer is utf-8")
    }
}

impl std::io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("buf lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedBuf {
    type Writer = SharedBuf;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Full AppState on a fresh temp DB — the standard drill-harness shape
/// (opaque auth, static embedder), self-contained so the pin is one file.
fn legal_state(dir: &tempfile::TempDir) -> Arc<brain_server::AppState> {
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let db_path = dir.path().join("brain.db");
    let mgr = brain_server::pool::SqliteConnectionManager::file(&db_path);
    let pool: brain_server::Pool = r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
    brain_server::migration::run_migration(
        &mut pool.get().unwrap(),
        brain_server::config::DB_MMAP_SIZE_MIB,
    )
    .expect("migration");
    Arc::new(brain_server::AppState {
        token_store: brain_server::auth::TokenStore::new(),
        jwt_middleware_state: Arc::new(brain_server::JwtMiddlewareState::opaque_for_tests(
            pool.clone(),
            db_path.clone(),
        )),
        cors: tower_http::cors::CorsLayer::new(),
        durability: Default::default(),
        loom: Default::default(),
        pool: pool.clone(),
        registry: brain_server::domain_registry::DomainRegistry::new(
            pool.clone(),
            dir.path(),
            false,
        ),
        model: Arc::new(
            brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID)
                .expect("model"),
        ),
        db_path,
        connection_tracker: Arc::new(brain_server::http_limit::ConnectionTracker::new()),
        rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
        snapshot: brain_server::integrity::SnapshotState::default(),
        audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
        auth_mode: brain_server::auth::AuthMode::Opaque,
        key_store: brain_server::auth::jwks::KeyStore::load(std::path::Path::new("/nonexistent"))
            .expect("empty key store"),
        revocation_cache: Arc::new(brain_server::auth::revocation::RevocationCache::new()),
        jwt_issuer: String::new(),
        jwt_audience: String::new(),
        oidc_config: brain_server::handlers::well_known::OidcConfig::unconfigured(),
        ump_events: tokio::sync::broadcast::channel(brain_server::config::UMP_EVENT_BUFFER).0,
        alert_events: tokio::sync::broadcast::channel(brain_server::config::ALERT_EVENT_BUFFER).0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: brain_server::alert::ChainWatchState::default(),
        concurrency: &brain_server::concurrency::CONCURRENCY,
    })
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).to_string()
}

/// The leak arm and the anti-vacuity arm, one sequence (the env var is this
/// file's alone, set and restored around the run).
#[tokio::test]
async fn legal_db_driver_text_never_crosses_into_the_body() {
    let dir = tempfile::TempDir::new().unwrap();
    let state = legal_state(&dir);
    let app = brain_server::server::router::app(state.clone());

    // ARM 1 — an unopenable legal DB: the 503 body is path-free and names
    // its posture; the captured log line carries the driver text (the
    // diagnosis preserved, not dropped).
    let buf = SharedBuf::new();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .finish();
    let secret_path = "/nonexistent/deeply/hidden/legal-pin.db";
    let _guard = tracing::subscriber::set_default(subscriber);
    // SAFETY: BRAIN_LEGAL_DB_PATH is read per-request by the legal handler
    // and by no other test in this binary; set + restore around this arm.
    unsafe { std::env::set_var("BRAIN_LEGAL_DB_PATH", secret_path) };
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/legal/rules")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    let raw = body_string(resp).await;
    // The law is NO DRIVER TEXT — the operator's machine-specific paths and
    // errno strings — not "no slash anywhere": the curated message
    // deliberately names `docs/legal-db-import.md` as the operator pointer.
    assert!(
        !raw.contains(secret_path) && !raw.contains("legal-pin.db"),
        "the configured path must not cross into the body: {raw}"
    );
    assert!(
        !raw.contains("unable to open"),
        "sqlite driver phrasing must not cross into the body: {raw}"
    );
    let v: serde_json::Value = serde_json::from_str(&raw).expect("json body");
    assert_eq!(v["error"]["code"], "legal_db_unavailable");
    assert!(
        v["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("BRAIN_LEGAL_DB_PATH")),
        "the body points the operator at the configured path: {raw}"
    );
    let log = buf.contents();
    assert!(
        log.contains(secret_path),
        "the log must carry the driver text (the diagnosis, preserved): {log}"
    );
    drop(_guard);

    // ARM 2 — anti-vacuity: env unset, the curated unconfigured refusal
    // renders its full operator message verbatim.
    // SAFETY: as above; removed for the unconfigured arm and left removed
    // (the ambient state).
    unsafe { std::env::remove_var("BRAIN_LEGAL_DB_PATH") };
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/legal/rules")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_IMPLEMENTED);
    let raw = body_string(resp).await;
    assert!(
        raw.contains("BRAIN_LEGAL_DB_PATH is unset"),
        "the named unconfigured refusal renders its full operator message: {raw}"
    );
}
