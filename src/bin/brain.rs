//! `brain` — command-line client for a running brain-server.
//!
//! Hand-rolled argument parsing (no `clap` dependency). Talks HTTP/1.1 to the
//! server using the shared, dependency-free client in `bin_common/http.rs`.
//!
//! Subcommands:
//!   brain query "<q>" [--k N] [--source S ...] [--phrase P ...]
//!                  [--exclude E ...] [--code C ...] [--since ISO]
//!                  [--intent I] [--profile P] [--explain]
//!   brain explain "<q>" [--source S ...] [--since ISO]
//!   brain get <id>
//!   brain ingest-dir <path> [--dry-run] [--replace] [--source S] [--domain D]
//!   brain bench
//!   brain status
//!   brain doctor

#[path = "../bin_common/http.rs"]
mod http;

use http::{delete, get, post};
use std::path::{Path, PathBuf};
use std::process::exit;

const DEFAULT_URL: &str = "http://127.0.0.1:8765";

/// v0.9.2: walk bounds for `ingest-dir`. Guards against pathological vaults
/// blowing the ingest budget. 50k files / 500 MiB matches the plan's RSS ceiling
/// with headroom for the model + index. ponytail ceiling: a vault larger than
/// this needs the paid live-sync tier (streaming ingest), not one-shot ingest.
const MAX_INGEST_FILES: usize = 50_000;
const MAX_INGEST_BYTES: u64 = 500 * 1024 * 1024;

/// Ground-truth corpus mirrored from `tests/eval.rs` DOCS. Used only by
/// `brain bench` to map a server result back to its judged doc index so we can
/// compute recall. Keep in sync with the eval fixture.
// ponytail: bench recall is only meaningful if this corpus has been ingested
// into the server (via `brain ingest-dir` or the eval harness). If the corpus
// is not present, recall will read 0 — that is an environment error, not a bug.
const DOCS: &[&str] = &[
    "Bignay is a tropical fruit and a good alternative to blueberry, rich in antioxidants.",
    "The Rust programming language guarantees memory safety without a garbage collector.",
    "Vitamin D3 supplementation improves immune function and bone density in deficient adults.",
    "The GDPR is a European regulation protecting the personal data of EU residents.",
    "Gut microbiome diversity affects inflammation markers and immune system regulation.",
    "SQLite is an embedded relational database with FTS5 full-text search support.",
    "ISO 9001 is the international standard for quality management systems.",
    "Ownership and borrowing are Rust's core concepts for compile-time memory safety.",
    "Antioxidants in tropical fruits like bignay help reduce oxidative stress.",
    "The GDPR covers any organization processing EU residents' data, with fines up to four percent of global revenue.",
];

fn base_url() -> String {
    std::env::var("BRAIN_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}

/// Resolve the bearer token for authenticated routes, mirroring the server's
/// `AUTH_TOKEN_FILE` → `AUTH_TOKEN` ladder (see `src/config.rs`).
///
/// 1. `BRAIN_TOKEN_FILE` — explicit path to a `0600`-mode secret file.
/// 2. `BRAIN_TOKEN` — raw env var (dev convenience).
/// 3. `~/.config/brain-server/auth-token` — default install path written by
///    `scripts/install-service.sh`. Zero-config for the common case.
///
/// Returns `None` if no token is resolvable — public routes still work, but
/// `/search`, `/stats`, `/recall`, `/ingest/*` will 401 against an
/// auth-enabled server. The CLI surfaces that error rather than silently
/// degrading.
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
    // Default install path: written by install-service.sh alongside the
    // launchd plist's AUTH_TOKEN_FILE. Same file, same value, no extra env.
    let default_path = dirs_home().join(".config/brain-server/auth-token");
    if let Ok(s) = std::fs::read_to_string(&default_path) {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

fn default_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("BRAIN_DB_PATH") {
        return PathBuf::from(p);
    }
    let home = dirs_home();
    home.join(".openclaw/workspace/brain.db")
}

fn dirs_home() -> PathBuf {
    // Minimal HOME discovery (mirrors dirs::home_dir without the dependency).
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        exit(0);
    }
    let cmd = args[0].as_str();
    let rest = &args[1..];

    let result = match cmd {
        "query" => cmd_query(rest),
        "explain" => cmd_explain(rest),
        "get" => cmd_get(rest),
        "ingest-dir" => cmd_ingest_dir(rest),
        "reconcile" => cmd_reconcile(rest),
        "resolve" => cmd_resolve(rest),
        "domain-move" => cmd_domain_move(rest),
        "domains-recompute" => cmd_domains_recompute(rest),
        "undo-resolve" => cmd_undo_resolve(rest),
        "check-consistency" => cmd_check_consistency(rest),
        "source-delete" => cmd_source_delete(rest),
        // v1.9.0 "Suggest": opt-in anticipation surface.
        "suggest" => cmd_suggest(rest),
        "suggest-feedback" => cmd_suggest_feedback(rest),
        "suggest-metrics" => cmd_suggest_metrics(rest),
        // v1.17.1 "Govern": per-kind retention policy + snapshot self-check.
        "retention" => cmd_retention(rest),
        "snapshot-status" => cmd_snapshot_status(rest),
        // v1.17.3 "UMP": the §4.3 file binding.
        "ump" => cmd_ump(rest),
        // v1.10.0 "Procedural": procedural memory + deterministic categorization.
        "procedure" => cmd_procedure(rest),
        "classify" => cmd_classify(rest),
        "evaluate" => cmd_evaluate(rest),
        "connect" => cmd_connect(rest),
        "sync" => cmd_sync(rest),
        "connector-status" => cmd_connector_status(rest),
        "backup" => cmd_backup(rest),
        "restore" => cmd_restore(rest),
        // v1.2.0 AuthN: JWT signing key management. Local-file operations;
        // no server roundtrip (the server picks up new keys via its own
        // KeyStore reload, currently on restart — hot-reload is a follow-up).
        "key" => cmd_key(rest),
        "bench" => cmd_bench(),
        "eval" => cmd_eval(rest),
        "status" => cmd_status(),
        "doctor" => cmd_doctor(rest),
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        "-V" | "--version" => {
            println!("brain {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => {
            eprintln!("error: unknown subcommand '{other}'");
            print_usage();
            exit(2);
        }
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        exit(1);
    }
}

fn print_usage() {
    // ponytail: raw string literal preserves the intended 2-space indentation.
    // The previous version used `\n\` line continuations, which strip leading
    // whitespace on the next line — so every subcommand rendered flush-left.
    // `r#"..."#` (raw with hash-delimiters) lets the embedded `"` survive too.
    println!(
        r#"brain — client for brain-server (default {DEFAULT_URL}; override with BRAIN_URL)

usage:
  brain query "<q>" [--k N] [--source S ...] [--phrase P ...]
                 [--exclude E ...] [--code C ...] [--since ISO]
                 [--intent I] [--profile P] [--graph] [--explain]
  brain explain "<q>" [--source S ...] [--since ISO]
  brain get <id>
  brain ingest-dir <path> [--dry-run] [--replace] [--source S] [--domain D]
  brain reconcile <path> [--kind vault] [--dry-run]
  brain resolve <new_id> <old_id>
  brain domain-move <id> [<id> ...] --to <domain> [--confirm global]
  brain domains-recompute
  brain undo-resolve <old_id> [<old_id> ...]
  brain check-consistency
  brain source-delete <id>
  brain suggest "<context>" [--exclude id[,id...]] [--k N] [--domain D] [--session S]
  brain suggest-feedback <chunk_id> accept|dismiss [--reason "..."] [--session S]
  brain suggest-metrics [--session S] [--since DATE]
  brain retention get
  brain retention set <kind> <days>
  brain snapshot-status
  brain eval [--floor r5=0.85 r10=0.9]
  brain procedure <title> [--step "title: content" ...] [--domain D]
  brain classify "<text>"
  brain evaluate <decision_id> --var name=value [--var name=value ...]
  brain connect github --app-id N --install-id N --key-file PATH \
                      --repo owner/repo [...] [--webhook-secret-file PATH]
  brain sync [github] [--config PATH]
  brain connector-status
  brain backup <out-path> [--passphrase-file PATH]
  brain restore <in-path> [--passphrase-file PATH]
  brain key generate [--kid ID] [--alg RS256] [--dir PATH]
  brain key list [--dir PATH]
  brain key prune [--dir PATH] [--keep N]
  brain bench
  brain status
  brain doctor [--backup <path> [--passphrase-file PATH]]

filters:
  --source S   OR filter over ingest kind (memory | markdown | structured |
               manual | vault); repeatable. Filters the `source` column, NOT
               source URIs. Sent as the `sources` list to /recall.

auth:
  Reads BRAIN_TOKEN_FILE, then BRAIN_TOKEN, then
  ~/.config/brain-server/auth-token (written by install-service.sh)."#
    );
}

// ── argument helpers ──────────────────────────────────────────────────────

/// Parse `--flag value` and `--flag=value` options from a slice, returning the
/// remaining positional arguments and a map of flag -> value (None if flag has
/// no `=` and is a boolean switch).
fn parse_flags(
    args: &[String],
) -> (
    Vec<String>,
    std::collections::HashMap<String, Option<String>>,
) {
    let mut positionals = Vec::new();
    let mut flags = std::collections::HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(rest) = a.strip_prefix("--") {
            if let Some((k, v)) = rest.split_once('=') {
                flags.insert(k.to_string(), Some(v.to_string()));
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                flags.insert(rest.to_string(), Some(args[i + 1].clone()));
                i += 1;
            } else {
                flags.insert(rest.to_string(), None);
            }
        } else {
            positionals.push(a.clone());
        }
        i += 1;
    }
    (positionals, flags)
}

fn require_positional(positionals: &[String], name: &str) -> Result<String, String> {
    positionals
        .first()
        .cloned()
        .ok_or_else(|| format!("missing required argument: {name}"))
}

fn json_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

// ── subcommands ────────────────────────────────────────────────────────────

/// Collect every value of a repeatable `--flag value` / `--flag=value` option
/// from raw args. Used for OR-scoped flags (`--source`, `--phrase`, …) where one
/// value per occurrence is appended to a list rather than overwriting.
fn multi_flag(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(rest) = a.strip_prefix("--") {
            if let Some((k, v)) = rest.split_once('=') {
                if k == name {
                    out.push(v.to_string());
                }
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") && rest == name {
                out.push(args[i + 1].clone());
                i += 1;
            }
        }
        i += 1;
    }
    out
}

/// Build a v0.9.5 structured `QueryDoc` body from the parsed CLI flags, lowering
/// the lexical controls into the `LexSpec` the server compiles (FTS5-quoted,
/// injection-safe). Returns the JSON body string.
fn build_query_doc(
    q: &str,
    flags: &std::collections::HashMap<String, Option<String>>,
    phrases: &[String],
    excludes: &[String],
    codes: &[String],
    sources: &[String],
    explain: bool,
) -> String {
    let k = flags.get("k").and_then(|o| o.as_ref());
    let since = flags.get("since").and_then(|o| o.clone());
    let intent = flags.get("intent").and_then(|o| o.clone());
    let profile = flags.get("profile").and_then(|o| o.clone());

    let mut body = serde_json::json!({ "query": q });
    if let Some(k) = k {
        body["limit"] = serde_json::json!(k.parse::<u32>().unwrap_or(5));
    }
    if !phrases.is_empty() || !excludes.is_empty() || !codes.is_empty() {
        let mut lex = serde_json::json!({});
        if !phrases.is_empty() {
            lex["phrases"] = serde_json::json!(phrases);
        }
        if !excludes.is_empty() {
            lex["exclude"] = serde_json::json!(excludes);
        }
        if !codes.is_empty() {
            lex["code"] = serde_json::json!(codes);
        }
        body["lex"] = lex;
    }
    if !sources.is_empty() {
        body["sources"] = serde_json::json!(sources);
    }
    if let Some(s) = flags.get("source").and_then(|o| o.clone()) {
        body["source"] = serde_json::json!(s);
    }
    if let Some(s) = since {
        body["since"] = serde_json::json!(s);
    }
    if let Some(s) = intent {
        body["intent"] = serde_json::json!(s);
    }
    if let Some(s) = profile {
        body["profile"] = serde_json::json!(s);
    }
    if let Some(v) = flags.get("graph") {
        // Bare `--graph` (None) or `--graph=true` enable the leg; only an
        // explicit `--graph=false` opts out.
        let off = v.as_deref().map(str::to_ascii_lowercase) == Some("false".to_string());
        body["graph"] = serde_json::json!(!off);
    }
    if explain {
        // `explain` maps to the unified prove nance/telemetry envelope on /recall.
        body["provenance"] = serde_json::json!(true);
    }
    body.to_string()
}

