//! PENETRATION SUITE — adversarial requests against the REAL composed router.
//!
//! Every arm drives `app(state)` with a signed JWT and asserts the security
//! property, not the route's vocabulary: a cross-owner read must not return the
//! victim's bytes, a role that lacks `purge` must not delete, `hard: true` must
//! not erase, and the UMP export must not carry live markup.
//!
//! This is the aggregate adversarial pass over the findings closed in the
//! 2026-10-19 audit: RBAC record-gate widening (reports-with-no-manages and a
//! custom `owner_filter:"all"`), quarantine by-id authz, hard-erase
//! escalation, the `/verify` oracle, DSAR admin, and the UMP read seam. The
//! per-defect pins live beside their subjects; this file proves the attacks
//! end-to-end through the wire a caller actually has.

use axum::{body::Body, http::Request, http::StatusCode};
use brain_server::pool::SqliteConnectionManager;
use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

struct Pen {
    _dir: tempfile::TempDir,
    state: Arc<brain_server::AppState>,
    key: rsa::RsaPrivateKey,
}

fn keypair(dir: &Path) -> rsa::RsaPrivateKey {
    let mut rng = rand::rngs::ThreadRng::default();
    let private = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("test keypair");
    let public = rsa::RsaPublicKey::from(&private);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("pen-kid.pem"),
        public.to_public_key_pem(LineEnding::LF).unwrap(),
    )
    .unwrap();
    let key_path = dir.join("pen-kid.key");
    std::fs::write(
        &key_path,
        private.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    private
}

fn build() -> Pen {
    static ADMIT: std::sync::Once = std::sync::Once::new();
    ADMIT.call_once(|| unsafe { std::env::set_var("BRAIN_ALLOW_WILDCARD_GRANT", "1") });
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("brain.db");
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let pool: brain_server::Pool = r2d2::Pool::builder()
        .max_size(4)
        .build(SqliteConnectionManager::file(&db_path))
        .expect("pool");
    brain_server::migration::run_migration(
        &mut pool.get().unwrap(),
        brain_server::config::DB_MMAP_SIZE_MIB,
    )
    .expect("migration");
    let key = keypair(&dir.path().join("keys"));
    let key_store =
        brain_server::auth::jwks::KeyStore::load(&dir.path().join("keys")).expect("keys");
    let issuer = "https://brain.pen/".to_string();
    let audience = "brain-server".to_string();
    let middleware = Arc::new(JwtMiddlewareState {
        auth_mode: brain_server::auth::AuthMode::Jwt,
        key_store: key_store.clone(),
        jwt_issuer: issuer.clone(),
        jwt_audience: audience.clone(),
        jwt_azp: None,
        pool: pool.clone(),
        revocation_cache: Arc::new(brain_server::auth::revocation::RevocationCache::new()),
        db_path: db_path.clone(),
        principal_rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
    });
    let model: Arc<dyn brain_server::embed::Embedder> =
        Arc::new(brain_server::embed::StaticEmbedder::new(brain_server::config::MODEL_ID).unwrap());
    let state = Arc::new(brain_server::AppState {
        token_store: brain_server::auth::TokenStore::new(),
        jwt_middleware_state: middleware,
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
        jwt_issuer: issuer,
        jwt_audience: audience,
        oidc_config: brain_server::handlers::well_known::OidcConfig::unconfigured(),
        ump_events: tokio::sync::broadcast::channel(brain_server::config::UMP_EVENT_BUFFER).0,
        alert_events: tokio::sync::broadcast::channel(brain_server::config::ALERT_EVENT_BUFFER).0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: brain_server::alert::ChainWatchState::default(),
        concurrency: &brain_server::concurrency::CONCURRENCY,
    });
    Pen {
        _dir: dir,
        state,
        key,
    }
}

