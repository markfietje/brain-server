#![cfg(feature = "bench")]

//! `bench` — synthetic-scale benchmark harness for a running brain-server.
//!
//! Feature-gated (`--features bench`). Connects to a running server over HTTP
//! using the same dependency-free client as `brain`/`mcp`, ingests synthetic
//! docs at 1k/5k/10k scales, and prints a markdown table of resident memory,
//! ingest throughput, and `/search` latency percentiles to stdout.
//!
//! v0.9.9: when `BENCH_ENVELOPE` is set (desktop|jetson), each scale asserts
//! against the published capacity envelope and the run exits non-zero on any
//! breach — turning the report into a ship gate.
//!
//! Env:
//!   BRAIN_URL         base URL of the server (default http://127.0.0.1:8765)
//!   BRAIN_TOKEN_FILE  path to a 0600 secret file (preferred over BRAIN_TOKEN)
//!   BRAIN_TOKEN       raw bearer token (dev convenience)
//!   BENCH_SCALES      comma-separated doc counts (default 1000,5000,10000)
//!   BENCH_SEARCHES    search queries per scale (default 100)
//!   BENCH_ENVELOPE    assert against this capacity envelope (desktop|jetson)
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
        if let Ok(s) = std::fs::read_to_string(p) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Ok(t) = std::env::var("BRAIN_TOKEN") {
        let t = t.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    let default_path = dirs_home().join(".config/brain-server/auth-token");
    if let Ok(s) = std::fs::read_to_string(&default_path) {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

fn dirs_home() -> std::path::PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return std::path::PathBuf::from(h);
    }
    std::path::PathBuf::from(".")
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

/// v0.9.9: capacity envelope to assert against. `None` when `BENCH_ENVELOPE`
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
    breaches
}

/// Resident memory (MB) the server reports via `/health` → `system.memory_used_mb`.
fn read_rss_mb(base: &str) -> Result<u64, String> {
    let resp = get(base, "/health", &[], None)?;
    if resp.status != 200 {
        return Err(format!("/health returned status {}", resp.status));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("/health non-JSON body: {e}"))?;
    v.get("system")
        .and_then(|s| s.get("memory_used_mb"))
        .and_then(|m| m.as_u64())
        .ok_or_else(|| "missing system.memory_used_mb in /health body".to_string())
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

fn main() {
    if let Err(e) = run() {
        eprintln!("bench: {e}");
        std::process::exit(1);
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
        "bench: target={base} scales={scales:?} searches={searches} (progress -> stderr, table -> stdout)"
    );

    let mut rss_after_batches: Vec<Vec<u64>> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    // Global doc index keeps synthetic titles/content unique across cumulative
    // scales, so the corpus actually grows instead of being deduped by content hash.
    let mut global_doc_index: usize = 0;

    for &scale in &scales {
        let rss_rest = read_rss_mb(&base)?;
        eprintln!("bench: scale {scale} — RSS at rest {rss_rest} MB; ingesting...");

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

        // Latency: run `searches` /search queries, record each.
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
        let p50 = percentile(&lats, 50.0);
        let p95 = percentile(&lats, 95.0);
        let p99 = percentile(&lats, 99.0);

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
            p50_ms: p50.as_secs_f64() * 1000.0,
            p95_ms: p95.as_secs_f64() * 1000.0,
            p99_ms: p99.as_secs_f64() * 1000.0,
        });
        eprintln!("bench: scale {scale} done");
    }

    print_report(&base, searches, &rows, &rss_after_batches);

    // v0.9.9: when BENCH_ENVELOPE is set, assert each scale against the
    // published capacity envelope. Any breach exits non-zero — the report
    // becomes a ship gate, not just a measurement.
    if let Some((env, target_name)) = envelope() {
        println!("\n### Capacity envelope assertion (target: {target_name})\n");
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
                r.scale, env.max_rss_mib, ENVELOPE_P95_MS_CEILING, result
            );
            for b in &breaches {
                eprintln!("ENVELOPE BREACH at scale {}: {b}", r.scale);
            }
        }
        if !all_ok {
            return Err("capacity envelope breached — see ENVELOPE BREACH lines above".into());
        }
    }

    Ok(())
}

fn print_report(base: &str, searches: usize, rows: &[Row], rss_after_batches: &[Vec<u64>]) {
    println!("## Brain Server benchmark\n");
    println!("Target: `{base}`");
    println!("Searches per scale: {searches}\n");

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
}

#[cfg(test)]
mod tests {
    use super::percentile;
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
}