fn cmd_query(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    let q = require_positional(&positionals, "query")?;
    let phrases = multi_flag(args, "phrase");
    let excludes = multi_flag(args, "exclude");
    let codes = multi_flag(args, "code");
    let sources = multi_flag(args, "source");
    let explain = flags.contains_key("explain");

    let body = build_query_doc(&q, &flags, &phrases, &excludes, &codes, &sources, explain);

    let resp = post(
        &base_url(),
        "/recall",
        &[],
        "application/json",
        &body,
        auth_token().as_deref(),
    )?;
    let value: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("server returned non-JSON (status {}): {e}", resp.status))?;

    if let Some(err) = value.get("error") {
        return Err(format!("server error: {err}"));
    }

    let hits = value
        .get("hits")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    println!("recall: \"{q}\"  ({} hits)", hits.len());
    if explain && value.get("telemetry").is_some() {
        print_telemetry(&value["telemetry"]);
    }
    print_hits(&hits, explain);
    Ok(())
}

fn cmd_explain(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    let q = require_positional(&positionals, "query")?;
    let sources = multi_flag(args, "source");
    let since = flags.get("since").and_then(|o| o.clone());

    let mut body = serde_json::json!({ "query": q, "provenance": true });
    if !sources.is_empty() {
        body["sources"] = serde_json::json!(sources);
    }
    if let Some(s) = since {
        body["since"] = serde_json::json!(s);
    }

    let resp = post(
        &base_url(),
        "/recall",
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    let value: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("server returned non-JSON (status {}): {e}", resp.status))?;

    if let Some(err) = value.get("error") {
        return Err(format!("server error: {err}"));
    }
    let hits = value
        .get("hits")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    println!("explain: \"{q}\"  ({} hits)", hits.len());
    print_telemetry(value.get("telemetry").unwrap_or(&serde_json::Value::Null));
    print_hits(&hits, true);
    Ok(())
}

/// Print the unified `/recall` telemetry block (the M3 envelope that folds the
/// old `/search` `query_plan` into one shape). Mirrors the real `SearchTelemetry`
/// struct in `src/search/mod.rs` — only fields the server actually emits are
/// listed, so nothing printed is fabricated.
fn print_telemetry(tel: &serde_json::Value) {
    if tel.is_null() {
        return;
    }
    println!("  telemetry:");
    for key in [
        "embedding_query",
        "intent",
        "fused_count",
        "rrf_k",
        "vec_candidates",
        "fts_candidates",
        "graph_candidates",
        "graph_ms",
        "graph_rescued",
        "confidence",
        "recommendation",
    ] {
        if let Some(v) = tel.get(key) {
            println!("    {key}: {v}");
        }
    }
}

/// Pretty-print a `/recall` hit list. `with_provenance` toggles the per-hit
/// retrieval provenance (vector/fts ranks, fused score, PRF decision).
fn print_hits(hits: &[serde_json::Value], with_provenance: bool) {
    if hits.is_empty() {
        println!("  (no results)");
        return;
    }
    for (rank, h) in hits.iter().enumerate() {
        let id = h.get("id").and_then(|x| x.as_i64()).unwrap_or(-1);
        let score = h.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0);
        // v1.20.24 "Sweep": recalled text is agent-facing — strip the same
        // invisible-Unicode class the server screen + client strip.
        let title = brain_server::strip_invisible::strip_invisible(
            &json_str(h, "title").unwrap_or_else(|| "(untitled)".into()),
        );
        let source = h
            .get("source")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let snippet = h
            .get("snippet")
            .and_then(|x| x.as_str())
            .map(|s| s.replace('\n', " "))
            .map(|s| brain_server::strip_invisible::strip_invisible(&s))
            .unwrap_or_default();

        println!(
            "{:>3}. [{:.4}] id={} source={}",
            rank + 1,
            score,
            id,
            source
        );
        println!("     title: {title}");
        if !snippet.is_empty() {
            println!("     {snippet}");
        }
        if with_provenance {
            if let Some(p) = h.get("provenance") {
                let vr = p.get("vector_rank").and_then(|x| x.as_u64());
                let fr = p.get("fts_rank").and_then(|x| x.as_u64());
                let fs = p.get("fused_score").and_then(|x| x.as_f64());
                let rs = p.get("rerank_score").and_then(|x| x.as_f64());
                let prf = p
                    .get("prf_expanded")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                println!(
                    "     provenance: vector_rank={:?} fts_rank={:?} fused={:?} rerank={:?} prf_expanded={}",
                    vr, fr, fs, rs, prf
                );
            } else {
                println!("     provenance: (none returned by server)");
            }
        }
    }
}

fn cmd_get(args: &[String]) -> Result<(), String> {
    let (positionals, _flags) = parse_flags(args);
    let id_str = require_positional(&positionals, "id")?;
    let id: i64 = id_str
        .parse()
        .map_err(|_| format!("id must be an integer, got '{id_str}'"))?;

    let path = format!("/get/{id}");
    let resp = get(&base_url(), &path, &[], auth_token().as_deref())?;
    if resp.status == 404 {
        return Err(format!("no chunk with id {id}"));
    }
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("non-JSON response (status {}): {e}", resp.status))?;

    let title = brain_server::strip_invisible::strip_invisible(
        &json_str(&v, "title").unwrap_or_else(|| "(untitled)".into()),
    );
    let source = json_str(&v, "source").unwrap_or_default();
    let heading = json_str(&v, "heading_path").unwrap_or_default();
    let line_start = v.get("line_start").and_then(|x| x.as_i64());
    let line_end = v.get("line_end").and_then(|x| x.as_i64());
    let source_uri = json_str(&v, "source_uri").unwrap_or_default();
    let revision_id = v.get("revision_id").and_then(|x| x.as_i64());

    println!("chunk {id}");
    println!("  title      : {title}");
    if !source.is_empty() {
        println!("  source     : {source}");
    }
    if !heading.is_empty() {
        println!("  heading    : {heading}");
    }
    if let (Some(a), Some(b)) = (line_start, line_end) {
        println!("  lines      : {a}..{b}");
    }
    if !source_uri.is_empty() {
        println!("  source_uri : {source_uri}");
    }
    if let Some(r) = revision_id {
        println!("  revision   : {r}");
    }
    println!("  {:-<60}", "");
    // v1.20.24 "Sweep": the CLI is an agent-facing surface — strip the same
    // invisible-Unicode class the server screen + client strip.
    let content = brain_server::strip_invisible::strip_invisible(
        &json_str(&v, "content").unwrap_or_default(),
    );
    println!("{content}");
    Ok(())
}

fn cmd_ingest_dir(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    let path = require_positional(&positionals, "path")?;
    let dry_run = flags.contains_key("dry-run");
    let replace = flags.contains_key("replace") || flags.contains_key("r");
    let source = flags.get("source").and_then(|o| o.clone());
    let domain = flags.get("domain").and_then(|o| o.clone());

    let root = Path::new(&path);
    if !root.is_dir() {
        return Err(format!("not a directory: {path}"));
    }

    let ignore = load_brainignore(root);
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(root, root, &ignore, &mut files)?;
    files.sort();

    // v0.9.2: bound the walk so a pathological vault can't blow the ingest
    // budget. Counts/bytes are total file content, not just markdown.
    if files.len() > MAX_INGEST_FILES {
        return Err(format!(
            "too many files: {} (max {MAX_INGEST_FILES}). Narrow the path or use .brainignore.",
            files.len()
        ));
    }
    let total_bytes: u64 = files
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok().map(|m| m.len()))
        .sum();
    if total_bytes > MAX_INGEST_BYTES {
        return Err(format!(
            "vault too large: {} bytes (max {MAX_INGEST_BYTES}). Narrow the path or use .brainignore.",
            total_bytes
        ));
    }

    if files.is_empty() {
        println!("no ingestable text/markdown files found in {path}");
        return Ok(());
    }

    let mut ingested = 0;
    let mut skipped = 0;
    for f in &files {
        let rel = f.strip_prefix(root).unwrap_or(f);
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_markdown = matches!(ext.as_str(), "md" | "markdown");
        let content = match std::fs::read_to_string(f) {
            Ok(c) => c,
            Err(e) => {
                println!("  skip {}: {e}", rel.display());
                skipped += 1;
                continue;
            }
        };
        if content.trim().is_empty() {
            skipped += 1;
            continue;
        }

        if dry_run {
            let target = if is_markdown {
                "/ingest/markdown"
            } else {
                "/ingest/memory"
            };
            let meta = source
                .as_ref()
                .map(|s| format!(", source={s}"))
                .unwrap_or_default();
            let meta = domain
                .as_ref()
                .map(|d| format!("{meta}, domain={d}"))
                .unwrap_or(meta);
            println!(
                "  [dry-run] {} -> {} ({} bytes{meta})",
                rel.display(),
                target,
                content.len(),
            );
            continue;
        }

        let outcome = if is_markdown {
            let title = derive_title(f);
            // v0.9.2: send the absolute path as source_path so the server can
            // dedup/replace per-file and surface provenance.
            let abs = std::fs::canonicalize(f)
                .unwrap_or_else(|_| f.to_path_buf())
                .to_string_lossy()
                .to_string();
            let mut body = serde_json::json!({
                "content": content,
                "title": title,
                "source_path": abs,
            });
            if let Some(d) = &domain {
                body["domain"] = serde_json::json!(d);
            }
            if replace {
                body["replace"] = serde_json::json!(true);
            }
            let body = body.to_string();
            post(
                &base_url(),
                "/ingest/markdown",
                &[],
                "application/json",
                &body,
                auth_token().as_deref(),
            )
            .map(|r| (r.status, r.body))
        } else {
            let q = source
                .as_ref()
                .map(|s| vec![("source".to_string(), s.clone())])
                .unwrap_or_default();
            post(
                &base_url(),
                "/ingest/memory",
                &q,
                "text/plain",
                &content,
                auth_token().as_deref(),
            )
            .map(|r| (r.status, r.body))
        };

        match outcome {
            Ok((status, body)) => {
                let ok = status == 200 && !body.contains("\"success\":false");
                if ok {
                    ingested += 1;
                    println!("  ok   {} -> {}", rel.display(), summarize_ingest(&body));
                } else {
                    skipped += 1;
                    println!(
                        "  fail {} ({}): {}",
                        rel.display(),
                        status,
                        truncate(&body, 120)
                    );
                }
            }
            Err(e) => {
                skipped += 1;
                println!("  error {}: {e}", rel.display());
            }
        }
    }

    println!("\ningest-dir complete: {ingested} ingested, {skipped} skipped");
    Ok(())
}

