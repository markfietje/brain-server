//! R103 — the role-less seam. `authorize_role` returned `Ok(())` for every
//! role-less principal, so `BRAIN_RBAC_ROLELESS_POSTURE=deny` was resolved,
//! validated, printed at boot and echoed by `/ops/authz/explain` — and read by
//! no enforcement path. The knob was a reported configuration, not an
//! authorization input.
//!
//! Two guards, one seam (`src/handlers/mod.rs::authorize_role`):
//!
//! 1. **The posture is finally an input.** Under `deny`, a role-less principal
//!    is refused at every role gate. `pass` (the shipped default) is
//!    byte-identical to before, and an opaque/loopback principal (`None`) is
//!    untouched in both postures.
//! 2. **Approval is a role act.** `approve`/`reject` are refused for a
//!    role-less principal under BOTH postures: a claim-less token cannot
//!    promote the proposal it just proposed. The seeded `agent` role already
//!    excludes `approve`, so this closes the claim-less gap and nothing else.
//!
//! Guard 2 also closes the B03 chain end-to-end: propose → self-approve with
//! no human in the loop, quorum defaulting to 1, digest principal-independent.
//!
//! ## Why the env is moved under a lock
//!
//! `BRAIN_RBAC_ROLELESS_POSTURE` is process-global and integration tests in one
//! binary share a process. Every test here takes `POSTURE_LOCK` and sets the
//! posture it needs, so the file is order-independent and race-free. The lock is
//! a **tokio** mutex, not a std one: the async pins hold it across `.await`, and
//! a std `MutexGuard` across an await is both a deadlock and a clippy
//! `await_holding_lock` error.

use axum::{body::Body, http::Request, http::StatusCode};
use brain_server::pool::SqliteConnectionManager;
use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

// ── the posture seam under test ─────────────────────────────────────────

static POSTURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Serializes on the process-global env var, sets the posture, and hands the
/// guard back: dropping the guard releases the lock.
async fn posture(v: &str) -> tokio::sync::MutexGuard<'static, ()> {
    let guard = POSTURE_LOCK.lock().await;
    // SAFETY: every holder of POSTURE_LOCK sets this before any read of it,
    // and no test reads the posture without the lock.
    unsafe { std::env::set_var("BRAIN_RBAC_ROLELESS_POSTURE", v) };
    guard
}

fn clear_posture() {
    // SAFETY: same lock discipline as `posture` — the caller still holds the
    // lock's guard.
    unsafe { std::env::remove_var("BRAIN_RBAC_ROLELESS_POSTURE") };
}

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
    std::fs::write(key_dir.join("r103-kid.pem"), pub_pem.as_bytes()).unwrap();
    let key_path = key_dir.join("r103-kid.key");
    std::fs::write(&key_path, priv_pem.as_bytes()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    priv_key
}

/// Presets only: this round changes what a role-LESS principal may do, and a
/// fixture role would be a second variable the pins cannot isolate.
fn seed_presets(pool: &brain_server::Pool) {
    let conn = pool.get().expect("conn");
    for (name, json) in brain_server::role::PRESETS_RAW {
        conn.execute(
            "INSERT OR IGNORE INTO roles(name, json) VALUES (?1, ?2)",
            rusqlite::params![name, json],
        )
        .unwrap();
    }
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
    seed_presets(&pool);
    let model: Arc<dyn brain_server::embed::Embedder> = Arc::new(
        brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).expect("model"),
    );

    let priv_key = rsa_keypair(&dir.path().join("keys"));
    let key_store =
        brain_server::auth::jwks::KeyStore::load(&dir.path().join("keys")).expect("load test keys");
    let jwt_issuer = "https://brain.r103/".to_string();
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
        iss: "https://brain.r103/".to_string(),
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
    header.kid = Some("r103-kid".to_string());
    let pem = srv.priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let encoding = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    encode(&header, &claims, &encoding).unwrap()
}