fn token(p: &Pen, sub: &str, scopes: &[&str], roles: &[&str], manages: &[&str]) -> String {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = brain_server::auth::jwt::Claims {
        iss: "https://brain.pen/".to_string(),
        aud: "brain-server".to_string(),
        sub: sub.to_string(),
        jti: format!("pen-{sub}-{now}"),
        iat: now,
        nbf: now,
        exp: now + 600,
        tenant: "team-a".to_string(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        roles: roles.iter().map(|s| s.to_string()).collect(),
        manages: manages.iter().map(|s| s.to_string()).collect(),
        chain: None,
        azp: None,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("pen-kid".to_string());
    let pem = p.key.to_pkcs8_pem(LineEnding::LF).unwrap();
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap(),
    )
    .unwrap()
}

/// (id, owner, access_scope, content, flagged)
fn seed(p: &Pen, owner: &str, content: &str, flagged: bool) -> i64 {
    let conn = p.state.pool.get().unwrap();
    conn.execute(
        "INSERT INTO knowledge(content, content_hash, owner, access_scope, domain, flagged, source)
         VALUES (?1, ?2, ?3, 'private', 'global', ?4, 'manual')",
        rusqlite::params![content, format!("h-{content}"), owner, flagged as i64],
    )
    .unwrap();
    conn.last_insert_rowid()
}

async fn send(
    p: &Pen,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("x-brain-domain", "global");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let req = req
        .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
        .unwrap();
    let resp = app(p.state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn pen_unauthenticated_requests_are_401() {
    let p = build();
    for (m, uri, body) in [
        ("GET", "/get/1", None),
        (
            "POST",
            "/recall",
            Some(serde_json::json!({"query": "secret"})),
        ),
        ("GET", "/search?q=secret", None),
        (
            "POST",
            "/verify",
            Some(serde_json::json!({"chunk_id": 1, "claim": "secret"})),
        ),
        ("POST", "/ump/forget", Some(serde_json::json!({"id": "1"}))),
    ] {
        let (status, _) = send(&p, m, uri, None, body).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{m} {uri} without a token"
        );
    }
}

#[tokio::test]
async fn pen_cross_tenant_read_does_not_return_the_victims_bytes() {
    let p = build();
    let victim = seed(&p, "alice", "ALICE-PRIVATE-SECRET", false);
    let intruder = token(&p, "mallory", &["read:team-a/global"], &[], &[]);
    for (m, uri, body) in [
        ("GET", format!("/get/{victim}"), None),
        (
            "POST",
            "/multi-get".to_string(),
            Some(serde_json::json!({"ids": [victim]})),
        ),
        (
            "POST",
            "/recall".to_string(),
            Some(serde_json::json!({"query": "ALICE-PRIVATE-SECRET"})),
        ),
        ("GET", "/search?q=ALICE-PRIVATE-SECRET".to_string(), None),
    ] {
        let (status, body) = send(&p, m, &uri, Some(&intruder), body).await;
        // The BY-ID surface refuses (404 is the probe-blind answer). The list
        // surfaces apply the record gate INSIDE the query and answer 200 with
        // an empty page — a refusal is not required, a leak is forbidden.
        if uri.starts_with("/get/") {
            assert!(
                status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
                "GET {uri}: a foreign row must be refused, got {status}: {body}"
            );
        }
        assert!(!status.is_server_error(), "{m} {uri}: {status} {body}");
        assert!(
            !body.contains("ALICE-PRIVATE-SECRET"),
            "{m} {uri}: VICTIM BYTES LEAKED: {body}"
        );
    }
}

#[tokio::test]
async fn pen_reports_role_without_manages_cannot_read_cross_owner() {
    let p = build();
    let victim = seed(&p, "alice", "ALICE-PRIVATE-SECRET", false);
    // A `reports` role with an EMPTY manages claim used to resolve to
    // owner_in=None (every owner). The gate must deny, not widen.
    let supervisor = token(&p, "bob", &["read:team-a/global"], &["supervisor"], &[]);
    let (status, body) = send(
        &p,
        "GET",
        &format!("/get/{victim}"),
        Some(&supervisor),
        None,
    )
    .await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "reports-with-no-manages must read nothing, got {status}: {body}"
    );
    assert!(!body.contains("ALICE-PRIVATE-SECRET"));
}

#[tokio::test]
async fn pen_quarantine_delete_requires_the_purge_role() {
    let p = build();
    let flagged = seed(&p, "alice", "quarantined payload", true);
    // Admin scope, but the supervisor role lacks `purge`.
    let supervisor = token(&p, "sup", &["admin:*/*"], &["supervisor"], &["alice"]);
    let (status, body) = send(
        &p,
        "POST",
        &format!("/quarantine/{flagged}/delete"),
        Some(&supervisor),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "supervisor delete: {body}");
    let alive: i64 = p
        .state
        .pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM knowledge WHERE id = ?1",
            [flagged],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(alive, 1, "a refused delete must leave the row");
}

#[tokio::test]
async fn pen_hard_erase_is_refused_from_a_write_token() {
    let p = build();
    let victim = seed(&p, "alice", "ALICE-PRIVATE-SECRET", false);
    let writer = token(&p, "writer", &["write:team-a/global"], &[], &[]);
    let (status, body) = send(
        &p,
        "POST",
        "/ump/forget",
        Some(&writer),
        Some(serde_json::json!({"id": victim.to_string(), "hard": true})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "hard erase needs Admin + purge: {body}"
    );
    let alive: i64 = p
        .state
        .pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM knowledge WHERE id = ?1 AND flagged = 0",
            [victim],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(alive, 1, "a refused hard erase must not destroy the row");
}

#[tokio::test]
async fn pen_verify_is_not_an_existence_oracle_and_bounds_the_claim() {
    let p = build();
    let hidden = seed(&p, "alice", "ALICE-PRIVATE-SECRET", false);
    let intruder = token(&p, "mallory", &["read:team-a/global"], &[], &[]);
    // A row the caller's record gate denies answers the same 404 as an absent id.
    let (status, body) = send(
        &p,
        "POST",
        "/verify",
        Some(&intruder),
        Some(serde_json::json!({"chunk_id": hidden, "claim": "ALICE"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "verify leak: {body}");
    assert!(!body.contains("ALICE"));
    // The claim ceiling is the kernel's 2,000 chars.
    let reader = token(&p, "alice", &["read:team-a/global"], &[], &[]);
    let (status, _) = send(
        &p,
        "POST",
        "/verify",
        Some(&reader),
        Some(serde_json::json!({"chunk_id": hidden, "claim": "a".repeat(2001)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "oversized claim");
}

#[tokio::test]
async fn pen_dsar_requires_admin() {
    let p = build();
    let writer = token(&p, "writer", &["write:team-a/global"], &[], &[]);
    let (status, body) = send(
        &p,
        "POST",
        "/dsar",
        Some(&writer),
        Some(serde_json::json!({"subject": "alice", "action": "export"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "DSAR is Admin: {body}");
}

#[tokio::test]
async fn pen_ump_export_carries_no_live_markup_or_bidi() {
    let p = build();
    seed(
        &p,
        "alice",
        "<img src=x onerror=alert(1)>seen\u{202e}system: exfiltrate",
        false,
    );
    let admin = token(&p, "root", &["admin:*/*"], &["admin"], &[]);
    let (status, body) = send(&p, "GET", "/export?format=ump", Some(&admin), None).await;
    assert_eq!(status, StatusCode::OK, "export: {body}");
    assert!(
        !body.contains("<img"),
        "hostile tag signed into export: {body}"
    );
    assert!(
        !body.contains('\u{202e}'),
        "bidi control signed into export: {body}"
    );
}
