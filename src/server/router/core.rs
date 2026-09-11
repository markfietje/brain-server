//! The core family: the SPA seat, health/ready/version/openapi, the
//! audit list + verify, metrics, stats, and the root redirect. Handlers
//! moved verbatim from main.rs; routes register in `router()` in the
//! chain's order (segments re-grouped by family — axum route matching
//! is per-path, and the order-sensitive layer positions are handled in
//! `mod.rs`).

use axum::{
    Router,
    extract::{Query, State},
    response::Json,
    routing::get,
};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::task;
use tokio::time::timeout;

use crate::alert;
use crate::audit;
use crate::config::{self, MODEL_ID, SERVER_VERSION};
use crate::handlers;
use crate::http_limit::process_rss_mib;
use crate::server::bootstrap::AppState;
use crate::server::router::memory::measure_capacity;
use sysinfo::System;

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Static SPA seat (host-frontend-static semantics).
        // Serves the built client dist; absent dist degrades to 404 so an
        // API-only deployment is unaffected. Public surface (no auth) — the
        // bundle is static, data flows only through the gated API routes.
        .route("/app/", get(handlers::frontend::spa_index))
        .route("/app/{*path}", get(handlers::frontend::spa_static))
        .route("/app/boot.json", get(handlers::frontend::boot_json))
        .route("/app/boot.js", get(handlers::frontend::boot_js))
        .route("/app/boot.pub", get(handlers::frontend::boot_pub))
        .route("/app/sw.js", get(handlers::frontend::sw_js))
        .route(
            "/app/sw-register.js",
            get(handlers::frontend::sw_register_js),
        )
        .route("/health", get(health))
        .route("/health/db", get(health_db))
        .route("/ready", get(ready))
        .route("/openapi.yaml", get(openapi))
        .route("/stats", get(stats))
        .route("/version", get(version))
        .route("/audit", get(list_audit))
        .route("/audit/verify", get(verify_audit_chain))
        .route("/metrics", get(metrics))
        // Static SPA seat is registered ABOVE (`/app/` + `/app/{*path}` →
        // `handlers::frontend`): MIME + path-traversal prevention + SPA
        // deep-link fallback to index.html live there. The historical
        // `nest_service("/app", ServeDir)` registration is GONE — axum 0.8
        // panics at boot on the conflicting internal wildcard
        // (/app/{*__private__axum_nest_tail_param} vs /app/{*path}), a
        // latent boot-blocker the 1.28.4 line never exercised end-to-end.
        // Root → the client shell (a 301 so browsers + the client's fetch base
        // both see a canonical `/app/`).
        .route(
            "/",
            get(|| async { axum::response::Redirect::permanent("/app/") }),
        )
}

/// Public `/health` — the load-balancer probe shape only (`status`/`version`).
/// Every deployment-fingerprinting field (model, otel, pool, backup, webhook,
/// hardening, DPO contact) moved behind the Read gate on `/health/db`
/// (surface-reduction — same class as the `/health/db` carve-out).
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "version": SERVER_VERSION }))
}

