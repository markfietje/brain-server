//! The bootstrap seam: everything between process start and the
//! composed router — env/config resolution, fail-closed checks (auth
//! misconfig, write posture, model pinning, non-loopback bind), pool +
//! registry + state construction, watchdog + maintenance-worker spawns,
//! and the JWT middleware wiring. Protocol-free by law: NOTHING here
//! takes an axum type (the Capstone grep gate enforces it; this module
//! lives by it now). `main_inner` calls [`bootstrap`], matches the
//! offline-mode outcomes, then composes `router::app` and serves.

use anyhow::{Context, Result};
use r2d2_sqlite::SqliteConnectionManager;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tracing::{debug, info, warn};

use crate::audit;
use crate::auth::TokenStore;
use crate::config;
#[allow(unused_imports)]
use crate::config::MAX_GRAPH_EDGES;
use crate::config::{
    POOL_CONNECTION_TIMEOUT_SECS, POOL_IDLE_TIMEOUT_SECS, POOL_MAX_LIFETIME_SECS, POOL_MAX_SIZE,
    POOL_MIN_IDLE, SERVER_VERSION,
};
use crate::handlers;
use crate::http_limit::{ConnectionTracker, RateLimiter};
use crate::http_limit::{spawn_connection_watchdog, spawn_rss_watchdog};
use crate::migration::run_migration_with_store_dim;
use crate::register_sqlite_vec::register_sqlite_vec;
use crate::server::router::auth::JwtMiddlewareState;
use crate::{Pool, alert, auth, domain_registry, integrity, webhook};
use rusqlite::{Connection, params};
use tower_http::cors::{AllowOrigin, CorsLayer};
use zerocopy::IntoBytes;

/// What `bootstrap` resolved: serve with this state + address, or an
/// offline maintenance mode already ran (the process exits Ok).
pub enum BootOutcome {
    /// Serve: every component is up; `main_inner` composes + serves.
    Serve(Bootstrap),
    /// An offline `--re-embed`/`--re-audit` mode ran INSTEAD of serving.
    Done,
}

/// The serving world `bootstrap` hands to `main_inner`.
pub struct Bootstrap {
    /// The fully-wired server state: every handler consumes this.
    pub state: Arc<crate::AppState>,
    /// A dedicated pool clone for the post-shutdown WAL checkpoint.
    pub shutdown_pool: Pool,
    /// The resolved bind address (loopback-guarded, opt-in enforced).
    pub addr: SocketAddr,
}

/// The server world-state: pool, registries, JWT material, event buses.
/// Constructed once by `bootstrap()`; consumed by every handler via
/// `State<Arc<AppState>>`. Fields are `pub`: the same-workspace binaries
/// (brain-server) + integration tests construct/read it; nothing outside
/// the workspace links this crate.
pub struct AppState {
    // The embedding model behind the `Embedder` trait so the
    // active profile (edge-default potion / enterprise bge-m3 / …) is selected
    // at boot by `embed::embedder_for_profile`, not compiled in. Recall/ingest
    // sites call `model.encode_one(&t)` and are profile-agnostic.
    pub model: Arc<dyn crate::embed::Embedder>,
    pub pool: Pool,
    /// Per-domain DB registry. In shim mode (BRAIN_MULTI_DB off) every
    /// domain resolves to `pool`; the domain-aware write/search paths use this.
    pub registry: domain_registry::DomainRegistry,
    #[allow(dead_code)]
    pub db_path: PathBuf,
    pub connection_tracker: std::sync::Arc<ConnectionTracker>,
    /// Axum accesses this by type (State<Arc<RateLimiter>>), not by field name.
    /// The compiler sees zero direct reads — false positive, required.
    #[allow(dead_code)]
    pub rate_limiter: Arc<RateLimiter>,
    /// last backup+integrity result for `/health`.
    pub snapshot: integrity::SnapshotState,
    /// TTL-memoized `audit::verify_chain` result for `/metrics`.
    /// `/audit/verify` always does a fresh full scan (authoritative answer);
    /// `/metrics` reads this cache and refreshes only if older than
    /// `AUDIT_CHAIN_CACHE_TTL`. The cached value is a real verified result —
    /// just briefly stale. Tradeoff: a tamper that lands between refreshes is
    /// reported on the next TTL boundary, not instantly. Ponytail ceiling:
    /// adequate for monitoring; an operator wanting a fresh answer hits
    /// `/audit/verify`.
    pub audit_chain_cache: Arc<std::sync::Mutex<Option<(std::time::Instant, bool)>>>,
    // ── JWT fields ─────────────────────────────────────
    /// Which auth mode the server resolved at startup. `Opaque` (v1.1 back-
    /// compat, default) or `Jwt` (opt-in via BRAIN_JWT_ISSUER + key dir).
    pub auth_mode: auth::AuthMode,
    /// Loaded signing + verifying keys. Empty in opaque mode.
    pub key_store: auth::jwks::KeyStore,
    /// Per-process negative-lookup cache for `(jti, iss)` revocation checks.
    pub revocation_cache: Arc<auth::revocation::RevocationCache>,
    /// Configured JWT issuer (verified against every token's `iss` claim).
    /// Empty in opaque mode.
    pub jwt_issuer: String,
    /// Configured JWT audience (verified against every token's `aud` claim).
    /// Empty in opaque mode.
    pub jwt_audience: String,
    /// OIDC discovery metadata (built from BRAIN_PUBLIC_BASE_URL). Served at
    /// `/.well-known/openid-configuration`. Empty placeholder when JWT is off.
    pub oidc_config: handlers::well_known::OidcConfig,
    /// `GET /ump/subscribe` SSE change events (`{kind, id}` —
    /// never record bodies). Published by remember/revise/forget.
    pub ump_events: tokio::sync::broadcast::Sender<serde_json::Value>,
    /// `GET /events` SSE live alert feed (`{kind, ts, seq,
    /// payload}` — never content/PII). Published by the four decision cores.
    pub alert_events: tokio::sync::broadcast::Sender<serde_json::Value>,
    /// monotonic alert sequence (the webhook delivery-id
    /// source + the receiver's idempotency key).
    pub alert_seq: std::sync::atomic::AtomicU64,
    /// cached audit-chain posture from the integrity watcher.
    /// Written by `alert::spawn_chain_watcher`; read by `/health` so the
    /// tamper-evident posture is visible without an on-demand full scan.
    pub chain_watch: alert::ChainWatchState,
    /// Contention telemetry (the Throughput milestone): pool-timeout + busy-error
    /// counters and the /health/db-refreshed WAL snapshot. A `&'static` alias
    /// of `concurrency::CONCURRENCY` — the same object the deep write-path
    /// error arms increment without state plumbing.
    pub concurrency: &'static crate::concurrency::Concurrency,
    // ── middleware-stack inputs (Vaulting: app(state) reads them here so
    // the composition is a pure function of state) ────────────────────
    /// The cached bearer-token store; `auth_middleware`'s from_fn state.
    pub token_store: TokenStore,
    /// JWT-mode verification state mirrored from the boot-resolved values.
    pub jwt_middleware_state: Arc<JwtMiddlewareState>,
    /// The boot-built CORS layer (env-resolved origins/methods/headers).
    pub cors: CorsLayer,
}