/// A role-less JWT principal, constructed directly (the unit-level seam).
fn roleless_principal(sub: &str) -> brain_server::auth::Principal {
    brain_server::auth::Principal {
        sub: sub.to_string(),
        tenant: "team-a".to_string(),
        scopes: vec![brain_server::auth::Scope::parse("write:team-a/*").unwrap()],
        jti: "r103-unit".to_string(),
        roles: vec![],
        manages: vec![],
        kind: brain_server::auth::PrincipalKind::Jwt,
    }
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
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn proposal_status(srv: &TestServer, id: i64) -> String {
    srv.state
        .pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM proposals WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
}

fn knowledge_count(srv: &TestServer) -> i64 {
    srv.state
        .pool
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
        .unwrap()
}

/// The `content_digest` the reviewer was shown — derived by the server's own
/// public fingerprint over the stored row, so the pin binds the approval to
/// the bytes that were rendered. Without it the approve route answers
/// `400 digest_required`, which would prove the digest gate, not the role gate.
fn proposal_digest(srv: &TestServer, id: i64) -> String {
    let content: String = srv
        .state
        .pool
        .get()
        .unwrap()
        .query_row(
            "SELECT content FROM proposals WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap();
    brain_server::handlers::gate::review_digest(&content)
}

/// Propose as a role-less write principal, then approve as the same principal
/// — carrying the digest, so the approval is refused (or, before the fix,
/// granted) on the merits of the ROLE, never on a digest mismatch.
async fn propose_then_approve(
    srv: &TestServer,
    token: &str,
) -> (StatusCode, i64, serde_json::Value) {
    let (st, body) = send(
        srv,
        token,
        "/ingest/proposal",
        r#"{"content":"the operator should decide this","kind":"fact"}"#,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "the propose leg must reach the queue: {body}"
    );
    let id = body["proposal_id"]
        .as_i64()
        .or_else(|| body["id"].as_i64())
        .unwrap_or_else(|| panic!("no proposal id in {body}"));
    let digest = proposal_digest(srv, id);
    let (approve_status, approve_body) = send(
        srv,
        token,
        &format!("/proposals/{id}/approve?digest={digest}"),
        "{}",
    )
    .await;
    (approve_status, id, approve_body)
}

// ── the pins ────────────────────────────────────────────────────────────

/// The B03 chain, end to end: a role-less write token proposes, then approves
/// its own proposal. Zero humans, quorum 1, digest principal-independent — the
/// approval must 403 and the proposal must still be pending.
#[tokio::test]
async fn roleless_jwt_cannot_self_approve_its_own_proposal() {
    let srv = build_server();
    let guard = posture("pass").await;
    let tok = mint(&srv, "r103-self", "user:self", &["write:team-a/*"], &[]);
    let before = knowledge_count(&srv);

    let (approve_status, id, approve_body) = propose_then_approve(&srv, &tok).await;

    assert_eq!(
        approve_status,
        StatusCode::FORBIDDEN,
        "a role-less principal must not approve — the self-approval chain is the \
         whole point of the guard: {approve_body}"
    );
    assert_eq!(
        proposal_status(&srv, id),
        "pending",
        "a refused approval must leave the proposal pending"
    );
    assert_eq!(
        knowledge_count(&srv),
        before,
        "a refused approval must not promote anything"
    );
    clear_posture();
    drop(guard);
}

/// Rejection is the same act with the other verb: refuse it for a claim-less
/// principal too, under the shipped default posture.
#[tokio::test]
async fn roleless_jwt_cannot_reject_a_proposal() {
    let srv = build_server();
    let guard = posture("pass").await;
    let tok = mint(&srv, "r103-rej", "user:rej", &["write:team-a/*"], &[]);
    let (_, body) = send(
        &srv,
        &tok,
        "/ingest/proposal",
        r#"{"content":"not this one","kind":"fact"}"#,
    )
    .await;
    let id = body["proposal_id"]
        .as_i64()
        .or_else(|| body["id"].as_i64())
        .unwrap_or_else(|| panic!("no proposal id in {body}"));

    let (st, _) = send(&srv, &tok, &format!("/proposals/{id}/reject"), "{}").await;

    assert_eq!(st, StatusCode::FORBIDDEN, "reject is an approval-class act");
    assert_eq!(proposal_status(&srv, id), "pending");
    clear_posture();
    drop(guard);
}

/// `pass` is the shipped default and it is load-bearing: every NON-approval
/// role gate still passes for a role-less principal, byte-identical to before.
#[tokio::test]
async fn pass_posture_leaves_non_approval_role_gates_open() {
    let srv = build_server();
    let guard = posture("pass").await;
    let p = Some(roleless_principal("user:compat"));
    for cap in [
        "workflow",
        "calibrate",
        "dsar_export",
        "publish",
        "purge",
        "retract",
    ] {
        assert!(
            brain_server::handlers::authorize_role(&p, &srv.state.pool, cap).is_ok(),
            "pass posture must not narrow the non-approval gates (cap `{cap}`)"
        );
    }
    clear_posture();
    drop(guard);
}

/// The knob: under `deny`, a role-less principal is refused at EVERY role gate,
/// and its record-read gate matches nothing.
#[tokio::test]
async fn deny_posture_refuses_every_role_gate_for_a_roleless_principal() {
    let srv = build_server();
    let guard = posture("deny").await;
    let p = Some(roleless_principal("user:denied"));

    for cap in [
        "approve",
        "reject",
        "workflow",
        "calibrate",
        "dsar_export",
        "publish",
        "purge",
    ] {
        let err = brain_server::handlers::authorize_role(&p, &srv.state.pool, cap)
            .expect_err("deny posture must refuse the role gate");
        assert_eq!(
            err.status,
            StatusCode::FORBIDDEN,
            "cap `{cap}` must be a 403, not a 5xx"
        );
    }

    let gate = brain_server::handlers::gate::record_read_gate(&p, &srv.state.pool);
    for (owner, scope) in [
        (Some("user:denied".to_string()), Some("private".to_string())),
        (Some("other".to_string()), Some("domain".to_string())),
        (None, None),
    ] {
        assert!(
            !gate.admits(&owner, &scope),
            "deny posture must match no record at all (owner={owner:?} scope={scope:?})"
        );
    }
    clear_posture();
    drop(guard);
}

/// The two principals the knob must never touch: the opaque/loopback
/// superuser (`None`) and a role-bearing principal whose role carries the
/// capability. Both keep working under `deny`.
#[tokio::test]
async fn deny_posture_leaves_loopback_and_role_bearing_principals_alone() {
    let srv = build_server();
    let guard = posture("deny").await;

    for cap in ["approve", "purge", "dsar_export", "calibrate"] {
        assert!(
            brain_server::handlers::authorize_role(&None, &srv.state.pool, cap).is_ok(),
            "the opaque/loopback principal is not role-gated (cap `{cap}`)"
        );
    }
    let gate = brain_server::handlers::gate::record_read_gate(&None, &srv.state.pool);
    assert!(
        gate.admits(&Some("anyone".to_string()), &Some("private".to_string())),
        "loopback reads stay unrestricted under deny"
    );

    // `solo` is the preset that carries every operator capability; the
    // workflow capability lives on its own preset (`workflow-operator`), so
    // the two are asserted separately rather than merged into one role.
    let mut solo = roleless_principal("user:solo");
    solo.roles = vec!["solo".to_string()];
    let solo = Some(solo);
    for cap in [
        "approve",
        "reject",
        "purge",
        "dsar_export",
        "calibrate",
        "admin",
    ] {
        assert!(
            brain_server::handlers::authorize_role(&solo, &srv.state.pool, cap).is_ok(),
            "a role that carries `{cap}` passes under deny"
        );
    }
    let mut operator = roleless_principal("user:wf");
    operator.roles = vec!["workflow-operator".to_string()];
    let operator = Some(operator);
    assert!(
        brain_server::handlers::authorize_role(&operator, &srv.state.pool, "workflow").is_ok(),
        "the workflow capability passes under deny for a role that carries it"
    );
    clear_posture();
    drop(guard);
}

/// Approval is refused under BOTH postures — the self-approval guard is not a
/// `deny`-only behavior a deployment can opt back into.
#[tokio::test]
async fn approval_is_refused_under_both_postures() {
    let srv = build_server();
    for value in ["pass", "deny"] {
        let guard = posture(value).await;
        let p = Some(roleless_principal("user:both"));
        for cap in ["approve", "reject"] {
            let err = brain_server::handlers::authorize_role(&p, &srv.state.pool, cap)
                .expect_err("approval is a role act under every posture");
            assert_eq!(
                err.status,
                StatusCode::FORBIDDEN,
                "posture {value}, cap {cap}"
            );
        }
        clear_posture();
        drop(guard);
    }
}

/// Route-level: under `deny`, the role-less token that proposes cannot approve
/// — and neither can it at the surface that reads the review queue.
#[tokio::test]
async fn deny_posture_refuses_the_approval_route_end_to_end() {
    let srv = build_server();
    let guard = posture("deny").await;
    let tok = mint(
        &srv,
        "r103-deny-rt",
        "user:deny-rt",
        &["write:team-a/*"],
        &[],
    );
    let (st, _, _) = propose_then_approve(&srv, &tok).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    clear_posture();
    drop(guard);
}

/// Neighbouring behaviour: `pass` still binds a role-less READ to its own
/// rows (the R85 seam) and still admits its own private row through the gate.
#[tokio::test]
async fn pass_posture_keeps_the_owner_bound_read_gate() {
    let srv = build_server();
    let guard = posture("pass").await;
    let p = Some(roleless_principal("user:owner"));
    let gate = brain_server::handlers::gate::record_read_gate(&p, &srv.state.pool);
    assert!(
        gate.admits(
            &Some("user:owner".to_string()),
            &Some("private".to_string())
        ),
        "own private row still passes under the shipped posture"
    );
    assert!(
        !gate.admits(
            &Some("someone-else".to_string()),
            &Some("private".to_string())
        ),
        "a foreign private row still fails under the shipped posture"
    );
    clear_posture();
    drop(guard);
}
