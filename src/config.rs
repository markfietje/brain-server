//! Configuration constants for brain-server

pub const MODEL_ID: &str = "minishlab/potion-retrieval-32M";
pub const DEFAULT_K: usize = 5;
pub const MAX_K: usize = 100;
/// Drive version from Cargo.toml so /version and logs always match the build.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const MAX_REQUEST_SIZE: usize = 1024 * 1024;
pub const MAX_QUERY_LENGTH: usize = 2000;

/// M2 evidence quality: bounded snippet window (chars) and the redaction cap
/// on `explain` payloads so they can never leak unbounded source text.
pub const MAX_SNIPPET_CHARS: usize = 240;
pub const SNIPPET_CONTEXT_CHARS: usize = 60;
pub const MAX_EXPLAIN_BYTES: usize = 64 * 1024;
pub const MAX_MULTI_GET: usize = 1000;

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

// ── Env-var helpers ──────────────────────────────────────────────────
// Each reads an env var with a fallback to the config constant above.
// Used by main() to wire the documented env vars into the real layers.

/// Comma-separated origins. Reads `CORS_ORIGINS`, falling back to
/// [`CORS_DEFAULT_ORIGINS`]. Safety guard: when the env var is unset, the
/// fallback is loopback-only — non-loopback origins are rejected unless the
/// deployer explicitly sets `CORS_ORIGINS`. This prevents an accidental open
/// CORS policy in production. Dev origins (localhost:3000/8080) are allowed
/// because they are loopback.
pub fn cors_origins() -> String {
    match std::env::var("CORS_ORIGINS") {
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
    }
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

// ── v0.9.7 "Guard" security constants ──────────────────────────────────

/// When `BIND_HOST` fails to parse as an IP, refuse to bind instead of falling
/// back to `0.0.0.0` (which would expose the server on all interfaces). An
/// operator who genuinely wants LAN exposure sets `BIND_PUBLIC=1` to opt in.
pub const BIND_PUBLIC_OPT_IN: &str = "BIND_PUBLIC";

/// Webhook replay protection: a delivery whose timestamp is further than this
/// many seconds from "now" is rejected (too old = replay risk, too new = clock
/// skew / forged). GitHub sends `X-GitHub-Delivery` + `X-Hub-Signature-256`.
pub const WEBHOOK_REPLAY_SECS: u64 = 300;

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

// ── v1.1.0 "Harden" M1.4: token rotation constants ───────────────────
// The token-store background task stats the file at this cadence and reloads
// on mtime change. The auth check itself is hot-path, so it reads from the
// cached set under a single RwLock — not from disk.
pub const TOKEN_ROTATION_POLL_SECS: u64 = 5;

/// v1.1.1: TTL for the `/metrics` audit-chain cache. `/metrics` is scraped
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
/// v1.1.0 Harden: `auth_tokens()` here reads from disk on every call (so it
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
        _ => PROFILE_EDGE_DEFAULT,
    }
}

/// Resolve the embedding model id for the active profile. Only `edge-default`
/// and `multilingual` change the model today; the others keep the static model
/// and instead enable heavier rerank/expansion (documented trade-off).
pub fn model_id_for_profile(profile: &str) -> &'static str {
    match profile {
        PROFILE_MULTILINGUAL => "minishlab/potion-base-2M", // multilingual static alternative
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