/// The offline `--re-embed <profile>` body. Loads the TARGET
/// profile's embedder, repoints the vec store at its dim (the fail-closed
/// guard's sanctioned bypass), then re-embeds every chunk — the same loop shape
/// as the `/reindex` handler, inline here because the handler needs a live
/// AppState (a server that can't boot under a dim mismatch) and this runs cold.
fn run_reembed(pool: &Pool, target_profile: &str) -> Result<()> {
    let model = crate::embed::embedder_for_profile(target_profile)?;
    let dim = model.store_dim();
    println!("re-embed → profile={target_profile} dim={dim}");
    let mut conn = pool.get().context("DB connection failed")?;
    crate::migration::rebuild_vec_store_at_dim(&mut conn, dim)?;
    // Same loop as /reindex: encode → delete + re-insert (vec0 has no UPSERT).
    let ids: Vec<(i64, String)> = conn
        .prepare("SELECT id, content FROM knowledge ORDER BY id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    let mut reembedded = 0usize;
    let mut skipped = 0usize;
    for (id, content) in &ids {
        let v = model.encode_one(content);
        if v.is_empty() {
            skipped += 1;
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        // was `let _ =` — a failed delete/insert here
        // would silently lose the vector for `id` (FTS-only retrieval).
        tx.execute(
            "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
            params![id],
        )?;
        tx.execute(
            "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
             SELECT ?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2),
                    COALESCE((SELECT source FROM knowledge WHERE id = ?1), 'manual'),
                    datetime('now')",
            params![id, v.as_bytes()],
        )?;
        tx.commit()?;
        reembedded += 1;
    }
    println!(
        "re-embed complete: {reembedded} re-embedded, {skipped} skipped — boot with BRAIN_MODEL_PROFILE={target_profile}"
    );
    Ok(())
}

/// The offline `--re-audit` body. Re-anchors
/// the audit chain of EVERY database — the global pool plus every registered
/// per-domain file — under the hmac256 scheme: full 8-field rows,
/// HMAC-SHA256 links, a fresh head pin, and one `AuditKind::Anchor` evidence
/// row per domain on the NEW chain. Runs INSTEAD of serving (the chain is
/// evidence; its format flips only under the documented operator protocol:
/// snapshot → quiesce writes → --re-audit → verify every domain → snapshot the
/// new evidence baseline). Per-domain failures are reported and fail the run —
/// never silent.
fn run_reaudit(pool: &Pool, db_path: &std::path::Path) -> Result<()> {
    use crate::audit;
    println!("re-audit → scheme=hmac256 (8-field rows, HMAC-SHA256 links, head pin)");
    println!(
        "operator protocol: `brain backup` FIRST (the pre-anchor chain stays readable \
         under the legacy scheme as the historical archive), writes quiesced, then re-anchor"
    );
    // Targets: the global pool + every registered domain pool (multi-db).
    // Shim mode collapses to the single shared pool.
    let registry = domain_registry::DomainRegistry::new(pool.clone(), db_path, config::multi_db());
    let targets = handlers::domain_pools(&registry, pool);
    let mut all_ok = true;
    for (name, pool_or) in &targets {
        let Some(domain_pool) = pool_or else {
            all_ok = false;
            eprintln!("  [{name}] FAILED: domain pool could not be opened");
            continue;
        };
        let Ok(conn) = domain_pool.get() else {
            all_ok = false;
            eprintln!("  [{name}] FAILED: connection could not be acquired");
            continue;
        };
        match audit::reanchor_to_hmac(&conn) {
            Ok(rewritten) => {
                // Evidence row ON the new chain: the epoch boundary itself is
                // audited (kind=anchor, target carries the rewrite count). A
                // failed Anchor row fails the run — it is the record of the
                // format change, not a best-effort side note.
                if audit::record(
                    &conn,
                    audit::AuditKind::Anchor,
                    "system",
                    &format!("reanchor:hmac256:rewritten={rewritten}"),
                    audit::AuditStatus::Ok,
                    "chain format re-anchored (8-field HMAC-SHA256 links)",
                )
                .is_none()
                {
                    all_ok = false;
                    eprintln!("  [{name}] FAILED: the anchor evidence row could not be written");
                    continue;
                }
                let ok = audit::verify_chain(&conn);
                println!("  [{name}] re-anchored ({rewritten} link(s) rewritten) — verify: {ok}");
                all_ok &= ok;
            }
            Err(e) => {
                all_ok = false;
                eprintln!("  [{name}] FAILED: {e:#}");
            }
        }
    }
    if !all_ok {
        anyhow::bail!(
            "re-audit completed with failures — no domain was left half-converted; \
             resolve the reported domains and re-run (the pre-anchor snapshot is the fallback)"
        );
    }
    println!(
        "re-audit complete — run `brain backup` now: the post-anchor snapshot is the new \
         evidence baseline (head epoch hmac256)"
    );
    Ok(())
}

/// Resolve the world: argv → fail-closed config checks → telemetry →
/// sqlite-vec → pool → offline modes → backups → model → migration →
/// watchdogs → middleware wiring → `AppState` → watchers → bind address.
/// Every step in here is boot-order-frozen; reordering is a behavior
/// change and needs its own release.
pub fn bootstrap() -> Result<BootOutcome> {
    // ── Argv guard (before any side effect) ────────────────────────────
    // Handle --version/-V and --help/-h, and reject unknown flags instead of
    // silently starting the server. MUST run before tracing init or bind() so
    // `brain-server --version` never logs, opens sockets, or loads the model.
    handle_cli_args();

    // ── fail-closed auth configuration ────────────────
    // An explicitly-set-but-broken token file must not silently disable auth
    // (the wrong failure direction); a secret file readable by group/world
    // must not be accepted at all. Both refuse startup with a clear message.
    if let Some(msg) = config::auth_token_misconfigured() {
        return Err(anyhow::anyhow!(msg));
    }
    if let Some(path) = config::auth_token_file() {
        auth::check_secret_permissions(&path)
            .map_err(|e| anyhow::anyhow!("fatal auth config: {e}"))?;
    }

    // ── fail-closed write posture ─────────────────────
    // An unknown BRAIN_WRITE_POSTURE value refuses startup rather than
    // silently degrading to `open` (the Seatbelt posture).
    if let Err(e) = config::validate_write_posture() {
        return Err(anyhow::anyhow!("fatal write posture: {e}"));
    }

    // ── fail-closed model-artifact pinning ─────────────
    // When BRAIN_MODEL_MANIFEST is set, every pinned artifact must match its
    // SHA-256 or the server refuses to start — a model file must never
    // silently differ from what the operator pinned.
    if let Err(e) = crate::model_pin::verify_configured_models() {
        return Err(anyhow::anyhow!("fatal model manifest: {e}"));
    }

    // ── optional OTLP trace export ──────────────
    // Default build (no `otel` feature): plain fmt logging, byte-for-byte
    // unchanged. With `--features otel`, when `BRAIN_OTEL_ENABLED` (default
    // on) the four decision-critical spans are ALSO exported via OTLP/HTTP to
    // `BRAIN_OTEL_ENDPOINT`. A failed exporter init never aborts startup —
    // telemetry is best-effort, recall is the job.
    #[cfg(feature = "otel")]
    {
        let filter = tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into());
        if config::otel_enabled() {
            match crate::otel::init_otel(&config::otel_endpoint()) {
                Ok(provider) => {
                    use opentelemetry::trace::TracerProvider;
                    use tracing_subscriber::layer::SubscriberExt;
                    use tracing_subscriber::util::SubscriberInitExt;
                    tracing_subscriber::registry()
                        .with(filter)
                        .with(tracing_subscriber::fmt::layer())
                        .with(
                            tracing_opentelemetry::layer()
                                .with_tracer(provider.tracer("brain-server")),
                        )
                        .init();
                    info!("OTLP trace export enabled -> {}", config::otel_endpoint());
                }
                Err(e) => {
                    tracing_subscriber::fmt().with_env_filter(filter).init();
                    eprintln!("[otel] exporter init failed; continuing without OTLP export: {e}");
                }
            }
        } else {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
    #[cfg(not(feature = "otel"))]
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("Starting Brain Server v{}", SERVER_VERSION);

    // ── Register sqlite-vec before ANY connection opens ────────────────
    // sqlite3_auto_extension registers the vec0 module + vec_* functions on
    // every new connection. MUST be called before r2d2 builds the pool.
    register_sqlite_vec();
    info!("sqlite-vec extension registered");

    let db_path = config::brain_db_path();
    if let Some(p) = db_path.parent() {
        std::fs::create_dir_all(p).ok();
    }
    info!("Database path: {:?}", db_path);

    // ── audit chain key bootstrap (env → file → generated 0600) ──
    // Resolved BEFORE any pool opens (lazy domain opens consult it for the
    // fresh-DB epoch bootstrap). Env > key file > a generated 0600
    // `audit-chain.key` beside the DB. A resolution failure is a loud warning,
    // not a boot refusal — a legacy-epoch deployment needs no key; writes to
    // an hmac256 DB fail closed per-write until it resolves.
    if let Err(e) = audit::init_chain_key(
        db_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    ) {
        warn!("audit chain key unavailable ({e}) — hmac256-epoch chains will fail closed");
    }

    let pool = r2d2::Pool::builder()
        .max_size(POOL_MAX_SIZE)
        .min_idle(Some(POOL_MIN_IDLE))
        .connection_timeout(StdDuration::from_secs(POOL_CONNECTION_TIMEOUT_SECS))
        .max_lifetime(Some(StdDuration::from_secs(POOL_MAX_LIFETIME_SECS)))
        .idle_timeout(Some(StdDuration::from_secs(POOL_IDLE_TIMEOUT_SECS)))
        .test_on_check_out(true)
        .build(
            SqliteConnectionManager::file(&db_path)
                .with_init(|c| c.execute_batch("PRAGMA busy_timeout=5000;")),
        )?;

    // Offline `--re-embed <profile>` — the fail-closed dim
    // guard's escape hatch. Runs INSTEAD of serving: rebuilds the vector store
    // at the target profile's dim and re-embeds every chunk, then exits.
    // Offline by design (the server can't boot under a dim mismatch).
    if let Some(target) = std::env::args()
        .nth(1)
        .filter(|a| a == "--re-embed")
        .and(std::env::args().nth(2))
    {
        run_reembed(&pool, &target)?;
        return Ok(BootOutcome::Done);
    }

    // ── Pre-migration safety backup ─────────────────────
    // One-shot `VACUUM INTO` snapshot taken BEFORE the vec0 schema migration
    // touches the DB, so the Rollback section's restore path is always possible.
    // Guarded by a marker file so restarts don't re-copy. Skipped for fresh DBs
    // (no knowledge rows yet) and when the backup already exists.
    {
        let backup_path = db_path.with_extension("db.pre-0.9.0.bak");
        let marker = db_path.with_extension("db.bak-done");
        let has_data: bool = pool
            .get()
            .ok()
            .and_then(|c| {
                c.query_row::<i64, _, _>(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='knowledge'",
                    [],
                    |r| r.get(0),
                )
                .ok()
            })
            .map(|n| n > 0)
            .unwrap_or(false);
        if has_data && !marker.exists() && !backup_path.exists() {
            info!("Pre-migration backup: VACUUM INTO {:?}", backup_path);
            match pool.get() {
                Ok(conn) => {
                    // Through the shared quote-escaping primitive —
                    // a `'` in the operator's data dir would break out of the
                    // raw literal.
                    match crate::backup::vacuum_into(&conn, &backup_path) {
                        Ok(_) => {
                            // Touch the marker so we never re-backup.
                            let _ = std::fs::write(&marker, b"v0.9.0 backup complete");
                            info!("Pre-migration backup complete");
                        }
                        Err(e) => warn!("Pre-migration backup failed (continuing): {e}"),
                    }
                }
                Err(e) => warn!("Pre-migration backup: pool unavailable: {e}"),
            }
        }
    }

    // Retrieval profile → embedder. MUST load before the migration: the
    // migration creates `vec_knowledge` at the embedder's `store_dim()` and
    // stamps `embedding_dim` so a later profile switch fails closed instead of
    // silently cross-dim-comparing.
    let profile = config::model_profile();
    let model_id = config::model_id_for_profile(profile);
    info!("Loading model: {} (profile: {})", model_id, profile);
    let model = crate::embed::embedder_for_profile(profile)?;
    info!(
        "Model loaded (profile: {}, dim: {})",
        profile,
        model.store_dim()
    );

    // Enable the cross-encoder rerank tier on the profiles
    // whose hardware can afford it (enterprise/desktop). search/mod.rs is lib
    // code and can't read the server-private profile, so the gate is an env var
    // the boot owns. edge-default/air-gapped stay rerank-free.
    if matches!(
        profile,
        config::PROFILE_ENTERPRISE | config::PROFILE_DESKTOP | config::PROFILE_QUALITY_LOCAL
    ) {
        unsafe { std::env::set_var("BRAIN_RERANK_ENABLED", "1") };
        info!(
            "rerank tier armed (profile={profile}); loading mxbai-rerank-large-v1 (fallback bge-reranker-v2-m3)…"
        );
        // Warm at boot, not on first recall: the lazy load would otherwise put
        // the model download inside the request path (observed: first-query 503
        // `recall timed out` while the reranker downloaded).
        #[cfg(feature = "rerank-tier")]
        search::rerank::warmup();
        info!("rerank tier ready (profile={profile})");
    }

    run_migration_with_store_dim(
        &mut *pool.get().context("migration failed")?,
        config::DB_MMAP_SIZE_MIB,
        model.store_dim(),
    )?;
    info!("Migration complete (embedding_dim = {})", model.store_dim());

    // ── offline --re-audit + fresh-DB bootstrap ────────────────────
    // The re-anchor runs INSTEAD of serving (writes must be quiesced — an
    // audit chain is evidence and its format flips only under the documented
    // operator protocol: snapshot → quiesce → --re-audit → verify → snapshot).
    if std::env::args()
        .nth(1)
        .map(|a| a == "--re-audit")
        .unwrap_or(false)
    {
        run_reaudit(&pool, &db_path)?;
        return Ok(BootOutcome::Done);
    }
    // A FRESH global DB (zero audit rows) starts directly on hmac256 when the
    // key resolved above; a DB with history stays legacy until --re-audit.
    if let Ok(conn) = pool.get()
        && audit::bootstrap_epoch(&conn)
    {
        info!("audit chain: fresh DB bootstrapped to the hmac256 epoch");
    }

    // ── legacy cutover: brain.db → global.db ─────────────────
    // When `BRAIN_MULTI_DB=true`, the per-domain system needs the legacy
    // single-DB content at `global.db`. We snapshot it ONCE, atomically, with
    // `VACUUM INTO` (consistent copy even under WAL) and stamp a marker so
    // restarts never re-copy. Skipped when:
    //   - shim mode (multi_db off — the legacy brain.db IS the global pool);
    //   - the marker exists (already cut over);
    //   - global.db already exists (operator provisioned it manually);
    //   - brain.db has no data (fresh install).
    //
    // ponytail: this is the smallest safe cutover — a one-shot file copy via
    // SQLite's own consistent-snapshot primitive. The rehearsal tool
    // covers the heavier per-row migration; this is the boot-time
    // safety net for the actual cutover day.
    if config::multi_db() {
        let layout = crate::storage_layout::StorageLayout::detect()?;
        let global_path = layout.global_domain_db();
        let legacy_path = layout.legacy_db();
        let marker = layout.root().join(".v1-legacy-cutover-done");
        let legacy_has_data = std::fs::metadata(&legacy_path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
            && pool
                .get()
                .ok()
                .and_then(|c| {
                    c.query_row::<i64, _, _>(
                        "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table' AND name='knowledge'",
                        [],
                        |r| r.get(0),
                    )
                    .ok()
                })
                .map(|n| n > 0)
                .unwrap_or(false);
        if legacy_has_data && !marker.exists() && !global_path.exists() {
            info!(
                "v1.0 legacy cutover: snapshotting {:?} → {:?}",
                legacy_path, global_path
            );
            match pool.get() {
                Ok(conn) => {
                    // Escaped primitive (see the pre-migration backup
                    // above).
                    match crate::backup::vacuum_into(&conn, &global_path) {
                        Ok(_) => {
                            let _ = std::fs::write(&marker, b"v1.0 legacy cutover complete");
                            info!("v1.0 legacy cutover complete");
                        }
                        Err(e) => {
                            warn!("v1.0 legacy cutover failed (continuing in shim mode): {e}")
                        }
                    }
                }
                Err(e) => warn!("v1.0 legacy cutover: pool unavailable: {e}"),
            }
        }
    }

    // Report effective PRF configuration so the retrieval behavior is
    // observable at startup (no hidden constants).
    let prf = crate::search::PrfConfig::from_env();
    info!(
        "PRF config: enabled={} depth={} terms={} max_rank={}",
        prf.enabled, prf.depth, prf.terms, prf.max_rank
    );

    // Initialize connection leak detection
    let connection_tracker = std::sync::Arc::new(ConnectionTracker::new());
    spawn_connection_watchdog(std::sync::Arc::clone(&connection_tracker));
    info!("Connection watchdog started");

    // RSS watchdog. Log-only by default; `BRAIN_RSS_RESTART=1`
    // opts in to exit-on-sustained-breach for supervisor-managed restarts.
    spawn_rss_watchdog();

    // cached, fail-safe bearer-token store + hot rotation.
    // The watcher polls `AUTH_TOKEN_FILE` mtime every 5s and reloads on change.
    let token_store = TokenStore::new();
    if token_store.has_file() {
        auth::spawn_rotation_watcher(token_store.clone(), db_path.clone());
        info!("token rotation watcher started");
    }

    // rolling backup + integrity self-check. Runs once on
    // boot then every 6h; `/health` reports `last_backup` + `integrity_ok`.
    let snapshot_state = integrity::SnapshotState::default();
    integrity::spawn_scheduler(db_path.clone(), snapshot_state.clone());

    // Spawn pool health check to prevent connection timeouts
    let pool_for_health = pool.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Ok(conn) = pool_for_health.get() {
                // was `let _ =` — a failing probe only
                // ever logged nothing (the OK branch logged).
                if let Err(e) = conn.query_row("SELECT 1", [], |_| Ok(())) {
                    warn!("pool health probe failed: {e}");
                } else {
                    debug!("Pool health check: OK");
                }
            }
        }
    });
    info!("Pool health check started");

    // Initialize rate limiter
    let rate_limiter = Arc::new(RateLimiter::new());
    info!("Rate limiter initialized");

    // annotator module removed.
    // The TOML domain engine (src/annotator/) has been deleted; the inline
    // [[relation::entity]] byte scanner (parse_annotations) is kept.
    // On a default deploy the annotator was already a no-op, so this is
    // behaviour-preserving for live boxes.

    // ── CORS: wire the documented env vars into the real layer ─────────
    // CORS_ORIGINS / CORS_METHODS / CORS_HEADERS override the defaults in
    // config.rs.  Origins are exact-matched (no wildcard) so that production
    // deployments are locked down by default.
    let origins: Vec<String> = config::cors_origins()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let methods: Vec<String> = config::cors_methods()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let headers: Vec<String> = config::cors_headers()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    info!(
        "CORS origins: {:?}, methods: {:?}, headers: {:?}",
        origins, methods, headers
    );
    info!(
        "Auth: {}",
        if config::auth_token().is_some() {
            "enabled (auth token resolved; non-public routes require Bearer token)"
        } else {
            "disabled (no auth token; loopback/proxy-only)"
        }
    );
    info!(
        "Per-domain DBs: {}",
        if config::multi_db() {
            "enabled (BRAIN_MULTI_DB=true; non-global domains use brain-<domain>.db)"
        } else {
            "disabled (shim mode; all domains share the global DB)"
        }
    );

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, _| {
            // origin is &HeaderValue; compare as string
            origin
                .to_str()
                .map(|o| origins.iter().any(|allowed| allowed == o))
                .unwrap_or(false)
        }))
        .allow_methods(
            methods
                .iter()
                .filter_map(|m| m.parse().ok())
                .collect::<Vec<_>>(),
        )
        .allow_headers(
            headers
                .iter()
                .filter_map(|h| h.parse().ok())
                .collect::<Vec<_>>(),
        )
        .max_age(std::time::Duration::from_secs(config::CORS_MAX_AGE_SECS));

    // clone the pool for the webhook drain worker before it is
    // moved into AppState below.
    let webhook_pool = pool.clone();
    // clone the pool for the post-shutdown WAL checkpoint.
    let shutdown_pool = pool.clone();

    // JWT/JWS key loading + middleware state setup. Done before
    // the router construction so the middleware state can be passed to
    // `from_fn_with_state` + the same values mirrored into AppState.
    let key_dir = auth::jwks::resolve_key_dir();
    let key_store = auth::jwks::KeyStore::load(&key_dir).unwrap_or_else(|e| {
        warn!("JWT key load failed ({e}); falling back to opaque-token mode");
        auth::jwks::KeyStore::default()
    });
    let auth_mode = auth::AuthMode::from_env(key_store.len());
    // the UMP operator Ed25519 signing key dir FAILS CLOSED on a
    // group/world-readable key file (same posture as every other secret — a
    // world-readable key still mints capability tokens, so "warn and mint"
    // was an inconsistency).
    #[cfg(unix)]
    {
        let dir = crate::config::ump_key_dir();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if e.path().is_file() {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = std::fs::metadata(e.path())
                        && meta.permissions().mode() & 0o077 != 0
                    {
                        return Err(anyhow::anyhow!(format!(
                            "fatal secret config: UMP operator signing key {:?} is \
                         group/world-readable (mode {:o}) — chmod 600 it",
                            e.path(),
                            meta.permissions().mode() & 0o777
                        )));
                    }
                }
            }
        }
    }
    let jwt_issuer = std::env::var("BRAIN_JWT_ISSUER")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    let jwt_audience = std::env::var("BRAIN_JWT_AUDIENCE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "brain-server".to_string());
    let public_base_url = std::env::var("BRAIN_PUBLIC_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    let oidc_config = if auth_mode.is_jwt() && !public_base_url.is_empty() {
        handlers::well_known::OidcConfig::build(&public_base_url)
    } else {
        handlers::well_known::OidcConfig::unconfigured()
    };
    if auth_mode.is_jwt() {
        info!(
            "JWT auth enabled: issuer={issuer} aud={aud} keys={n} dir={dir:?}",
            issuer = jwt_issuer,
            aud = jwt_audience,
            n = key_store.len(),
            dir = key_dir
        );
    } else {
        info!(
            "JWT auth not configured (set BRAIN_JWT_ISSUER + BRAIN_JWT_KEY_DIR); running in opaque-token mode"
        );
    }
    let revocation_cache = Arc::new(auth::revocation::RevocationCache::new());
    // Spawn the revocation purge job. Runs every PURGE_INTERVAL_SECS, drops
    // rows past their `exp`. Cheap (one indexed DELETE). Fresh connection per
    // tick — the job is rare, pooling it adds no value.
    // Also prunes the in-memory negative-lookup cache (purge_negatives) — that
    // HashMap grows one entry per unique (jti, iss) checked and would otherwise
    // grow unbounded for the process lifetime.
    {
        let purge_db_path = db_path.clone();
        let purge_cache = revocation_cache.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                auth::revocation::PURGE_INTERVAL_SECS,
            ));
            interval.tick().await; // skip the immediate first tick
            loop {
                interval.tick().await;
                purge_cache.purge_negatives();
                if let Ok(conn) = Connection::open(&purge_db_path) {
                    // was `let _ =` — a failed purge is
                    // fail-safe (stale denylist rows linger; tokens expire
                    // sooner, never later) but must be visible.
                    if let Err(e) = auth::revocation::purge_expired(&conn) {
                        warn!("revocation purge failed: {e}");
                    }
                }
            }
        });
    }
    let jwt_middleware_state = Arc::new(JwtMiddlewareState {
        auth_mode,
        key_store: key_store.clone(),
        jwt_issuer: jwt_issuer.clone(),
        jwt_audience: jwt_audience.clone(),
        pool: pool.clone(),
        revocation_cache: revocation_cache.clone(),
        db_path: db_path.clone(),
        principal_rate_limiter: Arc::new(RateLimiter::new()),
    });

    // the import route gets its OWN body-limit
    // layer — 1 GiB, matching the handler's `to_bytes` cap. Tower-http
    // semantics: a router-level `RequestBodyLimitLayer` is applied eagerly to
    // the routes present at `.layer()` time, so the 1 MiB shared limit below
    // must be applied BEFORE the merge — otherwise the outer 1 MiB layer would
    // pre-empt the import cap (Tower-http pitfall: an outer limit can never be
    // raised by an inner one) and real DB imports would be uncapturable.
    // Compliance-pack evidence surfaces. Feature-gated: without the feature
    // the router is EMPTY — the routes do not exist on the wire at all.

    let app_state = Arc::new(AppState {
        model,
        registry: domain_registry::DomainRegistry::new(pool.clone(), &db_path, config::multi_db()),
        pool,
        db_path: db_path.clone(),
        connection_tracker,
        rate_limiter,
        snapshot: snapshot_state,
        audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
        auth_mode,
        key_store,
        revocation_cache,
        jwt_issuer,
        jwt_audience,
        oidc_config,
        ump_events: tokio::sync::broadcast::channel(config::UMP_EVENT_BUFFER).0,
        alert_events: tokio::sync::broadcast::channel(config::ALERT_EVENT_BUFFER).0,
        alert_seq: std::sync::atomic::AtomicU64::new(0),
        chain_watch: alert::ChainWatchState::default(),
        concurrency: &crate::concurrency::CONCURRENCY,
        token_store,
        jwt_middleware_state,
        cors,
    });
    // watch pending proposals and fire the `expiry`
    // alert once per SLA-tier boundary crossed.
    let watcher_state = Arc::clone(&app_state);
    tokio::spawn(async move { alert::spawn_expiry_watcher(watcher_state).await });
    // Beacon: watch published articles whose freshness review is due
    // (the demand-reduction loop's staleness signal, `expiry` kind).
    let fresh_state = Arc::clone(&app_state);
    tokio::spawn(async move { alert::spawn_freshness_watcher(fresh_state).await });
    // watch the audit hash chain and raise an
    // `integrity` alert on ok↔broken transitions; /health reads the
    // cached posture.
    let cw_state = Arc::clone(&app_state);
    let cw_watch = app_state.chain_watch.clone();
    tokio::spawn(async move { alert::spawn_chain_watcher(cw_state, cw_watch).await });
    // The workflow-outbox → SSE bridge: every 2s the drained
    // `workflow/*` events publish on the /events bus (explicitly
    // subscribed consumers only, per-domain Read-gated at fan-out).
    let we_state = Arc::clone(&app_state);
    alert::spawn_workflow_event_worker(we_state);
    // seed the multi-db registry from
    // the clients register — a client's domain must resolve even if
    // its per-domain file vanished between boots (register recreates
    // it: cap-bounded; a refused seed is logged, never fatal).
    if config::multi_db()
        && let Ok(conn) = app_state.pool.get()
    {
        let domains: Vec<String> = conn
            .prepare("SELECT DISTINCT domain FROM clients")
            .and_then(|mut stmt| {
                stmt.query_map([], |r| r.get::<_, String>(0))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();
        for d in domains {
            if let Err(e) = app_state.registry.seed_registered(&d) {
                warn!("clients-table domain seed refused: {e}");
            }
        }
    }

    // spawn the webhook drain worker. It processes verified
    // deliveries off the bounded queue without an HTTP round-trip.
    webhook::spawn_drain_worker(webhook_pool);
    info!("webhook drain worker started");

    let bind_host = std::env::var("BIND_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let bind_port: u16 = std::env::var("BIND_PORT")
        .unwrap_or_else(|_| "8765".to_string())
        .parse()
        .unwrap_or(8765);

    let public_opt_in = std::env::var(config::BIND_PUBLIC_OPT_IN).is_ok();
    let addr = match bind_host.parse::<std::net::IpAddr>() {
        Ok(ip) => {
            if ip == std::net::IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0)) && !public_opt_in {
                eprintln!(
                    "WARNING: BIND_HOST is 0.0.0.0 (all interfaces). Set {}=1 to explicitly opt in to public exposure.",
                    config::BIND_PUBLIC_OPT_IN
                );
            }
            SocketAddr::from((ip, bind_port))
        }
        Err(_) => {
            if public_opt_in {
                eprintln!(
                    "Invalid BIND_HOST '{}'; BIND_PUBLIC is set, falling back to 0.0.0.0 (public exposure opted in).",
                    bind_host
                );
                SocketAddr::from(([0, 0, 0, 0], bind_port))
            } else {
                eprintln!(
                    "Invalid BIND_HOST '{}'; refusing to bind on all interfaces. Set BIND_PUBLIC=1 to expose publicly, or fix BIND_HOST.",
                    bind_host
                );
                std::process::exit(2);
            }
        }
    };

    // refuse to serve on a non-loopback bind with no auth.
    // Runs after `addr` resolves + `auth_mode` is known, before the socket is
    // bound. The guard is a pure function (unit-tested) so the startup path
    // stays deterministic. See `enforce_loopback_bind_guard`.
    enforce_loopback_bind_guard(&addr, auth_mode)?;

    println!("🚀 Server: http://{}:{}", bind_host, bind_port);
    // make the two unsigned-by-default egress signatures
    // a visible startup warning, never a silent default — an operator shipping a
    // webhook sink should know the payload integrity is off until the secret is
    // set. `eprintln!` so it lands in `err.log` beside the rest of the warnings.
    if crate::config::alert_webhook_url().is_some()
        && crate::config::alert_webhook_secret().is_none()
    {
        eprintln!(
            "⚠️ BRAIN_ALERT_WEBHOOK_URL is set but BRAIN_ALERT_WEBHOOK_SECRET is not — \
         alert webhook payloads are sent UNSIGNED (a receiver cannot verify integrity)."
        );
    }
    if crate::config::dsar_webhook_url().is_some() && crate::config::dsar_webhook_secret().is_none()
    {
        eprintln!(
            "⚠️ BRAIN_DSAR_WEBHOOK_URL is set but BRAIN_DSAR_WEBHOOK_SECRET is not — \
         DSAR Art-19 notifications are sent UNSIGNED."
        );
    }

    Ok(BootOutcome::Serve(Bootstrap {
        state: app_state,
        shutdown_pool,
        addr,
    }))
}