/// `brain reconcile <path>`: walks the live filesystem, computes the canonical
/// absolute-path URIs the server indexes (the same form `ingest-dir` sends),
/// and POSTs them to `/sources/reconcile`. The server retires any active
/// `--kind` source whose URI is no longer on disk — how a vault delete or
/// rename is reflected after a re-ingest cycle.
///
/// `--dry-run`: print the orphan URIs the server WOULD retire without actually
///   issuing the reconcile request.
///
/// ponytail: this is a one-shot walk, not a watch. A real vault workflow should
/// invoke `brain reconcile <vault>` after `brain ingest-dir <vault>` on every
/// sync. Live incremental reconcile needs the streaming-sync tier (P3+).
fn cmd_reconcile(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    let path = require_positional(&positionals, "path")?;
    let dry_run = flags.contains_key("dry-run");
    let kind = flags
        .get("kind")
        .and_then(|o| o.as_deref())
        .unwrap_or("vault")
        .to_string();

    let root = Path::new(&path);
    if !root.is_dir() {
        return Err(format!("not a directory: {path}"));
    }

    // Reuse ingest-dir's walker + ignore file so the live URI set matches the
    // set ingest-dir would have sent. A URI computed differently here would
    // never match a stored source_path, making reconcile a no-op at best.
    let ignore = load_brainignore(root);
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(root, root, &ignore, &mut files)?;
    files.sort();

    if files.len() > MAX_INGEST_FILES {
        return Err(format!(
            "too many files: {} (max {MAX_INGEST_FILES}). Narrow the path or use .brainignore.",
            files.len()
        ));
    }

    // Canonicalize each file path the same way cmd_ingest_dir does so the URIs
    // match what's stored in `sources.uri`. Non-canonicalizable files fall back
    // to the literal path — matches the ingest-dir fallback.
    let live_uris: Vec<String> = files
        .iter()
        .map(|f| {
            std::fs::canonicalize(f)
                .unwrap_or_else(|_| f.to_path_buf())
                .to_string_lossy()
                .to_string()
        })
        .collect();

    println!(
        "reconcile {path}: {count} live files (kind={kind}){dry}",
        count = live_uris.len(),
        dry = if dry_run { " [dry-run]" } else { "" }
    );

    if dry_run {
        // No server roundtrip — just show what we'd send.
        for uri in &live_uris {
            println!("  live  {uri}");
        }
        println!(
            "\n[dry-run] would POST {n} URIs to /sources/reconcile",
            n = live_uris.len()
        );
        return Ok(());
    }

    let body = serde_json::json!({
        "kind": kind,
        "live_uris": live_uris,
    })
    .to_string();
    let resp = post(
        &base_url(),
        "/sources/reconcile",
        &[],
        "application/json",
        &body,
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("non-JSON response (status {}): {e}", resp.status))?;
    let deleted_sources = v
        .get("deleted_sources")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let deleted_chunks = v
        .get("deleted_chunks")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let orphan_uris = v
        .get("orphan_uris")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    println!(
        "\nreconcile complete: {deleted_sources} source(s) retired, {deleted_chunks} chunk(s) swept"
    );
    for uri in &orphan_uris {
        println!("  orphan {uri}");
    }
    Ok(())
}

