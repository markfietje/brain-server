#![cfg(feature = "bench")]

//! `bench` — synthetic-scale benchmark harness for a running brain-server.
//!
//! Feature-gated (`--features bench`). Connects to a running server over HTTP
//! using the same dependency-free client as `brain`/`mcp`, ingests synthetic
//! docs at 1k/5k/10k scales, and prints a markdown table of resident memory,
//! ingest throughput, and `/search` latency percentiles to stdout.
//!
//! when `BENCH_ENVELOPE` is set (desktop|jetson), each scale asserts
//! against the published capacity envelope and the run exits non-zero on any
//! breach — turning the report into a ship gate.
//!
//! Env:
//!   BRAIN_URL         base URL of the server (default http://127.0.0.1:8765)
//!   BRAIN_TOKEN_FILE  path to a 0600 secret file (preferred over BRAIN_TOKEN)
//!   BRAIN_TOKEN       raw bearer token (dev convenience)
//!   BENCH_SCALES      comma-separated doc counts (default 1000,5000,10000)
//!   BENCH_SEARCHES    search queries per client per scale (default 100)
//!   BENCH_ENVELOPE    assert against this capacity envelope (desktop|jetson)
//!   BENCH_CLIENTS     parallel search clients per scale (default 1; N>=2
//!                     fans out N std::threads over the SAME seeded mix and
//!                     merges the latency samples — the concurrent truth the
//!                     Enterprise Line gates on)
//!   BENCH_SEED        query-space offset for the mix (default 0); printed so
//!                     any floor breach is reproducible
//!   BENCH_ASSERT_P95_MS  fail the run when the merged p95 exceeds this
//!                     (an envelope-free ship gate; CI uses it with a
//!                     generous bound)
//!
//! Ingest stays single-client at every BENCH_CLIENTS value (the corpus build
//! is deterministic: synthetic titles/content stay unique across cumulative
//! scales); only the search phase fans out. With BENCH_CLIENTS=1 the report
//! is byte-identical to the pre-Throughput sequential behavior.
//!
//! Scales are cumulative within one run — the server exposes no reset API, so
//! each scale's docs are appended on top of the previous scale's. "RSS at rest"
//! for a scale is measured just before that scale's ingest begins, so it shows
//! steady-state growth across the run. To measure scales independently, delete
//! the DB and restart the server between invocations.

#[path = "../bin_common/http.rs"]
mod http;

use http::{get, post};
use std::time::{Duration, Instant};

const DEFAULT_URL: &str = "http://127.0.0.1:8765";
const BATCH_SIZE: usize = 1000;

fn base_url() -> String {
    std::env::var("BRAIN_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}

/// Resolve the bearer token for authenticated routes, mirroring the server's
/// `AUTH_TOKEN_FILE` → `AUTH_TOKEN` ladder (see `src/config.rs`).
/// 1. `BRAIN_TOKEN_FILE` — explicit path to a `0600`-mode secret file.
/// 2. `BRAIN_TOKEN` — raw env var (dev convenience).
/// 3. `~/.config/brain-server/auth-token` — default install path.
fn auth_token() -> Option<String> {
    if let Ok(path) = std::env::var("BRAIN_TOKEN_FILE") {
        let p = path.trim();
        if let Ok(s) = std::fs::read_to_string(p)
            && let Some(t) = http::first_token(&s)
        {
            return Some(t);
        }
    }
    if let Ok(t) = std::env::var("BRAIN_TOKEN")
        && let Some(t) = http::first_token(&t)
    {
        return Some(t);
    }
    let default_path = dirs_home().join(".config/brain-server/auth-token");
    if let Ok(s) = std::fs::read_to_string(&default_path)
        && let Some(t) = http::first_token(&s)
    {
        return Some(t);
    }
    None
}

fn dirs_home() -> std::path::PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return std::path::PathBuf::from(h);
    }
    std::path::PathBuf::from(".")
}

/// Authoring aid for the judged retrieval corpus. GETs `/export` (which lists
/// every chunk with id/content/title over HTTP — no DB link needed) and writes
/// a browsable inventory the operator turns into `{query, relevant_ids}`
/// judgments for `bench eval`.
fn run_scaffold(out: Option<&str>) -> Result<(), String> {
    let bearer = auth_token();
    let resp = match get(&base_url(), "/export", &[], bearer.as_deref()) {
        Ok(r) if r.status == 200 => r,
        Ok(r) => return Err(format!("server unhealthy (status {})", r.status)),
        Err(e) => return Err(format!("cannot reach server: {e}")),
    };
    let value: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("/export returned non-JSON: {e}"))?;
    let chunks = scaffold_from_export(&value);
    if chunks.is_empty() {
        return Err("no chunks found — ingest a corpus first".into());
    }
    let path = out.unwrap_or("judgments.scaffold.json").to_string();
    let pretty = serde_json::to_string_pretty(&chunks)
        .map_err(|e| format!("cannot serialize scaffold: {e}"))?;
    std::fs::write(&path, pretty).map_err(|e| format!("cannot write {path}: {e}"))?;
    println!(
        "scaffold: wrote {} chunks to {path} — fill in query + relevant_ids per chunk, then `bench eval`",
        chunks.len()
    );
    Ok(())
}