// ── boot guards folded in (Vaulting): argv gate, worker threads, loopback
// bind guard, and the constant-time token compare.
// replaced a hand-rolled fold with `subtle::ConstantTimeEq`, which
// is backed by asm/black_box primitives that the optimizer cannot short-
// circuit. `subtle` is already a transitive dep (sha2/hmac/aes-gcm), so this
// adds zero build surface. The length check below is inherently leaky, but
// token length is not secret for a fixed-format random token.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq as _;
    a.len() == b.len() && a.ct_eq(b).unwrap_u8() == 1
}

/// Handle CLI flags before any side effect. Prints version/usage and exits;
/// rejects unknown `-`-prefixed flags so the server never starts silently on
/// a typo (e.g. `brain-server --version` previously launched the server).
/// Positional args are allowed through (back-compat for any wrapper script).
pub fn handle_cli_args() {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-V" | "--version" => {
                println!("brain-server {}", SERVER_VERSION);
                std::process::exit(0);
            }
            "-h" | "--help" => {
                println!(
                    "brain-server {} — HTTP memory/recall server",
                    SERVER_VERSION
                );
                println!();
                println!("Run as a launchd service (see scripts/install-service.sh) or directly:");
                println!("  brain-server                start server on $BIND_HOST:$BIND_PORT");
                println!("  brain-server --version      print version and exit");
                println!("  brain-server --help         print this help and exit");
                println!(
                    "  brain-server --re-embed <profile>  rebuild the vector store at a profile's dim, then exit"
                );
                println!(
                    "  brain-server --re-audit     re-anchor the audit chain under hmac256 (v1.27.31), then exit"
                );
                println!();
                println!("Env: BIND_HOST, BIND_PORT, BRAIN_DB_PATH, AUTH_TOKEN_FILE, RUST_LOG");
                println!(
                    "      BRAIN_AUDIT_CHAIN_KEY / BRAIN_AUDIT_CHAIN_KEY_FILE (audit chain HMAC key)"
                );
                std::process::exit(0);
            }
            // Offline one-shot modes handled later in main_inner — passthrough.
            "--re-embed" | "--re-audit" => {}
            other if other.starts_with('-') => {
                eprintln!("brain-server: unknown flag '{other}'");
                eprintln!("  pass --help for usage, or run with no args to start the server");
                std::process::exit(2);
            }
            _ => {}
        }
    }
}