/// `brain resolve <new_id> <old_id>`: mark `new_id` as superseding `old_id`.
/// v1.6.0 "Reconcile" — operator-facing shortcut for the most common
/// consolidation case. POSTs one `{from:new, to:old, kind:"supersedes"}` link
/// to `/consolidate/apply`; the server expires `old_id` (sets `valid_to=now`)
/// atomically. The old chunk stays retrievable via `/recall?at=<past>`.
fn cmd_resolve(args: &[String]) -> Result<(), String> {
    let (positionals, _flags) = parse_flags(args);
    if positionals.len() < 2 {
        return Err("usage: brain resolve <new_id> <old_id>".into());
    }
    let new_id: i64 = positionals[0]
        .parse()
        .map_err(|_| format!("new_id must be an integer, got '{}'", positionals[0]))?;
    let old_id: i64 = positionals[1]
        .parse()
        .map_err(|_| format!("old_id must be an integer, got '{}'", positionals[1]))?;
    if new_id == old_id {
        return Err("new_id and old_id must differ".into());
    }
    let body = serde_json::json!({
        "links": [{ "from_chunk": new_id, "to_chunk": old_id, "kind": "supersedes" }]
    });
    let resp = post(
        &base_url(),
        "/consolidate/apply",
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let recorded = v.get("recorded").and_then(|n| n.as_u64()).unwrap_or(0);
    let rejected = v.get("rejected").and_then(|a| a.as_array());
    if recorded == 0 {
        return Err(format!(
            "resolution recorded no link (rejected: {:?})",
            rejected
        ));
    }
    println!("resolved: chunk {new_id} supersedes chunk {old_id} (old chunk expired; ");
    println!("  still retrievable via /recall?at=<before-now>)");
    Ok(())
}

/// `brain domain-move <id> [<id> ...] --to <domain> [--confirm global]`:
/// v1.13.0 M3 — bulk-relabel chunks across domains via `POST /domains/move`.
/// This is the non-re-ingest fix for the 99%-in-`global` corpus: relabels
/// `knowledge.domain`, recomputes the affected centroids, and leaves the
/// content (and its embedding) untouched. Moving rows OUT of `global` needs
/// `--confirm global` (typo-replay guard, mirror of `DELETE /domains/{name}`).
fn cmd_domain_move(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    if positionals.is_empty() {
        return Err(
            "usage: brain domain-move <id> [<id> ...] --to <domain> [--confirm global]".into(),
        );
    }
    let to = flags
        .get("to")
        .and_then(|v| v.clone())
        .ok_or_else(|| "missing required flag: --to <domain>".to_string())?;
    let mut ids: Vec<i64> = Vec::new();
    for p in &positionals {
        ids.push(
            p.parse()
                .map_err(|_| format!("id must be an integer, got '{p}'"))?,
        );
    }
    let body = serde_json::json!({ "ids": ids, "to": to });
    let mut query: Vec<(String, String)> = Vec::new();
    if let Some(confirm) = flags.get("confirm").and_then(|v| v.clone()) {
        query.push(("confirm".to_string(), confirm));
    }
    let resp = post(
        &base_url(),
        "/domains/move",
        &query,
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let moved = v.get("moved").and_then(|n| n.as_u64()).unwrap_or(0);
    let from_domains: Vec<String> = v
        .get("from_domains")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    println!("moved {moved} chunk(s) {} -> {to}", from_domains.join(","));
    if from_domains.iter().any(|d| d == "global") {
        println!("  note: these were in 'global'; still retrievable via the global domain's historical paths");
    }
    Ok(())
}

/// `brain domains-recompute`: v1.13.0 M4 — one-shot recompute of every known
/// domain's centroid via `POST /domains/recompute`. Run once right after
/// deploying v1.13.0 (before any auto-routed ingest accumulates) so M2's
/// auto-route sees real centroids, and again after `domain-move` passes.
fn cmd_domains_recompute(_args: &[String]) -> Result<(), String> {
    let resp = post(
        &base_url(),
        "/domains/recompute",
        &[],
        "application/json",
        "{}",
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let rows = v.get("recomputed").and_then(|a| a.as_array()).unwrap();
    if rows.is_empty() {
        println!("no domains to recompute");
        return Ok(());
    }
    println!("domain : chunks");
    for r in rows {
        let d = r.get(0).and_then(|x| x.as_str()).unwrap_or("?");
        let c = r.get(1).and_then(|x| x.as_u64()).unwrap_or(0);
        println!("  {d:<20} {c}");
    }
    Ok(())
}

/// `brain undo-resolve <old_id> [<old_id> ...]`: v1.8.0 — reverse prior
/// supersession resolutions. The roadmap exit criterion's undo arm: "reject
/// or undo them without retrieval regression." For each `old_id`, clears
/// `valid_to` back to NULL + removes the `supersedes` link, restoring the
/// chunk to current recall.
fn cmd_undo_resolve(args: &[String]) -> Result<(), String> {
    let (positionals, _flags) = parse_flags(args);
    if positionals.is_empty() {
        return Err("usage: brain undo-resolve <old_id> [<old_id> ...]".into());
    }
    let mut chunks: Vec<i64> = Vec::new();
    for p in &positionals {
        chunks.push(
            p.parse()
                .map_err(|_| format!("id must be an integer, got '{p}'"))?,
        );
    }
    let body = serde_json::json!({ "old_chunks": chunks });
    let resp = post(
        &base_url(),
        "/consolidate/undo",
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let undone = v.get("undone").and_then(|n| n.as_u64()).unwrap_or(0);
    let rejected = v.get("rejected").and_then(|a| a.as_array());
    println!(
        "undone: {undone} state change(s) across {} chunk(s)",
        chunks.len()
    );
    println!("  chunks restored to current recall: {chunks:?}");
    if let Some(r) = rejected {
        if !r.is_empty() {
            println!("  rejected: {r:?}");
        }
    }
    Ok(())
}

/// `brain check-consistency`: v1.6.0 M5 — surface unresolved contradictions.
/// Calls `/consolidate/propose` and reports the `unresolved_contradictions`
/// list (contradicts links with no paired supersedes). Never auto-fixes;
/// operator uses `brain resolve <new> <old>` to act on each.
fn cmd_check_consistency(_args: &[String]) -> Result<(), String> {
    let resp = post(
        &base_url(),
        "/consolidate/propose",
        &[],
        "application/json",
        "",
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let dups = v["exact_duplicates"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let conflicts = v["conflicts"].as_array().map(|a| a.len()).unwrap_or(0);
    let unresolved = v["unresolved_contradictions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    println!("brain-server consistency report:");
    println!("  exact_duplicate_groups : {dups}");
    println!("  subject_conflicts       : {conflicts}");
    println!("  unresolved_contradictions: {}", unresolved.len());
    // v1.8.0: stale sources + near-duplicates.
    let stale = v["stale_sources"].as_array().cloned().unwrap_or_default();
    let near = v["near_duplicates"].as_array().cloned().unwrap_or_default();
    println!("  stale_sources           : {}", stale.len());
    println!("  near_duplicates         : {}", near.len());
    if unresolved.is_empty() {
        println!("  ✓ no unresolved contradictions");
    } else {
        println!("  action items (resolve with `brain resolve <new> <old>`):");
        for pair in unresolved {
            let from = pair[0].as_i64().unwrap_or(0);
            let to = pair[1].as_i64().unwrap_or(0);
            println!("    contradicts: chunk {from} <-> chunk {to}");
        }
    }
    if !stale.is_empty() {
        println!("  stale sources (re-ingest or `brain source-delete <id>`):");
        for s in &stale {
            let id = s["source_id"].as_i64().unwrap_or(0);
            let chunks = s["chunk_count"].as_i64().unwrap_or(0);
            let uri = s["uri"].as_str().unwrap_or("?");
            println!("    source {id} ({chunks} chunks): {uri}");
        }
    }
    if !near.is_empty() {
        println!("  near-duplicates (resolve with `brain resolve <new> <old>`):");
        for n in &near {
            let a = n["chunk_a"].as_i64().unwrap_or(0);
            let b = n["chunk_b"].as_i64().unwrap_or(0);
            let sim = n["similarity"].as_f64().unwrap_or(0.0);
            println!("    chunks {a} ≈ {b} (cosine {sim:.3})");
        }
    }
    Ok(())
}

/// `brain source-delete <id>`: retires a single source by id via
/// `DELETE /sources/{id}`. Sweeps that source's chunks from retrieval and
/// tombstones the source + active revision. The companion to `brain reconcile`
/// for one-off deletes (a vault file you want forgotten without rescanning
/// the whole directory).
fn cmd_source_delete(args: &[String]) -> Result<(), String> {
    let (positionals, _flags) = parse_flags(args);
    let id_str = require_positional(&positionals, "id")?;
    let id: i64 = id_str
        .parse()
        .map_err(|_| format!("id must be an integer, got '{id_str}'"))?;

    let path = format!("/sources/{id}");
    let resp = delete(&base_url(), &path, &[], auth_token().as_deref())?;
    if resp.status == 404 {
        return Err(format!("no source with id {id}"));
    }
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    println!("deleted source {id}");
    Ok(())
}

// ── v1.9.0 "Suggest": opt-in anticipation CLI ────────────────────────────────

/// `brain suggest "<context>"`: opt-in pull for related-but-not-surfaced
/// chunks. The caller explicitly asks "what else might be relevant?" — the
/// server never pushes. Each hit is tagged `reason: "anticipated"` so the
/// consuming agent may ignore it.
fn cmd_suggest(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    let context = require_positional(&positionals, "context")?;
    if context.trim().is_empty() {
        return Err("context must be non-empty".into());
    }
    // --exclude accepts comma-separated (--exclude 1,2,3) like /search's sources.
    let exclude: Vec<i64> = flags
        .get("exclude")
        .and_then(|o| o.clone())
        .map(|s| {
            s.split(',')
                .filter(|p| !p.trim().is_empty())
                .map(|p| {
                    p.trim()
                        .parse::<i64>()
                        .map_err(|_| format!("exclude id must be an integer, got '{p}'"))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let k: u32 = flags
        .get("k")
        .and_then(|o| o.clone())
        .map(|s| {
            s.parse::<u32>()
                .map_err(|_| format!("--k must be an integer, got '{s}'"))
        })
        .transpose()?
        .unwrap_or(5);
    let domain = flags.get("domain").and_then(|o| o.clone());
    let session = flags.get("session").and_then(|o| o.clone());

    let mut body = serde_json::json!({ "context": context, "k": k });
    if !exclude.is_empty() {
        body["exclude"] = serde_json::json!(exclude);
    }
    if let Some(d) = domain {
        body["domain"] = serde_json::json!(d);
    }
    if let Some(s) = session {
        body["session"] = serde_json::json!(s);
    }
    let resp = post(
        &base_url(),
        "/suggest",
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    if resp.status == 501 {
        return Err("/suggest is disabled on the server (BRAIN_SUGGEST_ENABLED=false)".into());
    }
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let hits = v["suggestions"].as_array().cloned().unwrap_or_default();
    if hits.is_empty() {
        println!(
            "no suggestions (corpus may be small or everything relevant was already surfaced)"
        );
        return Ok(());
    }
    println!("suggestions (anticipated; the agent may ignore these):");
    for h in &hits {
        let id = h["id"].as_i64().unwrap_or(0);
        let score = h["score"].as_f64().unwrap_or(0.0);
        let title = h["title"].as_str().unwrap_or("");
        let content = h["content"].as_str().unwrap_or("");
        println!("  [{id}] score={score:.3} {title}");
        println!("        {}", truncate(content, 120));
    }
    let tel = &v["telemetry"];
    println!(
        "  telemetry: k={} excluded={} retrieved={}",
        tel["k"].as_u64().unwrap_or(0),
        tel["excluded"].as_u64().unwrap_or(0),
        tel["retrieved"].as_u64().unwrap_or(0),
    );
    println!("  record outcomes with: brain suggest-feedback <id> accept|dismiss");
    Ok(())
}

/// `brain suggest-feedback <chunk_id> accept|dismiss`: record the outcome for
/// a surfaced suggestion. Accepts optional `--reason` (hashed server-side,
/// never stored raw) and `--session`. The aggregate drives the false-positive
/// metric surfaced by `brain suggest-metrics`.
fn cmd_suggest_feedback(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    if positionals.len() < 2 {
        return Err("usage: brain suggest-feedback <chunk_id> accept|dismiss [--reason \"...\"] [--session S]".into());
    }
    let chunk_id: i64 = positionals[0]
        .parse()
        .map_err(|_| format!("chunk_id must be an integer, got '{}'", positionals[0]))?;
    let outcome = positionals[1].trim().to_lowercase();
    if outcome != "accept" && outcome != "dismiss" {
        return Err(format!(
            "feedback must be 'accept' or 'dismiss', got '{outcome}'"
        ));
    }
    let reason = flags.get("reason").and_then(|o| o.clone());
    let session = flags.get("session").and_then(|o| o.clone());
    let mut body = serde_json::json!({ "chunk_id": chunk_id, "feedback": outcome });
    if let Some(r) = reason {
        body["reason"] = serde_json::json!(r);
    }
    if let Some(s) = session {
        body["session"] = serde_json::json!(s);
    }
    let resp = post(
        &base_url(),
        "/suggest/feedback",
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    if resp.status == 501 {
        return Err("/suggest is disabled on the server (BRAIN_SUGGEST_ENABLED=false)".into());
    }
    if resp.status == 404 {
        return Err(format!("no chunk with id {chunk_id}"));
    }
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    println!("recorded: chunk {chunk_id} -> {outcome}");
    Ok(())
}

/// `brain suggest-metrics`: the false-positive rate over the feedback ledger.
/// This is the v1.9 roadmap exit criterion, made queryable. Optional
/// `--session` / `--since` filter the window.
fn cmd_suggest_metrics(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args);
    let mut query: Vec<(String, String)> = Vec::new();
    if let Some(s) = flags.get("session").and_then(|o| o.clone()) {
        query.push(("session".into(), s));
    }
    if let Some(s) = flags.get("since").and_then(|o| o.clone()) {
        query.push(("since".into(), s));
    }
    let resp = get(
        &base_url(),
        "/suggest/metrics",
        &query,
        auth_token().as_deref(),
    )?;
    if resp.status == 501 {
        return Err("/suggest is disabled on the server (BRAIN_SUGGEST_ENABLED=false)".into());
    }
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let total = v["total"].as_u64().unwrap_or(0);
    let accepts = v["accepts"].as_u64().unwrap_or(0);
    let dismisses = v["dismisses"].as_u64().unwrap_or(0);
    let accept_rate = v["accept_rate"].as_f64().unwrap_or(0.0);
    let fpr = v["false_positive_rate"].as_f64().unwrap_or(0.0);
    println!("brain-server suggest metrics:");
    println!("  total               : {total}");
    println!("  accepts             : {accepts}");
    println!("  dismisses           : {dismisses}");
    println!("  accept_rate         : {accept_rate:.3}");
    println!("  false_positive_rate : {fpr:.3}  (roadmap exit criterion)");
    if let Some(w) = v.get("window").and_then(|w| w.as_object()) {
        let since = w.get("since").and_then(|s| s.as_str()).unwrap_or("-");
        let session = w.get("session").and_then(|s| s.as_str()).unwrap_or("-");
        println!("  window              : since={since} session={session}");
    }
    Ok(())
}

/// `brain retention get` — print the effective per-kind retention policy.
/// `brain retention set <kind> <days>` — override one kind (Admin + audited).
/// The override persists across restarts; retention applies at query time,
/// never by a background sweeper.
fn cmd_retention(args: &[String]) -> Result<(), String> {
    if args.first().map(|s| s.as_str()) == Some("get") {
        let resp = get(&base_url(), "/retention", &[], auth_token().as_deref())?;
        if resp.status != 200 {
            return Err(format!(
                "server returned status {}: {}",
                resp.status,
                truncate(&resp.body, 200)
            ));
        }
        let v: serde_json::Value =
            serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
        println!(
            "per-kind retention policy (BRAIN_RETENTION_ENABLED={})",
            v["enabled"]
        );
        if let Some(policy) = v["policy"].as_object() {
            for (kind, days) in policy {
                println!("  {kind:<12} : {days} days");
            }
        }
        if let Some(counts) = v["counts"].as_object() {
            println!("current chunks per kind:");
            for (kind, n) in counts {
                println!("  {kind:<12} : {n}");
            }
        }
        return Ok(());
    }
    if args.first().map(|s| s.as_str()) == Some("set") {
        let kind = args
            .get(1)
            .ok_or("usage: brain retention set <kind> <days>")?;
        let days = args
            .get(2)
            .ok_or("usage: brain retention set <kind> <days>")?
            .parse::<i64>()
            .map_err(|e| format!("days must be an integer: {e}"))?;
        let body = serde_json::json!({ "kind": kind, "days": days }).to_string();
        let resp = post(
            &base_url(),
            "/retention",
            &[],
            "application/json",
            &body,
            auth_token().as_deref(),
        )?;
        if resp.status != 200 {
            return Err(format!(
                "server returned status {}: {}",
                resp.status,
                truncate(&resp.body, 200)
            ));
        }
        let v: serde_json::Value =
            serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
        let updated = v["updated"].as_u64().unwrap_or(0);
        println!("retention policy updated: {updated} row(s) for {kind} -> {days} days");
        return Ok(());
    }
    Err("usage: brain retention get | set <kind> <days>".into())
}

/// `brain snapshot-status` — run the snapshot self-check panel and exit
/// `brain ump export|import` — the v1.17.3 UMP §4.3 file binding. Export pulls
/// the full-record projection from `GET /export` (markdown records joined by
/// `\n---\n`, or the JSON envelope with `--format ump`); import pushes a file
/// back through `POST /ingest` (format detected by extension: `.md` →
/// `?format=ump-md`, else the UMP JSON envelope).
fn cmd_ump(args: &[String]) -> Result<(), String> {
    match args.first().map(|s| s.as_str()) {
        Some("export") => cmd_ump_export(&args[1..]),
        Some("import") => cmd_ump_import(&args[1..]),
        Some("keygen") => cmd_ump_keygen(&args[1..]).map(|_| ()),
        _ => Err(
            "usage: brain ump export [--format md|ump] [--out FILE] | import <file> | keygen [--dir PATH]"
                .into(),
        ),
    }
}

/// `brain ump keygen` — generate an Ed25519 operator key for the UMP
/// identity surface (§5). Writes a raw 32-byte seed to
/// `<dir>/operator.key` (0600) and prints the `did:key`. The server reads
/// any seed file in `BRAIN_UMP_KEY_DIR` (default `~/.config/brain-server/ump/`).
fn cmd_ump_keygen(args: &[String]) -> Result<String, String> {
    let (_positionals, flags) = parse_flags(args);
    let dir = flags
        .get("dir")
        .and_then(|o| o.clone())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("BRAIN_UMP_KEY_DIR")
                .ok()
                .filter(|d| !d.trim().is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| dirs_home().join(".config/brain-server/ump"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create key dir {dir:?}: {e}"))?;
    set_mode_0700(&dir)?;
    let path = dir.join("operator.key");
    if path.exists() {
        return Err(format!(
            "refusing to overwrite existing operator key {path:?}; delete it first to rotate"
        ));
    }
    use rand::RngCore;
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes();
    let did = brain_server::ump_integrity::did_key_from_ed25519(&pk);
    std::fs::write(&path, seed).map_err(|e| format!("write {path:?}: {e}"))?;
    set_mode_0600(&path)?;
    println!("wrote UMP operator key {path:?}");
    println!("did: {did}");
    Ok(did)
}

fn cmd_ump_export(args: &[String]) -> Result<(), String> {
    let mut format = "md";
    let mut out = "records.ump.md".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                format = args.get(i + 1).ok_or("--format needs a value")?;
                i += 1;
            }
            "--out" => {
                out = args.get(i + 1).ok_or("--out needs a value")?.clone();
                i += 1;
            }
            other => return Err(format!("unknown flag: {other}")),
        }
        i += 1;
    }
    let q = if format == "md" {
        vec![("format".to_string(), "ump-md".to_string())]
    } else if format == "ump" {
        vec![("format".to_string(), "ump".to_string())]
    } else {
        return Err("format must be 'md' or 'ump'".into());
    };
    let resp = get(&base_url(), "/export", &q, auth_token().as_deref())?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let n = v["records"]
        .as_u64()
        .or_else(|| v["records"].as_array().map(|a| a.len() as u64))
        .unwrap_or(0);
    let content = if format == "md" {
        v["content"]
            .as_str()
            .ok_or("response missing 'content'")?
            .to_string()
    } else {
        v.to_string()
    };
    std::fs::write(&out, content).map_err(|e| format!("write {out}: {e}"))?;
    println!("wrote {out} ({n} records)");
    Ok(())
}

fn cmd_ump_import(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("usage: brain ump import <file>")?;
    let content = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let (q, ct, body) = if path.ends_with(".md") {
        (
            vec![("format".to_string(), "ump-md".to_string())],
            "text/plain",
            content,
        )
    } else {
        let v: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| format!("not a UMP JSON file: {e}"))?;
        (
            vec![("format".to_string(), "ump".to_string())],
            "application/json",
            v.to_string(),
        )
    };
    let resp = post(
        &base_url(),
        "/ingest",
        &q,
        ct,
        &body,
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    println!("{}", resp.body);
    Ok(())
}

/// `brain snapshot-status` — run the snapshot self-check panel and exit
/// non-zero if ANY snapshot is missing, not-0600, failing integrity_check, or
/// failing audit-chain verification. Wraps `GET /snapshot/status`.
fn cmd_snapshot_status(_args: &[String]) -> Result<(), String> {
    let resp = get(
        &base_url(),
        "/snapshot/status",
        &[],
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    println!("snapshot self-check for {}", v["db"]);
    let mut all_ok = true;
    if let Some(snaps) = v["snapshots"].as_array() {
        if snaps.is_empty() {
            println!("  (no VACUUM INTO snapshots found)");
        }
        for s in snaps {
            let name = s["file"].as_str().unwrap_or("?");
            let ok = s["ok"].as_bool().unwrap_or(false);
            all_ok &= ok;
            println!(
                "  {name}: exists={} size={} mode0600={} integrity={} chain={} {}",
                s["exists"],
                s["size_bytes"],
                s["mode_0600"],
                s["integrity_check"],
                s["audit_chain_ok"],
                if ok { "OK" } else { "FAIL" }
            );
        }
    }
    if !all_ok {
        return Err("one or more snapshots failed the self-check".into());
    }
    Ok(())
}

// ── v1.10.0 "Procedural": procedural-memory + categorization CLI ──────────

/// `brain procedure <title> [--step "title: content" ...] [--domain D]`:
/// ingest a procedure with ordered steps. Each `--step` is `"title: content"`
/// (colon-separated); step order = flag order. The procedure root + steps are
/// stored as `procedure`/`step`-kind chunks linked by `next_step` edges.
fn cmd_procedure(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    let title = require_positional(&positionals, "title")?;
    if title.trim().is_empty() {
        return Err("title must be non-empty".into());
    }
    let raw_steps = multi_flag(args, "step");
    let mut steps: Vec<serde_json::Value> = Vec::new();
    for (i, s) in raw_steps.iter().enumerate() {
        let (step_title, step_content) = s
            .split_once(':')
            .ok_or_else(|| format!("--step {i} must be 'title: content' (colon-separated)"))?;
        let t = step_title.trim();
        let c = step_content.trim();
        if t.is_empty() || c.is_empty() {
            return Err(format!("--step {i}: title and content must be non-empty"));
        }
        steps.push(serde_json::json!({ "title": t, "content": c }));
    }
    let domain = flags.get("domain").and_then(|o| o.clone());
    // Default content: the title itself (a procedure with no body, just steps).
    let content = title.clone();
    let mut body = serde_json::json!({ "title": title, "content": content });
    if !steps.is_empty() {
        body["steps"] = serde_json::json!(steps);
    }
    if let Some(d) = domain {
        body["domain"] = serde_json::json!(d);
    }
    let resp = post(
        &base_url(),
        "/procedure",
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let id = v["id"].as_i64().unwrap_or(0);
    let step_ids = v["step_ids"].as_array().cloned().unwrap_or_default();
    println!("created procedure {id} with {} step(s)", step_ids.len());
    if !step_ids.is_empty() {
        let ids: Vec<i64> = step_ids.iter().filter_map(|s| s.as_i64()).collect();
        println!("  steps: {ids:?}");
        println!("  retrieve with: brain get {} (root) or the step ids", id);
    }
    Ok(())
}

/// `brain classify "<text>"`: deterministic categorization. No LLM, no cloud.
/// Returns the category + confidence + matched keywords. The premium Mem0
/// feature, free and local.
fn cmd_classify(args: &[String]) -> Result<(), String> {
    let (positionals, _flags) = parse_flags(args);
    let text = require_positional(&positionals, "text")?;
    if text.trim().is_empty() {
        return Err("text must be non-empty".into());
    }
    let body = serde_json::json!({ "text": text });
    let resp = post(
        &base_url(),
        "/classify",
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let cat = v["result"]["category"].as_str().unwrap_or("?");
    let conf = v["result"]["confidence"].as_f64().unwrap_or(0.0);
    let kws = v["result"]["matched_keywords"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    println!("category   : {cat}");
    println!("confidence : {conf:.3}");
    if !kws.is_empty() {
        let kws: Vec<String> = kws
            .iter()
            .filter_map(|k| k.as_str().map(|s| s.to_string()))
            .collect();
        println!("matched    : {kws:?}");
    } else {
        println!("matched    : (none — fell through to 'general')");
    }
    let cats = v["categories"].as_array().cloned().unwrap_or_default();
    if !cats.is_empty() {
        let cats: Vec<String> = cats
            .iter()
            .filter_map(|c| c.as_str().map(|s| s.to_string()))
            .collect();
        println!("taxonomy   : {cats:?}");
    }
    Ok(())
}

/// `brain evaluate <decision_id> --var name=value [--var ...]`: evaluate a
/// stored decision rule against numeric input variables. Returns the matched
/// branch (or default) + the citation chain. The consultant's reasoning core,
/// deterministic + auditable.
fn cmd_evaluate(args: &[String]) -> Result<(), String> {
    let (positionals, _flags) = parse_flags(args);
    let id_str = require_positional(&positionals, "decision_id")?;
    let id: i64 = id_str
        .parse()
        .map_err(|_| format!("decision_id must be an integer, got '{id_str}'"))?;
    let vars = multi_flag(args, "var");
    let mut variables = serde_json::Map::new();
    for v in &vars {
        let (name, val_str) = v
            .split_once('=')
            .ok_or_else(|| format!("--var must be name=value, got '{v}'"))?;
        let val: f64 = val_str
            .trim()
            .parse()
            .map_err(|_| format!("--var value must be numeric, got '{val_str}'"))?;
        variables.insert(name.trim().to_string(), serde_json::json!(val));
    }
    let body = serde_json::json!({ "variables": variables });
    let path = format!("/decision/{id}/evaluate");
    let resp = post(
        &base_url(),
        &path,
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    if resp.status == 404 {
        return Err(format!("no decision rule with id {id}"));
    }
    if resp.status == 400 {
        return Err(format!(
            "stored content for chunk {id} is not a valid decision rule JSON"
        ));
    }
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let result = v["result"].as_str().unwrap_or("?");
    let used_default = v["used_default"].as_bool().unwrap_or(false);
    let matched = v["matched_condition"].as_str();
    let citation = v["citation"].as_i64();
    println!("result     : {result}");
    if used_default {
        println!("  (no branch matched — used the rule's default)");
    } else if let Some(m) = matched {
        println!("  matched   : {m}");
    }
    if let Some(c) = citation {
        println!("  citation  : chunk {c} (verify with: brain verify {c} ...)");
    }
    Ok(())
}

// ── v0.9.6 Bridge: connector CLI ────────────────────────────────────────────

/// `brain connect github --app-id N --install-id N --key-file PATH --repo O/R [...]`
///
/// Writes the connector config to `~/.config/brain-server/connectors/github-{instance}.json`
/// (mode 0600). The instance name is derived from the first repo (or a hash
/// of the sorted repo set for multi-repo). Does NOT spawn the connector —
/// use `brain sync github` to run one backfill pass.
///
/// ponytail: this is a thin file-authoring command. No server roundtrip —
/// the server has no `/connectors` POST route (registration is local-file).
/// Adding a server route would be appropriate when connectors are managed
/// remotely; v0.9.6 keeps the operator surface on the host that runs them.
fn cmd_connect(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args);
    let kind = flags
        .get("kind")
        .and_then(|o| o.as_deref())
        .unwrap_or("github")
        .to_string();
    if kind != "github" {
        return Err(format!(
            "v0.9.6 supports only kind=github (got '{kind}'); other connectors land in v0.9.7+"
        ));
    }

    let app_id: i64 = flags
        .get("app-id")
        .and_then(|o| o.as_deref())
        .ok_or_else(|| "--app-id <N> is required".to_string())?
        .parse()
        .map_err(|_| "--app-id must be an integer".to_string())?;
    let installation_id: i64 = flags
        .get("install-id")
        .and_then(|o| o.as_deref())
        .ok_or_else(|| "--install-id <N> is required".to_string())?
        .parse()
        .map_err(|_| "--install-id must be an integer".to_string())?;
    let key_file = flags
        .get("key-file")
        .and_then(|o| o.as_deref())
        .ok_or_else(|| "--key-file <PATH> is required".to_string())?
        .to_string();
    let webhook_secret_file = flags
        .get("webhook-secret-file")
        .and_then(|o| o.as_deref())
        .map(|s| s.to_string());

    // Collect --repo flags (can appear multiple times). The parse_flags helper
    // overwrites on duplicate keys, so we re-scan argv manually for repeats.
    let repos: Vec<String> = args
        .windows(2)
        .filter_map(|w| {
            if w[0] == "--repo" {
                Some(w[1].clone())
            } else {
                w[0].strip_prefix("--repo=").map(|v| v.to_string())
            }
        })
        .collect();
    if repos.is_empty() {
        return Err("at least one --repo owner/name is required".to_string());
    }
    for r in &repos {
        if !r.contains('/') || r.split('/').count() != 2 {
            return Err(format!("repo must be 'owner/name', got '{r}'"));
        }
    }

    // Validate the key file exists + is readable BEFORE we write the config.
    // Failing after writing leaves a stale config the user has to clean up.
    let key_path = Path::new(&key_file);
    if !key_path.is_file() {
        return Err(format!("key file not found: {key_file}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(key_path)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            eprintln!(
                "warn: key file {key_file} is mode {:o} (not 0600); recommend `chmod 600 {key_file}`",
                mode & 0o777
            );
        }
    }

    // Construct the config + derive the instance name (matches the connector
    // binary's `derive_instance_name` so the config path is stable).
    let instance = if repos.len() == 1 {
        repos[0].replace('/', "_")
    } else {
        let mut sorted = repos.clone();
        sorted.sort();
        format!(
            "multi-{:016x}",
            xxhash_rust::xxh3::xxh3_64(sorted.join(",").as_bytes())
        )
    };

    let config = serde_json::json!({
        "app_id": app_id,
        "installation_id": installation_id,
        "private_key_path": key_file,
        "webhook_secret_path": webhook_secret_file,
        "repositories": repos,
    });

    let config_dir = match std::env::var("BRAIN_CONNECTOR_CONFIG_DIR") {
        Ok(s) if !s.trim().is_empty() => PathBuf::from(s),
        _ => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config/brain-server/connectors")
        }
    };
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("mkdir {config_dir:?}: {e}"))?;
    let config_path = config_dir.join(format!("github-{instance}.json"));

    // Atomic write: tempfile in the same dir, chmod 0600, rename.
    let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    let tmp_suffix = format!(
        ".{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp_path = config_path.with_file_name(format!("github-{instance}.json.tmp{tmp_suffix}"));
    std::fs::write(&tmp_path, &bytes).map_err(|e| format!("write {tmp_path:?}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp_path, &config_path)
        .map_err(|e| format!("rename -> {config_path:?}: {e}"))?;

    // Best-effort audit of connector registration (local-file, but recorded).
    if let Ok(db_path) = std::env::var("BRAIN_DB_PATH") {
        if let Ok(conn) = rusqlite::Connection::open(&db_path) {
            brain_server::audit::record(
                &conn,
                brain_server::audit::AuditKind::Connector,
                "github",
                &instance,
                brain_server::audit::AuditStatus::Ok,
                "connect config written",
            );
        }
    }

    println!("wrote {config_path:?}");
    println!("instance: github-{instance}");
    println!("repos:    {}", repos.join(", "));
    println!("\nnext: brain sync github --instance {instance}");
    Ok(())
}

/// `brain sync [github] [--instance <name>]`
///
/// Spawns `brain-connector-gh` with the right argv. Inherits stdout/stderr so
/// the connector's JSON-lines event stream surfaces to the operator. If
/// `--instance` is omitted and exactly one github config exists, uses it;
/// otherwise lists available instances and asks the operator to specify.
///
/// ponytail: spawns the binary, doesn't try to be a long-running supervisor.
/// Long-running supervision (restart on crash, periodic schedule) lands with
/// the server-side auto-start in v0.9.7+.
fn cmd_sync(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args);
    let kind = flags
        .get("kind")
        .and_then(|o| o.as_deref())
        .or_else(|| _positionals.first().map(|s| s.as_str()))
        .unwrap_or("github");
    if kind != "github" {
        return Err(format!(
            "v0.9.6 supports only kind=github (got '{kind}'); other connectors land in v0.9.7+"
        ));
    }

    // Find the binary. $PATH first (installed via install-service.sh), then
    // target/debug (for dev). Falls back to target/release.
    let bin = which("brain-connector-gh")
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|d| d.join("target/debug/brain-connector-gh"))
                .filter(|p| p.exists())
        })
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|d| d.join("target/release/brain-connector-gh"))
                .filter(|p| p.exists())
        })
        .ok_or_else(|| {
            "brain-connector-gh not found. Build with:\n  \
             cargo build --release --features connector-github --bin brain-connector-gh"
                .to_string()
        })?;

    // Resolve the config path.
    let config_path = if let Some(p) = flags.get("config").and_then(|o| o.as_deref()) {
        PathBuf::from(p)
    } else {
        let instance = flags.get("instance").and_then(|o| o.as_deref());
        let config_dir = match std::env::var("BRAIN_CONNECTOR_CONFIG_DIR") {
            Ok(s) if !s.trim().is_empty() => PathBuf::from(s),
            _ => {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".config/brain-server/connectors")
            }
        };
        let matches = match instance {
            Some(name) => vec![config_dir.join(format!("github-{name}.json"))],
            None => glob_github_configs(&config_dir)?,
        };
        match matches.len() {
            0 => {
                return Err(format!(
                    "no github connector config in {config_dir:?}. Run `brain connect github ...` first."
                ));
            }
            1 => matches[0].clone(),
            n => {
                eprintln!("multiple github configs found; pick one with --instance:");
                for p in &matches {
                    eprintln!("  {}", p.file_name().unwrap_or_default().to_string_lossy());
                }
                return Err(format!("{n} configs; specify --instance <name>"));
            }
        }
    };
    if !config_path.is_file() {
        return Err(format!("config not found: {config_path:?}"));
    }

    // Resolve the brain-server DB path (the connector writes checkpoints there).
    let db_path = match std::env::var("BRAIN_DB_PATH") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".openclaw/workspace/brain.db")
        }
    };
    if !db_path.is_file() {
        return Err(format!(
            "brain-server DB not found at {db_path:?}. Override with BRAIN_DB_PATH."
        ));
    }

    let status = std::process::Command::new(&bin)
        .arg("--config")
        .arg(&config_path)
        .arg("--checkpoint")
        .arg(&db_path)
        // Inherit stdout/stderr so the connector's JSON-lines events surface
        // directly to the operator. No piping — the connector is short-lived.
        .status()
        .map_err(|e| format!("failed to spawn {bin:?}: {e}"))?;
    if !status.success() {
        return Err(format!("brain-connector-gh exited with {status}"));
    }
    Ok(())
}

/// Resolve the backup passphrase from `--passphrase-file` / `-flag`, then the
/// `BRAIN_BACKUP_PASSPHRASE_FILE` env var. Errors clearly if none is given.
fn resolve_passphrase(
    flags: &std::collections::HashMap<String, Option<String>>,
) -> Result<Vec<u8>, String> {
    let path = flags
        .get("passphrase-file")
        .and_then(|o| o.clone())
        .or_else(|| std::env::var("BRAIN_BACKUP_PASSPHRASE_FILE").ok())
        .ok_or_else(|| {
            "a passphrase file is required: pass --passphrase-file PATH or set BRAIN_BACKUP_PASSPHRASE_FILE"
                .to_string()
        })?;
    std::fs::read(&path).map_err(|e| format!("cannot read passphrase file {path}: {e}"))
}

/// `brain backup <out-path> [--passphrase-file PATH]`
fn cmd_backup(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    let out = positionals
        .first()
        .cloned()
        .ok_or_else(|| "usage: brain backup <out-path> [--passphrase-file PATH]".to_string())?;
    let pass = resolve_passphrase(&flags)?;
    let db = default_db_path();
    brain_server::backup::backup(&db, Path::new(&out), &pass)
        .map_err(|e| format!("backup failed: {e:#}"))?;
    println!("backup written: {out} (+ {out}.sha256 checksum)");
    Ok(())
}

/// `brain restore <in-path> [--passphrase-file PATH]`
fn cmd_restore(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args);
    let in_path = positionals
        .first()
        .cloned()
        .ok_or_else(|| "usage: brain restore <in-path> [--passphrase-file PATH]".to_string())?;
    let pass = resolve_passphrase(&flags)?;
    let db = default_db_path();
    brain_server::backup::restore(Path::new(&in_path), &db, &pass)
        .map_err(|e| format!("restore failed: {e:#}"))?;
    println!("restored: {db:?} (safety snapshot saved as <db>.bak)");
    Ok(())
}

// ── v1.2.0 "AuthN": JWT signing key management ──────────────────────────────
// Local-file operations — no server roundtrip. The server picks up new keys
// on restart (hot-reload via KeyStore::reload is a follow-up; the rotation
// watcher pattern from v1.1's TokenStore is the template).

/// Resolve the key directory: explicit `--dir`, else `BRAIN_JWT_KEY_DIR`,
/// else the platform default `~/.config/brain-server/keys/`.
fn key_dir(flags: &std::collections::HashMap<String, Option<String>>) -> PathBuf {
    if let Some(d) = flags.get("dir").and_then(|o| o.clone()) {
        return PathBuf::from(d);
    }
    if let Ok(d) = std::env::var("BRAIN_JWT_KEY_DIR") {
        let p = d.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs_home().join(".config/brain-server/keys")
}

fn cmd_key(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: brain key <generate|list|prune> [...]".to_string());
    }
    let sub = args[0].as_str();
    let rest = &args[1..];
    match sub {
        "generate" => cmd_key_generate(rest),
        "list" => cmd_key_list(rest),
        "prune" => cmd_key_prune(rest),
        other => Err(format!(
            "unknown 'brain key' subcommand: '{other}' (try generate|list|prune)"
        )),
    }
}

/// `brain key generate` — create a new RSA keypair, write to `<dir>/<kid>.pem`
/// + `<dir>/<kid>.key` (0600). The kid defaults to a short timestamped id.
fn cmd_key_generate(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args);
    let dir = key_dir(&flags);
    let kid = flags.get("kid").and_then(|o| o.clone()).unwrap_or_else(|| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("brain-{ts:x}")
    });
    // Create the dir 0700 if missing (OWASP Secrets Management: key dir mode).
    std::fs::create_dir_all(&dir).map_err(|e| format!("create key dir {dir:?}: {e}"))?;
    set_mode_0700(&dir)?;

    // Generate the RSA keypair. 2048 bits is the documented default for RS256;
    // matching every major IdP's minimum.
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
    use rsa::RsaPrivateKey;
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| format!("RSA keypair generation failed: {e}"))?;
    let pub_key = rsa::RsaPublicKey::from(&priv_key);
    let pub_pem = pub_key
        .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
        .map_err(|e| format!("public PEM encode failed: {e}"))?;
    let priv_pem = priv_key
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .map_err(|e| format!("private PEM encode failed: {e}"))?;

    let pub_path = dir.join(format!("{kid}.pem"));
    let priv_path = dir.join(format!("{kid}.key"));
    // Refuse to overwrite an existing keypair (silent overwrite would lose
    // the old signing key, breaking every outstanding token).
    if pub_path.exists() || priv_path.exists() {
        return Err(format!(
            "refusing to overwrite existing key '{kid}' in {dir:?}; pick a different --kid"
        ));
    }
    std::fs::write(&pub_path, pub_pem.as_bytes())
        .map_err(|e| format!("write {pub_path:?}: {e}"))?;
    std::fs::write(&priv_path, priv_pem.as_bytes())
        .map_err(|e| format!("write {priv_path:?}: {e}"))?;
    set_mode_0600(&priv_path)?;

    println!("generated RSA-2048 keypair:");
    println!("  kid     : {kid}");
    println!("  public  : {pub_path:?}");
    println!("  private : {priv_path:?} (mode 0600)");
    println!("restart brain-server to load the new key; existing tokens stay valid until the old key is pruned");
    Ok(())
}

/// `brain key list` — list every key in the dir (kid, has-private, size).
fn cmd_key_list(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args);
    let dir = key_dir(&flags);
    if !dir.exists() {
        println!("key dir {dir:?} does not exist (no keys configured)");
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| format!("read key dir {dir:?}: {e}"))?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str())?;
            if ext == "pem" {
                Some(p.file_stem()?.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();
    entries.sort();
    if entries.is_empty() {
        println!("no keys in {dir:?}");
        return Ok(());
    }
    println!("{:<24} {:<8} path", "kid", "signing");
    for kid in entries {
        let has_priv = dir.join(format!("{kid}.key")).exists();
        let role = if has_priv { "yes" } else { "verify" };
        println!("{kid:<24} {role:<8} {dir:?}");
    }
    Ok(())
}

/// `brain key prune` — remove public PEMs that have no matching private key
/// AND are older than `--keep` (default 1, meaning keep the most recent N
/// verify-only keys). Used after rotation to drop keys whose tokens have all
/// expired. Refuses to touch keys with a private half (those are still active).
fn cmd_key_prune(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args);
    let dir = key_dir(&flags);
    let keep: usize = flags
        .get("keep")
        .and_then(|o| o.clone())
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    if !dir.exists() {
        println!("key dir {dir:?} does not exist; nothing to prune");
        return Ok(());
    }
    // Verify-only keys = PEMs without a matching `.key`. Sort by mtime desc;
    // keep the N most recent, prune the rest.
    let mut verify_only: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| format!("read key dir {dir:?}: {e}"))?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str())?;
            if ext != "pem" {
                return None;
            }
            let kid = p.file_stem()?.to_string_lossy().to_string();
            let has_priv = dir.join(format!("{kid}.key")).exists();
            if has_priv {
                return None;
            }
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((p, mtime))
        })
        .collect();
    // Sort newest first. clippy suggests `sort_by_key` with `Reverse` for
    // descending order — equivalent, slightly clearer intent.
    verify_only.sort_by_key(|(_, mtime)| std::cmp::Reverse(*mtime));
    let to_prune = if verify_only.len() > keep {
        &verify_only[keep..]
    } else {
        &[]
    };
    if to_prune.is_empty() {
        println!(
            "no verify-only keys to prune (keep={keep}, found={})",
            verify_only.len()
        );
        return Ok(());
    }
    for (p, _) in to_prune {
        std::fs::remove_file(p).map_err(|e| format!("prune {p:?}: {e}"))?;
        println!("pruned: {p:?}");
    }
    println!(
        "pruned {} verify-only keys (kept {})",
        to_prune.len(),
        verify_only.len() - to_prune.len()
    );
    Ok(())
}

