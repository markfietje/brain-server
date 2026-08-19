//! Configuration constants for brain-server

pub const MODEL_ID: &str = "minishlab/potion-retrieval-32M";
pub const DEFAULT_K: usize = 5;
pub const MAX_K: usize = 100;
/// Drive version from Cargo.toml so /version and logs always match the build.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const MAX_REQUEST_SIZE: usize = 1024 * 1024;
pub const MAX_QUERY_LENGTH: usize = 2000;

/// max distinct client-IP buckets the rate limiter tracks
/// before evicting the oldest 25%. Bounds memory against an attacker cycling
/// spoofed `X-Forwarded-For` values. ponytail: in-process LRU — multi-instance
/// rate limiting needs a shared store (v2.1).
pub const RATE_LIMIT_MAX_KEYS: usize = 10_000;

/// the maximum number of per-domain DB files a
/// multi-db registry may register/create. The domain-creation surface is the
/// ONLY file-creation path; every other name resolves `domain_unknown`, so
/// this cap bounds disk-fill from a probeable API. Override with
/// `BRAIN_MAX_DOMAIN_DBS`.
pub const MAX_DOMAIN_DBS: usize = 256;

/// bounds for the opt-in `/suggest` surface. `k` is capped
/// small because suggestions are supplementary context, not a replacement for
/// `/recall`; `exclude` is capped so a caller can't OOM the NOT IN clause.
pub const MAX_SUGGEST_K: u32 = 20;
pub const MAX_SUGGEST_EXCLUDE: usize = 100;
pub const DEFAULT_SUGGEST_K: u32 = 5;

/// M2 evidence quality: bounded snippet window (chars) and the redaction cap
/// on `explain` payloads so they can never leak unbounded source text.
pub const MAX_SNIPPET_CHARS: usize = 240;
pub const SNIPPET_CONTEXT_CHARS: usize = 60;
pub const MAX_EXPLAIN_BYTES: usize = 64 * 1024;
pub const MAX_MULTI_GET: usize = 1000;

/// graph endpoints return a bounded edge set. The KG's clean
/// semantic edge set is far below this; the cap exists for the pathological
/// mega-hub. Mirrors the `MAX_MULTI_GET` bound pattern (consts, not env limits).
pub const MAX_GRAPH_EDGES: i64 = 500;

/// `/decayed` returns a paged, bounded first page (default
/// 500, `?offset=` pages the rest). Matches the graph cap for operator review.
pub const MAX_DECAYED: i64 = 500;

/// how long an auto-captured proposal can sit in the
/// review queue before a human decides it. A stale prompt's context-loss makes
/// the candidate unverifiable, so beyond this the queue refuses to act and the
/// row is audited as expired. Default 7 days. `BRAIN_PROPOSAL_TTL_SECS` overrides.
pub const DEFAULT_PROPOSAL_TTL_SECS: i64 = 7 * 24 * 3600;

/// the four fixed alert kinds fed by the decision cores.
/// The `/events` SSE feed + `BRAIN_ALERT_WEBHOOK_URL` sink both speak these.
pub const ALERT_KIND_PENDING: &str = "pending";
pub const ALERT_KIND_EXPIRY: &str = "expiry";
pub const ALERT_KIND_SCREEN: &str = "screen";
pub const ALERT_KIND_CHAIN: &str = "chain";

/// `GET /events` SSE broadcast buffer. Bounded — a slow
/// consumer drops missed events (broadcast lag semantics), never blocks the
/// publish path (same posture as `UMP_EVENT_BUFFER`).
pub const ALERT_EVENT_BUFFER: usize = 256;

/// how often the expiry watcher scans pending proposals.
/// The tier clock is a watch, not a tick-per-event — a proposal only alerts
/// as it crosses a boundary, so a coarse interval is honest.
pub const ALERT_WATCH_INTERVAL_SECS: u64 = 30;

/// default cadence for the integrity watcher's full-chain
/// check (`audit::verify_chain`). The chain is a tamper-evident log; a 60s
/// watch makes a break a watched signal instead of a manual `/audit/verify`.
/// Read via [`chain_check_secs`].
pub const DEFAULT_CHAIN_CHECK_SECS: u64 = 60;

/// SLA tier boundaries (seconds remaining), mirroring the
/// client ops-clock budgets (`< 5 min` critical, `< 1 hr` warn). A proposal
/// alerts once per boundary crossed, not on every scan.
pub const ALERT_CRITICAL_SECS: i64 = 5 * 60;
pub const ALERT_WARN_SECS: i64 = 60 * 60;

