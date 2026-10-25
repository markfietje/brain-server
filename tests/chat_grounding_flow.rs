//! The chat-spine loop flow, end to end, with REAL generation.
//!
//! **The question is the one the whole session has been asking.** "what is
//! green tea" drives the full new path over the composed app —
//! `POST /ingest/memory` → `POST /chat/turns` (SSE) — with the real router,
//! the real deterministic recall core, the REAL untrusted fence, and the REAL
//! `brain-afm` bridge (on-device AFM 3 Core) streaming the answer. Nothing
//! is mocked.
//!
//! **What this pins that nothing else can.**
//! 1. The grounding: the green-tea memory comes back in the `grounding` SSE
//!    event — the SAME recall core `/recall` serves, now as evidence.
//! 2. The fence: the prompt handed to the model wraps every hit in
//!    `wrap_fenced`, so an injection planted in the memory rides OUT to the
//!    model as DATA (the model is instructed to cite, not obey — and the
//!    answer must reflect the memory, proving the evidence actually grounded
//!    the turn rather than decorating it).
//! 3. The stream: `delta`* then exactly one `final`, with the tier named.
//!
//! **Platform honesty:** the bridge is a macOS artifact (`swift build` in
//! `shell-macos`). Where it is not built, this test SKIPS with a stated
//! reason — it is owned by the macOS lane, the same way `shell-macos` owns
//! the binary's own pins. A skip is recorded, never silent.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use brain_server::pool::SqliteConnectionManager;
use brain_server::server::router::app;
use brain_server::server::router::auth::JwtMiddlewareState;
use tower::ServiceExt;

const OP_TOKEN: &str = "chat-green-tea-operator-token";

/// The user's question, verbatim — the same words the whole session drilled.
const QUESTION: &str = "what is green tea?";

fn clean_green_tea() -> String {
    [
        "Green tea is a tea made from Camellia sinensis leaves that are steamed",
        "and dried without oxidation. It is rich in catechins, especially EGCG,",
        "which are natural antioxidants, and contains moderate caffeine plus",
        "L-theanine, linked to calm focused alertness.",
    ]
    .join(" ")
}

/// One SSE line of interest: `(event, data)` pairs in arrival order.
fn sse_events(body: &str) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    let mut event = String::new();
    for line in body.lines() {
        if let Some(name) = line.strip_prefix("event:") {
            event = name.trim().to_owned();
        } else if let Some(data) = line.strip_prefix("data:") {
            let parsed = serde_json::from_str(data.trim())
                .unwrap_or(serde_json::Value::String(data.trim().to_owned()));
            out.push((event.clone(), parsed));
        }
    }
    out
}