/// Set file mode 0600 on Unix. No-op on non-Unix (the brain-server target is
/// macOS/Linux, but this keeps the CLI portable for tests on Windows-hosted CI).
#[cfg(unix)]
fn set_mode_0600(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("chmod 0600 {path:?}: {e}"))
}

#[cfg(not(unix))]
fn set_mode_0600(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Set dir mode 0700 on Unix.
#[cfg(unix)]
fn set_mode_0700(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("chmod 0700 {path:?}: {e}"))
}

#[cfg(not(unix))]
fn set_mode_0700(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// `brain connector-status` — lists every registered connector across all
/// kinds. Calls the existing `GET /connectors` route.
fn cmd_connector_status(_args: &[String]) -> Result<(), String> {
    let resp = get(&base_url(), "/connectors", &[], auth_token().as_deref())?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("non-JSON /connectors response: {e}"))?;
    let connectors = v
        .get("connectors")
        .and_then(|x| x.as_array())
        .ok_or_else(|| "response missing 'connectors' array".to_string())?;
    if connectors.is_empty() {
        println!("no connectors registered");
        println!("\nregister one with: brain connect github --app-id N --install-id N --key-file PATH --repo owner/name");
        return Ok(());
    }
    println!(
        "{:<6} {:<10} {:<32} {:<10} {:<22}",
        "id", "kind", "instance", "state", "last_sync"
    );
    for c in connectors {
        let id = c.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
        let kind = c.get("kind").and_then(|x| x.as_str()).unwrap_or("?");
        let instance = c.get("instance").and_then(|x| x.as_str()).unwrap_or("?");
        let state = c.get("state").and_then(|x| x.as_str()).unwrap_or("?");
        let last_sync = c.get("last_sync_at").and_then(|x| x.as_str()).unwrap_or("");
        println!(
            "{:<6} {:<10} {:<32} {:<10} {:<22}",
            id, kind, instance, state, last_sync
        );
    }
    Ok(())
}

