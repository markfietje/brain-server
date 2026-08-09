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

/// Process RSS (MB) the server reports via `/health` → `capacity.rss_mib`.
/// This is the *process's own* resident memory (v0.9.9: measured via sysinfo's
/// Process API on the server), NOT system-wide memory. The envelope check
/// compares against this; using `system.memory_used_mb` (whole-host) would
/// always exceed the per-process ceiling on any machine with real workload.
fn read_rss_mb(base: &str) -> Result<u64, String> {
    let resp = get(base, "/health", &[], None)?;
    if resp.status != 200 {
        return Err(format!("/health returned status {}", resp.status));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("/health non-JSON body: {e}"))?;
    // v0.9.9: prefer the process RSS from the capacity object (accurate).
    // Fall back to system.memory_used_mb if the server predates v0.9.9 (no
    // capacity field yet) so the harness still works against older servers.
    v.get("capacity")
        .and_then(|c| c.get("rss_mib"))
        .and_then(|m| m.as_u64())
        .or_else(|| {
            v.get("system")
                .and_then(|s| s.get("memory_used_mb"))
                .and_then(|m| m.as_u64())
        })
        .ok_or_else(|| {
            "missing capacity.rss_mib (and system.memory_used_mb fallback) in /health body"
                .to_string()
        })
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
    // v1.4.0 "Calibrate" M5: `bench eval` runs the retrieval-quality regression
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
        _ => run(),
    };
    if let Err(e) = res {
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

// ── v1.4.0 "Calibrate" M5: retrieval-quality regression harness ────────────

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
    use brain_server::eval::{evaluate, Judgment};

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
        println!("\n✓ recall@5 within regression threshold ({threshold:.4}) vs baseline {base_recall:.4}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{percentile, scaffold_from_export};
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
}