/// Build the detailed (Read-gated, `/health/db`) health body. Extracted as a
/// pure function so a regression test can pin the top-level key set — it must
/// never leak memory content or PII (CVE-2026-29787 class: health-endpoint
/// information disclosure). Public `/health` (see `health`) no longer carries
/// any of these fields.
#[allow(clippy::too_many_arguments)] // 8 health fields; a struct would add ceremony to the single call site
pub fn health_body(
    used_mb: u64,
    total_mb: u64,
    pool_connections: u32,
    pool_idle: u32,
    backup: serde_json::Value,
    capacity: Option<serde_json::Value>,
    integrity: serde_json::Value,
    audit_commit_failures: usize,
    db_busy_hits: usize,
) -> serde_json::Value {
    let mut body = serde_json::json!({
            "status": "ok",
            "version": SERVER_VERSION,
            "model": MODEL_ID,
            "system": {
                "memory_used_mb": used_mb,
                "memory_total_mb": total_mb,
                "memory_percent": if total_mb > 0 { (used_mb as f64 / total_mb as f64) * 100.0 } else { 0.0 }
            },
            "pool": {
                "connections": pool_connections,
                "idle_connections": pool_idle,
                "busy_connections": pool_connections.saturating_sub(pool_idle)
            },
            "backup": backup,
            // effective webhook posture at a glance.
            // `scheme` is the Standard Webhooks handshake when the timestamp-required
            // flag is set, else the legacy GitHub delivery-id idempotency path.
            "webhook": {
                "replay_secs": crate::config::WEBHOOK_REPLAY_SECS,
                "timestamp_required": crate::config::webhook_timestamp_required(),
                "scheme": if crate::config::webhook_timestamp_required() {
                    "standard-webhooks"
                } else {
                    "legacy"
                }
            },
            // OTLP export posture at a glance. `enabled`
            // reflects the runtime kill switch; `endpoint` the configured OTLP/HTTP
            // trace endpoint. Always present (defaults to disabled/loopback) so
            // health is uniform across builds.
            "otel": {
                "enabled": crate::config::otel_enabled(),
                "endpoint": crate::config::otel_endpoint(),
            },
    // the named Data Protection Officer
            // contact (from BRAIN_DPO_CONTACT) surfaced on the Read-gated
            // detail + the privacy notice. `null` when unset — the posture never
            // invents a contact. A data-subject / breach event needs a named
            // channel, and this proves the deployment configured one.
            "compliance": {
                "dpo_contact": crate::config::dpo_contact(),
            },
            "authn": {
                "enabled": !crate::config::auth_tokens().is_empty(),
                "required": crate::config::require_auth().unwrap_or(false),
            },
    // hardening observability. Lets ops see the
            // memory-safety posture at a glance. `unsafe_blocks` is the
            // audited count (each has a SAFETY comment); `panics_caught`
            // comes from CatchPanicLayer (would be >0 only if a handler
            // panicked and was caught).
    // `audit_commit_failures` — monotonic count of
            // best-effort audit-chain settles that could not COMMIT/ROLLBACK since
            // process start. Zero is the green state; >0 means a row the caller
            // believes is on the durable chain may not be. Read-only, no secrets.
            "hardening": {
                "unsafe_blocks": 1, // single shared lib call (register_sqlite_vec), no transmute
                "panics_caught": 0,
                "memory_leaks_detected": 0,
                "audit_commit_failures": audit_commit_failures,
                // law-13 contention gauge: SQLITE_BUSY surfaced as audit-seam
                // commit failures (busy_timeout burn-through), monotonic.
                "db_busy_hits": db_busy_hits,
                // whether the layer-2 injection classifier
                // is loaded. Mirrors `screen::screen_classifier_loaded()`; lets ops
                // confirm the opt-in model is actually active.
                "injection_classifier_loaded": crate::screen::screen_classifier_loaded(),
                // Pores: the classifier POSTURE tri-state — `on` (loaded and
                // scoring), `off` (explicit BRAIN_INJECTION_CLASSIFIER=off
                // opt-out), `absent` (no artifact resolved / feature not
                // compiled). Additive beside the boolean above.
                "injection_classifier": crate::screen::screen_classifier_state(),
                // the resolved INJECTION_POLICY (quarantine|reject|allow):
                // `allow` disables the screen entirely, so it is never silent
                // — the boot warn + this echo surface it.
                "injection_policy": crate::config::injection_policy_echo(),
                // tripwire: ingests screened-as-clean because the policy is
                // `allow` (monotonic; >0 with untrusted sources = re-posture).
                "allow_policy_bypasses": crate::screen::allow_policy_bypasses()
            }
        });
    if let Some(c) = capacity
        && let serde_json::Value::Object(ref mut m) = body
    {
        m.insert("capacity".to_string(), c);
    }
    // cached audit-chain posture from the integrity watcher —
    // never content, never PII (a hash + two booleans/timestamps).
    if let serde_json::Value::Object(ref mut m) = body {
        m.insert("integrity".to_string(), integrity);
    }
    body
}

pub(crate) async fn ready(State(s): State<Arc<AppState>>) -> impl axum::response::IntoResponse {
    let pool = s.pool.clone();
    let ready_future = task::spawn_blocking(move || {
        pool.get()
            .ok()
            .and_then(|c| c.query_row("SELECT 1", [], |_| Ok(true)).ok())
            .unwrap_or(false)
    });

    match timeout(StdDuration::from_secs(3), ready_future).await {
        Ok(Ok(true)) => "OK",
        _ => "NOT_READY",
    }
}

pub(crate) async fn version() -> impl axum::response::IntoResponse {
    SERVER_VERSION
}