/// Optional alert webhook sink. When set, every alert is enqueued to the
/// existing `webhook_queue` (kind='alert') AND POSTed to this URL via the
/// Standard Webhooks handshake. Fail-soft: a sink failure never
/// blocks the SSE publish path.
pub fn alert_webhook_url() -> Option<String> {
    std::env::var("BRAIN_ALERT_WEBHOOK_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// HMAC secret for the alert webhook's `webhook-signature: v1,<base64>`.
/// When unset, the sink sends the headers unsigned (documented — the caller
/// should still receive the notification; signing is best practice).
pub fn alert_webhook_secret() -> Option<String> {
    std::env::var("BRAIN_ALERT_WEBHOOK_SECRET")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn proposal_ttl_secs() -> i64 {
    std::env::var("BRAIN_PROPOSAL_TTL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_PROPOSAL_TTL_SECS)
}

/// the GDPR Art 17/12 DSAR response window (days). How
/// long after a request's `created_at` the erosive-erasure deadline lapses.
/// This is a *commitment the ledger shows*, not an enforced bound — a
/// reminder/notification channel is v2.x, and the ledger TTL is the
/// only automatic bound.
pub const DEFAULT_DSAR_WINDOW_DAYS: i64 = 30;

/// the Art 17 erasure window in seconds.
/// `BRAIN_DSAR_WINDOW_DAYS` overrides the default (same env-resolution pattern
/// as `BRAIN_PROPOSAL_TTL_SECS`).
pub fn dsar_window_secs() -> i64 {
    std::env::var("BRAIN_DSAR_WINDOW_DAYS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|d| *d > 0)
        .map(|d| d * 86400)
        .unwrap_or(DEFAULT_DSAR_WINDOW_DAYS * 86400)
}

/// integrity-watcher cadence (seconds). Reads
/// `BRAIN_CHAIN_CHECK_SECS`, defaulting to [`DEFAULT_CHAIN_CHECK_SECS`].
pub fn chain_check_secs() -> u64 {
    std::env::var("BRAIN_CHAIN_CHECK_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_CHAIN_CHECK_SECS)
}

pub const POOL_MAX_SIZE: u32 = 20;
pub const POOL_MIN_IDLE: u32 = 2;
pub const POOL_CONNECTION_TIMEOUT_SECS: u64 = 30;
pub const POOL_MAX_LIFETIME_SECS: u64 = 300;
pub const POOL_IDLE_TIMEOUT_SECS: u64 = 60;

/// mmap budget (MiB) for SQLite memory-mapped I/O. Tuned for the target RSS
/// ceiling: lets SQLite page the DB from disk without loading it all into RSS.
pub const DB_MMAP_SIZE_MIB: i64 = 256;

pub const CONNECTION_WATCHDOG_INTERVAL_SECS: u64 = 30;
pub const CONNECTION_WATCHDOG_THRESHOLD_SECS: u64 = 300;

pub const CORS_DEFAULT_ORIGINS: &str = "http://localhost:3000,http://localhost:8080";
pub const CORS_DEFAULT_METHODS: &str = "GET,POST,PUT,DELETE,OPTIONS";
pub const CORS_DEFAULT_HEADERS: &str = "content-type,authorization";
pub const CORS_MAX_AGE_SECS: u64 = 3600;

/// Retrieval profiles (P3). The default keeps the edge budget; the others are
/// opt-in and change the model footprint and rerank behaviour.
pub const PROFILE_EDGE_DEFAULT: &str = "edge-default";
pub const PROFILE_QUALITY_LOCAL: &str = "quality-local";
pub const PROFILE_MULTILINGUAL: &str = "multilingual";
pub const PROFILE_AIR_GAPPED: &str = "air-gapped";
/// v1.28 "Caliber": the enterprise profile selects the neural tier (BGE-M3 via
/// FastEmbed-rs, `--features neural-embed`) when compiled in, else falls back to
/// the static default. See `embed::embedder_for_profile`.
pub const PROFILE_ENTERPRISE: &str = "enterprise";
/// v1.28 "Caliber": the desktop profile (laptop/AMD-desktop) selects
/// `gte-base-en-v1.5` (768-d) — the light-English neural tier. Same feature
/// gate as enterprise.
pub const PROFILE_DESKTOP: &str = "desktop";

// ── Env-var helpers ──────────────────────────────────────────────────
// Each reads an env var with a fallback to the config constant above.
// Used by main() to wire the documented env vars into the real layers.

/// Comma-separated origins. Reads `CORS_ORIGINS`, falling back to
/// [`CORS_DEFAULT_ORIGINS`]. Safety guard: when the env var is unset, the
/// fallback is loopback-only — non-loopback origins are rejected unless the
/// deployer explicitly sets `CORS_ORIGINS`. This prevents an accidental open
/// CORS policy in production. Dev origins (localhost:3000/8080) are allowed
/// because they are loopback.
///
/// Audit G4: `*` is never a valid CORS origin and is stripped from
/// whatever source the list came from. The layer exact-matches origin strings
/// (see `build_app`), so a literal `*` entry would otherwise silently match
/// nothing — a deployer who writes `CORS_ORIGINS=*` deserves an error, not a
/// config that looks permissive but grants zero cross-origin access.
pub fn cors_origins() -> String {
    let raw = match std::env::var("CORS_ORIGINS") {
        Ok(v) => v,
        Err(_) => {
            // Unset: lock down to loopback-only. The default list contains only
            // loopback origins; any non-loopback entry is stripped.
            CORS_DEFAULT_ORIGINS
                .split(',')
                .filter(|o| is_loopback_origin(o.trim()))
                .collect::<Vec<_>>()
                .join(",")
        }
    };
    sanitize_origins(&raw)
}

/// Trim, drop empties, and reject the literal `*` (never a valid CORS origin).
/// Pure so the no-wildcard guard is unit-testable without mutating env.
fn sanitize_origins(raw: &str) -> String {
    raw.split(',')
        .map(|o| o.trim())
        .filter(|o| !o.is_empty() && *o != "*")
        .collect::<Vec<_>>()
        .join(",")
}

/// True for loopback origins (`http(s)://localhost`, `127.0.0.1`, `::1`).
fn is_loopback_origin(origin: &str) -> bool {
    let host = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or(origin)
        .split([':', '/'])
        .next()
        .unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Comma-separated methods. Reads `CORS_METHODS`, falls back to
/// [`CORS_DEFAULT_METHODS`].
pub fn cors_methods() -> String {
    std::env::var("CORS_METHODS").unwrap_or_else(|_| CORS_DEFAULT_METHODS.to_string())
}

/// Comma-separated headers. Reads `CORS_HEADERS`, falls back to
/// [`CORS_DEFAULT_HEADERS`].
pub fn cors_headers() -> String {
    std::env::var("CORS_HEADERS").unwrap_or_else(|_| CORS_DEFAULT_HEADERS.to_string())
}

// ── v1.3.0 "Bedrock" fix: SHUTDOWN_DRAIN_SECS was removed. The v1.1.0
// implementation wrapped the ENTIRE `axum::serve(...)` future in a timeout,
// capping total server lifetime at 30s — not just the drain phase. This
// caused a 30s crash-loop on systemd deployments. Fixed: the server now runs
// indefinitely until SIGTERM, then axum's built-in drain handles the rest.
// If a request hangs after SIGTERM, systemd's TimeoutStopSec (default 90s)
// is the outer cap.

// ── security constants ──────────────────────────────────

/// When `BIND_HOST` fails to parse as an IP, refuse to bind instead of falling
/// back to `0.0.0.0` (which would expose the server on all interfaces). An
/// operator who genuinely wants LAN exposure sets `BIND_PUBLIC=1` to opt in.
pub const BIND_PUBLIC_OPT_IN: &str = "BIND_PUBLIC";

/// Webhook replay protection: a delivery whose timestamp is further than this
/// many seconds from "now" is rejected (too old = replay risk, too new = clock
/// skew / forged). GitHub sends `X-GitHub-Delivery` + `X-Hub-Signature-256`.
pub const WEBHOOK_REPLAY_SECS: u64 = 300;

/// when `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED=1`, the
/// webhook receiver requires the Standard Webhooks header set
/// (`webhook-id`/`webhook-timestamp`/`webhook-signature`) and verifies the
/// signature over `{id}.{timestamp}.{raw body}` (spec's canonical form), so a
/// sender cannot re-stamp a stale timestamp. Off by default — GitHub sends no
/// such timestamp and keeps its legacy delivery-id idempotency path.
pub fn webhook_timestamp_required() -> bool {
    std::env::var("BRAIN_WEBHOOK_TIMESTAMP_REQUIRED")
        .ok()
        .map(|s| s == "1")
        .unwrap_or(false)
}

/// Hard cap on the verified-webhook ingest queue (rows in `webhook_queue`).
/// Backpressure: once full, new verified deliveries are rejected with 503 until
/// the drain worker catches up. Bounds the blast radius of a webhook storm.
pub const WEBHOOK_QUEUE_MAX: usize = 10_000;

/// Injection-screen policy. `quarantine` (default) ingests flagged chunks but
/// marks them `flagged=1` so they are excluded from recall until reviewed;
/// `reject` preserves the old behavior (HTTP 400 at ingest); `allow` disables
/// the screen entirely (trusted local sources only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InjectionPolicy {
    #[default]
    Quarantine,
    Reject,
    Allow,
}

/// Resolve the injection-screen policy from `INJECTION_POLICY`
/// (quarantine|reject|allow). Unknown/empty → `quarantine`.
pub fn injection_policy() -> InjectionPolicy {
    match std::env::var("INJECTION_POLICY")
        .ok()
        .map(|s| s.trim().to_lowercase())
        .as_deref()
    {
        Some("reject") => InjectionPolicy::Reject,
        Some("allow") => InjectionPolicy::Allow,
        _ => InjectionPolicy::Quarantine,
    }
}

// ── layer-2 classifier config ─────────────────────

/// Default reject threshold (score ≥ this → HTTP 400). Conservative: a false
/// positive here costs a legit memory at the boundary, so `reject` sits above
/// `quarantine`.
pub const INJECTION_THRESHOLD_HIGH: f32 = 0.9;
/// Default quarantine threshold (score ≥ this → stored flagged). Deliberately
/// below the reject band so ambiguous scores favor quarantine over a hard
/// reject — the cost of a false quarantine (a legit memory held for review) is
/// lower than a false negative (a plant stored unflagged).
pub const INJECTION_THRESHOLD_LOW: f32 = 0.7;

/// Path to the layer-2 ONNX classifier model. Empty → layer-2 off (the
/// deterministic blocklist remains). Only compiled under the
/// `injection-classifier` feature.
#[cfg(feature = "injection-classifier")]
pub fn injection_classifier_path() -> String {
    std::env::var("BRAIN_INJECTION_CLASSIFIER").unwrap_or_default()
}

/// Path to the matching BERT `tokenizer.json`. Required (with the model) to
/// enable layer-2.
#[cfg(feature = "injection-classifier")]
pub fn injection_tokenizer_path() -> String {
    std::env::var("BRAIN_INJECTION_TOKENIZER").unwrap_or_default()
}

/// Resolve `BRAIN_INJECTION_THRESHOLD_HIGH` (0..1); unparsable → default.
pub fn injection_threshold_high() -> f32 {
    std::env::var("BRAIN_INJECTION_THRESHOLD_HIGH")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| *v >= 0.0 && *v <= 1.0)
        .unwrap_or(INJECTION_THRESHOLD_HIGH)
}

/// Resolve `BRAIN_INJECTION_THRESHOLD_LOW` (0..1); unparsable → default.
pub fn injection_threshold_low() -> f32 {
    std::env::var("BRAIN_INJECTION_THRESHOLD_LOW")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| *v >= 0.0 && *v <= 1.0)
        .unwrap_or(INJECTION_THRESHOLD_LOW)
}

/// Database file path. Reads `BRAIN_DB_PATH`; falls back to
/// `~/.openclaw/workspace/brain.db`. v0.9.9: delegates to
/// `StorageLayout::detect()?.legacy_db()` so this path and the layout's path
/// are byte-identical (the back-compat invariant locked by
/// `storage_layout::tests::legacy_db_matches_brain_db_path_env_when_set`).
/// The historical default is preserved exactly.
pub fn brain_db_path() -> std::path::PathBuf {
    brain_server::storage_layout::StorageLayout::detect()
        .map(|l| l.legacy_db())
        .unwrap_or_else(|_| {
            // Defensive: detect() only fails on a non-absolute BRAIN_DATA_ROOT,
            // in which case we fall back to the historical default rather than
            // panic at startup. The bad env var is logged by the caller.
            let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            home.join(".openclaw/workspace/brain.db")
        })
}

/// directory of the built brain-client web assets
/// (index.html + WASM + CSS). Served at `/app` by `tower_http::services::ServeDir`.
/// Default is `client/dist` relative to CWD; override with `BRAIN_CLIENT_DIR`.
/// If the dir doesn't exist, `/app` routes 404 and the API is unaffected.
pub fn client_dir() -> std::path::PathBuf {
    std::env::var("BRAIN_CLIENT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("client/dist"))
}

/// `GET /ump/subscribe` SSE broadcast buffer. Bounded —
/// a slow consumer drops missed events (broadcast lag semantics), never
/// blocks the publish path.
pub const UMP_EVENT_BUFFER: usize = 128;

/// directory holding the operator Ed25519 signing key
/// (raw 32-byte seed file, e.g. `operator.key`). `None` → L2 conformance
/// (hash-only integrity); a key present → L3 (records signed + verified).
pub fn ump_key_dir() -> std::path::PathBuf {
    std::env::var("BRAIN_UMP_KEY_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".config/brain-server/ump")
        })
}

// ── token rotation constants ───────────────────
// The token-store background task stats the file at this cadence and reloads
// on mtime change. The auth check itself is hot-path, so it reads from the
// cached set under a single RwLock — not from disk.
pub const TOKEN_ROTATION_POLL_SECS: u64 = 5;

/// TTL for the `/metrics` audit-chain cache. `/metrics` is scraped
/// frequently (Prometheus default 15s); running a full O(n) chain scan on
/// every scrape wastes CPU. `/audit/verify` stays authoritative (always scans);
/// `/metrics` returns the cached result and refreshes when older than this.
/// 60s = max staleness window for the `brain_audit_chain_ok` gauge.
pub const AUDIT_CHAIN_CACHE_TTL_SECS: u64 = 60;

/// Optional bearer token for authenticated routes. When no token is resolvable,
/// the server runs unauthenticated (loopback-only is still the safe default).
/// When set, mutating/authenticated routes require `Authorization: Bearer <t>`.
///
/// Resolution order (first non-empty wins):
/// 1. `AUTH_TOKEN_FILE` — path to a `0600`-mode file containing the token(s). This
///    is the secret-file convention (à la Docker/K8s `*_FILE`); it keeps the
///    token out of the process/launchd environment and off `launchctl print`.
/// 2. `AUTH_TOKEN` — the raw env var. Kept for back-compat/dev convenience.
///
/// `auth_tokens()` here reads from disk on every call (so it
/// reflects rotation immediately when called directly). The server's hot path
/// uses `auth::TokenStore` which caches this result + reloads on mtime change,
/// audited + fail-safe against file deletion (see `auth.rs`).
pub fn auth_token() -> Option<String> {
    if let Ok(path) = std::env::var("AUTH_TOKEN_FILE") {
        let path = path.trim();
        if let Ok(s) = std::fs::read_to_string(path) {
            // Return the multi-token set as a single string; `auth_middleware`
            // splits on whitespace when comparing. Non-empty file => auth on.
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    std::env::var("AUTH_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// All accepted tokens from the resolved token source (newline/whitespace
/// separated). Empty when auth is disabled.
pub fn auth_tokens() -> Vec<String> {
    auth_token()
        .map(|s| {
            s.split_whitespace()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The explicitly-configured token file path (`AUTH_TOKEN_FILE`, if set and
/// non-empty). `None` when auth is env-driven or file-less.
pub fn auth_token_file() -> Option<std::path::PathBuf> {
    std::env::var("AUTH_TOKEN_FILE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
}

/// when `AUTH_TOKEN_FILE` is explicitly
/// set but cannot yield tokens (missing, unreadable, or empty) AND no
/// `AUTH_TOKEN` env fallback is configured, the server would otherwise start
/// silently unauthenticated — the wrong failure direction. Returns the fatal
/// message; the server refuses to start. ponytail: with an `AUTH_TOKEN`
/// fallback present the ladder still works (auth stays ON); the no-auth
/// loopback default is unchanged when no file is configured at all.
pub fn auth_token_misconfigured() -> Option<String> {
    let path = auth_token_file()?;
    let file_ok = std::fs::read_to_string(&path)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if file_ok {
        return None;
    }
    let env_fallback = std::env::var("AUTH_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_some();
    if env_fallback {
        return None;
    }
    Some(format!(
        "AUTH_TOKEN_FILE={} is set but missing, unreadable, or empty, and AUTH_TOKEN \
         is unset — refusing to start with authentication disabled. Fix the token file \
         (e.g. `install-service.sh`), set AUTH_TOKEN, or unset AUTH_TOKEN_FILE.",
        path.display()
    ))
}

/// Whether per-domain database isolation is active (P2). When false (default),
/// every domain resolves to the shared global DB (legacy single-DB back-compat).
/// When true, non-`global` domains get their own `brain-<domain>.db` file.
pub fn multi_db() -> bool {
    std::env::var("BRAIN_MULTI_DB")
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// the per-domain DB registration cap.
/// `BRAIN_MAX_DOMAIN_DBS` overrides [`MAX_DOMAIN_DBS`]; unparsable → default.
pub fn max_domain_dbs() -> usize {
    std::env::var("BRAIN_MAX_DOMAIN_DBS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(MAX_DOMAIN_DBS)
}

/// whether the opt-in `/suggest` routes are live. Defaults
/// to `true` (the feature ships on); set `BRAIN_SUGGEST_ENABLED=false` to
/// disable the surface entirely — the routes then return `501 Not Implemented`.
/// This is the roadmap's "otherwise the feature is removed" kill switch: an
/// operator who measures an unacceptable false-positive rate can disable
/// `/suggest` without a rebuild.
pub fn brain_suggest_enabled() -> bool {
    !matches!(
        std::env::var("BRAIN_SUGGEST_ENABLED")
            .map(|v| v.trim().to_lowercase())
            .unwrap_or_default()
            .as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// whether the complexity-gated graph rescue is live.
/// Defaults to `true` (the feature ships on); set `BRAIN_GRAPH_RESCUE_ENABLED
/// =false` to restore exact v1.11.0 abstention behavior (a `ClarifyQuery`
/// query always returns the empty `low_confidence` envelope). Same pattern as
/// [`brain_suggest_enabled`]: an operator who measures an unacceptable
/// rescue-latency cost can disable it without a rebuild.
pub fn brain_graph_rescue_enabled() -> bool {
    !matches!(
        std::env::var("BRAIN_GRAPH_RESCUE_ENABLED")
            .map(|v| v.trim().to_lowercase())
            .unwrap_or_default()
            .as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// whether the graph-PPR retriever is a DEFAULT recall leg (the "olderData"
/// cross-topic hop that links e.g. VMware↔VxRail↔vSAN↔storage↔fabric in one
/// multi-hop walk). Defaults to `true` — the third PPR leg ships on so callers
/// get the connected, best-possible recall without opting in. Set
/// `BRAIN_RECALL_GRAPH_ENABLED=false` to restore the exact two-retriever
/// (vector + FTS) default. Same kill-switch pattern as
/// [`brain_graph_rescue_enabled`]/[`brain_recall_routing_enabled`].
pub fn brain_recall_graph_enabled() -> bool {
    !matches!(
        std::env::var("BRAIN_RECALL_GRAPH_ENABLED")
            .map(|v| v.trim().to_lowercase())
            .unwrap_or_default()
            .as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// whether automatic retrieval routing is live. Defaults to
/// `true` (the fix ships on); set `BRAIN_RECALL_ROUTING_ENABLED=false` to
/// restore the exact pre-v1.15.0 shim behavior (recall searches the `global`
/// pool only, no centroid routing). Same kill-switch pattern as
/// [`brain_suggest_enabled`]/[`brain_graph_rescue_enabled`]: an operator who
/// measures a routing regression can disable it without a rebuild.
pub fn brain_recall_routing_enabled() -> bool {
    !matches!(
        std::env::var("BRAIN_RECALL_ROUTING_ENABLED")
            .map(|v| v.trim().to_lowercase())
            .unwrap_or_default()
            .as_str(),
        "0" | "false" | "no" | "off"
    )
}

// ── OpenTelemetry export ─────────────────────────

/// Whether OTLP trace export is live. Defaults to `false` (the exporter is
/// feature-gated behind `--features otel`; this flag only decides whether the
/// compiled-in exporter is *installed*). Same kill-switch pattern as
/// [`brain_suggest_enabled`]: an operator can disable export at runtime without
/// a rebuild via `BRAIN_OTEL_ENABLED=false`. Read at startup only.
pub fn otel_enabled() -> bool {
    !matches!(
        std::env::var("BRAIN_OTEL_ENABLED")
            .map(|v| v.trim().to_lowercase())
            .unwrap_or_default()
            .as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// The OTLP/HTTP endpoint spans are exported to. Defaults to the standard
/// collector HTTP path `http://127.0.0.1:4318/v1/traces`. Override with
/// `BRAIN_OTEL_ENDPOINT` (e.g. a self-hosted collector or a gateway like
/// `http://collector:4318/v1/traces`). Read at startup only.
pub fn otel_endpoint() -> String {
    std::env::var("BRAIN_OTEL_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:4318/v1/traces".to_string())
}

// ── per-kind retention ─────────────────────────────

/// Default retention (days) per `memory_kind` for chunks with no explicit
/// `expires_at`. v1.14 made per-chunk `expires_at` the decay primitive; M2 adds
/// a kind-level default so a retention policy can govern whole classes of
/// memory without per-row authoring. `ponytail:` these are defaults — a chunk
/// with its own `expires_at` always wins, and the policy is query-time only
/// (no sweeper; nothing is deleted autonomously).
pub const DEFAULT_RETENTION_KIND_DAYS: &[(&str, i64)] = &[
    ("fact", 365),
    ("episodic", 30),
    ("procedure", 730),
    ("step", 730),
    ("decision", 730),
];

/// Whether per-kind retention is live. Defaults to `true`; set
/// `BRAIN_RETENTION_ENABLED=false` to restore exact pre-v1.17.1 behavior (only
/// per-chunk `expires_at` governs decay). Same kill-switch pattern as
/// [`brain_suggest_enabled`].
pub fn brain_retention_enabled() -> bool {
    !matches!(
        std::env::var("BRAIN_RETENTION_ENABLED")
            .map(|v| v.trim().to_lowercase())
            .unwrap_or_default()
            .as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// Resolve the effective per-kind retention (days) from the env override
/// `BRAIN_RETENTION_KIND_DAYS` (a JSON map like `{"fact":365,"episodic":30}`)
/// merged over the defaults. Keys unknown to the defaults are accepted so an
/// operator can govern a future kind. Invalid JSON or a non-integer value
/// degrades to the default for that key (never panics at the trust boundary).
pub fn retention_kind_days() -> std::collections::BTreeMap<String, i64> {
    let mut map: std::collections::BTreeMap<String, i64> = DEFAULT_RETENTION_KIND_DAYS
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    if let Ok(raw) = std::env::var("BRAIN_RETENTION_KIND_DAYS") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(obj) = v.as_object() {
                for (k, val) in obj {
                    if let Some(days) = val.as_i64() {
                        map.insert(k.to_string(), days);
                    }
                }
            }
        }
    }
    map
}

/// minimum chunk count for a domain to keep a routing centroid.
/// Defaults to 1 (a 1-vector centroid is exact for that vector, so nothing is
/// suppressed). A domain below this floor gets its centroid deleted so `route()`
/// stops sending traffic to a near-empty bucket. `ponytail:` corpus-tuned, not
/// learned — an operator who measures weak routing on sub-N domains raises it.
pub fn brain_domain_min_count() -> i64 {
    std::env::var("DOMAIN_MIN_COUNT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1)
        .max(1)
}

/// only trust `X-Forwarded-For` when the operator has
/// explicitly told us we're behind a reversing proxy that sets it (and that
/// the proxy overwrites any client-supplied value). Default `false` — direct
/// connections use the socket address, defeating the per-request IP-spoofing
/// attack that evades the limiter and grows its HashMap unboundedly.
pub fn brain_trust_proxy() -> bool {
    matches!(
        std::env::var("BRAIN_TRUST_PROXY")
            .map(|v| v.trim().to_lowercase())
            .unwrap_or_default()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

// ── read-event audit + DSAR ─────────────────────────

/// Whether read events (`/recall`, `/search`, `/get`, `/multi-get`) are
/// appended to the audit hash chain. `BRAIN_AUDIT_READ_EVENTS` explicit value
/// wins; when unset the default follows the posture: **on in JWT mode** (the
/// enterprise posture — the plan's "default on for JWT"), **off for loopback/
/// opaque personal use** (noise + the personal-use contract). Read events are
/// hash-only (chunk id + scores + decision; never content) and never change
/// the primary action — best-effort by construction.
///
/// the unset case is now domain-aware — a bound
/// profile's `audit_level` decides before the JWT/loopback default does (see
/// `profile::audit_read_events_for`). This fn stays for the unbound surfaces
/// (`/search`, `/get`, `/multi-get`) and as the layer under the profile.
pub fn audit_read_events(principal_is_jwt: bool) -> bool {
    audit_read_events_explicit().unwrap_or(principal_is_jwt)
}

/// the explicit `BRAIN_AUDIT_READ_EVENTS` override as a
/// tri-state (`None` = unset, deployer left the decision to the posture /
/// the bound profile). Extracted so the recall path can layer
/// env → profile → default without re-reading the env string twice.
pub fn audit_read_events_explicit() -> Option<bool> {
    match std::env::var("BRAIN_AUDIT_READ_EVENTS")
        .map(|v| v.trim().to_lowercase())
        .ok()
        .as_deref()
    {
        Some("on" | "true" | "1" | "yes") => Some(true),
        Some("off" | "false" | "0" | "no") => Some(false),
        _ => None,
    }
}

/// Sampling rate for read events (0.0..=1.0, default 1.0 = all). A sampled-out
/// read event is simply not recorded (no trace). Bounded below 1.0 so an
/// operator can cut the noise on a busy multi-tenant server.
pub fn audit_read_sample_rate() -> f64 {
    std::env::var("BRAIN_AUDIT_READ_SAMPLE_RATE")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1.0_f64)
        .clamp(0.0, 1.0)
}

/// Read-event / audit retention window in days. Unset = keep forever (the
/// personal-use contract). When set (deployers: ≥180 per AI Act Art 26(6)),
/// rows older than the window are pruned on read-event writes and the chain
/// re-anchored to the oldest survivor. Never runs when unset.
pub fn audit_read_retention_days() -> Option<u32> {
    std::env::var("BRAIN_AUDIT_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|d| *d > 0)
}

/// DSAR ledger retention window in days. Completed `dsar_requests` rows keep
/// only the bundle hash + certificate after the purge; rows
/// older than this window are deleted on the read-event prune cadence. Default
/// 30; the GDPR needs the erasure record, not the bundle bytes forever.
pub fn dsar_ledger_retention_days() -> u32 {
    std::env::var("BRAIN_DSAR_LEDGER_DAYS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|d| *d > 0)
        .unwrap_or(30)
}

/// Optional Art 19 onward-notification webhook. When set, a completed DSAR
/// purge POSTs `{subject, certified_at, certificate_id}` to this URL, HMAC-
/// signed with [`dsar_webhook_secret`]. Fail-soft: a webhook failure never
/// rolls back the purge.
pub fn dsar_webhook_url() -> Option<String> {
    std::env::var("BRAIN_DSAR_WEBHOOK_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// HMAC secret for the DSAR webhook signature (`X-Brain-Signature-256`).
/// When unset, the webhook is sent unsigned (documented — the caller should
/// still receive the notification; signing is best practice).
pub fn dsar_webhook_secret() -> Option<String> {
    std::env::var("BRAIN_DSAR_WEBHOOK_SECRET")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The controller name for the Art 30 register (`GET /art30`). Defaults to
/// "brain-server operator". `BRAIN_CONTROLLER_NAME` is an operator-facing,
/// non-secret label — it must not hold PII or anything sensitive.
pub fn controller_name() -> String {
    std::env::var("BRAIN_CONTROLLER_NAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "brain-server operator".to_string())
}

/// the named **Data Protection Officer** contact.
/// A non-secret, operator-facing label (name/role/duty-officer email) surfaced
/// on `/health` + the public privacy notice so a data-subject/breach event has
/// a named channel. `None` when unset — the posture degrades to "no named
/// contact" rather than inventing one (the operator fills `BRAIN_DPO_CONTACT`).
pub fn dpo_contact() -> Option<String> {
    std::env::var("BRAIN_DPO_CONTACT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Active retrieval profile (P3). Reads `MODEL_PROFILE`; falls back to
/// `edge-default`. Unknown values fall back to `edge-default`.
pub fn model_profile() -> &'static str {
    match std::env::var("MODEL_PROFILE")
        .ok()
        .as_deref()
        .map(str::trim)
        .map(str::to_lowercase)
        .as_deref()
    {
        Some(PROFILE_QUALITY_LOCAL) => PROFILE_QUALITY_LOCAL,
        Some(PROFILE_MULTILINGUAL) => PROFILE_MULTILINGUAL,
        Some(PROFILE_AIR_GAPPED) => PROFILE_AIR_GAPPED,
        Some(PROFILE_ENTERPRISE) => PROFILE_ENTERPRISE,
        Some(PROFILE_DESKTOP) => PROFILE_DESKTOP,
        _ => PROFILE_EDGE_DEFAULT,
    }
}

/// Resolve the embedding model id for the active profile. Only `edge-default`
/// and `multilingual` change the model today; the others keep the static model
/// and instead enable heavier rerank/expansion (documented trade-off).
pub fn model_id_for_profile(profile: &str) -> &'static str {
    match profile {
        PROFILE_MULTILINGUAL => "minishlab/potion-base-2M", // multilingual static alternative
        PROFILE_ENTERPRISE => "BAAI/bge-m3", // v1.28 neural tier (1024-d, dense+sparse+colbert)
        PROFILE_DESKTOP => "Alibaba-NLP/gte-base-en-v1.5", // v1.28 desktop tier (768-d, dense)
        _ => MODEL_ID, // edge-default / quality-local / air-gapped keep the default static model
    }
}

// ── Retrieval quality estimator config ─────────────────────────────────

// ── Retrieval quality estimator config ─────────────────────────────────

/// Quality estimator weights and thresholds (read from env with defaults).
#[derive(Debug, Clone)]
pub struct QualityConfig {
    pub overlap_weight: f32,
    pub gap_weight: f32,
    pub rr_weight: f32,
    pub lex_weight: f32,
    pub agreement_min: usize,
    pub gap_threshold: f32,
    pub confidence_threshold: f32,
    pub rerank_threshold: f32,
}

impl Default for QualityConfig {
    fn default() -> Self {
        Self {
            overlap_weight: 0.4,
            gap_weight: 0.3,
            rr_weight: 0.2,
            lex_weight: 0.1,
            agreement_min: 2,
            gap_threshold: 0.023,
            confidence_threshold: 0.6,
            rerank_threshold: 0.85,
        }
    }
}

impl QualityConfig {
    pub fn from_env() -> Self {
        fn env_f32(key: &str, default: f32) -> f32 {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        fn env_usize(key: &str, default: usize) -> usize {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        Self {
            overlap_weight: env_f32("QUALITY_OVERLAP_WEIGHT", 0.4),
            gap_weight: env_f32("QUALITY_GAP_WEIGHT", 0.3),
            rr_weight: env_f32("QUALITY_RR_WEIGHT", 0.2),
            lex_weight: env_f32("QUALITY_LEX_WEIGHT", 0.1),
            agreement_min: env_usize("QUALITY_AGREEMENT_MIN", 2),
            gap_threshold: env_f32("QUALITY_GAP_THRESHOLD", 0.023),
            confidence_threshold: env_f32("QUALITY_CONFIDENCE_THRESHOLD", 0.6),
            rerank_threshold: env_f32("QUALITY_RERANK_THRESHOLD", 0.85),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_origins_drops_wildcard_and_empties() {
        // Audit G4: `*` must never survive into the allow-list, and
        // stray commas/whitespace produce no empty entries.
        assert_eq!(sanitize_origins("*"), "");
        assert_eq!(sanitize_origins("*,https://a.test"), "https://a.test");
        assert_eq!(
            sanitize_origins(" https://a.test , * , https://b.test "),
            "https://a.test,https://b.test"
        );
        assert_eq!(sanitize_origins("https://a.test"), "https://a.test");
        assert_eq!(sanitize_origins(""), "");
        assert_eq!(sanitize_origins(","), "");
    }

    #[test]
    fn loopback_origins_are_recognized() {
        assert!(is_loopback_origin("http://localhost:3000"));
        assert!(is_loopback_origin("http://127.0.0.1:8765"));
        assert!(is_loopback_origin("https://localhost"));
        assert!(!is_loopback_origin("https://example.com"));
    }

    /// an explicitly configured but broken token file
    /// is fatal ONLY when there is no AUTH_TOKEN env fallback — the ladder
    /// (file → env) must keep auth ON, and the no-file default stays off.
    /// Save/restore pattern as in capacity.rs (parallel-runner safe).
    #[test]
    fn auth_token_misconfigured_fail_closed_ladder() {
        let prev_file = std::env::var("AUTH_TOKEN_FILE").ok();
        let prev_env = std::env::var("AUTH_TOKEN").ok();

        let valid = std::env::temp_dir().join("brain-test-valid-token");
        std::fs::write(&valid, "tok\n").unwrap();
        let missing = std::env::temp_dir().join("brain-test-missing-token");
        let _ = std::fs::remove_file(&missing);

        // 1. Valid file → no error, regardless of env.
        std::env::set_var("AUTH_TOKEN_FILE", &valid);
        assert_eq!(auth_token_misconfigured(), None);
        // 2. Broken file + no env fallback → fatal.
        std::env::set_var("AUTH_TOKEN_FILE", &missing);
        std::env::remove_var("AUTH_TOKEN");
        let msg = auth_token_misconfigured();
        assert!(msg.is_some(), "broken explicit file must refuse to start");
        assert!(msg.as_ref().unwrap().contains("AUTH_TOKEN_FILE"));
        // 3. Broken file + env fallback → ladder keeps auth ON.
        std::env::set_var("AUTH_TOKEN", "fallback");
        assert_eq!(auth_token_misconfigured(), None);
        // 4. No file configured → unchanged no-auth default.
        std::env::remove_var("AUTH_TOKEN_FILE");
        assert_eq!(auth_token_misconfigured(), None);

        let _ = std::fs::remove_file(&valid);
        match prev_file {
            Some(v) => std::env::set_var("AUTH_TOKEN_FILE", v),
            None => std::env::remove_var("AUTH_TOKEN_FILE"),
        }
        match prev_env {
            Some(v) => std::env::set_var("AUTH_TOKEN", v),
            None => std::env::remove_var("AUTH_TOKEN"),
        }
    }
}