pub fn worker_threads() -> Option<usize> {
    std::env::var("BRAIN_WORKER_THREADS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&n| n > 0)
}

// ── startup bind fail-closed ────────────────
// `handlers/mod.rs` treats a `None` principal as superuser (by-design
// loopback). The symmetric gap: a non-loopback bind with no AUTH_TOKEN/JWT is
// an open superuser API. Fail-closed file-perms checks already exist; this is the
// matching posture on the bind side. Two pure predicates + one guard, all
// unit-testable without a live socket.
//
// ponytail: startup-only enforcement — once running, a rebind is not re-checked
// (the OS socket is already bound). Does NOT add per-principal rate limiting
// (v2.1, needs Redis) or change the in-memory per-IP limiter.

/// True when the resolved bind address is loopback (`127.0.0.0/8` or `::1`).
/// `SocketAddr` always carries a resolved IP, so a hostname like `localhost`
/// never reaches here as a string — it either resolved to 127.0.0.1 (loopback)
/// or the startup path already exited in the parse-failure branch above.
pub(crate) fn bind_is_loopback(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

/// True when SOME auth gate is configured: a non-empty opaque-token set OR JWT
/// mode. Reuses `config::auth_tokens()` + `AuthMode` — does not duplicate token
/// resolution. A non-loopback bind with this false is an open superuser API.
pub(crate) fn auth_configured(auth_mode: auth::AuthMode) -> bool {
    auth_mode.is_jwt() || !config::auth_tokens().is_empty()
}

/// Refuse to start if the bind is beyond loopback AND no auth is configured.
/// The same posture applied to the bind side (fail-closed, clear message, exit).
pub(crate) fn enforce_loopback_bind_guard(
    addr: &SocketAddr,
    auth_mode: auth::AuthMode,
) -> Result<()> {
    if !bind_is_loopback(addr) && !auth_configured(auth_mode) {
        return Err(anyhow::anyhow!(
            "refusing to start: non-loopback bind ({}) with no AUTH_TOKEN/JWT — \
             this would expose an unauthenticated superuser API. \
             Set AUTH_TOKEN_FILE, configure JWT, or bind to 127.0.0.1/::1.",
            addr
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ct_eq() {
        // Equal, same length.
        assert!(ct_eq(b"abcdef", b"abcdef"));
        // Differ in one byte, same length → false (no early exit path).
        assert!(!ct_eq(b"abcdef", b"abcXef"));
        // Differ in last byte → false.
        assert!(!ct_eq(b"abcdef", b"abcdeX"));
        // Different length → false.
        assert!(!ct_eq(b"abcdef", b"abc"));
        assert!(!ct_eq(b"abc", b"abcdef"));
        // Empty slices compare equal.
        assert!(ct_eq(b"", b""));
    }

    /// every non-public route's handler must
    /// `None`-principal-is-superuser behavior above — a non-loopback bind with
    /// no auth must refuse startup. Pure predicates + guard, no live socket.
    #[test]
    fn bind_is_loopback_and_auth_configured_predicates() {
        use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

        let loopback_v4 = SocketAddr::from((Ipv4Addr::new(127, 0, 0, 1), 8765));
        let loopback_v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, 8765));
        let any_v4 = SocketAddr::from((Ipv4Addr::new(0, 0, 0, 0), 8765));
        let site = SocketAddr::from((Ipv4Addr::new(192, 168, 1, 10), 8765));

        // bind_is_loopback: 127.0.0.0/8 + ::1 are loopback; 0.0.0.0 / site IPs
        // are not. (`localhost` as a hostname never reaches here as a SocketAddr
        // — it resolves to 127.0.0.1 upstream or exits in the parse-fail branch.)
        assert!(bind_is_loopback(&loopback_v4));
        assert!(bind_is_loopback(&loopback_v6));
        assert!(!bind_is_loopback(&any_v4));
        assert!(!bind_is_loopback(&site));

        // auth_configured: opaque tokens OR JWT. Empty tokens + Opaque => false.
        // SAFETY for env mutation: this process has no AUTH_TOKEN_FILE set during
        // the normal test run, so auth_tokens() is empty here. We assert both
        // arms of AuthMode against the same (empty-token) environment.
        let no_tokens_no_jwt = auth_configured(auth::AuthMode::Opaque);
        let jwt_mode = auth_configured(auth::AuthMode::Jwt);
        assert!(
            !no_tokens_no_jwt,
            "opaque mode with no tokens must be unauthenticated"
        );
        assert!(
            jwt_mode,
            "JWT mode counts as configured even with no opaque token"
        );

        // The guard: back-compat preserved (loopback + no-auth => Ok), and the
        // gap closed (non-loopback + no-auth => Err). Non-loopback + JWT => Ok.
        assert!(enforce_loopback_bind_guard(&loopback_v4, auth::AuthMode::Opaque).is_ok());
        assert!(enforce_loopback_bind_guard(&loopback_v6, auth::AuthMode::Opaque).is_ok());
        assert!(
            enforce_loopback_bind_guard(&any_v4, auth::AuthMode::Opaque).is_err(),
            "0.0.0.0 with no auth must refuse startup"
        );
        assert!(
            enforce_loopback_bind_guard(&site, auth::AuthMode::Opaque).is_err(),
            "site IP with no auth must refuse startup"
        );
        assert!(
            enforce_loopback_bind_guard(&site, auth::AuthMode::Jwt).is_ok(),
            "site IP with JWT configured is a valid (authenticated) public bind"
        );
    }
}