/// Serve the API contract as OpenAPI 3.0 (YAML) so third parties and generated
/// clients can discover the routes without reading source. The document is
/// embedded at compile time (`include_str!`) so it ships with the binary and
/// cannot drift from the repo's canonical `openapi.yaml`.
pub(crate) async fn openapi() -> impl axum::response::IntoResponse {
    (
        axum::http::StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/yaml"),
        )],
        OPENAPI_YAML,
    )
}

/// Canonical OpenAPI document (kept in sync with the route table in
/// `build_app`). The `test_openapi_covers_routes` unit test asserts every
/// registered route path appears here.
pub const OPENAPI_YAML: &str = include_str!("../../../openapi.yaml");

pub async fn health_db(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
) -> Result<Json<serde_json::Value>, crate::handlers::HandlerError> {
    // Twokeys (X-A5): the FULL body rises to Admin on global. The detailed
    // deployment surface (model, otel, pool, backup, webhook, hardening,
    // DPO contact, durability posture, per-domain WAL) is operator
    // telemetry — a cross-tenant reader has no business enumerating it. A
    // Read principal gets the reduced public shape + `db_ok` (the one bit a
    // monitor needs); everyone else 403s. Public `/health` stays the
    // minimal probe shape.
    let is_admin =
        crate::handlers::authorize(&principal.0, crate::auth::Action::Admin, "", "global").is_ok();
    if !is_admin {
        crate::handlers::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
        let pool = s.pool.clone();
        let probe = task::spawn_blocking(move || pool.get().is_ok());
        let db_ok = timeout(StdDuration::from_secs(3), probe)
            .await
            .map(|inner| inner.unwrap_or(false))
            .unwrap_or(false);
        return Ok(Json(serde_json::json!({
            "status": "ok",
            "version": SERVER_VERSION,
            "db_ok": db_ok,
        })));
    }
    let pool = s.pool.clone();
    let snapshot = s.snapshot.clone();
    // Throughput: the WAL sweep covers EVERY registered domain (same target
    // list the audit chain verify uses). Collected owned so the blocking
    // closure stays 'static.
    let wal_targets = handlers::domain_pools(&s.registry, &pool);

    let db_future = task::spawn_blocking(move || {
        let mut sys = System::new();
        sys.refresh_memory();
        let pool_state = pool.state();
        // capacity measurement needs a connection. Best-effort — if the
        // pool is exhausted, capacity is omitted rather than failing the probe.
        let conn = pool.get().ok();
        let capacity = conn.as_ref().map(|c| measure_capacity(c));
        // DB size measured through the open connection (see
        // `capacity::db_size_bytes`) — no state-derived path expression.
        let db_size = conn
            .as_ref()
            .map(|c| crate::capacity::db_size_bytes(c))
            .unwrap_or(0);
        let last_write: Option<String> = conn.as_ref().and_then(|c| {
            c.query_row("SELECT MAX(created_at) FROM knowledge", [], |r| r.get(0))
                .ok()
        });
        // Throughput: the ONLY place the WAL PRAGMA runs — admin cold path
        // (Read-gated detail), never per request. PASSIVE never blocks
        // writers; the row is (busy, log, checkpointed) and "pending" is
        // log − checkpointed. A domain that cannot open, or a pragma that
        // errors, is absent from the snapshot — never invented as zero.
        let mut wal: Vec<(String, u64)> = Vec::new();
        for (domain, pool) in &wal_targets {
            let Some(p) = pool else { continue };
            let Ok(c) = p.get() else { continue };
            let (log, checkpointed): (i64, i64) =
                match c.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| {
                    Ok((r.get(1)?, r.get(2)?))
                }) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
            wal.push((
                domain.clone(),
                log.saturating_sub(checkpointed).max(0) as u64,
            ));
        }
        crate::concurrency::CONCURRENCY.set_wal_snapshot(wal);
        Ok::<_, anyhow::Error>((
            sys.used_memory() / 1_000_000,
            sys.total_memory() / 1_000_000,
            pool_state,
            capacity,
            snapshot.read(),
            db_size,
            last_write,
        ))
    });

    match timeout(StdDuration::from_secs(3), db_future).await {
        Ok(Ok(Ok((used_mb, total_mb, pool_state, capacity, snapshot, db_size, last_write)))) => {
            let backup = snapshot.to_json();
            let cw = s.chain_watch.read();
            let integrity = serde_json::json!({
                "chain_ok": cw.chain_ok,
                "last_checked_at": cw.checked_at,
                "chain_head": cw.chain_head,
            });
            let mut body = health_body(
                used_mb,
                total_mb,
                pool_state.connections,
                pool_state.idle_connections,
                backup,
                capacity,
                integrity,
                crate::audit::audit_commit_failures(),
                crate::audit::busy_hits(),
            );
            if let serde_json::Value::Object(ref mut m) = body {
                m.insert("database_size_bytes".to_string(), db_size.into());
                m.insert("last_write".to_string(), last_write.into());
                // Throughput: contention telemetry as additive JSON keys
                // (the same numbers /metrics scrapes; counters process-local).
                let wal: serde_json::Map<String, serde_json::Value> =
                    crate::concurrency::CONCURRENCY
                        .wal_snapshot()
                        .into_iter()
                        .map(|(d, n)| (d, serde_json::Value::from(n)))
                        .collect();
                m.insert(
                    "concurrency".to_string(),
                    serde_json::json!({
                        "pool_timeouts_total": crate::concurrency::CONCURRENCY.pool_timeouts(),
                        "busy_errors_total": crate::concurrency::CONCURRENCY.busy_errors(),
                        "wal_pages_pending": serde_json::Value::Object(wal),
                    }),
                );
                // Headroom: static boot-time config echo — the durability
                // policy every pooled connection was initialized with. A
                // snapshot of boot decisions, never a per-request pragma
                // read, so the Read-gated cold path stays cold.
                m.insert(
                    "durability".to_string(),
                    serde_json::json!({
                        "synchronous": s.durability.synchronous_mode.as_str(),
                        "wal_autocheckpoint_pages": s.durability.wal_autocheckpoint_pages,
                        "capacity_target": match crate::capacity::capacity_target() {
                            crate::capacity::CapacityTarget::Desktop => "desktop",
                            crate::capacity::CapacityTarget::Jetson => "jetson",
                        },
                    }),
                );
                // Loom: the boot-resolved parallelism decision, same class as
                // the durability echo — a snapshot of a boot decision, never
                // a per-request read. `active (N threads)` | `off:<reason>`.
                m.insert("loom".to_string(), serde_json::json!(s.loom.describe()));
            }
            Ok(Json(body))
        }
        _ => Ok(Json(
            serde_json::json!({ "status": "error", "error": "Health check failed" }),
        )),
    }
}