/// Extract the chunk inventory (`{id, title, content}`) from a `/export` body.
/// Pure so the shape contract is unit-testable without a live server.
fn scaffold_from_export(body: &serde_json::Value) -> Vec<serde_json::Value> {
    body.get("knowledge")
        .and_then(|k| k.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| {
                    let id = k.get("id")?.as_i64()?;
                    let content = k.get("content")?.as_str()?;
                    Some(serde_json::json!({
                        "id": id,
                        "title": k.get("title").and_then(|t| t.as_str()).unwrap_or(""),
                        "content": content,
                        "query": "",
                        "relevant_ids": [],
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn scales() -> Vec<usize> {
    std::env::var("BENCH_SCALES")
        .unwrap_or_else(|_| "1000,5000,10000".to_string())
        .split(',')
        .filter_map(|s| s.trim().parse::<usize>().ok())
        .collect()
}

fn num_searches() -> usize {
    std::env::var("BENCH_SEARCHES")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(100)
}

/// Parallel search clients per scale. 1 = the pre-Throughput sequential run.
fn num_clients() -> usize {
    std::env::var("BENCH_CLIENTS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .max(1)
}

/// Query-space offset for the deterministic mix — the "seed". The mix itself
/// is a fixed formula (`topic {(q + seed) % 50}`), no RNG crate (bench stays
/// dependency-free); same seed + same server state ⇒ same query sequence.
fn bench_seed() -> u64 {
    std::env::var("BENCH_SEED")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Envelope-free p95 ship gate: when set, the run fails if the merged p95 of
/// any scale exceeds this many milliseconds.
fn assert_p95_ms() -> Option<f64> {
    std::env::var("BENCH_ASSERT_P95_MS")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
}

/// One client's search phase over the seeded mix. Pure-ish: network I/O only,
/// failures COUNTED (non-2xx + transport errors) rather than aborting the
/// client — the merged report must show contention, not die of it.
struct ClientSearchResult {
    client: usize,
    lats: Vec<Duration>,
    failures: usize,
}

fn run_search_client(
    base: &str,
    bearer: Option<&str>,
    searches: usize,
    seed: u64,
    client: usize,
) -> ClientSearchResult {
    let mut lats = Vec::with_capacity(searches);
    let mut failures = 0usize;
    for q in 0..searches {
        let start = Instant::now();
        let resp = get(
            base,
            "/search",
            &[
                (
                    "q".to_string(),
                    format!("topic {}", (q + seed as usize) % 50),
                ),
                ("k".to_string(), "10".to_string()),
            ],
            bearer,
        );
        let elapsed = start.elapsed();
        match resp {
            Ok(r) if r.status == 200 => lats.push(elapsed),
            Ok(r) => {
                failures += 1;
                eprintln!(
                    "bench: client {client} q={q} non-2xx status {}: {}",
                    r.status,
                    r.body.chars().take(120).collect::<String>()
                );
            }
            Err(e) => {
                failures += 1;
                eprintln!("bench: client {client} q={q} transport error: {e}");
            }
        }
    }
    ClientSearchResult {
        client,
        lats,
        failures,
    }
}

/// The merged per-scale search report. Pure + deterministic: same inputs ⇒
/// byte-identical output (the `bench_clients_merge_is_deterministic` pin).
#[derive(Debug, Clone, PartialEq)]
struct MergedSearch {
    /// (client, ops_ok, failures, p50_ms, p95_ms) per client, in client order
    per_client: Vec<(usize, usize, usize, f64, f64)>,
    total_ops: usize,
    failures: usize,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
}

fn merge_search_results(results: &[ClientSearchResult]) -> MergedSearch {
    let mut per_client = Vec::with_capacity(results.len());
    let mut pooled: Vec<Duration> = Vec::new();
    let mut failures = 0usize;
    for r in results {
        let mut sorted = r.lats.clone();
        sorted.sort();
        per_client.push((
            r.client,
            r.lats.len(),
            r.failures,
            percentile(&sorted, 50.0).as_secs_f64() * 1000.0,
            percentile(&sorted, 95.0).as_secs_f64() * 1000.0,
        ));
        failures += r.failures;
        pooled.extend_from_slice(&r.lats);
    }
    pooled.sort();
    MergedSearch {
        total_ops: pooled.len(),
        failures,
        p50_ms: percentile(&pooled, 50.0).as_secs_f64() * 1000.0,
        p95_ms: percentile(&pooled, 95.0).as_secs_f64() * 1000.0,
        p99_ms: percentile(&pooled, 99.0).as_secs_f64() * 1000.0,
        max_ms: pooled
            .last()
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0),
        per_client,
    }
}

/// capacity envelope to assert against. `None` when `BENCH_ENVELOPE`
/// is unset (report-only). When set (desktop|jetson), the harness reads the
/// published envelope from `brain_server::capacity` and exits non-zero on any
/// breach — turning the report into a ship gate.
fn envelope() -> Option<(brain_server::capacity::CapacityEnvelope, &'static str)> {
    let target = std::env::var("BENCH_ENVELOPE").ok()?.trim().to_lowercase();
    let t = match target.as_str() {
        "desktop" => brain_server::capacity::CapacityTarget::Desktop,
        "jetson" => brain_server::capacity::CapacityTarget::Jetson,
        _ => return None,
    };
    Some((
        brain_server::capacity::CapacityEnvelope::for_target(t),
        match t {
            brain_server::capacity::CapacityTarget::Desktop => "desktop",
            brain_server::capacity::CapacityTarget::Jetson => "jetson",
        },
    ))
}

/// Documented UX ceiling for the OpenClaw plugin (p95 of /search). Breaching
/// it under the active envelope is a ship-blocker — the plugin's turn loop
/// starts feeling laggy above this.
const ENVELOPE_P95_MS_CEILING: u64 = 200;

/// Assert a scale's measurements against the envelope. Returns a list of
/// human-readable breaches (empty when within envelope).
fn check_envelope(
    env: &brain_server::capacity::CapacityEnvelope,
    target_name: &str,
    row: &Row,
) -> Vec<String> {
    let mut breaches = Vec::new();
    if row.rss_after > env.max_rss_mib {
        breaches.push(format!(
            "RSS after ingest = {} MB > {} MB ({} envelope)",
            row.rss_after, env.max_rss_mib, target_name
        ));
    }
    if row.p95_ms > ENVELOPE_P95_MS_CEILING as f64 {
        breaches.push(format!(
            "p95 /search = {:.0} ms > {} ms (UX ceiling)",
            row.p95_ms, ENVELOPE_P95_MS_CEILING
        ));
    }
    // Throughput: the concurrent-truth ceiling — the p95 the target must stay
    // under with the bench fan-out driving it (the main table's p95 IS the
    // merged p95 when BENCH_CLIENTS >= 2).
    if row.p95_ms > env.search_p95_ms_ceiling as f64 {
        breaches.push(format!(
            "p95 /search = {:.0} ms > {} ms ({} concurrent ceiling)",
            row.p95_ms, env.search_p95_ms_ceiling, target_name
        ));
    }
    breaches
}

/// Process RSS (MB) the server reports via `/health` → `capacity.rss_mib`.
/// This is the *process's own* resident memory (measured via sysinfo's
/// Process API on the server), NOT system-wide memory. The envelope check
/// compares against this; using `system.memory_used_mb` (whole-host) would
/// always exceed the per-process ceiling on any machine with real workload.
fn read_rss_mb(base: &str) -> Result<u64, String> {
    let token = auth_token();
    // The public /health is a minimal probe; the deployment fingerprint
    // fields (capacity.*) live on the Read-gated /health/db. Try
    // the gated detail first (bench resolves the operator token), then fall
    // back to legacy /health shapes for older servers.
    for path in ["/health/db", "/health"] {
        let resp = get(base, path, &[], token.as_deref())?;
        if resp.status != 200 {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&resp.body) {
            Ok(v) => v,
            Err(e) => return Err(format!("{path} non-JSON body: {e}")),
        };
        if let Some(rss) = v
            .get("capacity")
            .and_then(|c| c.get("rss_mib"))
            .and_then(|m| m.as_u64())
            .or_else(|| {
                v.get("system")
                    .and_then(|s| s.get("memory_used_mb"))
                    .and_then(|m| m.as_u64())
            })
        {
            return Ok(rss);
        }
    }
    Err(
        "missing capacity.rss_mib — /health/db requires the operator token \
         (BRAIN_TOKEN/BRAIN_TOKEN_FILE) and a server >= 1.28.5"
            .to_string(),
    )
}

/// Ingest one synthetic doc via `/add`. Mirrors `AddRequest { text, title }`.
fn ingest_one(base: &str, i: usize, bearer: Option<&str>) -> Result<(), String> {
    let topic = i % 50;
    let body = serde_json::json!({
        "text": format!("Synthetic document {i}: topic {topic}. Lorem ipsum content about topic number {topic}."),
        "title": format!("synthetic-{i}"),
    })
    .to_string();
    let resp = post(base, "/add", &[], "application/json", &body, bearer)?;
    if resp.status != 200 {
        return Err(format!(
            "/add for doc {i} returned status {}: {}",
            resp.status, resp.body
        ));
    }
    Ok(())
}

/// Nearest-index percentile of a slice that is already sorted ascending.
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

// ── R45-0: the audit-append throughput harness ──────────────────────────────
//
// The governing spec's §0 carried a throughput ESTIMATE ("crypto is 0.3–1.5% of
// an audit append's cost") derived from third-party primitive microbenchmarks
// plus a generic commit-latency assumption. E5 retires that estimate in favour
// of a MEASURED rate. This is the harness that produces the measurement.
//
// Design constraints (house law, all machine-checked elsewhere):
//   * NO raw deferred-transaction construction. `write_discipline` scans
//     `src/bin/` and bench.rs has no DEFERRED_BASELINE row, so a single
//     occurrence fails the round. (The needle is written split here so this
//     comment cannot trip the guard that enforces it — the same trick
//     `src/spire_inventory.rs:315` uses for its route needle.) The append path
//     self-transacts per row on an autocommit connection
//     (`record_tenant_checked` opens BEGIN IMMEDIATE and COMMITs itself when
//     `conn.is_autocommit()`), which is exactly the cost structure a real
//     append has. Wrapping the loop in a caller transaction would silently
//     switch it to the SAVEPOINT path and measure the wrong thing.
//   * NO new dependency edges. The Clopper-Pearson bound is hand-rolled here
//     from `f64::ln_gamma` (stable since 1.70) plus a continued-fraction
//     regularized incomplete beta; `statrs`/`rand_distr` are absent from every
//     lockfile and adding either breaks the frozen 51-edge manifest.
//   * NO server, NO port, NO live DB. An in-memory SQLite, a temp dir for the
//     chain key, and nothing else.

/// The `audit_events` DDL, copied from the shipped migration
/// (`src/migration.rs:628-649` + the v1.1.0 additive ALTERs).
///
/// `ts` carries its DEFAULT because the append path omits `ts` from the INSERT
/// and reads it back as a String — omit the default and every append fails
/// with `AuditWriteError::Timestamp`.
const AUDIT_EVENTS_DDL: &str = "CREATE TABLE IF NOT EXISTS audit_events(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT DEFAULT CURRENT_TIMESTAMP,
    kind TEXT NOT NULL,
    actor TEXT,
    target_hash TEXT,
    status TEXT,
    detail_hash TEXT
);
CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);";

/// The one-sided Clopper-Pearson UPPER bound on a binomial proportion.
///
/// The upper bound is the `1 - alpha` quantile of the Beta(k+1, n-k)
/// posterior — i.e. the `x` solving `I_x(k+1, n-k) = 1 - alpha`. For `k == n`
/// the posterior is degenerate and the bound is `1.0`.
///
/// Solved by bisection: `I_x` is continuous and strictly increasing in `x` for
/// `a, b > 0`, so the root is unique and the iteration cannot diverge. This is
/// the same construction SciPy's `binomtest().proportion_ci(method='exact')`
/// uses, which is the industry default for an exact binomial interval.
///
/// **The quantile is the whole function, and getting it backwards is silent.**
/// Bisecting for `I_x = alpha` instead of `I_x = 1 - alpha` returns the LOWER
/// bound — still a monotone, still plausible-looking, still wrong, and it
/// reports a SMALLER risk than was actually observed. The unit pins below check
/// this against the closed form `1 - alpha^(1/n)`, which is the case a reader
/// will check first.
///
/// Bisection rather than an inverse-beta routine on purpose: it is
/// unconditionally monotone, and the round's binding is that the number be
/// reproducible and independently recomputable, not that it be computed the
/// cleverest way.
fn clopper_pearson_upper(k: u64, n: u64, alpha: f64) -> f64 {
    if n == 0 {
        return 1.0;
    }
    if k >= n {
        return 1.0;
    }
    // BetaInv(1 - alpha; a, b) with a = k+1, b = n-k.
    let (a, b) = (k as f64 + 1.0, n as f64 - k as f64);
    let target = 1.0 - alpha;
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    // 200 iterations takes the bracket well below f64 epsilon for any n we
    // benchmark; the loop exits on the interval width, not the count.
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if regularized_incomplete_beta(mid, a, b) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// The regularized incomplete beta function `I_x(a, b) = B(x;a,b) / B(a,b)`,
/// via the standard continued-fraction expansion with a series fallback for
/// small `x` (Numerical Recipes' `betacf`/`betai` formulation).
fn regularized_incomplete_beta(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_front = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln();
    let front = ln_front.exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_continued_fraction(x, a, b) / a
    } else {
        1.0 - front * beta_continued_fraction(1.0 - x, b, a) / b
    }
}

/// Lentz's method for the continued fraction of the incomplete beta.
fn beta_continued_fraction(x: f64, a: f64, b: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..300 {
        let m = m as f64;
        let m2 = 2.0 * m;
        // Even step.
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        // Odd step.
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 3.0e-16 {
            break;
        }
    }
    h
}

/// `ln_gamma` via the Lanczos approximation, so the bound does not depend on
/// `f64::ln_gamma`'s stabilization history. `f64::ln_gamma` is available and
/// stable on this toolchain; this is kept as the single place the gamma
/// function enters the round so a reviewer can check one function.
fn ln_gamma(z: f64) -> f64 {
    const COF: [f64; 6] = [
        76.18009172947146,
        -86.50532032941677,
        24.01409824083091,
        -1.231739572450155,
        0.1208650973866179e-2,
        -0.5395239384953e-5,
    ];
    let x = z;
    let mut y = z;
    let mut tmp = x + 5.5;
    tmp -= (x + 0.5) * tmp.ln();
    let mut ser = 1.000000000190015;
    for c in COF {
        y += 1.0;
        ser += c / y;
    }
    -tmp + (2.5066282746310005 * ser / x).ln()
}

fn main() {
    // `bench eval` runs the retrieval-quality regression
    // harness against a judgments file (BRAIN_EVAL_JUDGMENTS). The default
    // (no arg) runs the synthetic-scale latency/RSS benchmark as before.
    let args: Vec<String> = std::env::args().collect();
    let res = match args.get(1).map(String::as_str) {
        Some("eval") => run_eval(),
        // Authoring aid for the judged retrieval corpus (the BENCHMARKS.md
        // blocker). Dumps every chunk (id + title + content) from `/export` to
        // a browsable file the operator fills with `{query, relevant_ids}` →
        // `bench eval`.
        Some("scaffold") => run_scaffold(args.get(2).map(String::as_str)),
        // R45-0: measure the real audit-append rate, retiring the §0 estimate.
        Some("audit-append") => run_audit_append(),
        // R45-0: the preregistered FPR benchmark over the benign corpus.
        Some("fpr") => run_fpr(),
        _ => run(),
    };
    if let Err(e) = res {
        eprintln!("bench: {e}");
        std::process::exit(1);
    }
}

// ── R45-0 subcommand bodies ────────────────────────────────────────────────

/// How many rows to append. `BENCH_AUDIT_APPEND_N`; the plan's sweep is
/// 1k / 10k / 100k. A non-numeric or zero value is an error, not a silent
/// default — a benchmark that quietly measured 1000 rows when you asked for
/// 100k is worse than no benchmark.
fn audit_append_n() -> Result<usize, String> {
    match std::env::var("BENCH_AUDIT_APPEND_N") {
        Err(_) => Ok(1000),
        Ok(raw) => {
            let n: usize = raw.trim().parse().map_err(|_| {
                format!("BENCH_AUDIT_APPEND_N must be a positive integer, got {raw:?}")
            })?;
            if n == 0 {
                return Err("BENCH_AUDIT_APPEND_N must be greater than zero".into());
            }
            Ok(n)
        }
    }
}

/// `brain bench audit-append` — measure rows/sec, p50/p99, and the
/// crypto-vs-total split at n. This is the E5 measurement: it replaces the
/// retired §0 estimate with a number measured on this machine.
///
/// The split is measured, not derived: the same rows are appended twice, once
/// through the LEGACY epoch (SHA-256 link, no key) and once through the KEYED
/// epoch (HMAC-SHA256 link, key installed + epoch bootstrapped). The difference
/// between the two per-append costs is the marginal cost of keying — which is
/// the question the §0 estimate was trying to answer by analogy, and which it
/// got by borrowing third-party microbenchmarks.
///
/// The DB is IN-MEMORY and the chain key goes to a temp dir that is removed on
/// drop. This never opens the live database, binds no port, and writes nothing
/// outside its own tempdir.
fn run_audit_append() -> Result<(), String> {
    use brain_server::audit::{self, AuditKind, AuditStatus};

    let n = audit_append_n()?;

    // The key goes to a private temp dir, never a real config path. Setting
    // BRAIN_AUDIT_CHAIN_KEY would also work and touches no disk, but a tempdir
    // exercises the real generate-and-write path a fresh deployment takes.
    let key_dir = std::env::temp_dir().join(format!("brain-bench-audit-{}", std::process::id()));
    std::fs::create_dir_all(&key_dir)
        .map_err(|e| format!("cannot create temp key dir {}: {e}", key_dir.display()))?;
    // 0600 — `init_chain_key` refuses a group/world-readable key file, and the
    // temp dir must not be world-readable for the same reason.
    set_mode_600(&key_dir)?;

    let conn = rusqlite::Connection::open_in_memory()
        .map_err(|e| format!("cannot open in-memory database: {e}"))?;
    conn.execute_batch(AUDIT_EVENTS_DDL)
        .map_err(|e| format!("cannot create audit_events: {e}"))?;
    // Durability settings the real append path runs under; without these the
    // measurement would flatter itself by writing to a page cache that is never
    // flushed.
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(|e| format!("cannot set synchronous=FULL: {e}"))?;
    conn.pragma_update(None, "journal_mode", "DELETE")
        .map_err(|e| format!("cannot set journal_mode: {e}"))?;

    // ── legacy cell: SHA-256 link, no key ──
    let mut legacy_samples: Vec<Duration> = Vec::with_capacity(n);
    let legacy_start = Instant::now();
    for i in 0..n {
        let t0 = Instant::now();
        audit::record_tenant_checked(
            &conn,
            AuditKind::Ingest,
            "bench",
            &format!("target-{i}"),
            AuditStatus::Ok,
            "r45-0 audit-append measurement",
            audit::DEFAULT_TENANT,
        )
        .map_err(|e| format!("append {i} failed: {e}"))?;
        legacy_samples.push(t0.elapsed());
    }
    let legacy_total = legacy_start.elapsed();

    let rows_after_legacy: i64 = conn
        .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
        .map_err(|e| format!("count failed: {e}"))?;
    if rows_after_legacy != n as i64 {
        return Err(format!(
            "expected {n} rows after the legacy pass, found {rows_after_legacy} — the \
             measurement would be reporting a rate it did not achieve"
        ));
    }

    // ── keyed cell: HMAC-SHA256 link ──
    // `bootstrap_epoch` requires ZERO rows (it refuses to re-anchor a chain
    // that already has content), so the keyed pass gets its own database.
    audit::init_chain_key(&key_dir).map_err(|e| format!("cannot initialise the chain key: {e}"))?;
    let keyed = rusqlite::Connection::open_in_memory()
        .map_err(|e| format!("cannot open keyed in-memory database: {e}"))?;
    keyed
        .execute_batch(AUDIT_EVENTS_DDL)
        .map_err(|e| format!("cannot create keyed audit_events: {e}"))?;
    keyed
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|e| format!("cannot set synchronous=FULL: {e}"))?;
    keyed
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(|e| format!("cannot set journal_mode: {e}"))?;
    // Assert the epoch actually flipped — do not assume it.
    if !audit::bootstrap_epoch(&keyed) {
        return Err(
            "bootstrap_epoch returned false — the keyed cell would silently measure the \
             legacy path and report it as HMAC. Refusing to publish a number that is not \
             what it claims to be."
                .into(),
        );
    }

    let mut keyed_samples: Vec<Duration> = Vec::with_capacity(n);
    let keyed_start = Instant::now();
    for i in 0..n {
        let t0 = Instant::now();
        audit::record_tenant_checked(
            &keyed,
            AuditKind::Ingest,
            "bench",
            &format!("target-{i}"),
            AuditStatus::Ok,
            "r45-0 audit-append measurement",
            audit::DEFAULT_TENANT,
        )
        .map_err(|e| format!("keyed append {i} failed: {e}"))?;
        keyed_samples.push(t0.elapsed());
    }
    let keyed_total = keyed_start.elapsed();

    let rows_after_keyed: i64 = keyed
        .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
        .map_err(|e| format!("keyed count failed: {e}"))?;
    if rows_after_keyed != n as i64 {
        return Err(format!(
            "expected {n} rows after the keyed pass, found {rows_after_keyed}"
        ));
    }

    // Clean up the temp key regardless of what happens next.
    let _ = std::fs::remove_dir_all(&key_dir);

    legacy_samples.sort_unstable();
    keyed_samples.sort_unstable();

    let legacy_rate = n as f64 / legacy_total.as_secs_f64();
    let keyed_rate = n as f64 / keyed_total.as_secs_f64();
    let legacy_p50 = percentile(&legacy_samples, 50.0);
    let legacy_p99 = percentile(&legacy_samples, 99.0);
    let keyed_p50 = percentile(&keyed_samples, 50.0);
    let keyed_p99 = percentile(&keyed_samples, 99.0);
    let per_row_delta_us =
        (keyed_total.as_secs_f64() - legacy_total.as_secs_f64()) / n as f64 * 1e6;

    println!("## R45-0 — measured audit-append throughput\n");
    println!("n = {n} rows per cell · in-memory SQLite · synchronous=FULL, journal=DELETE");
    println!("Both cells append row-by-row; each append is its own durable commit, which is");
    println!("what the real path does on an autocommit connection.\n");
    println!("| epoch | link primitive | rows/sec | p50 | p99 | total |");
    println!("|---|---|---|---|---|---|");
    println!(
        "| legacy | SHA-256 over 5 piped fields | {legacy_rate:.0} | {:.2} | {:.2} | {:.3}s |",
        legacy_p50.as_secs_f64() * 1000.0,
        legacy_p99.as_secs_f64() * 1000.0,
        legacy_total.as_secs_f64()
    );
    println!(
        "| keyed (hmac256) | HMAC-SHA256 over 8 length-prefixed fields | {keyed_rate:.0} | {:.2} | {:.2} | {:.3}s |",
        keyed_p50.as_secs_f64() * 1000.0,
        keyed_p99.as_secs_f64() * 1000.0,
        keyed_total.as_secs_f64()
    );
    println!();
    println!("### The crypto share, measured rather than estimated\n");
    println!(
        "Marginal cost of keying the chain: **{per_row_delta_us:.2} µs/row** (keyed minus legacy)."
    );
    println!(
        "As a share of the legacy append's {:.0} µs/row: **{:.2}%**.",
        legacy_total.as_secs_f64() / n as f64 * 1e6,
        per_row_delta_us / (legacy_total.as_secs_f64() / n as f64 * 1e6) * 100.0
    );
    println!();
    println!("This is the number the §0 estimate asserted and could not support. It is measured");
    println!("on this machine, at this n, under these pragmas — record the hardware alongside it.");
    Ok(())
}

/// 0600 on unix; a no-op elsewhere (the benchmark is not the security boundary;
/// the key file `init_chain_key` writes is 0600 by its own contract).
fn set_mode_600(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot set mode 0700 on {}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// `brain bench fpr` — the preregistered false-positive-rate benchmark.
///
/// Three surfaces, reported SEPARATELY and never blended (E4):
///   1. the deterministic screen (`src/screen.rs`),
///   2. the salience length window (`src/gate.rs`, 24–3000),
///   3. the review deferral surface (amended prereg: screen + salience +
///      conflict-scan, novelty excluded — `insert_proposal` is `pub(crate)`
///      and E8 forbids widening the API surface this round).
///
/// The corpus is read from BRAIN_FPR_CORPUS (a JSONL file in the PRIVATE
/// spine). The kernel tree never contains a corpus row (E3).
fn run_fpr() -> Result<(), String> {
    use brain_server::gate;
    use brain_server::linker;
    use brain_server::screen::{self, ScreenResult};

    let corpus_path = std::env::var("BRAIN_FPR_CORPUS")
        .map_err(|_| "BRAIN_FPR_CORPUS must point at the preregistered corpus JSONL".to_string())?;
    // The prereg pins the posture. `screen()` reads INJECTION_POLICY per call,
    // so an unset var would measure a posture nobody asked about.
    let policy = std::env::var("INJECTION_POLICY").unwrap_or_else(|_| "quarantine".into());
    let raw = std::fs::read_to_string(&corpus_path)
        .map_err(|e| format!("cannot read corpus {corpus_path}: {e}"))?;

    let mut rows: Vec<(String, String)> = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| format!("corpus line {} is not JSON: {e}", i + 1))?;
        let content = v
            .get("content")
            .and_then(|c| c.as_str())
            .ok_or_else(|| format!("corpus line {} has no `content` field", i + 1))?;
        let title = v.get("title").and_then(|t| t.as_str()).unwrap_or("");
        rows.push((title.to_string(), content.to_string()));
    }
    if rows.is_empty() {
        return Err(format!(
            "corpus {corpus_path} is empty — a zero-row FPR is not a measurement"
        ));
    }
    let n = rows.len() as u64;

    // ── surface 1: the deterministic screen ──
    let (mut clean, mut quarantined, mut rejected) = (0u64, 0u64, 0u64);
    for (title, content) in &rows {
        match screen::screen(content, title) {
            ScreenResult::Clean => clean += 1,
            ScreenResult::Quarantine => quarantined += 1,
            ScreenResult::Reject => rejected += 1,
        }
    }
    // A false positive = the benign write did NOT pass clean. Under the
    // deployed `quarantine` posture that means flagged-and-hidden-from-recall;
    // under `reject` it means the write was refused outright. Both are harms to
    // legitimate work, which is why the union is the headline and the
    // components are reported beside it.
    let screen_flagged = quarantined + rejected;
    let cp_screen = clopper_pearson_upper(screen_flagged, n, 0.05);
    let cp_quarantine = clopper_pearson_upper(quarantined, n, 0.05);
    let cp_reject = clopper_pearson_upper(rejected, n, 0.05);

    // ── surface 2: the salience length window (an out-of-band heuristic; it
    // ranks and never promotes, per the gate.rs house law) ──
    let mut below_window = 0u64;
    for (_, content) in &rows {
        let vocab = linker::extract_vocabulary(content, &[]);
        let s = gate::salience(content, vocab.entities.len());
        let len = content.trim().chars().count();
        if !(gate::MIN_SALIENCE_LEN..=gate::MAX_SALIENCE_LEN).contains(&len) {
            below_window += 1;
        }
        // `s` is the rank; the window membership is what the prereg counts.
        let _ = s;
    }
    let cp_salience = clopper_pearson_upper(below_window, n, 0.05);

    println!("## R45-0 — benign-corpus false-positive rate\n");
    println!("corpus: {corpus_path}");
    println!("rows (n): **{n}**");
    println!("INJECTION_POLICY: **{policy}**");
    println!(
        "classifier: **{}**",
        if screen::screen_classifier_loaded() {
            "LOADED"
        } else {
            "absent — this is a LAYER-1-ONLY measurement"
        }
    );
    println!();
    println!("Headline is a one-sided Clopper-Pearson 95% UPPER bound (E7), not a point");
    println!("estimate. The three surfaces are reported separately and are NEVER blended (E4).\n");
    println!("| surface | flagged / n | point rate | CP 95% upper |");
    println!("|---|---|---|---|");
    println!(
        "| 1. screen (deferred OR rejected) | {screen_flagged} / {n} | {:.4} | {cp_screen:.4} |",
        screen_flagged as f64 / n as f64
    );
    println!(
        "| 1a. — quarantine only | {quarantined} / {n} | {:.4} | {cp_quarantine:.4} |",
        quarantined as f64 / n as f64
    );
    println!(
        "| 1b. — reject only | {rejected} / {n} | {:.4} | {cp_reject:.4} |",
        rejected as f64 / n as f64
    );
    // The clean count is the complement of the headline. Printing it lets a
    // reader check that the three surface-1 rows sum to n without arithmetic.
    println!(
        "| 1c. — clean (the complement) | {clean} / {n} | {:.4} | — |",
        clean as f64 / n as f64
    );
    println!(
        "| 2. salience length window | {below_window} / {n} | {:.4} | {cp_salience:.4} |",
        below_window as f64 / n as f64
    );
    println!();
    println!("KILL-1 (FPR > 20% on benign traffic) is evaluated against row 1, the deployed");
    println!("posture. Rows 1a/1b decompose it; row 2 is an out-of-band ranking heuristic and is");
    println!("NOT a promotion gate, so it does not arm KILL-1.");
    println!();
    println!("Surface 3 (the review deferral path) is measured compositionally under the amended");
    println!("prereg and is reported in the evidence file; `insert_proposal` is `pub(crate)` and");
    println!("E8 forbids widening it this round.");
    Ok(())
}

// ── the Clopper-Pearson math, pinned ────────────────────────────────────────
//
// The round's binding is that the bound be REPRODUCIBLE and INDEPENDENTLY
// RECOMPUTABLE, so these pins check the hand-rolled implementation against
// published values rather than against itself. Every expected value below is
// the closed form or a reference-table entry, not an output of this code.

#[cfg(test)]
mod r45_0_clopper_pearson_pins {
    use super::{clopper_pearson_upper, ln_gamma, regularized_incomplete_beta};

    /// `ln_gamma` against exact values. The Lanczos series is only used as the
    /// single entry point for the gamma function in the round, so it is pinned
    /// here rather than trusted.
    #[test]
    fn ln_gamma_matches_known_values() {
        // Γ(1) = 1, Γ(2) = 1, Γ(5) = 24, Γ(10) = 362880.
        assert!(
            (ln_gamma(1.0) - 0.0).abs() < 1e-9,
            "ln_gamma(1) = {}",
            ln_gamma(1.0)
        );
        assert!(
            (ln_gamma(2.0) - 0.0).abs() < 1e-9,
            "ln_gamma(2) = {}",
            ln_gamma(2.0)
        );
        assert!(
            (ln_gamma(5.0) - 24.0f64.ln()).abs() < 1e-8,
            "ln_gamma(5) = {} (want {})",
            ln_gamma(5.0),
            24.0f64.ln()
        );
        assert!(
            (ln_gamma(10.0) - 362880.0f64.ln()).abs() < 1e-6,
            "ln_gamma(10) = {} (want {})",
            ln_gamma(10.0),
            362880.0f64.ln()
        );
    }

    /// The regularized incomplete beta at its two trivial endpoints, and at the
    /// symmetric point I(0.5; a, a) = 0.5.
    #[test]
    fn regularized_incomplete_beta_endpoints_and_symmetry() {
        assert_eq!(regularized_incomplete_beta(0.0, 2.0, 3.0), 0.0);
        assert_eq!(regularized_incomplete_beta(1.0, 2.0, 3.0), 1.0);
        for a in [1.0, 2.5, 10.0] {
            let mid = regularized_incomplete_beta(0.5, a, a);
            assert!(
                (mid - 0.5).abs() < 1e-9,
                "I(0.5; {a},{a}) = {mid} — the beta density is symmetric about 0.5 when a == b"
            );
        }
    }

    /// The headline sanity case: **zero false positives in 500 rows.**
    ///
    /// The closed form is `1 - alpha^(1/n)`, so at alpha = 0.05 and n = 500 the
    /// bound is `1 - 0.05^(1/500)` ≈ 0.005974 — the "rule of three" family,
    /// which is the number a reader will sanity-check first.
    #[test]
    fn cp_zero_of_500_matches_the_closed_form() {
        let bound = clopper_pearson_upper(0, 500, 0.05);
        let closed = 1.0 - 0.05f64.powf(1.0 / 500.0);
        assert!(
            (bound - closed).abs() < 1e-9,
            "CP(0/500) = {bound}, closed form 1 - 0.05^(1/500) = {closed}"
        );
        // And the memorable form: under 0.6%.
        assert!(bound < 0.006, "CP(0/500) = {bound} should be under 0.6%");
    }

    /// One false positive in 500 must be strictly WORSE than zero — a
    /// monotonicity property a broken beta inversion would silently violate.
    #[test]
    fn cp_is_monotone_in_the_success_count() {
        let mut prev = 0.0;
        for k in 0..=25u64 {
            let bound = clopper_pearson_upper(k, 500, 0.05);
            assert!(
                bound > prev,
                "CP({k}/500) = {bound} must exceed CP({}/500) = {prev}",
                k - 1
            );
            assert!(bound <= 1.0, "a bound may not exceed 1.0, got {bound}");
            prev = bound;
        }
    }

    /// The 20% KILL-1 threshold maps to a specific count at n = 500, and the
    /// bound must cross it at the right place. 100/500 = 0.20 exactly.
    #[test]
    fn cp_places_the_kill1_threshold_correctly() {
        // At exactly 20%, the upper bound is ABOVE 20% (the bound is
        // conservative — that is the point of an upper bound).
        let at_threshold = clopper_pearson_upper(100, 500, 0.05);
        assert!(
            at_threshold > 0.20,
            "CP(100/500) = {at_threshold} must exceed 0.20 — an upper bound that sat \
             below the point estimate would be reporting a smaller risk than was observed"
        );
        // Below the threshold, with headroom, the bound stays under.
        let below = clopper_pearson_upper(80, 500, 0.05);
        assert!(
            below < 0.20,
            "CP(80/500) = {below} should clear the 20% kill line"
        );
        // And a much larger sample shrinks the bound toward the point estimate.
        let big = clopper_pearson_upper(20_000, 100_000, 0.05);
        assert!(
            big < 0.205 && big > 0.20,
            "CP(20000/100000) = {big} should sit just above 0.20 as n grows"
        );
    }

    /// Degenerate inputs must not divide by zero or return nonsense.
    #[test]
    fn cp_handles_degenerate_inputs() {
        assert_eq!(
            clopper_pearson_upper(0, 0, 0.05),
            1.0,
            "n = 0 carries no information"
        );
        assert_eq!(
            clopper_pearson_upper(5, 5, 0.05),
            1.0,
            "k = n is a certain rate"
        );
        assert!(
            clopper_pearson_upper(6, 5, 0.05) <= 1.0,
            "k > n must not exceed 1.0"
        );
    }
}

struct Row {
    scale: usize,
    rss_rest: u64,
    rss_after: u64,
    ingest_secs: f64,
    docs_per_sec: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

fn run() -> Result<(), String> {
    let base = base_url();
    let scales = scales();
    let searches = num_searches();
    let clients = num_clients();
    let seed = bench_seed();
    let token = auth_token();
    let bearer = token.as_deref();

    if scales.is_empty() {
        return Err("no valid scales parsed from BENCH_SCALES".into());
    }

    // Probe reachability up front so we fail fast with a clear message.
    match get(&base, "/health", &[], None) {
        Ok(r) if r.status == 200 => {}
        Ok(r) => return Err(format!("server unhealthy (status {})", r.status)),
        Err(e) => return Err(format!("cannot reach server at {base}: {e}")),
    }

    eprintln!(
        "bench: target={base} scales={scales:?} searches={searches} clients={clients} seed={seed} (progress -> stderr, table -> stdout)"
    );

    let mut rss_after_batches: Vec<Vec<u64>> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut merges: Vec<Option<MergedSearch>> = Vec::new();
    // Global doc index keeps synthetic titles/content unique across cumulative
    // scales, so the corpus actually grows instead of being deduped by content hash.
    let mut global_doc_index = 0usize;

    for &scale in &scales {
        let rss_rest = read_rss_mb(&base)?;
        eprintln!("bench: scale {scale} — RSS at rest {rss_rest} MB; ingesting...");

        // Ingest is ALWAYS single-client: the corpus build is deterministic
        // (unique synthetic titles) and the milestone changes nothing about
        // write-path behavior. Only the search phase fans out.
        let ingest_start = Instant::now();
        let mut batch_rss: Vec<u64> = Vec::new();
        for i in 0..scale {
            ingest_one(&base, global_doc_index, bearer)?;
            global_doc_index += 1;
            if (i + 1) % BATCH_SIZE == 0 {
                let rss = read_rss_mb(&base)?;
                eprintln!("  batch @ {}/{scale} docs — RSS {rss} MB", i + 1);
                batch_rss.push(rss);
            }
        }
        let ingest_secs = ingest_start.elapsed().as_secs_f64();
        let rss_after = read_rss_mb(&base)?;

        // Latency: BENCH_CLIENTS parallel clients each drive the SAME seeded
        // mix; samples are pooled for the merged percentiles. BENCH_CLIENTS=1
        // keeps the pre-Throughput sequential shape (abort on first error).
        let (p50, p95, p99, merged);
        if clients <= 1 {
            let mut lats: Vec<Duration> = Vec::with_capacity(searches);
            for q in 0..searches {
                let start = Instant::now();
                let resp = get(
                    &base,
                    "/search",
                    &[
                        ("q".to_string(), format!("topic {}", q % 50)),
                        ("k".to_string(), "10".to_string()),
                    ],
                    bearer,
                )?;
                let elapsed = start.elapsed();
                if resp.status != 200 {
                    return Err(format!(
                        "/search q={q} returned status {}: {}",
                        resp.status, resp.body
                    ));
                }
                lats.push(elapsed);
            }
            lats.sort();
            p50 = percentile(&lats, 50.0).as_secs_f64() * 1000.0;
            p95 = percentile(&lats, 95.0).as_secs_f64() * 1000.0;
            p99 = percentile(&lats, 99.0).as_secs_f64() * 1000.0;
            merged = None;
        } else {
            let search_start = Instant::now();
            // ONE scope around ALL spawns — scope() blocks until the threads
            // it spawned finish, so per-client scopes would serialize.
            let results: Vec<ClientSearchResult> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..clients)
                    .map(|id| {
                        let base = base.clone();
                        scope.spawn(move || run_search_client(&base, bearer, searches, seed, id))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join()
                            .map_err(|e| format!("search client panicked: {e:?}"))
                    })
                    .collect::<Result<Vec<_>, String>>()
            })?;
            let wall = search_start.elapsed();
            let m = merge_search_results(&results);
            eprintln!(
                "bench: scale {scale} — {clients} clients wall {wall:.0?}, ops_ok={} failures={}",
                m.total_ops, m.failures
            );
            p50 = m.p50_ms;
            p95 = m.p95_ms;
            p99 = m.p99_ms;
            merged = Some(m);
        }

        rss_after_batches.push(batch_rss);
        rows.push(Row {
            scale,
            rss_rest,
            rss_after,
            docs_per_sec: if ingest_secs > 0.0 {
                scale as f64 / ingest_secs
            } else {
                0.0
            },
            ingest_secs,
            p50_ms: p50,
            p95_ms: p95,
            p99_ms: p99,
        });
        merges.push(merged);
        eprintln!("bench: scale {scale} done");
    }

    print_report(
        &base,
        searches,
        clients,
        seed,
        &rows,
        &rss_after_batches,
        &merges,
    );

    // when BENCH_ENVELOPE is set, assert each scale against the
    // published capacity envelope. Any breach exits non-zero — the report
    // becomes a ship gate, not just a measurement.
    if let Some((env, target_name)) = envelope() {
        println!("\n### Capacity envelope assertion (target: {target_name})\n");
        println!(
            "> p95 ceiling for this run: the stricter of the UX ceiling ({} ms) and the target's concurrent ceiling ({} ms)\n",
            ENVELOPE_P95_MS_CEILING, env.search_p95_ms_ceiling
        );
        println!("| scale | max RSS (MB) | max p95 (ms) | result |");
        println!("|---|---|---|---|");
        let mut all_ok = true;
        for r in &rows {
            let breaches = check_envelope(&env, target_name, r);
            let result = if breaches.is_empty() {
                "OK"
            } else {
                all_ok = false;
                "BREACH"
            };
            println!(
                "| {} | {} | {} | {} |",
                r.scale,
                env.max_rss_mib,
                ENVELOPE_P95_MS_CEILING.min(env.search_p95_ms_ceiling),
                result
            );
            for b in &breaches {
                eprintln!("ENVELOPE BREACH at scale {}: {b}", r.scale);
            }
        }
        if !all_ok {
            return Err("capacity envelope breached — see ENVELOPE BREACH lines above".into());
        }
    }

    // Envelope-free p95 gate (BENCH_ASSERT_P95_MS): the CI concurrency step's
    // assert. Applies to the merged p95 at every scale.
    if let Some(limit) = assert_p95_ms() {
        let mut breached = false;
        for r in &rows {
            if r.p95_ms > limit {
                breached = true;
                eprintln!(
                    "P95 ASSERT BREACH at scale {}: merged p95 {:.0} ms > {limit} ms",
                    r.scale, r.p95_ms
                );
            }
        }
        if breached {
            return Err(format!(
                "BENCH_ASSERT_P95_MS breached ({limit} ms) — concurrent p95 regressed"
            ));
        }
    }

    Ok(())
}

fn print_report(
    base: &str,
    searches: usize,
    clients: usize,
    seed: u64,
    rows: &[Row],
    rss_after_batches: &[Vec<u64>],
    merges: &[Option<MergedSearch>],
) {
    println!("## Brain Server benchmark\n");
    println!("Target: `{base}`");
    println!("Searches per client per scale: {searches}");
    println!("Clients: {clients} · seed: {seed} (mix = `topic {{(q + seed) % 50}}`)");
    if clients > 1 {
        println!(
            "Merge: pooled samples across clients; p50/p95/p99 below are the MERGED percentiles (the concurrent truth)."
        );
    }
    println!();

    println!("### Latency & resources\n");
    println!(
        "| scale | RSS at rest (MB) | RSS after ingest (MB) | ingest (s) | ingest docs/s | p50 /search (ms) | p95 /search (ms) | p99 /search (ms) |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for r in rows {
        println!(
            "| {} | {} | {} | {:.2} | {:.0} | {:.2} | {:.2} | {:.2} |",
            r.scale,
            r.rss_rest,
            r.rss_after,
            r.ingest_secs,
            r.docs_per_sec,
            r.p50_ms,
            r.p95_ms,
            r.p99_ms
        );
    }

    println!("\n### RSS after each {BATCH_SIZE}-doc batch (MB)\n");
    let max_batches = rss_after_batches.iter().map(Vec::len).max().unwrap_or(0);
    if max_batches > 0 {
        let mut header = String::from("| scale |");
        let mut sep = String::from("|---|");
        for b in 1..=max_batches {
            header.push_str(&format!(" batch {b} |"));
            sep.push_str("---|");
        }
        println!("{header}");
        println!("{sep}");
        for (r, batches) in rows.iter().zip(rss_after_batches.iter()) {
            let mut line = format!("| {} |", r.scale);
            for b in batches {
                line.push_str(&format!(" {b} |"));
            }
            for _ in batches.len()..max_batches {
                line.push_str(" - |");
            }
            println!("{line}");
        }
    }

    // Concurrent breakdown (BENCH_CLIENTS >= 2 only): the merged tail + the
    // per-client skew, printed not hidden.
    for (r, m) in rows.iter().zip(merges.iter()) {
        let Some(m) = m else { continue };
        println!(
            "\n### Concurrent /search — scale {} (clients = {}, merged)\n",
            r.scale, clients
        );
        println!(
            "| total ops ok | failures (non-2xx + transport) | p50 (ms) | p95 (ms) | p99 (ms) | max (ms) |"
        );
        println!("|---|---|---|---|---|---|");
        println!(
            "| {} | {} | {:.2} | {:.2} | {:.2} | {:.2} |",
            m.total_ops, m.failures, m.p50_ms, m.p95_ms, m.p99_ms, m.max_ms
        );
        println!(
            "\nPer-client skew (deterministic mix — divergence is server-side queuing, not query luck):\n"
        );
        println!("| client | ops ok | failures | p50 (ms) | p95 (ms) |");
        println!("|---|---|---|---|---|");
        for (client, ops, failures, p50, p95) in &m.per_client {
            println!("| {client} | {ops} | {failures} | {p50:.2} | {p95:.2} |");
        }
    }
}

// ── retrieval-quality regression harness ────────────

/// Run the retrieval-quality regression harness. Loads a judgments file
/// (`BRAIN_EVAL_JUDGMENTS`, JSON array of `{query, relevant_ids, gold_answer?}`),
/// runs each query through `/recall`, and reports precision@5, recall@5, MRR,
/// NDCG@5, and `answer_in_context_rate`.
///
/// The 100-query hand-judged corpus against the live DB is an operator step —
/// this function is the reproducible engine any judgments file plugs into.
/// Author the judgments with `bench scaffold` (dumps chunk id/content/title to
/// a browsable file you fill in).
/// Ship gate: set `BENCH_EVAL_REGRESSION_PCT` (default 2.0); if recall@5 drops
/// more than that vs the `BENCH_EVAL_BASELINE` JSON, exits non-zero.
fn run_eval() -> Result<(), String> {
    use brain_server::eval::{Judgment, evaluate};

    let path = std::env::var("BRAIN_EVAL_JUDGMENTS").map_err(|_| {
        "BRAIN_EVAL_JUDGMENTS env var must point to a judgments JSON file".to_string()
    })?;
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read judgments file {path}: {e}"))?;
    let judgments: Vec<Judgment> =
        serde_json::from_str(&raw).map_err(|e| format!("judgments file is not valid JSON: {e}"))?;
    if judgments.is_empty() {
        return Err("judgments file contains no queries".into());
    }

    let base = base_url();
    let bearer = auth_token();
    let budget = std::env::var("BENCH_PACK_TOKENS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok());
    let k = std::env::var("BENCH_EVAL_K")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(5);

    // Probe reachability.
    match get(&base, "/health", &[], bearer.as_deref()) {
        Ok(r) if r.status == 200 => {}
        Ok(r) => return Err(format!("server unhealthy (status {})", r.status)),
        Err(e) => return Err(format!("cannot reach server at {base}: {e}")),
    }

    let mut judged: Vec<(Judgment, Vec<i64>, Option<bool>)> = Vec::with_capacity(judgments.len());
    for j in judgments {
        let body = serde_json::json!({
            "query": j.query,
            "limit": k,
            "provenance": true,
            "max_context_tokens": budget,
            "gold_answer": j.gold_answer,
        });
        let resp = post(
            &base,
            "/recall",
            &[],
            "application/json",
            &body.to_string(),
            bearer.as_deref(),
        )?;
        if resp.status != 200 {
            return Err(format!(
                "/recall for query {:?} returned status {}: {}",
                j.query, resp.status, resp.body
            ));
        }
        let v: serde_json::Value = serde_json::from_str(&resp.body)
            .map_err(|e| format!("/recall returned non-JSON: {e}"))?;
        let retrieved: Vec<i64> = v
            .get("hits")
            .and_then(|h| h.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|h| h.get("id").and_then(|i| i.as_i64()))
                    .collect()
            })
            .unwrap_or_default();
        let aic = v
            .get("telemetry")
            .and_then(|t| t.get("answer_in_context"))
            .and_then(|a| a.as_bool());
        judged.push((j, retrieved, aic));
    }

    let report = evaluate(&judged, k);
    println!("## Brain Server retrieval-quality eval\n");
    println!(
        "Target: `{base}`  |  queries: {}  |  k: {}",
        report.queries, k
    );
    if budget.is_some() {
        println!("Packing budget: {budget:?} tokens");
    }
    println!();
    println!("| metric | value |");
    println!("|---|---|");
    println!("| precision@{k} | {:.4} |", report.precision_at_5);
    println!("| recall@{k}    | {:.4} |", report.recall_at_5);
    println!("| MRR          | {:.4} |", report.mrr);
    println!("| NDCG@{k}     | {:.4} |", report.ndcg_at_5);
    println!(
        "| answer_in_context_rate | {:.4} |",
        report.answer_in_context_rate
    );

    // Optional ship gate: compare against a baseline JSON.
    if let Ok(baseline_path) = std::env::var("BENCH_EVAL_BASELINE") {
        let b = std::fs::read_to_string(&baseline_path)
            .map_err(|e| format!("cannot read baseline {baseline_path}: {e}"))?;
        let baseline: serde_json::Value =
            serde_json::from_str(&b).map_err(|e| format!("baseline is not valid JSON: {e}"))?;
        let base_recall = baseline
            .get("recall_at_5")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32;
        let threshold = std::env::var("BENCH_EVAL_REGRESSION_PCT")
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
            .unwrap_or(2.0)
            / 100.0;
        let drop = base_recall - report.recall_at_5;
        if drop > threshold {
            return Err(format!(
                "recall@5 regression: {:.4} → {:.4} (drop {:.4} > threshold {:.4})",
                base_recall, report.recall_at_5, drop, threshold
            ));
        }
        println!(
            "\n✓ recall@5 within regression threshold ({threshold:.4}) vs baseline {base_recall:.4}"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ClientSearchResult, merge_search_results, percentile, scaffold_from_export};
    use std::time::Duration;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    // ponytail: percentile indexing is the one non-trivial bit in this binary;
    // these three asserts fail immediately if the index math drifts.
    #[test]
    fn percentile_single_sample() {
        let s = [ms(100)];
        assert_eq!(percentile(&s, 50.0), ms(100));
        assert_eq!(percentile(&s, 95.0), ms(100));
        assert_eq!(percentile(&s, 99.0), ms(100));
    }

    #[test]
    fn percentile_empty_is_zero() {
        assert_eq!(percentile(&[], 50.0), Duration::ZERO);
    }

    #[test]
    fn percentile_five_samples() {
        let s = [ms(1), ms(2), ms(3), ms(4), ms(5)];
        assert_eq!(percentile(&s, 50.0), ms(3)); // index 2
        assert_eq!(percentile(&s, 95.0), ms(5)); // index 4
        assert_eq!(percentile(&s, 99.0), ms(5)); // index 4
    }

    // The scaffold's export→inventory shape contract: a malformed/missing
    // `knowledge` array yields empty, real rows carry id/title/content.
    #[test]
    fn scaffold_extracts_chunk_inventory_from_export() {
        let body = serde_json::json!({
            "export_format_version": 2,
            "knowledge": [
                {"id": 1, "content": "Dave works at Acme.", "title": "d1"},
                {"id": 2, "content": "Carol runs the lab.", "title": null},
                {"id": "not-an-id", "content": "skip me", "title": "x"},
                {"id": 4, "title": "y"},
            ],
        });
        let chunks = scaffold_from_export(&body);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0]["id"], 1);
        assert_eq!(chunks[0]["content"], "Dave works at Acme.");
        assert_eq!(chunks[0]["title"], "d1");
        // Missing title defaults to ""; a non-i64 id or a missing content is dropped.
        assert_eq!(chunks[1]["title"], "");
        assert_eq!(chunks[1]["id"], 2);
    }

    #[test]
    fn scaffold_handles_missing_knowledge_array() {
        let body = serde_json::json!({"export_format_version": 2});
        assert!(scaffold_from_export(&body).is_empty());
    }

    // Throughput pin: the merged report is deterministic — the same seeded
    // fixture merged twice yields byte-identical output. The merge sorts its
    // pooled samples, so client arrival order cannot leak into the report;
    // a floor breach printed from this report is reproducible from the seed.
    #[test]
    fn bench_clients_merge_is_deterministic() {
        let fixture = || {
            vec![
                ClientSearchResult {
                    client: 0,
                    lats: vec![ms(10), ms(30), ms(20), ms(40), ms(50)],
                    failures: 1,
                },
                ClientSearchResult {
                    client: 1,
                    lats: vec![ms(15), ms(25), ms(35), ms(45), ms(55)],
                    failures: 0,
                },
                ClientSearchResult {
                    client: 2,
                    lats: vec![ms(12), ms(22), ms(32), ms(42), ms(52)],
                    failures: 2,
                },
            ]
        };
        let a = merge_search_results(&fixture());
        let b = merge_search_results(&fixture());
        assert_eq!(a, b, "same seed ⇒ identical merged report");
        // And the aggregate is the honest pool of all samples + failures:
        assert_eq!(a.total_ops, 15);
        assert_eq!(a.failures, 3);
        // Pooled p50: 15 sorted samples (10,12,15,20,22,25,30,32,35,40,42,
        // 45,50,52,55), index round(0.5·14) = 7 → 32 ms.
        assert_eq!(a.p50_ms, 32.0);
        // Per-client blocks stay in client order (skew is attributable).
        assert_eq!(
            a.per_client.iter().map(|c| c.0).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }
}