/// Look up a binary on `$PATH`. Hand-rolled because `which` is not a dep.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Glob `github-*.json` files in a directory. Hand-rolled (no `glob` dep).
/// Returns paths sorted alphabetically.
fn glob_github_configs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("read {dir:?}: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| match p.file_name() {
            Some(name) => name
                .to_str()
                .map(|s| s.starts_with("github-") && s.ends_with(".json"))
                .unwrap_or(false),
            None => false,
        })
        .collect();
    out.sort();
    Ok(out)
}

fn summarize_ingest(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("status")
                .or_else(|| v.get("id"))
                .map(|x| x.to_string())
        })
        .unwrap_or_else(|| "done".into())
}

fn derive_title(path: &Path) -> String {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();
    name.replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn load_brainignore(root: &Path) -> Vec<String> {
    let path = root.join(".brainignore");
    match std::fs::read_to_string(&path) {
        Ok(c) => c
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.to_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Recursively collect ingestable files, skipping hidden dirs (`.git`, etc.),
/// applying `.brainignore` patterns, and bounding depth to avoid runaways.
fn collect_files(
    root: &Path,
    dir: &Path,
    ignore: &[String],
    out: &mut Vec<PathBuf>,
) -> Result<(), String> {
    // ponytail: bounded by filesystem depth (~32) rather than a node count;
    // a symlink loop is prevented by not following symlinked dirs.
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {:?}: {e}", dir))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("file_type error: {e}"))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            if ignore_matches(ignore, &path, root) {
                continue;
            }
            collect_files(root, &path, ignore, out)?;
        } else if file_type.is_file() {
            if ignore_matches(ignore, &path, root) {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(
                ext.as_str(),
                "md" | "markdown"
                    | "txt"
                    | "rst"
                    | "org"
                    | "json"
                    | "csv"
                    | "log"
                    | "rs"
                    | "py"
                    | "js"
                    | "ts"
                    | "go"
                    | "c"
                    | "cpp"
                    | "h"
                    | "java"
                    | "sh"
                    | "yaml"
                    | "yml"
                    | "toml"
                    | "html"
                    | "htm"
            ) {
                out.push(path);
            }
        }
    }
    Ok(())
}

/// Check whether `path` (relative to `root`) matches any gitignore-style glob.
fn ignore_matches(patterns: &[String], path: &Path, root: &Path) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    patterns
        .iter()
        .any(|p| glob_match(p, &rel_str) || glob_match(p, &name))
}