/// `GET /audit` — read-only operator diagnostics. Returns recent
/// audit events, optionally filtered by `kind` and bounded by `limit` (default
/// 100, capped at `config::MAX_MULTI_GET`). All rows are hashes only — no
/// secrets survive the round-trip. Gated by `auth_middleware` like other
/// non-public routes.
///
/// optional `tenant` filter scopes rows at the SQL layer.
pub(crate) async fn list_audit(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Query(params): Query<AuditQuery>,
) -> Json<serde_json::Value> {
    // Admin gate + tenant scope. The v1.2 matrix makes
    // `/audit` an Admin surface AND forbids cross-tenant reads: a principal
    // only ever sees its own tenant's rows (superuser `None` keeps v1.1
    // passthrough). Legacy shape.
    if let Err(e) =
        crate::handlers::authorize(&principal.0, crate::auth::Action::Admin, "", "global")
    {
        return Json(serde_json::json!({ "error": e.inner.message }));
    }
    let tenant_scope = match crate::handlers::audit_scope(&principal.0, &params.tenant) {
        Ok(t) => t,
        Err(e) => return Json(serde_json::json!({ "error": e.inner.message })),
    };
    let limit = params.limit.unwrap_or(100).min(config::MAX_MULTI_GET);
    let kind = params.kind;
    let tenant = tenant_scope;
    // The operator audit list covers EVERY registered
    // domain's chain in multi-db mode (rows carry their domain tag), not just
    // the global pool. Shim mode stays the single shared pool. Merged rows
    // sort newest-first across domains (ts text sort + id tiebreak — ids are
    // per-DB, only meaningful within their domain).
    let targets = crate::handlers::domain_pools(&s.registry, &s.pool);
    let offset = params.offset;
    let rows = task::spawn_blocking(move || -> Vec<serde_json::Value> {
        let mut merged: Vec<serde_json::Value> = Vec::new();
        for (domain, pool) in &targets {
            let Some(pool) = pool else {
                continue; // an unopenable domain contributes no rows; the
                // chain-verify surfaces report it as not-ok (fail-closed
                // lives there — a row listing cannot attest anything).
            };
            let Ok(conn) = pool.get() else {
                continue;
            };
            if let Ok(page) = audit::recent_tenant(
                &conn,
                kind.as_deref(),
                tenant.as_deref(),
                limit.saturating_add(offset),
                0,
            ) {
                for r in page {
                    let mut v = serde_json::to_value(&r).unwrap_or(serde_json::Value::Null);
                    v["domain"] = serde_json::Value::String(domain.to_owned());
                    merged.push(v);
                }
            }
        }
        merged.sort_by(|a, b| {
            let ts_a = a["ts"].as_str().unwrap_or("");
            let ts_b = b["ts"].as_str().unwrap_or("");
            ts_b.cmp(ts_a).then_with(|| {
                let id_a = a["id"].as_i64().unwrap_or(0);
                let id_b = b["id"].as_i64().unwrap_or(0);
                id_b.cmp(&id_a)
            })
        });
        merged.into_iter().skip(offset).take(limit).collect()
    })
    .await
    .unwrap_or_else(|e| {
        tracing::warn!("audit list join failed: {e}");
        Vec::new()
    });
    Json(serde_json::json!({ "events": rows }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuditQuery {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    tenant: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    /// pagination cursor. `offset` past the last row returns [].
    #[serde(default)]
    offset: usize,
}

/// The per-domain label a principal's `/metrics` scrape may carry: the
/// domain NAME renders only when the principal's scope
/// grants Read there — the same predicate the read paths use
/// (`can_read_domain`); otherwise the label collapses to `"other"` (the
/// count stays visible, the name does not). The None superuser sees every
/// name — byte-identical to the single-token scrape.
pub fn scoped_domain_label(principal: &Option<crate::auth::Principal>, domain: &str) -> String {
    if crate::handlers::can_read_domain(principal, domain) {
        domain.to_string()
    } else {
        "other".to_string()
    }
}

/// `GET /metrics` — Prometheus text-format exporter.
/// Reuses the same numbers `/health` reports; no Prometheus client dep, just
/// the wire format. Auth-gated like other operator surfaces (`auth_middleware`).
///
/// ponytail: hand-rolled text format (4 lines of HELP/TYPE + gauge). Pulling
/// `prometheus` or `metrics` crate for this would be a 12+ transitive-dep tax
/// for a feature the plan itself flags as risky. Format is stable and trivial.
pub(crate) async fn metrics(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
) -> (axum::http::StatusCode, String) {
    // AuthZ read gate. Prometheus text is the body; a 403
    // with the reason keeps the non-JSON contract.
    if let Err(e) =
        crate::handlers::authorize(&principal.0, crate::auth::Action::Read, "", "global")
    {
        return (axum::http::StatusCode::FORBIDDEN, e.inner.message);
    }
    let pool = s.pool.clone();
    let audit_cache = std::sync::Arc::clone(&s.audit_chain_cache);
    // The gauge aggregates EVERY registered domain's chain —
    // collected here (owned) so the blocking closure stays 'static.
    // Throughput: the SAME target list feeds the per-domain pool gauges
    // (`brain_pool_in_use`/`brain_pool_idle` from `r2d2::State` snapshots
    // taken at scrape) — no extra registry work.
    let chain_targets = crate::handlers::domain_pools(&s.registry, &pool);
    // Twokeys (X-A5): per-domain labels render only for in-scope principals;
    // out-of-scope domains collapse to `domain="other"` (summed — duplicate
    // series labels are illegal in the text format). Global gauges are
    // unchanged; the None superuser sees every name.
    let principal = principal.0.clone();
    let body = task::spawn_blocking(move || -> String {
        let pool_state = pool.state();
        let busy = pool_state
            .connections
            .saturating_sub(pool_state.idle_connections);
        // Reuse the capacity measurement so `/metrics` and `/health` agree.
        let cap = pool.get().ok().map(|c| measure_capacity(&c));
        // report THIS process's RSS, not system-wide used memory.
        // `System::used_memory()` is the whole-host figure; the gauge's HELP
        // says "Process RSS in MiB" and must match the per-process 320 MB
        // capacity envelope that `/health` reports (see `process_rss_mib`).
        let used_mib = process_rss_mib();
        let cap_status = cap
            .as_ref()
            .and_then(|c| c.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        // Throughput contention telemetry: per-domain pool gauges
        // from the SAME target list the chain gauge consumes (state() is an
        // in-memory snapshot — no syscalls). Computed BEFORE the chain block
        // below, which may consume (move) the target vec on a cache miss.
        // Twokeys: out-of-scope domains sum into one `other` series per gauge.
        let mut domain_pool_lines = String::new();
        let mut other_in_use: Option<u32> = None;
        let mut other_idle: Option<u32> = None;
        for (domain, pool) in &chain_targets {
            if let Some(p) = pool {
                let st = p.state();
                let in_use = st.connections.saturating_sub(st.idle_connections);
                if scoped_domain_label(&principal, domain) == "other" {
                    *other_in_use.get_or_insert(0) += in_use;
                    *other_idle.get_or_insert(0) += st.idle_connections;
                    continue;
                }
                domain_pool_lines.push_str(&format!(
                    "brain_pool_in_use{{domain=\"{domain}\"}} {in_use}\n"
                ));
                domain_pool_lines.push_str(&format!(
                    "brain_pool_idle{{domain=\"{domain}\"}} {}\n",
                    st.idle_connections
                ));
            }
        }
        if let Some(in_use) = other_in_use {
            domain_pool_lines.push_str(&format!(
                "brain_pool_in_use{{domain=\"other\"}} {in_use}\n"
            ));
            if let Some(idle) = other_idle {
                domain_pool_lines
                    .push_str(&format!("brain_pool_idle{{domain=\"other\"}} {idle}\n"));
            }
        }
        let chain_ok = {
            // /metrics path: use the TTL cache so a scrape doesn't trigger a
            // full O(n) chain scan (now × N domains). /audit/verify bypasses
            // this for the authoritative answer.
            let now = std::time::Instant::now();
            let cached = crate::concurrency::mutex_guard_measured(&audit_cache)
                .ok()
                .and_then(|g| *g)
                .filter(|(ts, _)| {
                    now.duration_since(*ts).as_secs() < config::AUDIT_CHAIN_CACHE_TTL_SECS
                });
            match cached {
                Some((_, ok)) => ok,
                None => {
                    let fresh = crate::handlers::verify_domain_targets(chain_targets)
                        .iter()
                        .all(|(_, ok)| *ok);
                    if let Ok(mut g) = crate::concurrency::mutex_guard_measured(&audit_cache) {
                        *g = Some((now, fresh));
                    }
                    fresh
                }
            }
        };
        let mut out = String::with_capacity(512);
        out.push_str("# HELP brain_rss_mib Process RSS in MiB.\n");
        out.push_str("# TYPE brain_rss_mib gauge\n");
        out.push_str(&format!("brain_rss_mib {used_mib}\n"));
        out.push_str("# HELP brain_pool_connections Pool connection counts.\n");
        out.push_str("# TYPE brain_pool_connections gauge\n");
        out.push_str(&format!(
            "brain_pool_connections{{state=\"idle\"}} {}\n",
            pool_state.idle_connections
        ));
        out.push_str(&format!(
            "brain_pool_connections{{state=\"busy\"}} {busy}\n"
        ));
        // Throughput: the precomputed per-domain gauges, then the
        // process-local counters, then the /health/db-refreshed WAL snapshot.
        // Additive series; the scrape adds nothing to any request path.
        out.push_str(&domain_pool_lines);
        out.push_str("# HELP brain_pool_timeouts_total Pool checkouts that timed out (r2d2 get() failure) since process start. Monotonic.\n");
        out.push_str("# TYPE brain_pool_timeouts_total counter\n");
        out.push_str(&format!(
            "brain_pool_timeouts_total {}\n",
            crate::concurrency::CONCURRENCY.pool_timeouts()
        ));
        out.push_str("# HELP brain_busy_errors_total SQLITE_BUSY-family errors observed at the governed-write BEGIN sites (WorkflowTx::begin + the workflow lane) since process start. Audit-seam busy is brain_db_busy_total. Monotonic.\n");
        out.push_str("# TYPE brain_busy_errors_total counter\n");
        out.push_str(&format!(
            "brain_busy_errors_total {}\n",
            crate::concurrency::CONCURRENCY.busy_errors()
        ));
        out.push_str("# HELP brain_wal_pages_pending WAL frames not yet checkpointed, per domain — last /health/db snapshot (the PASSIVE checkpoint PRAGMA runs ONLY there, cold path). Absent domains: no snapshot yet. Twokeys: domains outside the scrape principal's read scope collapse into the summed `other` label.\n");
        out.push_str("# TYPE brain_wal_pages_pending gauge\n");
        let mut other_wal: Option<u64> = None;
        for (domain, pending) in crate::concurrency::CONCURRENCY.wal_snapshot() {
            if scoped_domain_label(&principal, &domain) == "other" {
                *other_wal.get_or_insert(0) += pending;
                continue;
            }
            out.push_str(&format!(
                "brain_wal_pages_pending{{domain=\"{domain}\"}} {pending}\n"
            ));
        }
        if let Some(pending) = other_wal {
            out.push_str(&format!(
                "brain_wal_pages_pending{{domain=\"other\"}} {pending}\n"
            ));
        }
        // Headroom: lock-wait bucket-quantiles. Only CONTENDED acquisitions
        // are recorded (try_lock fast path costs nothing), so 0 means "no
        // contention observed", never "gauge wired off". The values are
        // bucket lower edges in µs — deterministic read of the histogram, not
        // an interpolated percentile.
        let lock_hist = crate::concurrency::CONCURRENCY.lock_wait_histogram();
        out.push_str("# HELP brain_lock_wait_micros_p50 Bucket-quantile (lower edge, µs) of contended lock-acquire waits across the instrumented request-path locks; 0 = no contention observed.\n");
        out.push_str("# TYPE brain_lock_wait_micros_p50 gauge\n");
        out.push_str(&format!(
            "brain_lock_wait_micros_p50 {}\n",
            lock_hist.quantile_edge_us(0.50)
        ));
        out.push_str("# HELP brain_lock_wait_micros_p95 Bucket-quantile (lower edge, µs) of contended lock-acquire waits across the instrumented request-path locks; 0 = no contention observed.\n");
        out.push_str("# TYPE brain_lock_wait_micros_p95 gauge\n");
        out.push_str(&format!(
            "brain_lock_wait_micros_p95 {}\n",
            lock_hist.quantile_edge_us(0.95)
        ));
        out.push_str("# HELP brain_capacity_status 1=ok 2=warning 3=exceeded.\n");
        out.push_str("# TYPE brain_capacity_status gauge\n");
        let cap_num = match cap_status {
            "ok" => 1,
            "warning" => 2,
            "exceeded" => 3,
            _ => 0,
        };
        out.push_str(&format!("brain_capacity_status {cap_num}\n"));
        out.push_str("# HELP brain_audit_chain_ok 1=every registered domain's chain verifies, 0=tamper detected.\n");
        out.push_str("# TYPE brain_audit_chain_ok gauge\n");
        out.push_str(&format!("brain_audit_chain_ok {}\n", u8::from(chain_ok)));
        out.push_str("# HELP brain_db_busy_total SQLITE_BUSY events surfaced at the audit seam (busy_timeout burn-through). Monotonic.\n");
        out.push_str("# TYPE brain_db_busy_total counter\n");
        out.push_str(&format!(
            "brain_db_busy_total {}\n",
            crate::audit::busy_hits()
        ));
        out
    })
    .await
    .unwrap_or_default();
    (axum::http::StatusCode::OK, body)
}

/// `GET /audit/verify` — read-only check that the audit hash
/// chain is intact. Returns `{ "ok": bool, "domains": {name: bool} }`.
/// Exposed separately from `GET /audit` because the chain check is a
/// full-table scan and shouldn't run on every list call.
/// Verifies EVERY registered domain's chain, not just the global
/// pool — `ok` is the all-domains aggregate and a per-domain breakdown is
/// attached so the failing domain is named, never silent.
pub async fn verify_audit_chain(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
) -> Json<serde_json::Value> {
    // Admin gate (tamper-detection surface). Legacy shape.
    if let Err(e) =
        crate::handlers::authorize(&principal.0, crate::auth::Action::Admin, "", "global")
    {
        return Json(serde_json::json!({ "error": e.inner.message }));
    }
    let targets = crate::handlers::domain_pools(&s.registry, &s.pool);
    let results = task::spawn_blocking(move || crate::handlers::verify_domain_targets(targets))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("verify chain join failed: {e}");
            Vec::new()
        });
    let ok = results.iter().all(|(_, ok)| *ok);
    // a failed chain verify is a decision-critical alert —
    // the payload names the failing domains.
    if !ok {
        let failing: Vec<&str> = results
            .iter()
            .filter(|(_, ok)| !ok)
            .map(|(d, _)| d.as_str())
            .collect();
        alert::publish(
            &s,
            alert::ALERT_KIND_CHAIN,
            serde_json::json!({ "failing_domains": failing }),
        );
    }
    let domains: serde_json::Map<String, serde_json::Value> = results
        .into_iter()
        .map(|(d, ok)| (d, serde_json::Value::Bool(ok)))
        .collect();
    Json(serde_json::json!({ "ok": ok, "domains": domains }))
}

pub(crate) async fn stats(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Query(params): Query<StatsQuery>,
) -> Json<serde_json::Value> {
    // AuthZ read gate. Legacy shape — see `/add`.
    if let Err(e) = crate::handlers::authorize(
        &principal.0,
        crate::auth::Action::Read,
        "",
        params.domain.as_deref().unwrap_or("global"),
    ) {
        return Json(serde_json::json!({ "success": false, "error": e.inner.message }));
    }
    // resolve per-domain pool from the ?domain= query param.
    let pool = handlers::resolve_domain_pool(&s.registry, params.domain.as_deref())
        .unwrap_or_else(|_| s.pool.clone());
    // Shim mode binds the label into every count — the pool
    // is shared there, so unscoped COUNTs reported another tenant's corpus
    // size to a per-domain reader. Multi-db pools are territory-scoped.
    let shim_label = if s.registry.is_multi_db() {
        None
    } else {
        Some(
            params
                .domain
                .clone()
                .unwrap_or_else(|| "global".to_string()),
        )
    };
    let stats_future = task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| anyhow::anyhow!(e))?;
        let (count, embed_count): (i64, i64) = if let Some(label) = &shim_label {
            (
                conn.query_row(
                    "SELECT COUNT(*) FROM knowledge WHERE domain = ?1",
                    [label],
                    |r| r.get(0),
                )?,
                conn.query_row(
                    "SELECT COUNT(*) FROM vec_knowledge v JOIN knowledge k ON k.id = v.knowledge_id
                     WHERE k.domain = ?1",
                    [label],
                    |r| r.get(0),
                )
                .unwrap_or(0),
            )
        } else {
            (
                conn.query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM vec_knowledge", [], |r| r.get(0))
                    .unwrap_or(0),
            )
        };
        // Entities/relationships are linked to chunks; scope by the chunk's
        // domain in shim mode (edges with no chunk link are unscopable — the
        // documented NULL-knowledge_id graph ceiling).
        let (entities, relationships): (i64, i64) = if let Some(label) = &shim_label {
            (
                conn.query_row(
                    "SELECT COUNT(DISTINCT e.id) FROM entities e
                     WHERE EXISTS (
                       SELECT 1 FROM relationships r JOIN knowledge k ON k.id = r.knowledge_id
                        WHERE (r.from_entity_id = e.id OR r.to_entity_id = e.id) AND k.domain = ?1)",
                    [label],
                    |r| r.get(0),
                )
                .unwrap_or(0),
                conn.query_row(
                    "SELECT COUNT(*) FROM relationships r JOIN knowledge k ON k.id = r.knowledge_id
                     WHERE k.domain = ?1",
                    [label],
                    |r| r.get(0),
                )
                .unwrap_or(0),
            )
        } else {
            (
                conn.query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
                    .unwrap_or(0),
                conn.query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
                    .unwrap_or(0),
            )
        };
        Ok::<_, anyhow::Error>((count, embed_count, entities, relationships))
    });

    match timeout(StdDuration::from_secs(10), stats_future).await {
        Ok(Ok(Ok((count, embed_count, entities, relationships)))) => Json(serde_json::json!({
            "count": count,
            "embeddings": embed_count,
            "entities": entities,
            "relationships": relationships,
            "model": MODEL_ID,
            "version": SERVER_VERSION
        })),
        Ok(Ok(Err(e))) => Json(serde_json::json!({
            "count": 0,
            "embeddings": 0,
            "entities": 0,
            "relationships": 0,
            "model": MODEL_ID,
            "version": SERVER_VERSION,
            "error": e.to_string()
        })),
        Ok(Err(_)) => Json(serde_json::json!({
            "count": 0,
            "embeddings": 0,
            "entities": 0,
            "relationships": 0,
            "model": MODEL_ID,
            "version": SERVER_VERSION,
            "error": "Task join error"
        })),
        Err(_) => Json(serde_json::json!({
            "count": 0,
            "embeddings": 0,
            "entities": 0,
            "relationships": 0,
            "model": MODEL_ID,
            "version": SERVER_VERSION,
            "error": "Request timed out"
        })),
    }
}

#[derive(Deserialize)]
pub(crate) struct StatsQuery {
    #[serde(default)]
    domain: Option<String>,
}