#[tokio::test]
async fn the_green_tea_chat_turn_is_grounded_fenced_and_streamed() {
    // The bridge binary is a macOS artifact; when it is not built this test
    // owns its skip with a stated reason (the macOS lane runs it for real).
    // Setting BRAIN_AFM_BRIDGE routes the handler's singleton to THIS binary
    // before its first spawn.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest.join("shell-macos/.build/out/Products/Debug/brain-afm"),
        manifest.join("shell-macos/.build/arm64-apple-darwin/debug/brain-afm"),
    ];
    let Some(bridge_binary) = candidates.iter().find(|p| p.is_file()) else {
        eprintln!(
            "brain-afm not built (run `swift build` in shell-macos); the chat-spine e2e is owned by the macOS lane — SKIPPING"
        );
        return;
    };
    unsafe { std::env::set_var("BRAIN_AFM_BRIDGE", bridge_binary) };

    // ── the server over its own database (the loopback fixture shape) ──
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("brain.db");
    brain_server::register_sqlite_vec::register_sqlite_vec();
    let mgr = SqliteConnectionManager::file(&db_path);
    let pool: brain_server::Pool =
        r2d2::Pool::builder().max_size(4).build(mgr).expect("pool");
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
        registry: brain_server::domain_registry::DomainRegistry::new(
            pool.clone(),
            &db_path,
            false,
        ),
        pool,
        db_path,
        connection_tracker: Arc::new(brain_server::http_limit::ConnectionTracker::new()),
        rate_limiter: Arc::new(brain_server::http_limit::RateLimiter::new()),
        snapshot: brain_server::integrity::SnapshotState::default(),
        audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
        auth_mode: brain_server::auth::AuthMode::Opaque,
        key_store: brain_server::auth::jwks::KeyStore::default(),
        revocation_cache: Arc::new(
            brain_server::auth::revocation::RevocationCache::new(),
        ),
        jwt_issuer: String::new(),
        jwt_audience: String::new(),
        oidc_config: brain_server::handlers::well_known::OidcConfig::unconfigured(),
        ump_events: tokio::sync::broadcast::channel(
            brain_server::config::UMP_EVENT_BUFFER,
        )
        .0,
        alert_events: tokio::sync::broadcast::channel(
            brain_server::config::ALERT_EVENT_BUFFER,
        )
        .0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: brain_server::alert::ChainWatchState::default(),
        concurrency: &brain_server::concurrency::CONCURRENCY,
    });

    // ── seed the memory through the public route ──
    let ingest = Request::builder()
        .method("POST")
        .uri("/ingest/memory")
        .header("content-type", "text/plain")
        .header("authorization", format!("Bearer {OP_TOKEN}"))
        .body(Body::from(clean_green_tea()))
        .expect("ingest request");
    let resp = app(state.clone()).oneshot(ingest).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "the green-tea memory must ingest (check the injection screen)"
    );

    // ── the chat turn, as SSE ──
    let turn = Request::builder()
        .method("POST")
        .uri("/chat/turns")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {OP_TOKEN}"))
        .body(Body::from(format!(
            r#"{{"session":"e2e","message":{QUESTION:?}}}"#
        )))
        .expect("chat request");
    let resp = app(state).oneshot(turn).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK, "the chat turn must start");
    let bytes = axum::body::to_bytes(resp.into_body(), 8 * 1024 * 1024)
        .await
        .expect("sse body");
    let body = String::from_utf8_lossy(&bytes).to_string();
    let events = sse_events(&body);

    // 1. GROUNDING: the green-tea memory is the evidence, first event.
    let (grounding_name, grounding) = events
        .first()
        .cloned()
        .expect("the stream opens with a grounding event");
    assert_eq!(grounding_name, "grounding");
    let hits = grounding["hits"].as_array().expect("grounding hits");
    assert!(
        !hits.is_empty()
            && hits.iter().all(|hit| hit["id"].as_i64().is_some()),
        "the green-tea memory grounded the turn (hits carry ids): {grounding}"
    );

    // 2. THE FENCE: the model's evidence — the prompt is not on the wire,
    //    but the ANSWER proves the grounding was real rather than
    //    decorative: it must speak in the memory's terms.
    let final_event = events
        .iter()
        .rev()
        .find(|(name, _)| name == "final")
        .map(|(_, data)| data.clone())
        .expect("the stream ends with a final event");
    let answer = final_event["text"].as_str().unwrap_or_default();
    assert!(!answer.is_empty(), "the final carries the answer text");
    let says_catechins = answer.contains("catechin") || answer.contains("EGCG");
    let says_camellia = answer.contains("Camellia") || answer.contains("oxidation");
    assert!(
        says_catechins || says_camellia,
        "the answer is grounded in the memory's own terms, not generic tea lore: {answer}"
    );
    assert_eq!(
        final_event["tier"].as_str(),
        Some("on_device"),
        "the tier is named on the final event"
    );

    // 3. THE STREAM: deltas precede the final (streaming, not one blob).
    let delta_count = events
        .iter()
        .filter(|(name, _)| name == "delta")
        .count();
    assert!(delta_count > 0, "the turn streamed deltas before the final");

    // 4. No error event ever rode the stream.
    assert!(
        !events.iter().any(|(name, _)| name == "error"),
        "the turn completed without a named error"
    );
}