/// Minimal glob matcher supporting `*` (any chars) and `**` (across `/`).
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    // Recursive backtracking matcher.
    fn matches(pat: &[char], p: usize, txt: &[char], t: usize) -> bool {
        let mut pi = p;
        let mut ti = t;
        while pi < pat.len() {
            match pat[pi] {
                '*' => {
                    if pi + 1 < pat.len() && pat[pi + 1] == '*' {
                        // "**": consume following '/' then match rest greedily
                        if pi + 2 < pat.len() && pat[pi + 2] == '/' {
                            pi += 3;
                            let mut ti2 = ti;
                            while ti2 <= txt.len() {
                                if matches(pat, pi, txt, ti2) {
                                    return true;
                                }
                                ti2 += 1;
                            }
                            return false;
                        }
                        // bare "**" as single star
                        let mut ti2 = ti;
                        while ti2 <= txt.len() {
                            if matches(pat, pi + 1, txt, ti2) {
                                return true;
                            }
                            ti2 += 1;
                        }
                        return false;
                    }
                    // single '*'
                    let mut ti2 = ti;
                    while ti2 <= txt.len() {
                        if matches(pat, pi + 1, txt, ti2) {
                            return true;
                        }
                        ti2 += 1;
                    }
                    return false;
                }
                c if pi < pat.len() && c == txt.get(ti).copied().unwrap_or('\0') => {
                    pi += 1;
                    ti += 1;
                }
                _ => return false,
            }
        }
        ti == txt.len()
    }
    matches(&pat, 0, &txt, 0)
}

fn cmd_status() -> Result<(), String> {
    let resp = get(&base_url(), "/stats", &[], auth_token().as_deref())?;
    if resp.status != 200 {
        return Err(format!("server returned status {}", resp.status));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON stats response: {e}"))?;
    println!("brain-server status");
    println!(
        "  version : {}",
        json_str(&v, "version").unwrap_or_default()
    );
    println!("  model   : {}", json_str(&v, "model").unwrap_or_default());
    println!(
        "  documents   : {}",
        v.get("count").and_then(|x| x.as_i64()).unwrap_or(-1)
    );
    println!(
        "  embeddings  : {}",
        v.get("embeddings").and_then(|x| x.as_i64()).unwrap_or(-1)
    );
    println!(
        "  entities    : {}",
        v.get("entities").and_then(|x| x.as_i64()).unwrap_or(-1)
    );
    println!(
        "  relationships: {}",
        v.get("relationships")
            .and_then(|x| x.as_i64())
            .unwrap_or(-1)
    );
    Ok(())
}

fn cmd_doctor(args: &[String]) -> Result<(), String> {
    // `brain doctor --backup <path> [--passphrase-file PATH]` — verify-only mode.
    let (positionals, flags) = parse_flags(args);
    if let Some(backup_path) = flags
        .get("backup")
        .and_then(|o| o.clone())
        .or_else(|| positionals.first().cloned())
    {
        let pass = resolve_passphrase(&flags)?;
        let manifest = brain_server::backup::verify(Path::new(&backup_path), &pass)
            .map_err(|e| format!("backup verify failed: {e:#}"))?;
        println!("brain doctor — backup verification\n");
        println!("  backup:    {}", backup_path);
        println!("  created:   {}", manifest.created_at);
        println!("  version:   {}", manifest.version);
        println!("  components:");
        for c in &manifest.components {
            let tag = if c.secret { " (secret: path only)" } else { "" };
            println!("    - {}  xxh3={}  size={}{}", c.name, c.xxh3, c.size, tag);
        }
        println!("\nbackup integrity: OK");
        return Ok(());
    }

    println!("brain doctor — health report\n");

    // 1. reachable
    let health = get(&base_url(), "/health", &[], None);
    let (reachable, version, model) = match &health {
        Ok(r) if r.status == 200 => {
            let v: serde_json::Value =
                serde_json::from_str(&r.body).unwrap_or(serde_json::Value::Null);
            (
                true,
                json_str(&v, "version").unwrap_or_default(),
                json_str(&v, "model").unwrap_or_default(),
            )
        }
        Ok(r) => {
            println!("  [✗] server reachable but returned status {}", r.status);
            (true, String::new(), String::new())
        }
        Err(e) => {
            println!("  [✗] server NOT reachable: {e}");
            (false, String::new(), String::new())
        }
    };
    println!(
        "  [{}] server reachable at {}",
        if reachable { "✓" } else { "✗" },
        base_url()
    );

    // 2. model loaded (via /version)
    match get(&base_url(), "/version", &[], None) {
        Ok(r) if !r.body.trim().is_empty() => {
            println!("  [✓] model loaded (version endpoint: {})", r.body.trim());
        }
        Ok(r) => println!("  [✗] /version returned empty body (status {})", r.status),
        Err(e) => println!("  [✗] cannot reach /version: {e}"),
    }
    if !model.is_empty() {
        println!("  [✓] model id: {model}  (server v{version})");
    }

    // 3. DB path — not exposed by the API, so we check the expected local file.
    let db = default_db_path();
    let exists = db.exists();
    println!(
        "  [{}] local DB file: {}",
        if exists { "✓" } else { "·" },
        db.display()
    );
    if !exists {
        println!("      (file not present; server will create it on first run)");
    }

    // 4. DB health (optional, only if reachable)
    if reachable {
        match get(&base_url(), "/health/db", &[], None) {
            Ok(r) if r.status == 200 => {
                let v: serde_json::Value =
                    serde_json::from_str(&r.body).unwrap_or(serde_json::Value::Null);
                let size = v.get("database_size_mb").and_then(|x| x.as_f64());
                let last = json_str(&v, "last_write");
                println!(
                    "  [✓] database healthy ({:.2} MB, last write: {})",
                    size.unwrap_or(0.0),
                    last.unwrap_or_else(|| "n/a".into())
                );
            }
            Ok(r) => println!("  [·] /health/db returned status {}", r.status),
            Err(e) => println!("  [·] /health/db unavailable: {e}"),
        }
    }

    // 5. local DB integrity + journal mode (reads the file directly).
    if db.exists() {
        match rusqlite::Connection::open(&db) {
            Ok(conn) => {
                let integrity: String = conn
                    .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                    .unwrap_or_else(|_| "error".to_string());
                if integrity == "ok" {
                    println!("  [✓] integrity_check: ok");
                } else {
                    println!("  [✗] integrity_check: {integrity}");
                }
                let jmode: String = conn
                    .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                    .unwrap_or_default();
                println!("  [·] journal_mode: {jmode}");
            }
            Err(e) => println!("  [·] cannot open DB for local check: {e}"),
        }
    }

    println!("\ndoctor complete.");
    Ok(())
}

fn cmd_bench() -> Result<(), String> {
    // v1.17.1 "Govern" M3: optional recall floors as a ship gate, mirroring the
    // `BENCH_ENVELOPE` RSS/p95 gate. Format: `r5:0.85,r10:0.9` (or `r5=0.85`).
    let floors = match std::env::var("BENCH_RECALL_FLOOR") {
        Ok(spec) if !spec.trim().is_empty() => parse_floors(&spec)?,
        _ => Vec::new(),
    };
    let ok = run_eval("/search", &floors)?;
    if !ok {
        return Err("recall floor breached (see BENCH_RECALL_FLOOR)".into());
    }
    Ok(())
}

/// `brain eval [--floor r5=0.85 r10=0.9]` — v1.17.1 "Govern" M3: run the frozen
/// judged corpus (`tests/fixtures/eval_queries.md`) against `/recall`, report
/// the metrics, and exit non-zero when any `--floor` is breached. The fixture
/// ships in the repo so the gate is reproducible on any machine with a live
/// server; the operator's private judged corpus remains a separate step.
fn cmd_eval(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args);
    let mut floors: Vec<(String, f32)> = Vec::new();
    for (k, v) in &flags {
        if let Some(name) = k.strip_prefix("floor") {
            if name.is_empty() {
                let spec = v
                    .as_deref()
                    .ok_or("--floor requires r5=0.85-style values")?;
                floors.extend(parse_floors(spec)?);
            } else {
                let metric = name.trim_start_matches('=').trim_start_matches(':');
                let val = v
                    .as_deref()
                    .ok_or_else(|| format!("--floor{name} requires a value"))?
                    .parse::<f32>()
                    .map_err(|e| format!("floor value for {metric}: {e}"))?;
                floors.push((metric.to_string(), val));
            }
        }
    }
    let ok = run_eval("/recall", &floors)?;
    if !ok {
        return Err("recall floor breached".into());
    }
    Ok(())
}

/// Parse `r5:0.85,r10:0.9` (or `r5=0.85`; mixed separators allowed) into
/// (metric, floor) pairs. Unknown metric names are rejected so a typo can't
/// silently disable the gate.
fn parse_floors(spec: &str) -> Result<Vec<(String, f32)>, String> {
    let mut out = Vec::new();
    for part in spec.split([',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (metric, val) = part
            .split_once(['=', ':'])
            .ok_or_else(|| format!("floor '{part}' must be metric=value"))?;
        if !["r5", "r10", "p5", "p10", "mrr", "ndcg"].contains(&metric) {
            return Err(format!(
                "unknown floor metric '{metric}' (r5|r10|p5|p10|mrr|ndcg)"
            ));
        }
        let v = val
            .parse::<f32>()
            .map_err(|e| format!("floor value for {metric}: {e}"))?;
        if !(0.0..=1.0).contains(&v) {
            return Err(format!("floor for {metric} must be in [0,1]"));
        }
        out.push((metric.to_string(), v));
    }
    Ok(out)
}

/// Run the frozen eval fixture against `endpoint` (`/search` or `/recall`),
/// print per-query + mean metrics, and return whether every floor held.
/// Floors are (metric, min) pairs over the means: r5/r10 = recall@k,
/// p5/p10 = precision@k, mrr, ndcg.
fn run_eval(endpoint: &str, floors: &[(String, f32)]) -> Result<bool, String> {
    let fixture = "tests/fixtures/eval_queries.md";
    let raw = std::fs::read_to_string(fixture)
        .map_err(|e| format!("cannot read {fixture}: {e} (run from the repo root)"))?;
    let queries = parse_eval_fixture(&raw)?;
    if queries.is_empty() {
        return Err("no queries parsed from eval fixture".into());
    }

    match get(&base_url(), "/health", &[], None) {
        Ok(r) if r.status == 200 => {}
        Ok(r) => return Err(format!("server unhealthy (status {})", r.status)),
        Err(e) => return Err(format!("cannot reach server: {e}")),
    }

    println!(
        "brain eval — frozen set ({} queries, {endpoint})",
        queries.len()
    );
    println!("query                                            r@5    r@10");
    println!("{:-<52} ------ ------", "");

    let mut sums = [0.0_f32; 5]; // r5, r10, p5, p10, mrr
    let mut ndcg_sum = 0.0_f32;
    for q in &queries {
        // GET /search reads `q`+`k` (the pre-v0.9.5 params); POST /recall is a
        // JSON body (`query`+`limit`) — it was added as POST-only, so a GET
        // returns 405.
        let resp = if endpoint == "/search" {
            get(
                &base_url(),
                endpoint,
                &[
                    ("q".to_string(), q.query.clone()),
                    ("k".to_string(), "10".to_string()),
                ],
                auth_token().as_deref(),
            )?
        } else {
            post(
                &base_url(),
                endpoint,
                &[],
                "application/json",
                &serde_json::json!({ "query": q.query.clone(), "limit": 10 }).to_string(),
                auth_token().as_deref(),
            )?
        };
        if resp.status != 200 {
            return Err(format!(
                "eval query failed ({}, status {}): {}",
                q.query,
                resp.status,
                truncate(&resp.body, 200)
            ));
        }
        let ids = results_to_doc_indices(&resp.body);
        let relevant: Vec<i64> = q.relevant.iter().map(|&i| i as i64).collect();
        let r5 = brain_server::eval::recall_at_k(&ids, &relevant, 5);
        let r10 = brain_server::eval::recall_at_k(&ids, &relevant, 10);
        let p5 = brain_server::eval::precision_at_k(&ids, &relevant, 5);
        let p10 = brain_server::eval::precision_at_k(&ids, &relevant, 10);
        let m = brain_server::eval::mrr(&ids, &relevant);
        sums[0] += r5;
        sums[1] += r10;
        sums[2] += p5;
        sums[3] += p10;
        sums[4] += m;
        ndcg_sum += brain_server::eval::ndcg(&ids, &relevant, 10);
        let label = if q.query.chars().count() > 48 {
            let t: String = q.query.chars().take(45).collect();
            format!("{t}...")
        } else {
            q.query.clone()
        };
        println!("{:<52} {:<6.2} {:<6.2}", label, r5, r10);
    }

    let n = queries.len() as f32;
    let mean = [
        sums[0] / n,
        sums[1] / n,
        sums[2] / n,
        sums[3] / n,
        sums[4] / n,
        ndcg_sum / n,
    ];
    println!("{:-<52} ------ ------", "");
    println!(
        "mean  r@5={:.3} r@10={:.3} p@5={:.3} p@10={:.3} mrr={:.3} ndcg@10={:.3}  (over {} queries)",
        mean[0], mean[1], mean[2], mean[3], mean[4], mean[5], queries.len()
    );
    let names = ["r5", "r10", "p5", "p10", "mrr", "ndcg"];
    let mut held = true;
    for (metric, floor) in floors {
        if let Some((i, _)) = names.iter().enumerate().find(|(_, m)| **m == metric) {
            if mean[i] < *floor {
                held = false;
                println!("FLOOR BREACH: {metric} = {:.3} < {floor:.3}", mean[i]);
            } else {
                println!("floor ok    : {metric} = {:.3} >= {floor:.3}", mean[i]);
            }
        }
    }
    Ok(held)
}

struct EvalQuery {
    query: String,
    relevant: Vec<usize>,
    #[allow(dead_code)]
    category: String,
}

fn parse_eval_fixture(raw: &str) -> Result<Vec<EvalQuery>, String> {
    let mut out = Vec::new();
    let mut current: Option<EvalQuery> = None;
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("###") {
            // new query block: "Q<n> — [category]"
            let cat = rest
                .rsplit_once('[')
                .and_then(|(_, c)| c.strip_suffix(']'))
                .unwrap_or("")
                .to_string();
            current = Some(EvalQuery {
                query: String::new(),
                relevant: Vec::new(),
                category: cat,
            });
        } else if let Some(q) = line.strip_prefix("Query:") {
            if let Some(c) = &mut current {
                c.query = q.trim().trim_matches('"').to_string();
            }
        } else if let Some(r) = line.strip_prefix("Relevant:") {
            if let Some(mut c) = current.take() {
                c.relevant = parse_index_list(r);
                out.push(c);
            }
        }
    }
    Ok(out)
}

fn parse_index_list(s: &str) -> Vec<usize> {
    let s = s.trim().trim_start_matches('[').trim_end_matches(']');
    s.split(',')
        .filter_map(|x| x.trim().parse::<usize>().ok())
        .collect()
}

/// Map a `/search` JSON body to a rank-ordered list of doc indices (matching
/// `DOCS`). Unknown results map to -1 so they never count as relevant.
fn results_to_doc_indices(body: &str) -> Vec<i64> {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    // `/search` (GET) wraps results under `results`; `/recall` (POST) under
    // `hits`. Both hit shapes carry `content`.
    let results = v
        .get("results")
        .or_else(|| v.get("hits"))
        .and_then(|r| r.as_array());
    let Some(results) = results else {
        return Vec::new();
    };
    // Match content against DOCS directly: the judged `Relevant:` indices are
    // DOCS-array positions, so the position must be the slice index — a
    // HashSet `.position()` would be arbitrary (hash order), poisoning recall.
    let docs: Vec<&str> = DOCS.iter().map(|d| d.trim()).collect();
    results
        .iter()
        .map(|r| {
            let content = r
                .get("content")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            docs.iter()
                .position(|d| *d == content)
                .map(|i| i as i64)
                .unwrap_or(-1)
        })
        .collect()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn doc_indices_parse_recall_hits_and_search_results() {
        let hit = DOCS[0].trim();
        let recall_body = format!(r#"{{"hits":[{{"content":"{hit}"}}],"decision":"ok"}}"#);
        let search_body = format!(r#"{{"results":[{{"content":"{hit}"}}]}}"#);
        assert_eq!(results_to_doc_indices(&recall_body), vec![0]);
        assert_eq!(results_to_doc_indices(&search_body), vec![0]);
        assert_eq!(results_to_doc_indices(r#"{"hits":[]}"#), Vec::<i64>::new());
    }

    #[test]
    fn glob_matches_simple_name() {
        assert!(glob_match("*.tmp", "foo.tmp"));
        assert!(!glob_match("*.tmp", "foo.md"));
    }

    #[test]
    fn glob_matches_directory_prefix() {
        // `**/` spans path separators.
        assert!(glob_match("draft/**", "draft/note.md"));
        assert!(glob_match("draft/**", "draft/sub/deep.md"));
        assert!(!glob_match("draft/**", "final/note.md"));
    }

    #[test]
    fn glob_matches_exact_name() {
        assert!(glob_match("secret.md", "secret.md"));
        assert!(!glob_match("secret.md", "other.md"));
    }

    #[test]
    fn ignore_skips_brainignore_entries() {
        // Patterns are matched against the path relative to the vault root and
        // against the bare filename. A `.brainignore` entry must suppress both
        // whole-subtree and per-file matches.
        let root = Path::new("/vault");
        let patterns = vec!["drafts/**".to_string(), "*.tmp".to_string()];
        assert!(ignore_matches(
            &patterns,
            Path::new("/vault/drafts/note.md"),
            root
        ));
        assert!(ignore_matches(
            &patterns,
            Path::new("/vault/scratch.tmp"),
            root
        ));
        assert!(!ignore_matches(
            &patterns,
            Path::new("/vault/keep.md"),
            root
        ));
    }

    /// v1.17.1 M3: floor specs parse from either separator; unknown metrics and
    /// out-of-range values are rejected so a typo can't silently disable the gate.
    #[test]
    fn floors_parse_and_reject_typos() {
        assert_eq!(
            parse_floors("r5:0.85,r10=0.9").unwrap(),
            vec![("r5".to_string(), 0.85), ("r10".to_string(), 0.9)]
        );
        assert!(parse_floors("r5=1.5").is_err(), "floor > 1 rejected");
        assert!(parse_floors("r99=0.5").is_err(), "unknown metric rejected");
        assert!(parse_floors("0.5").is_err(), "missing metric rejected");
        assert!(parse_floors("r5=abc").is_err(), "non-numeric rejected");
        assert_eq!(parse_floors("").unwrap(), vec![]);
    }

    /// v1.17.3 M4: `brain ump` rejects a missing subcommand and `import`
    /// rejects a missing file — both without any network roundtrip.
    #[test]
    fn ump_requires_a_subcommand() {
        assert!(cmd_ump(&[]).is_err(), "bare `brain ump` is a usage error");
        assert!(
            cmd_ump(&["import".to_string()]).is_err(),
            "import needs a file"
        );
        assert!(
            cmd_ump(&[
                "export".to_string(),
                "--format".to_string(),
                "yaml".to_string()
            ])
            .is_err(),
            "unknown format rejected before any request"
        );
    }

    /// v1.17.3 M5: `brain ump keygen` writes a 32-byte operator seed (0600)
    /// and prints a `did:key`, and refuses to overwrite an existing key.
    #[test]
    fn ump_keygen_writes_0600_seed_and_prints_did() {
        let dir = tempfile::tempdir().expect("temp dir");
        let did = cmd_ump_keygen(&["--dir".to_string(), dir.path().to_string_lossy().into()])
            .expect("keygen succeeds");
        assert!(
            did.starts_with("did:key:"),
            "keygen returns the did:key, got {did:?}"
        );
        let path = dir.path().join("operator.key");
        let meta = std::fs::metadata(&path).expect("operator.key written");
        assert_eq!(meta.len(), 32, "raw 32-byte Ed25519 seed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "private key is 0600");
        }
        // The DID is derived from the stored seed — recompute and compare
        // (did:key base58btc is not always `z6Mk`-prefixed; the bytes matter).
        let seed: [u8; 32] = std::fs::read(&path)
            .expect("seed readable")
            .try_into()
            .unwrap();
        let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
        let recomputed =
            brain_server::ump_integrity::did_key_from_ed25519(&sk.verifying_key().to_bytes());
        assert_eq!(did, recomputed, "DID matches the written key");
        assert!(
            cmd_ump_keygen(&["--dir".to_string(), dir.path().to_string_lossy().into()]).is_err(),
            "refuses to overwrite an existing key"
        );
    }
}
