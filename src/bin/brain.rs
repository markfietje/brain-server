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

#[path = "../bin_common/wfm_import.rs"]
mod wfm_import;

use http::{delete, get, post, url_encode};
use std::path::{Path, PathBuf};
use std::process::exit;

const DEFAULT_URL: &str = "http://127.0.0.1:8765";

/// walk bounds for `ingest-dir`. Guards against pathological vaults
/// blowing the ingest budget. 50k files / 500 MiB matches the documented RSS ceiling
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
    "VxRail LCM upgrades require a green RCM release certification manifest before any upgrade wave is scheduled.",
    "A stretched-cluster rolling reboot reboots one ESXi node at a time; never reboot two nodes concurrently.",
    "vSAN storage policies set FTT failures to tolerate and FTM failure tolerance method per virtual machine.",
    "PowerFlex protection domains map fault sets to failure boundaries across SDS storage pools.",
    "NSX-T managers push micro-segmentation firewall rules to transport nodes over the control plane.",
    "A DPA data processing agreement under GDPR Article 28 binds the processor to the controller's instructions.",
    "Standard Contractual Clauses 2021 are the approved EU transfer mechanism for processors outside the EEA.",
    "RA 10173 the Philippine Data Privacy Act requires NPC breach notification within 72 hours.",
    "Schrems II requires a transfer impact assessment before any personal-data transfer to a third country.",
    "Legal holds freeze erasure until every hold is explicitly released by the operator.",
    "Intermittent storage fabric latency usually traces to a failing SFP on one uplink port, not the array.",
    "High VM disk latency triage order: vSAN backend congestion, then host cache, then the physical disk group.",
    "A node flapping out of vCenter management is most often NTP drift breaking certificate validation.",
    "PSOD purple diagnostic screen dumps land in var log and must be collected before any reboot clears them.",
    "vMotion failing at ten percent points to VMkernel port mobility or a missing shared datastore.",
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
    // Default install path: written by install-service.sh alongside the
    // launchd plist's AUTH_TOKEN_FILE. Same file, same value, no extra env.
    let default_path = dirs_home().join(".config/brain-server/auth-token");
    if let Ok(s) = std::fs::read_to_string(&default_path)
        && let Some(t) = http::first_token(&s)
    {
        return Some(t);
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

/// the ONE subcommand table: consumed by both the dispatcher
/// (name → run fn) and the help printer (name → usage line), so help can
/// never drift from the dispatch set. `--json` marks the data commands whose
/// output is a single machine envelope.
struct Subcommand {
    name: &'static str,
    usage: &'static str,
    run: fn(&[String]) -> Result<(), String>,
    json: bool,
}

const SUBCOMMANDS: &[Subcommand] = &[
    Subcommand {
        name: "query",
        json: true,
        run: cmd_query,
        usage: "brain query \"<q>\" [--k N] [--source S ...] [--phrase P ...]\n                 [--exclude E ...] [--code C ...] [--since ISO]\n                 [--intent I] [--profile P] [--graph] [--explain]",
    },
    Subcommand {
        name: "explain",
        json: true,
        run: cmd_explain,
        usage: "brain explain \"<q>\" [--source S ...] [--since ISO]",
    },
    Subcommand {
        name: "get",
        json: true,
        run: cmd_get,
        usage: "brain get <id>",
    },
    Subcommand {
        name: "ingest-dir",
        json: true,
        run: cmd_ingest_dir,
        usage: "brain ingest-dir <path> [--dry-run] [--replace] [--source S] [--domain D]",
    },
    Subcommand {
        name: "reconcile",
        json: false,
        run: cmd_reconcile,
        usage: "brain reconcile <path> [--kind vault] [--dry-run]",
    },
    Subcommand {
        name: "resolve",
        json: false,
        run: cmd_resolve,
        usage: "brain resolve <new_id> <old_id>",
    },
    Subcommand {
        name: "domain-move",
        json: false,
        run: cmd_domain_move,
        usage: "brain domain-move <id> [<id> ...] --to <domain> [--confirm global]",
    },
    Subcommand {
        name: "domains-recompute",
        json: false,
        run: cmd_domains_recompute,
        usage: "brain domains-recompute",
    },
    Subcommand {
        name: "undo-resolve",
        json: false,
        run: cmd_undo_resolve,
        usage: "brain undo-resolve <old_id> [<old_id> ...]",
    },
    Subcommand {
        name: "check-consistency",
        json: false,
        run: cmd_check_consistency,
        usage: "brain check-consistency",
    },
    Subcommand {
        name: "source-delete",
        json: false,
        run: cmd_source_delete,
        usage: "brain source-delete <id> [--yes]",
    },
    Subcommand {
        name: "suggest",
        json: true,
        run: cmd_suggest,
        usage: "brain suggest \"<context>\" [--exclude id[,id...]] [--k N] [--domain D] [--session S]",
    },
    Subcommand {
        name: "suggest-feedback",
        json: false,
        run: cmd_suggest_feedback,
        usage: "brain suggest-feedback <chunk_id> accept|dismiss [--reason \"...\"] [--session S]",
    },
    Subcommand {
        name: "suggest-metrics",
        json: true,
        run: cmd_suggest_metrics,
        usage: "brain suggest-metrics [--session S] [--since DATE]",
    },
    Subcommand {
        name: "retention",
        json: true,
        run: cmd_retention,
        usage: "brain retention get\n  brain retention set <kind> <days>",
    },
    Subcommand {
        name: "setup",
        json: false,
        run: cmd_setup,
        usage: "brain setup [domain] [--profile NAME] [--yes]",
    },
    Subcommand {
        name: "client",
        json: false,
        run: cmd_client,
        usage: "brain client add <name> --domain D --jurisdiction J [--profile P] [--yes]\n  brain client dpa get <name>\n  brain client dpa set <name> --retention R --deletion D --audit A --breach B --onward O --sub-sub S\n  brain client hold add <name> <id> [<id> ...] --reason R | list <name>",
    },
    Subcommand {
        name: "snapshot-status",
        json: true,
        run: cmd_snapshot_status,
        usage: "brain snapshot-status",
    },
    Subcommand {
        name: "eval",
        json: true,
        run: cmd_eval,
        usage: "brain eval [--floor r5=0.85 r10=0.9]",
    },
    Subcommand {
        name: "procedure",
        json: false,
        run: cmd_procedure,
        usage: "brain procedure <title> [--step \"title: content\" ...] [--domain D]",
    },
    Subcommand {
        name: "classify",
        json: false,
        run: cmd_classify,
        usage: "brain classify \"<text>\"",
    },
    Subcommand {
        name: "evaluate",
        json: false,
        run: cmd_evaluate,
        usage: "brain evaluate <decision_id> --var name=value [--var name=value ...]",
    },
    Subcommand {
        name: "connect",
        json: false,
        run: cmd_connect,
        usage: "brain connect github --app-id N --install-id N --key-file PATH \\\n                      --repo owner/repo [...] [--webhook-secret-file PATH]\n  brain connect --kind crm-salesforce ...   (vocabulary accepts the v1.24 set;\n                                             only github has a runnable binary)",
    },
    Subcommand {
        name: "sync",
        json: false,
        run: cmd_sync,
        usage: "brain sync [github] [--config PATH]",
    },
    Subcommand {
        name: "connector-status",
        json: true,
        run: cmd_connector_status,
        usage: "brain connector-status",
    },
    Subcommand {
        name: "backup",
        json: false,
        run: cmd_backup,
        usage: "brain backup <out-path> [--passphrase-file PATH] [--format v1|v2|v3]",
    },
    Subcommand {
        name: "restore",
        json: false,
        run: cmd_restore,
        usage: "brain restore <in-path> [--passphrase-file PATH]",
    },
    Subcommand {
        name: "key",
        json: false,
        run: cmd_key,
        usage: "brain key generate [--kid ID] [--alg RS256] [--dir PATH]\n  brain key list [--dir PATH]\n  brain key prune [--dir PATH] [--keep N]",
    },
    Subcommand {
        name: "ump",
        json: false,
        run: cmd_ump,
        usage: "brain ump export [--format md|ump] [--out FILE]\n  brain ump import <file>\n  brain ump keygen [--dir PATH]",
    },
    Subcommand {
        name: "token",
        json: false,
        run: cmd_token,
        usage: "brain token rotate",
    },
    Subcommand {
        name: "workflow",
        json: true,
        run: cmd_workflow,
        usage: "brain workflow open [DOMAIN]\n  brain workflow status <run>\n  brain workflow answer <run> <text>\n  brain workflow approve <run> <step>\n  brain workflow crank <run> [steps]\n  brain workflow handoff <run>",
    },
    Subcommand {
        name: "bench",
        json: false,
        run: |_| cmd_bench(),
        usage: "brain bench",
    },
    Subcommand {
        name: "status",
        json: true,
        run: |_| cmd_status(),
        usage: "brain status",
    },
    Subcommand {
        name: "doctor",
        json: false,
        run: cmd_doctor,
        usage: "brain doctor [--backup <path> [--passphrase-file PATH]]",
    },
    Subcommand {
        name: "kb",
        json: false,
        run: cmd_kb,
        usage: "brain kb build --domain <d> --out <dir> [--db <path>] [--base-url <url>]\n                 [--with-case-status] [--locales en,de,fr,es,nl]\n                 (the public KB is a static build artifact; sign the tarball with\n                  scripts/release-sign.sh before hosting)",
    },
    Subcommand {
        name: "parcel",
        json: false,
        run: cmd_parcel,
        usage: "brain parcel export --domain <d> [--since <ts>] --out <file>\n       brain parcel import --file <file> --domain <d> [--expected-signer <did>]\n       brain parcel ledger [--domain <d>]",
    },
    Subcommand {
        name: "wfm-import",
        json: false,
        run: cmd_wfm_import,
        usage: "brain wfm-import <file.csv|file.json> [--domain D] [--dry-run]\n                 (shift rows POST to /ops/shifts; skill rows become\n                  crew_skills_update proposals — never direct writes)",
    },
];

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        exit(0);
    }
    // Resolve the audit-chain HMAC key before any
    // subcommand touches a DB — restore certification, connector events and
    // `doctor` chain checks on an hmac256-epoch DB all need it. Env > key
    // file > a generated 0600 `audit-chain.key` beside the DB; a failure is a
    // warning (legacy-epoch DBs need no key — their writes fail closed per-
    // write with a visible /health counter, not a broken CLI).
    if let Err(e) = brain_server::audit::init_chain_key(
        default_db_path()
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    ) {
        eprintln!("warning: audit chain key unavailable ({e})");
    }
    // `--json` is a global mode: `brain --json recall "…" or per-subcommand.
    if args.iter().any(|a| a == "--json") {
        JSON_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
        args.retain(|a| a != "--json");
        if args.is_empty() {
            eprintln!("error: --json needs a subcommand");
            print_usage();
            exit(2);
        }
    }
    let cmd = args[0].as_str();
    let _ = ENVELOPE_CMD.set(cmd.to_string());
    let rest = &args[1..];

    let result = match cmd {
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        "-V" | "--version" => {
            println!("brain {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => match SUBCOMMANDS.iter().find(|sub| sub.name == cmd) {
            Some(sub) => {
                if json_mode() && !sub.json {
                    eprintln!("error: --json is not supported for subcommand '{cmd}'");
                    exit(2);
                }
                (sub.run)(rest)
            }
            None => {
                eprintln!("error: unknown subcommand '{cmd}'");
                print_usage();
                exit(2);
            }
        },
    };

    if let Err(e) = result {
        if json_mode() && !ENVELOPE_EMITTED.swap(false, std::sync::atomic::Ordering::SeqCst) {
            // The command did not emit its own fail envelope (a pre-parse
            // error); main emits the generic one so stdout stays one object.
            let cmd = ENVELOPE_CMD.get().map(|s| s.as_str()).unwrap_or("");
            println!(
                "{}",
                serde_json::json!({
                    "ok": false,
                    "cmd": cmd,
                    "error": { "code": "error", "message": e }
                })
            );
        }
        if !json_mode() {
            eprintln!("error: {e}");
        }
        let usage = LAST_ERR_IS_USAGE.swap(false, std::sync::atomic::Ordering::SeqCst);
        exit(if usage { 2 } else { 1 });
    }
}

fn print_usage() {
    let text = usage_text();
    println!("{text}");
}

/// The full help text, generated from the SUBCOMMANDS table — the dispatch
/// set and the help can never drift.
fn usage_text() -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "brain — client for brain-server (default {DEFAULT_URL}; override with BRAIN_URL)\n\n"
    ));
    out.push_str("usage:\n");
    for sub in SUBCOMMANDS {
        out.push_str(&format!("  {}\n", sub.usage));
    }
    out.push_str("\nflags:\n");
    out.push_str("  --json     one machine envelope per call (data commands: query, explain,\n");
    out.push_str("             get, ingest-dir, suggest, suggest-metrics, retention,\n");
    out.push_str("             snapshot-status, connector-status, status, eval)\n");
    out.push_str("  --dry-run  simulate, change nothing\n");
    out.push_str("  --yes      auto-confirm prompts;   --force  override checks\n");
    out.push_str("\nexit codes:\n");
    out.push_str("  0  ok (or --help/--version)\n");
    out.push_str("  1  runtime failure (server unreachable, ingest-dir all files failed, ...)\n");
    out.push_str("  2  usage error (unknown subcommand/flag, bad integer value, --json on a\n");
    out.push_str("     non-data subcommand)\n");
    out.push_str("\nfilters:\n");
    out.push_str("  --source S   OR filter over ingest kind (memory | markdown | structured |\n");
    out.push_str("               manual | vault); repeatable. Filters the `source` column, NOT\n");
    out.push_str("               source URIs. Sent as the `sources` list to /recall.\n");
    out.push_str("\nauth:\n");
    out.push_str("  Reads BRAIN_TOKEN_FILE, then BRAIN_TOKEN, then\n");
    out.push_str("  ~/.config/brain-server/auth-token (written by install-service.sh).\n");
    out
}

// ── argument helpers ──────────────────────────────────────────────────────

/// flag vocabulary discipline. Boolean flags NEVER
/// consume the next token (a positional after `--dry-run` stayed swallowed —
/// `brain ingest-dir --dry-run ~/vault` ate the vault); value flags come from
/// the explicit list; `--flag=value` works for both; `--` ends flag parsing
/// (paths starting with `-` survive); an unknown flag is a usage error (exit 2).
type FlagMap = std::collections::HashMap<String, Option<String>>;

fn parse_flags(args: &[String]) -> Result<(Vec<String>, FlagMap), String> {
    let mut positionals = Vec::new();
    let mut flags = FlagMap::new();
    let mut i = 0;
    let mut after_double_dash = false;
    while i < args.len() {
        let a = &args[i];
        if after_double_dash || !a.starts_with('-') || a == "-" {
            positionals.push(a.clone());
            i += 1;
            continue;
        }
        if a == "--" {
            after_double_dash = true;
            i += 1;
            continue;
        }
        let Some(rest) = a.strip_prefix("--") else {
            // Single-dash option (e.g. `-r`): not in the vocabulary.
            return usage_err(format!("unknown flag '{a}'"));
        };
        let (k, v) = match rest.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (rest.to_string(), None),
        };
        if BOOL_FLAGS.contains(&k.as_str()) {
            if v.is_some() {
                return usage_err(format!("flag '--{k}' does not take a value"));
            }
            flags.insert(k, None);
        } else if VALUE_FLAGS.contains(&k.as_str()) {
            let v = match v {
                Some(v) => Some(v),
                None if i + 1 < args.len() && !args[i + 1].starts_with('-') => {
                    i += 1;
                    Some(args[i].clone())
                }
                None => None,
            };
            flags.insert(k, v);
        } else {
            return usage_err(format!("unknown flag '--{k}'"));
        }
        i += 1;
    }
    Ok((positionals, flags))
}

/// The boolean flag vocabulary — never take a value, never eat the next token.
const BOOL_FLAGS: &[&str] = &[
    "dry-run",
    "yes",
    "json",
    "force",
    "explain",
    "flag",
    "graph",
    "purge",
    "r",
    "replace",
    "with-case-status",
    "return",
    "help",
    "version",
];

/// The value flag vocabulary — the next non-dash token (or `=v`) is the value.
const VALUE_FLAGS: &[&str] = &[
    "action",
    "locales",
    "alg",
    "app-id",
    "audit",
    "backup",
    "bin",
    "breach",
    "checkpoint",
    "code",
    "config",
    "confirm",
    "dataset",
    "deletion",
    "dir",
    "domain",
    "exclude",
    "expected-signer",
    "features",
    "file",
    "floor",
    "format",
    "install-id",
    "instance",
    "intent",
    "jurisdiction",
    "k",
    "keep",
    "key-file",
    "kid",
    "kind",
    "note",
    "onward",
    "out",
    "db",
    "base-url",
    "passphrase-file",
    "phrase",
    "profile",
    "reason",
    "repo",
    "retention",
    "session",
    "since",
    "source",
    "step",
    "sub-sub",
    "to",
    "var",
    "webhook-secret-file",
];

/// Exit-code discipline: parse/flag misuse is a usage error (2), distinct from
/// runtime failure (1). Markers are set by `usage_err` and read by `main`.
static LAST_ERR_IS_USAGE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn usage_err<T>(msg: String) -> Result<T, String> {
    LAST_ERR_IS_USAGE.store(true, std::sync::atomic::Ordering::SeqCst);
    Err(msg)
}

/// Mark a runtime `Err(String)` as a usage error (exit 2) — the value-flag
/// validation seam (`--k abc`, bad `--floor`, ...) used before `?`.
fn usage(msg: String) -> String {
    LAST_ERR_IS_USAGE.store(true, std::sync::atomic::Ordering::SeqCst);
    msg
}

/// `--json` envelope mode — every supported data command
/// prints exactly one machine envelope instead of human lines.
static JSON_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn json_mode() -> bool {
    JSON_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Tracks whether the command already emitted its envelope; when it did,
/// main's error handler stays silent (stdout keeps exactly one JSON object).
static ENVELOPE_EMITTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ENVELOPE_CMD: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Emit the `--json` success envelope. The command's typed data (the parsed
/// server response / computed summary) rides in `data`.
fn emit_json_ok(cmd: &str, data: serde_json::Value) -> Result<(), String> {
    if !json_mode() {
        return Ok(());
    }
    ENVELOPE_EMITTED.store(true, std::sync::atomic::Ordering::SeqCst);
    println!(
        "{}",
        serde_json::json!({ "ok": true, "cmd": cmd, "data": data })
    );
    Ok(())
}

/// Emit the `--json` failure envelope (`error.code` + `message`); the caller
/// then returns the runtime Err (exit 1). The `code` distinguishes the
/// documented per-command failure classes (e.g. `all_files_failed`).
fn emit_json_err(cmd: &str, code: &str, message: &str) {
    if json_mode() {
        ENVELOPE_EMITTED.store(true, std::sync::atomic::Ordering::SeqCst);
        println!(
            "{}",
            serde_json::json!({
                "ok": false,
                "cmd": cmd,
                "error": { "code": code, "message": message }
            })
        );
    }
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

/// Build a structured `QueryDoc` body from the parsed CLI flags, lowering
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
) -> Result<String, String> {
    let k = flags.get("k").and_then(|o| o.clone());
    let since = flags.get("since").and_then(|o| o.clone());
    let intent = flags.get("intent").and_then(|o| o.clone());
    let profile = flags.get("profile").and_then(|o| o.clone());

    let mut body = serde_json::json!({ "query": q });
    if let Some(k) = k {
        let n: u32 = k
            .parse()
            .map_err(|_| usage(format!("--k must be an integer, got '{k}'")))?;
        body["limit"] = serde_json::json!(n);
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
    Ok(body.to_string())
}

fn cmd_query(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let q = require_positional(&positionals, "query")?;
    let phrases = multi_flag(args, "phrase");
    let excludes = multi_flag(args, "exclude");
    let codes = multi_flag(args, "code");
    let sources = multi_flag(args, "source");
    let explain = flags.contains_key("explain");

    let body = build_query_doc(&q, &flags, &phrases, &excludes, &codes, &sources, explain)?;

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
    if json_mode() {
        return emit_json_ok("query", value);
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
    let (positionals, flags) = parse_flags(args)?;
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
    if json_mode() {
        return emit_json_ok("explain", value);
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

/// Print the unified `/recall` telemetry block (the envelope that folds the
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
    // Boundary parity: a hit listing is untrusted retrieved memory — emit it
    // inside the shared fence (`wrap_fenced`), never as bare terminal text.
    let mut listing = String::new();
    for (rank, h) in hits.iter().enumerate() {
        let id = h.get("id").and_then(|x| x.as_i64()).unwrap_or(-1);
        let score = h.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0);
        // recalled text is agent-facing — strip the same
        // invisible-Unicode class the server screen + client strip.
        // + markdown-ref strip + control chars
        // (parity with the MCP `tool_result_payload` seam).
        let title = brain_server::fence::strip_markdown_refs(
            &brain_server::strip_invisible::strip_control_chars(
                &brain_server::strip_invisible::strip_invisible(
                    &json_str(h, "title").unwrap_or_else(|| "(untitled)".into()),
                ),
            ),
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
            .map(|s| {
                brain_server::strip_invisible::strip_control_chars(
                    &brain_server::strip_invisible::strip_invisible(&s),
                )
            })
            .map(|s| brain_server::fence::strip_markdown_refs(&s))
            .unwrap_or_default();

        listing.push_str(&format!(
            "{:>3}. [{:.4}] id={} source={}\n     title: {title}\n",
            rank + 1,
            score,
            id,
            source
        ));
        if !snippet.is_empty() {
            listing.push_str(&format!("     {snippet}\n"));
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
                listing.push_str(&format!(
                    "     provenance: vector_rank={:?} fts_rank={:?} fused={:?} rerank={:?} prf_expanded={}\n",
                    vr, fr, fs, rs, prf
                ));
            } else {
                listing.push_str("     provenance: (none returned by server)\n");
            }
        }
    }
    println!("{}", brain_server::fence::wrap_fenced(&listing));
}

fn cmd_get(args: &[String]) -> Result<(), String> {
    let (positionals, _flags) = parse_flags(args)?;
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
    if json_mode() {
        return emit_json_ok("get", v.clone());
    }

    let title =
        brain_server::fence::strip_markdown_refs(&brain_server::strip_invisible::strip_invisible(
            &json_str(&v, "title").unwrap_or_else(|| "(untitled)".into()),
        ));
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
    // Boundary parity with the MCP seam: the full content body is untrusted
    // retrieved memory — one shared fenced envelope.
    println!(
        "{}",
        brain_server::fence::wrap_fenced(&json_str(&v, "content").unwrap_or_default())
    );
    Ok(())
}

fn cmd_ingest_dir(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
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

    // bound the walk so a pathological vault can't blow the ingest
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
        if !json_mode() {
            println!("no ingestable text/markdown files found in {path}");
        }
        return Ok(());
    }

    let mut ingested = 0;
    let mut skipped = 0;
    let mut failed = 0;
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
                if !json_mode() {
                    println!("  skip {}: {e}", rel.display());
                }
                skipped += 1;
                continue;
            }
        };
        if content.trim().is_empty() {
            skipped += 1;
            continue;
        }

        if dry_run {
            ingested += 1;
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
            if !json_mode() {
                println!(
                    "  [dry-run] {} -> {} ({} bytes{meta})",
                    rel.display(),
                    target,
                    content.len(),
                );
            }
            continue;
        }

        let outcome = if is_markdown {
            let title = derive_title(f);
            // send the absolute path as source_path so the server can
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
                    if !json_mode() {
                        println!("  ok   {} -> {}", rel.display(), summarize_ingest(&body));
                    }
                } else {
                    failed += 1;
                    if !json_mode() {
                        println!(
                            "  fail {} ({}): {}",
                            rel.display(),
                            status,
                            truncate(&body, 120)
                        );
                    }
                }
            }
            Err(e) => {
                failed += 1;
                if !json_mode() {
                    println!("  error {}: {e}", rel.display());
                }
            }
        }
    }

    if !json_mode() {
        println!("\ningest-dir complete: {ingested} ingested, {skipped} skipped, {failed} failed");
    }
    // all-fail must exit non-zero in BOTH modes — checked before the `--json`
    // ok-envelope emit so a JSON consumer also sees `ok:false` + exit 1.
    if !files.is_empty() && ingested == 0 && failed > 0 {
        let msg =
            format!("every file failed: {failed} failed, {skipped} skipped, {ingested} ingested");
        emit_json_err("ingest-dir", "all_files_failed", &msg);
        return Err(msg);
    }
    if json_mode() {
        return emit_json_ok(
            "ingest-dir",
            serde_json::json!({
                "files": files.len(),
                "ingested": ingested,
                "skipped": skipped,
                "failed": failed,
                "dry_run": dry_run,
            }),
        );
    }
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
/// sync. Live incremental reconcile needs the streaming-sync tier (future work).
fn cmd_reconcile(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
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
/// operator-facing shortcut for the most common
/// consolidation case. POSTs one `{from:new, to:old, kind:"supersedes"}` link
/// to `/consolidate/apply`; the server expires `old_id` (sets `valid_to=now`)
/// atomically. The old chunk stays retrievable via `/recall?at=<past>`.
fn cmd_resolve(args: &[String]) -> Result<(), String> {
    let (positionals, _flags) = parse_flags(args)?;
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
/// bulk-relabel chunks across domains via `POST /domains/move`.
/// This is the non-re-ingest fix for the 99%-in-`global` corpus: relabels
/// `knowledge.domain`, recomputes the affected centroids, and leaves the
/// content (and its embedding) untouched. Moving rows OUT of `global` needs
/// `--confirm global` (typo-replay guard, mirror of `DELETE /domains/{name}`).
fn cmd_domain_move(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
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
        println!(
            "  note: these were in 'global'; still retrievable via the global domain's historical paths"
        );
    }
    Ok(())
}

/// `brain domains-recompute`: one-shot recompute of every known
/// domain's centroid via `POST /domains/recompute`. Run once right after
/// the centroid source is corrected (before any auto-routed ingest
/// accumulates) so auto-route sees real centroids, and again after
/// `domain-move` passes.
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
    let Some(rows) = v.get("recomputed").and_then(|a| a.as_array()) else {
        return Err("unexpected response shape (missing recomputed[])".into());
    };
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

/// `brain undo-resolve <old_id> [<old_id> ...]`: reverse prior
/// supersession resolutions. The roadmap exit criterion's undo arm: "reject
/// or undo them without retrieval regression." For each `old_id`, clears
/// `valid_to` back to NULL + removes the `supersedes` link, restoring the
/// chunk to current recall.
fn cmd_undo_resolve(args: &[String]) -> Result<(), String> {
    let (positionals, _flags) = parse_flags(args)?;
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
    if let Some(r) = rejected
        && !r.is_empty()
    {
        println!("  rejected: {r:?}");
    }
    Ok(())
}

/// `brain check-consistency`: surface unresolved contradictions.
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
    // stale sources + near-duplicates.
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
    let (positionals, flags) = parse_flags(args)?;
    let id_str = require_positional(&positionals, "id")?;
    let id: i64 = id_str
        .parse()
        .map_err(|_| format!("id must be an integer, got '{id_str}'"))?;

    if !flags.contains_key("yes") {
        let a = read_line(&format!(
            "Delete source {id} and all its chunks from retrieval? [y/N]"
        ))?;
        if !a.eq_ignore_ascii_case("y") {
            println!("aborted (nothing changed)");
            return Ok(());
        }
    }

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

// ── opt-in anticipation CLI ────────────────────────────────

/// `brain suggest "<context>"`: opt-in pull for related-but-not-surfaced
/// chunks. The caller explicitly asks "what else might be relevant?" — the
/// server never pushes. Each hit is tagged `reason: "anticipated"` so the
/// consuming agent may ignore it.
fn cmd_suggest(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
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
    if json_mode() {
        return emit_json_ok("suggest", v);
    }
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
        // strip parity with `print_hits`/`get` — a
        // recalled payload cannot forge UI text or fence markers.
        let strip = |s: &str| {
            brain_server::fence::strip_markdown_refs(
                &brain_server::strip_invisible::strip_control_chars(
                    &brain_server::strip_invisible::strip_invisible(s),
                ),
            )
        };
        let title = strip(h["title"].as_str().unwrap_or(""));
        let content = strip(h["content"].as_str().unwrap_or(""));
        println!("  [{id}] score={score:.3} {title}");
        println!("        {}", truncate(&content, 120));
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
    let (positionals, flags) = parse_flags(args)?;
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
/// This is the roadmap exit criterion, made queryable. Optional
/// `--session` / `--since` filter the window.
fn cmd_suggest_metrics(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args)?;
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
    if json_mode() {
        return emit_json_ok("suggest-metrics", v);
    }
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
        if json_mode() {
            return emit_json_ok("retention", v);
        }
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
        if json_mode() {
            return emit_json_ok("retention", v);
        }
        let updated = v["updated"].as_u64().unwrap_or(0);
        println!("retention policy updated: {updated} row(s) for {kind} -> {days} days");
        return Ok(());
    }
    Err("usage: brain retention get | set <kind> <days>".into())
}

// ── the use-case onboarding wizard ─────────────────────

/// Pure: render a profile's knobs as the wizard's confirm lines (one knob per
/// line, `null` retention shown as "no decay"). Unit-tested so the wizard's
/// displayed knobs can't drift from what the server stores.
fn render_knobs(p: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(s) = p["default_access_scope"].as_str() {
        out.push(format!("default access scope: {s}"));
    }
    if let Some(m) = p["pii_mode"].as_str() {
        out.push(format!("pii mode:              {m}"));
    }
    if let Some(obj) = p["retention"].as_object() {
        if obj.is_empty() {
            out.push("retention:             no decay (empty policy)".into());
        } else {
            let parts: Vec<String> = obj
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{k}={}",
                        v.as_i64()
                            .map(|d| format!("{d}d"))
                            .unwrap_or_else(|| "no-decay".into())
                    )
                })
                .collect();
            out.push(format!("retention:             {}", parts.join(", ")));
        }
    }
    if let Some(a) = p["audit_level"].as_str() {
        out.push(format!("audit level:           {a}"));
    }
    if let Some(ks) = p["kinds"].as_array() {
        let parts: Vec<&str> = ks.iter().filter_map(|k| k.as_str()).collect();
        out.push(format!("allowed kinds:         {}", parts.join(", ")));
    }
    if let Some(cs) = p["connectors_allowed"].as_array() {
        let parts: Vec<&str> = cs.iter().filter_map(|c| c.as_str()).collect();
        let joined = if parts.is_empty() {
            "(none — air-gap)".to_string()
        } else {
            parts.join(", ")
        };
        out.push(format!("connectors allowed:    {joined}"));
    }
    if let Some(h) = p["legal_hold_default"].as_bool() {
        out.push(format!("legal hold default:    {h}"));
    }
    out
}

/// Read one trimmed line from stdin (the wizard's prompts).
fn read_line(prompt: &str) -> Result<String, String> {
    use std::io::Write as _;
    print!("{prompt} ");
    let _ = std::io::stdout().flush();
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .map_err(|e| format!("stdin: {e}"))?;
    Ok(buf.trim().to_string())
}

/// `brain setup [domain] [--profile NAME] [--yes]` — the use-case wizard:
/// pick a preset, see the knobs it sets, bind it to a domain. The output is a
/// live configured store in under a minute — no feature tours. Non-interactive
/// form (`--profile` + `--yes`) is the scriptable path. The profile sets
/// DEFAULTS; an explicit row value always wins; existing rows are untouched.
fn cmd_setup(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let domain = positionals
        .first()
        .cloned()
        .unwrap_or_else(|| "global".to_string());
    let chosen = flags.get("profile").and_then(|o| o.clone());
    let yes = flags.contains_key("yes");

    // The pick list (seeded presets + operator clones).
    let resp = get(&base_url(), "/profiles", &[], auth_token().as_deref())?;
    if resp.status != 200 {
        return Err(format!(
            "server returned status {}: {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
    let profiles: Vec<serde_json::Value> = v["profiles"]
        .as_array()
        .cloned()
        .ok_or("server returned no profile list")?;
    if profiles.is_empty() {
        return Err("server has no profiles (migration did not seed presets?)".into());
    }

    let pick = match &chosen {
        Some(name) => profiles
            .iter()
            .find(|p| p["name"].as_str() == Some(name.as_str()))
            .cloned()
            .ok_or_else(|| format!("no profile named '{name}'"))?,
        None => {
            // The wizard step: "What best describes your team?"
            println!("What best describes your team?");
            for (i, p) in profiles.iter().enumerate() {
                let name = p["name"].as_str().unwrap_or("?");
                let desc = p["description"].as_str().unwrap_or("");
                println!("  {:>2}. {name:<20} {desc}", i + 1);
            }
            loop {
                let a = read_line("Pick a number (q to quit):")?;
                if a == "q" || a.is_empty() {
                    return Ok(());
                }
                let n: usize = a
                    .parse()
                    .map_err(|_| format!("enter a number 1-{} or q", profiles.len()))?;
                if n >= 1 && n <= profiles.len() {
                    break profiles[n - 1].clone();
                }
                eprintln!("  (pick 1-{})", profiles.len());
            }
        }
    };
    let name = pick["name"].as_str().unwrap_or("?").to_string();

    println!("\nProfile '{name}' sets these knobs for domain '{domain}':");
    for line in render_knobs(&pick) {
        println!("  {line}");
    }
    println!("  (defaults only — an explicit row value always wins)");

    if !yes {
        let a = read_line(&format!("Bind '{name}' to '{domain}'? [y/N]"))?;
        if !a.eq_ignore_ascii_case("y") {
            println!("aborted (nothing changed)");
            return Ok(());
        }
    }

    let body = serde_json::json!({ "profile": name }).to_string();
    let path = format!("/domains/{domain}/profile");
    let resp = post(
        &base_url(),
        &path,
        &[],
        "application/json",
        &body,
        auth_token().as_deref(),
    )?;
    if resp.status != 200 {
        return Err(format!(
            "bind failed (status {}): {}",
            resp.status,
            truncate(&resp.body, 200)
        ));
    }
    println!("\nbound: domain '{domain}' → profile '{name}'");
    println!("next: ingest something and check the Health panel (brain doctor / brain status)");
    Ok(())
}

/// `brain snapshot-status` — run the snapshot self-check panel and exit
/// `brain ump export|import` — the UMP §4.3 file binding. Export pulls
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

/// `brain client add ...` — the "Onboard" wizard: POST one compose call that
/// scaffolds the client's domain, binds its law-tuned profile, and registers
/// the `clients` row (src/handlers/clients.rs `register_client`).
fn cmd_client(args: &[String]) -> Result<(), String> {
    match args.first().map(|s| s.as_str()) {
        Some("add") => cmd_client_add(&args[1..]),
        Some("dpa") => cmd_client_dpa(&args[1..]),
        Some("dsar") => cmd_client_dsar(&args[1..]),
        Some("hold") => cmd_client_hold(&args[1..]),
        Some("qa") => cmd_client_qa(&args[1..]),
        Some("end") => cmd_client_end(&args[1..]),
        _ => Err(
            "usage: brain client add <name> --domain D --jurisdiction J [--profile P] [--yes]\n       brain client dpa get <name> | set <name> --retention R --deletion D --audit A --breach B --onward O --sub-sub S\n       brain client dsar <name> <subject> [--action purge|export|both] [--dry-run]\n       brain client hold add <name> <id> [<id> ...] --reason R | list <name>\n       brain client qa list <name> | coach <name> <id> --note N [--flag]\n       brain client end <name> [--purge|--return] [--dataset D] [--yes]"
                .into(),
        ),
    }
}

fn cmd_client_qa(args: &[String]) -> Result<(), String> {
    let cmd = args.first().map(|s| s.as_str()).ok_or_else(|| {
        "usage: brain client qa list <name> | coach <name> <id> --note N [--flag]".to_string()
    })?;
    let (positionals, flags) = parse_flags(&args[1..])?;
    let name = require_positional(&positionals, "name")?;
    match cmd {
        "list" => {
            let resp = get(
                &base_url(),
                &format!("/clients/{}/proposals", url_encode(&name)),
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
            println!("{}", resp.body);
            Ok(())
        }
        "coach" => {
            let id: i64 = positionals
                .get(1)
                .ok_or_else(|| "missing required argument: id".to_string())?
                .parse::<i64>()
                .map_err(|e| format!("invalid id: {e}"))?;
            let note = flags.get("note").and_then(|o| o.clone());
            let flagged = flags.contains_key("flag");
            let body = serde_json::json!({ "flagged": flagged, "note": note });
            let resp = post(
                &base_url(),
                &format!("/clients/{}/proposals/{id}/coach", url_encode(&name)),
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
            println!("{}", resp.body);
            Ok(())
        }
        _ => Err("usage: brain client qa list <name> | coach <name> <id> --note N [--flag]".into()),
    }
}

fn cmd_client_hold(args: &[String]) -> Result<(), String> {
    let cmd = args.first().map(|s| s.as_str()).ok_or_else(|| {
        "usage: brain client hold add <name> <id> [<id> ...] --reason R | list <name>".to_string()
    })?;
    let (positionals, flags) = parse_flags(&args[1..])?;
    let name = require_positional(&positionals, "name")?;
    match cmd {
        "add" => {
            let ids: Result<Vec<i64>, String> = positionals[1..]
                .iter()
                .map(|s| s.parse::<i64>().map_err(|_| format!("invalid id: {s}")))
                .collect();
            let ids = ids?;
            let reason = flags
                .get("reason")
                .and_then(|o| o.clone())
                .ok_or_else(|| "missing required argument: --reason R".to_string())?;
            let body = serde_json::json!({ "ids": ids, "reason": reason });
            let resp = post(
                &base_url(),
                &format!("/clients/{}/hold", url_encode(&name)),
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
            println!("{}", resp.body);
            Ok(())
        }
        "list" => {
            let resp = get(
                &base_url(),
                &format!("/clients/{}", url_encode(&name)),
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
            let client: serde_json::Value =
                serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
            let domain = client
                .get("domain")
                .and_then(|d| d.as_str())
                .ok_or("client response has no domain")?;
            let resp = get(&base_url(), "/legal-holds", &[], auth_token().as_deref())?;
            if resp.status != 200 {
                return Err(format!(
                    "server returned status {}: {}",
                    resp.status,
                    truncate(&resp.body, 200)
                ));
            }
            let list: serde_json::Value =
                serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
            let holds = list
                .get("holds")
                .and_then(|h| h.as_array())
                .cloned()
                .unwrap_or_default();
            let scoped: Vec<serde_json::Value> = holds
                .into_iter()
                .filter(|h| h["domain"] == domain)
                .collect();
            println!(
                "{}",
                serde_json::json!({ "domain": domain, "holds": scoped })
            );
            Ok(())
        }
        _ => Err(
            "usage: brain client hold add <name> <id> [<id> ...] --reason R | list <name>".into(),
        ),
    }
}

fn cmd_client_dsar(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let name = require_positional(&positionals, "name")?;
    let subject = positionals
        .get(1)
        .ok_or_else(|| "missing required argument: subject".to_string())?;
    let action = flags
        .get("action")
        .and_then(|o| o.clone())
        .unwrap_or_else(|| "purge".to_string());
    let dry_run = match flags.get("dry-run").and_then(|o| o.as_deref()) {
        Some("false") | Some("0") => false,
        _ => flags.contains_key("dry-run"),
    };
    let body = serde_json::json!({
        "subject": subject,
        "action": action,
        "dry_run": dry_run,
    });
    let path = format!("/clients/{}/dsar", url_encode(&name));
    let resp = post(
        &base_url(),
        &path,
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
    println!("{}", resp.body);
    Ok(())
}

fn cmd_client_end(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let name = require_positional(&positionals, "name")?;
    let purge_opt: Option<bool> = if flags.contains_key("purge") && flags.contains_key("return") {
        return Err("cannot pass both --purge and --return".to_string());
    } else if flags.contains_key("purge") {
        Some(true)
    } else if flags.contains_key("return") {
        Some(false)
    } else {
        None
    };
    let dataset = flags
        .get("dataset")
        .and_then(|o| o.clone())
        .unwrap_or_else(|| "termination".to_string());
    let yes = flags.contains_key("yes");
    let mode = match purge_opt {
        Some(true) => "and PURGE its data",
        Some(false) => "and EXPORT its data (no purge)",
        None => "(policy from its DPA terms)",
    };
    if !yes {
        let a = read_line(&format!("end contract for client '{name}' {mode}? [y/N]"))?;
        if !a.eq_ignore_ascii_case("y") {
            println!("aborted (nothing changed)");
            return Ok(());
        }
    }
    let mut body = serde_json::json!({ "dataset": dataset });
    if let Some(p) = purge_opt {
        body["purge"] = serde_json::json!(p);
    }
    let resp = post(
        &base_url(),
        &format!("/clients/{}/end", url_encode(&name)),
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
    println!("{}", resp.body);
    Ok(())
}

fn cmd_client_dpa(args: &[String]) -> Result<(), String> {
    let cmd = args
        .first()
        .map(|s| s.as_str())
        .ok_or_else(|| {
            "usage: brain client dpa get <name> | set <name> --retention R --deletion D --audit A --breach B --onward O --sub-sub S"
                .to_string()
        })?;
    let (positionals, flags) = parse_flags(&args[1..])?;
    let name = require_positional(&positionals, "name")?;
    let path = format!("/clients/{}/dpa", url_encode(&name));
    match cmd {
        "get" => {
            let resp = get(&base_url(), &path, &[], auth_token().as_deref())?;
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
        "set" => {
            let mappings = [
                ("retention", "retention_on_termination"),
                ("deletion", "deletion_timeline"),
                ("audit", "audit_rights"),
                ("breach", "breach_notification_timeline"),
                ("onward", "onward_transfer_restriction"),
                ("sub-sub", "sub_sub_processor_list"),
            ];
            let mut body = serde_json::json!({});
            for (flag, field) in mappings {
                let v = flags
                    .get(flag)
                    .and_then(|o| o.clone())
                    .ok_or_else(|| format!("missing required argument: --{flag} VALUE"))?;
                body[field] = serde_json::json!(v);
            }
            let resp = post(
                &base_url(),
                &path,
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
            println!("dpa terms set for '{name}'");
            Ok(())
        }
        _ => Err(
            "usage: brain client dpa get <name> | set <name> --retention R --deletion D --audit A --breach B --onward O --sub-sub S"
                .into(),
        ),
    }
}

fn cmd_client_add(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let name = require_positional(&positionals, "name")?;
    let jurisdiction = flags
        .get("jurisdiction")
        .and_then(|o| o.clone())
        .ok_or_else(|| "missing required argument: --jurisdiction J".to_string())?;
    let domain = flags
        .get("domain")
        .and_then(|o| o.clone())
        .unwrap_or_else(|| name.clone());
    // Profile optional: absent → the `cmd_setup` wizard pick (list + numbered
    // prompt), confirmed unless `--yes`.
    let profile = match flags.get("profile").and_then(|o| o.clone()) {
        Some(p) => Some(p),
        None => {
            let resp = get(&base_url(), "/profiles", &[], auth_token().as_deref())?;
            if resp.status != 200 {
                return Err(format!(
                    "server returned status {}: {}",
                    resp.status,
                    truncate(&resp.body, 200)
                ));
            }
            let v: serde_json::Value =
                serde_json::from_str(&resp.body).map_err(|e| format!("non-JSON response: {e}"))?;
            let profiles: Vec<serde_json::Value> = v["profiles"]
                .as_array()
                .cloned()
                .ok_or("server returned no profile list")?;
            if profiles.is_empty() {
                return Err("server has no profiles (migration did not seed presets?)".into());
            }
            println!("Bind a profile to domain '{domain}'?");
            for (i, p) in profiles.iter().enumerate() {
                let pname = p["name"].as_str().unwrap_or("?");
                let desc = p["description"].as_str().unwrap_or("");
                println!("  {:>2}. {pname:<20} {desc}", i + 1);
            }
            let a = read_line("Pick a number, or leave blank to skip:")?;
            if a.trim().is_empty() {
                None
            } else {
                let n: usize = a
                    .parse()
                    .map_err(|_| format!("enter a number 1-{} or blank", profiles.len()))?;
                if !(1..=profiles.len()).contains(&n) {
                    return Err(format!("pick 1-{}", profiles.len()));
                }
                profiles[n - 1]
                    .get("name")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            }
        }
    };

    if !flags.contains_key("yes") && profile.is_some() {
        let a = read_line(&format!(
            "Add client '{name}' (domain '{domain}', jurisdiction '{jurisdiction}'{} )? [y/N]",
            profile
                .as_ref()
                .map(|p| format!(", profile '{p}'"))
                .unwrap_or_default()
        ))?;
        if !a.eq_ignore_ascii_case("y") {
            println!("aborted (nothing changed)");
            return Ok(());
        }
    }

    let mut body = serde_json::json!({
        "name": name,
        "domain": domain,
        "jurisdiction": jurisdiction,
    });
    if let Some(p) = &profile {
        body["profile"] = serde_json::json!(p);
    }
    let resp = post(
        &base_url(),
        "/clients",
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
    let tail = profile
        .map(|p| format!(", profile '{p}'"))
        .unwrap_or_default();
    println!("client '{name}' registered \u{2192} domain '{domain}'{tail}");
    Ok(())
}

/// `brain ump keygen` — generate an Ed25519 operator key for the UMP
/// identity surface (§5). Writes a raw 32-byte seed to
/// `<dir>/operator.key` (0600) and prints the `did:key`. The server reads
/// any seed file in `BRAIN_UMP_KEY_DIR` (default `~/.config/brain-server/ump/`).
fn cmd_ump_keygen(args: &[String]) -> Result<String, String> {
    let (_positionals, flags) = parse_flags(args)?;
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
    use rand::{TryRng, rngs::SysRng};
    let mut seed = [0u8; 32];
    SysRng
        .try_fill_bytes(&mut seed)
        .expect("OS entropy source failed");
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
    if json_mode() {
        return emit_json_ok("snapshot-status", v);
    }
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

// ── procedural-memory + categorization CLI ──────────

/// `brain procedure <title> [--step "title: content" ...] [--domain D]`:
/// ingest a procedure with ordered steps. Each `--step` is `"title: content"`
/// (colon-separated); step order = flag order. The procedure root + steps are
/// stored as `procedure`/`step`-kind chunks linked by `next_step` edges.
fn cmd_procedure(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
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
    let (positionals, _flags) = parse_flags(args)?;
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
    let (positionals, _flags) = parse_flags(args)?;
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

// ── Bridge: connector CLI ────────────────────────────────────────────

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
/// remotely; the operator surface stays on the host that runs them.
fn cmd_connect(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args)?;
    let kind = flags
        .get("kind")
        .and_then(|o| o.as_deref())
        .unwrap_or("github")
        .to_string();
    if kind != "github" {
        return Err(format!(
            "v1.24.0 ships the {kind} connector kind in the registry (supervised \
             ingest-on-trigger; register via POST /connectors/register), but no \
             runnable binary yet — the v0.9.6 github connector is the working \
             backfill template. Shipped kinds: {}.",
            brain_server::connector::kind::CONNECTOR_KINDS.join(", ")
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
    if let Ok(db_path) = std::env::var("BRAIN_DB_PATH")
        && let Ok(conn) = rusqlite::Connection::open(&db_path)
    {
        brain_server::audit::record(
            &conn,
            brain_server::audit::AuditKind::Connector,
            "github",
            &instance,
            brain_server::audit::AuditStatus::Ok,
            "connect config written",
        );
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
/// server-side auto-start.
fn cmd_sync(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args)?;
    let kind = flags
        .get("kind")
        .and_then(|o| o.as_deref())
        .or_else(|| _positionals.first().map(|s| s.as_str()))
        .unwrap_or("github");
    if kind != "github" {
        return Err(format!(
            "kind '{kind}' has no sync binary yet — the v1.24 registry ships the \
             vocabulary (POST /connectors/register); the github backfill is the \
             working template. Shipped kinds: {}.",
            brain_server::connector::kind::CONNECTOR_KINDS.join(", ")
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

/// `brain backup <out-path> [--passphrase-file PATH] [--format v1|v2|v3]`
fn cmd_backup(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let out = positionals.first().cloned().ok_or_else(|| {
        "usage: brain backup <out-path> [--passphrase-file PATH] [--format v1|v2|v3]".to_string()
    })?;
    let pass = resolve_passphrase(&flags)?;
    let format = match flags.get("format").and_then(|o| o.clone()).as_deref() {
        None | Some("v3") => brain_server::backup::BackupFormat::V3,
        Some("v2") => brain_server::backup::BackupFormat::V2,
        Some("v1") => brain_server::backup::BackupFormat::V1,
        Some(v) => return Err(format!("unknown backup format {v:?} (use v1, v2 or v3)")),
    };
    let db = default_db_path();
    brain_server::backup::backup_with_config_dir_and_format(
        &db,
        Path::new(&out),
        &pass,
        None,
        format,
    )
    .map_err(|e| format!("backup failed: {e:#}"))?;
    println!("backup written: {out} (+ {out}.sha256 checksum)");
    Ok(())
}

/// `brain restore <in-path> [--passphrase-file PATH]`
fn cmd_restore(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let in_path = positionals.first().cloned().ok_or_else(|| {
        "usage: brain restore <in-path> [--passphrase-file PATH] [--force]".to_string()
    })?;
    let pass = resolve_passphrase(&flags)?;
    let db = default_db_path();
    // Split-brain guard: restoring while the launchd service holds the DB
    // open leaves the server writing the OLD inode — new connections see the
    // restored file, existing ones keep the pre-restore world. Refuse unless
    // the operator says --force.
    if !flags.contains_key("force") && brain_server_reachable() {
        return Err(
            "brain-server appears to be RUNNING (a listener answered on its port). \
             Stop it first: launchctl unload ~/Library/LaunchAgents/com.brain.server.plist \
             — or pass --force to restore anyway."
                .to_string(),
        );
    }
    brain_server::backup::restore(Path::new(&in_path), &db, &pass)
        .map_err(|e| format!("restore failed: {e:#}"))?;
    println!("restored: {db:?} (safety snapshot saved as <db>.bak)");
    Ok(())
}

/// Best-effort liveness probe for the split-brain guard: a TCP connect to
/// the configured base URL's host:port within 500ms means SOMETHING is
/// listening there. Unreachable / unparseable URL = not running.
fn brain_server_reachable() -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    let base = base_url();
    let host_port = base
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("127.0.0.1:8765")
        .to_string();
    let addr = if host_port.contains(':') {
        host_port
    } else {
        format!("{host_port}:8765")
    };
    use std::net::ToSocketAddrs;
    addr.to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_millis(500)).is_ok())
}

// ── JWT signing key management ──────────────────────────────
// Local-file operations — no server roundtrip. The server picks up new keys
// on restart (hot-reload via KeyStore::reload is a follow-up; the rotation
// watcher pattern from the old token store is the template).

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

/// `brain token rotate` — replace the shared static bearer token. Subcommands:
///   rotate  — generate a fresh random token + atomically rewrite the token file.
fn cmd_token(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: brain token <rotate>".to_string());
    }
    match args[0].as_str() {
        "rotate" => cmd_token_rotate(&args[1..]),
        other => Err(format!(
            "unknown 'brain token' subcommand: '{other}' (try rotate)"
        )),
    }
}

/// Mirror `auth_token`'s file resolution (BRAIN_TOKEN_FILE → default install
/// path) but return the PATH, for rotation. Env-only (`BRAIN_TOKEN`) sources
/// have no file to rewrite.
fn token_file_path() -> PathBuf {
    match std::env::var("BRAIN_TOKEN_FILE") {
        Ok(s) if !s.trim().is_empty() => PathBuf::from(s.trim()),
        _ => dirs_home().join(".config/brain-server/auth-token"),
    }
}

/// replace the shared static bearer token file so a leaked
/// copy (e.g. the historical openclaw DB rows) is retired on the next reload.
/// The running server + every file-reading consumer (brain/mcp/bench) pick the
/// new value up through their rotation watchers within a poll interval (~5s).
/// Atomic write + owner-only perms; refuses to rewrite a group/world-readable
/// secret (fail-closed, mirroring the server's `check_secret_permissions`).
///
/// ponytail: rotates the SERVER-side token FILE only. The openclaw plugin's
/// `authToken` is usually an env reference (`${BRAIN_SERVER_AUTH_TOKEN}`); this
/// CLI does NOT edit the live openclaw config it does not own — it prints that
/// the env value must be applied to match. Operator-in-the-loop (mantra #3): a
/// non-human actor never rewrites the shared secret unilaterally.
fn cmd_token_rotate(_args: &[String]) -> Result<(), String> {
    let path = token_file_path();
    rotate_token_file_at(&path)?;
    println!("rotated bearer token in {path:?}");
    println!("the running server + file-reading consumers reload it within ~5s (rotation poll).");
    println!(
        "keep the openclaw plugin in step: apply this token to BRAIN_SERVER_AUTH_TOKEN \
         (its auth source) if that is set."
    );
    Ok(())
}

fn rotate_token_file_at(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!(
            "token file {path:?} does not exist — nothing to rotate"
        ));
    }
    // Fail-closed: never rewrite a secret with group/world bits.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|e| format!("stat {path:?}: {e}"))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(format!(
                "token file {path:?} is group/world-accessible (mode {:o}) — chmod 0600 before rotating",
                mode & 0o777
            ));
        }
    }
    let new_token = random_hex_token();
    // Atomic replace: create a sibling temp already at 0600 (never umask-
    // dependent — the secret must not exist with broader perms for even a
    // moment), write, fsync, then rename over the target so a reader never
    // observes a partially-written token.
    let tmp = path.with_file_name(format!(".auth-token.rotate.{}.tmp", std::process::id()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true).mode(0o600);
        let mut f = opts
            .open(&tmp)
            .map_err(|e| format!("create {tmp:?}: {e}"))?;
        use std::io::Write;
        f.write_all(new_token.as_bytes())
            .map_err(|e| format!("write {tmp:?}: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync {tmp:?}: {e}"))?;
    }
    #[cfg(not(unix))]
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("create {tmp:?}: {e}"))?;
        f.write_all(new_token.as_bytes())
            .map_err(|e| format!("write {tmp:?}: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync {tmp:?}: {e}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))?;
    Ok(())
}

/// 32 random bytes hex-encoded (64 hex chars). Local (no `hex` dep): the token
/// is a high-entropy bearer secret, matching the server's opaqueness.
fn random_hex_token() -> String {
    use rand::{TryRng, rngs::SysRng};
    let mut bytes = [0u8; 32];
    SysRng
        .try_fill_bytes(&mut bytes)
        .expect("OS entropy source failed");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
    let (_positionals, flags) = parse_flags(args)?;
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
    use rsa::RsaPrivateKey;
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
    let mut rng = rand::rngs::ThreadRng::default();
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
    println!(
        "restart brain-server to load the new key; existing tokens stay valid until the old key is pruned"
    );
    Ok(())
}

/// `brain key list` — list every key in the dir (kid, has-private, size).
fn cmd_key_list(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args)?;
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
    let (_positionals, flags) = parse_flags(args)?;
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
    if json_mode() {
        return emit_json_ok("connector-status", v);
    }
    let connectors = v
        .get("connectors")
        .and_then(|x| x.as_array())
        .ok_or_else(|| "response missing 'connectors' array".to_string())?;
    if connectors.is_empty() {
        println!("no connectors registered");
        println!("\nregister one with:");
        println!(
            "  brain connect github --app-id N --install-id N --key-file PATH --repo owner/name"
        );
        println!(
            "  POST /connectors/register  (any v1.24 kind: {})",
            brain_server::connector::kind::CONNECTOR_KINDS.join(", ")
        );
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
    if json_mode() {
        return emit_json_ok("status", v);
    }
    println!("brain-server status");
    println!(
        "  version : {}",
        json_str(&v, "version").unwrap_or_default()
    );
    println!("  model   : {}", json_str(&v, "model").unwrap_or_default());
    println!(
        "  documents   : {}",
        fmt_count(v.get("count").and_then(|x| x.as_i64()).unwrap_or(-1))
    );
    println!(
        "  embeddings  : {}",
        fmt_count(v.get("embeddings").and_then(|x| x.as_i64()).unwrap_or(-1))
    );
    println!(
        "  entities    : {}",
        fmt_count(v.get("entities").and_then(|x| x.as_i64()).unwrap_or(-1))
    );
    println!(
        "  relationships: {}",
        fmt_count(
            v.get("relationships")
                .and_then(|x| x.as_i64())
                .unwrap_or(-1)
        )
    );
    Ok(())
}

/// `/stats` `-1` sentinels render as `n/a` — a missing field is
/// not a count of minus one chunk.
fn fmt_count(n: i64) -> String {
    if n < 0 {
        "n/a".to_string()
    } else {
        n.to_string()
    }
}

fn cmd_doctor(args: &[String]) -> Result<(), String> {
    // `brain doctor --backup <path> [--passphrase-file PATH]` — verify-only mode.
    let (positionals, flags) = parse_flags(args)?;
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
    // optional recall floors as a ship gate, mirroring the
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

/// `brain eval [--floor r5=0.85 r10=0.9]` — run the frozen
/// judged corpus (`tests/fixtures/eval_queries.md`) against `/recall`, report
/// the metrics, and exit non-zero when any `--floor` is breached. The fixture
/// ships in the repo so the gate is reproducible on any machine with a live
/// server; the operator's private judged corpus remains a separate step.
fn cmd_eval(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args)?;
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
    if json_mode() {
        let floors_json: serde_json::Value = floors
            .iter()
            .map(|(m, v)| serde_json::json!({"metric": m, "floor": v}))
            .collect();
        return emit_json_ok(
            "eval",
            serde_json::json!({ "floors": floors_json, "breached": !ok }),
        );
    }
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
        // GET /search reads `q`+`k` (the legacy params); POST /recall is a
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
        mean[0],
        mean[1],
        mean[2],
        mean[3],
        mean[4],
        mean[5],
        queries.len()
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
        } else if let Some(r) = line.strip_prefix("Relevant:")
            && let Some(mut c) = current.take()
        {
            c.relevant = parse_index_list(r);
            out.push(c);
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

    // ── flag vocabulary + help truth ──────────────────

    #[test]
    fn boolean_flag_does_not_eat_positional() {
        let (pos, flags) = parse_flags(&["--dry-run".into(), "~/vault".into()]).unwrap();
        assert_eq!(pos, vec!["~/vault"]);
        assert_eq!(flags.get("dry-run"), Some(&None));
    }

    #[test]
    fn value_flag_takes_next_token_or_equals() {
        let (pos, flags) = parse_flags(&["--k".into(), "5".into()]).unwrap();
        assert!(pos.is_empty());
        assert_eq!(flags.get("k").and_then(|o| o.clone()).unwrap(), "5");
        let (_, flags2) = parse_flags(&["--k=7".into()]).unwrap();
        assert_eq!(flags2.get("k").and_then(|o| o.clone()).unwrap(), "7");
    }

    #[test]
    fn unknown_flag_errors_as_usage() {
        assert!(parse_flags(&["--bogus".into()]).is_err());
        assert!(LAST_ERR_IS_USAGE.swap(false, std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn boolean_flag_rejects_explicit_value() {
        assert!(parse_flags(&["--dry-run=yes".into()]).is_err());
    }

    #[test]
    fn double_dash_ends_flag_parsing() {
        let (pos, flags) = parse_flags(&["--".into(), "--dry-run".into()]).unwrap();
        assert_eq!(pos, vec!["--dry-run"]);
        assert!(flags.is_empty());
    }

    #[test]
    fn help_text_is_generated_from_the_dispatch_table() {
        let text = usage_text();
        for sub in SUBCOMMANDS {
            assert!(
                text.contains(&format!("  brain {}", sub.name)),
                "help missing subcommand {}",
                sub.name
            );
        }
        // The pre-v1.27.20 flush-left survivor line is gone: every usage line
        // is emitted uniformly with 2-space indent.
        for line in text.lines() {
            assert!(
                !line.starts_with("brain ") || line.contains("brain — client"),
                "flush-left usage line: {line}"
            );
        }
        assert!(text.contains("--json"));
        assert!(text.contains("exit codes:"));
        assert!(!text.contains("all_files_failed"));
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

    /// floor specs parse from either separator; unknown metrics and
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

    /// `brain ump` rejects a missing subcommand and `import`
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

    /// the generated token is 32 random bytes hex-encoded
    /// (64 hex chars) — high-entropy, matching the server's opaque bearer.
    #[test]
    fn random_hex_token_is_64_hex_chars() {
        let t = random_hex_token();
        assert_eq!(t.len(), 64, "32 bytes → 64 hex chars");
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()), "hex only");
        assert_ne!(
            random_hex_token(),
            random_hex_token(),
            "two rotations differ"
        );
    }

    /// `brain token rotate` rewrites an owner-only token file
    /// to a fresh 64-hex token, preserving owner-only perms.
    #[test]
    #[cfg(unix)]
    fn token_rotate_rewrites_owner_only_file_to_fresh_token() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("auth-token");
        std::fs::write(&path, "old-token").expect("write old");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("0600");
        assert!(rotate_token_file_at(&path).is_ok(), "rotate success");
        let new = std::fs::read_to_string(&path).expect("read new");
        assert_eq!(new.trim().len(), 64, "fresh token is 64 hex");
        assert_ne!(new.trim(), "old-token", "old value retired");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "rotated secret stays owner-only");
    }

    /// the CLI refuses to rewrite a group/world-readable
    /// secret (fail-closed mirror of `check_secret_permissions`).
    #[test]
    #[cfg(unix)]
    fn token_rotate_refuses_group_readable_secret() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("auth-token-wide");
        std::fs::write(&path, "old").expect("write old");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("0644");
        assert!(
            rotate_token_file_at(&path).is_err(),
            "must refuse to rewrite a world-readable secret"
        );
    }

    /// `brain ump keygen` writes a 32-byte operator seed (0600)
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

/// the wizard's confirm lines render every knob the
/// chosen preset sets — the display can't drift from the stored shape.
#[test]
fn setup_render_knobs_shows_every_set_knob() {
    let p: serde_json::Value = serde_json::from_str(
        r#"{"name":"call-center","default_access_scope":"private","pii_mode":"standard",
                "retention":{"fact":730,"episodic":90},
                "audit_level":"verbose","kinds":["fact","episodic"],
                "connectors_allowed":["crm"],"legal_hold_default":false}"#,
    )
    .expect("preset parses");
    let lines = render_knobs(&p);
    let all = lines.join("\n");
    for want in [
        "default access scope: private",
        "pii mode:              standard",
        "retention:             episodic=90d, fact=730d",
        "audit level:           verbose",
        "allowed kinds:         fact, episodic",
        "connectors allowed:    crm",
        "legal hold default:    false",
    ] {
        assert!(all.contains(want), "missing {want:?} in:\n{all}");
    }
    // null retention = explicit no-decay for that kind.
    let q: serde_json::Value =
        serde_json::from_str(r#"{"name":"health-hipaa","retention":{"fact":null,"episodic":90}}"#)
            .unwrap();
    let lines = render_knobs(&q).join("\n");
    assert!(lines.contains("episodic=90d, fact=no-decay"), "{lines}");
    // An empty policy = nothing decays (the smb-simple posture).
    let e: serde_json::Value =
        serde_json::from_str(r#"{"name":"smb-simple","retention":{}}"#).unwrap();
    assert!(render_knobs(&e).join("\n").contains("no decay"));
}

fn cmd_workflow(args: &[String]) -> Result<(), String> {
    let usage = "usage: brain workflow open [DOMAIN]\n  \
brain workflow status <run>\n  \
brain workflow answer <run> <text>\n  \
brain workflow approve <run> <step>\n  \
brain workflow crank <run> [steps]\n  \
brain workflow handoff <run>\n  \
brain workflow note <run> <text> [--reask]";
    let sub = args.first().map(String::as_str).unwrap_or("");
    let base = base_url();
    let token = auth_token();
    let rest = args.get(1..).unwrap_or(&[]);
    let json_out = json_mode();
    // One-shot helper: POST JSON, parse the envelope.
    let post_json = |path: &str, body: serde_json::Value| -> Result<serde_json::Value, String> {
        let resp = http::post(
            &base,
            path,
            &[],
            "application/json",
            &body.to_string(),
            token.as_deref(),
        )?;
        serde_json::from_str(&resp.body)
            .map_err(|e| format!("bad response from {path}: {e}: {}", resp.body))
    };
    match sub {
        "open" => {
            let domain = rest.first().cloned().unwrap_or_else(|| "global".into());
            let v = post_json(
                "/workflow/runs",
                serde_json::json!({
                    "domain": domain,
                    "kind": "troubleshoot",
                    "state_json": "{}",
                }),
            )?;
            if json_out {
                return emit_json_ok("workflow", v);
            }
            println!("run {} opened (revision {})", v["run_id"], v["revision"]);
            Ok(())
        }
        "status" => {
            let Some(run) = rest.first() else {
                return Err(usage.into());
            };
            let resp = http::get(
                &base,
                &format!("/workflow/runs/{run}"),
                &[],
                token.as_deref(),
            )?;
            let v: serde_json::Value =
                serde_json::from_str(&resp.body).map_err(|e| format!("bad response: {e}"))?;
            if json_out {
                return emit_json_ok("workflow", v);
            }
            println!(
                "run {run}: status={} revision={} updated_at={}",
                v["status"], v["state_revision"], v["updated_at"]
            );
            Ok(())
        }
        "answer" | "approve" => {
            let Some(run) = rest.first() else {
                return Err(usage.into());
            };
            let text = rest.get(1..).map(|p| p.join(" ")).unwrap_or_default();
            if text.trim().is_empty() {
                return Err("answer text must not be empty".into());
            }
            // Digest-bind to the live pending_question (ReviewArmour at
            // question grain): read engine state, hash the question bytes.
            let st = http::get(
                &base,
                &format!("/workflow/runs/{run}/state"),
                &[],
                token.as_deref(),
            )?;
            let sv: serde_json::Value =
                serde_json::from_str(&st.body).map_err(|e| format!("bad response: {e}"))?;
            let question = sv["state_json"]
                .as_str()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| v["pending_question"].as_str().map(str::to_string))
                .ok_or_else(|| "run has no pending_question".to_string())?;
            let digest = brain_server::audit::hash(&question);
            let body = if sub == "approve" {
                format!("[approved:{text}]")
            } else {
                text
            };
            let v = post_json(
                &format!("/workflow/runs/{run}/answer"),
                serde_json::json!({"answer": body, "question_digest": digest}),
            )?;
            if json_out {
                return emit_json_ok("workflow", v);
            }
            println!("answered run {run} (revision {})", v["revision"]);
            Ok(())
        }
        "crank" => {
            let Some(run) = rest.first() else {
                return Err(usage.into());
            };
            // Resolve the harness binary beside this executable first, then
            // the BRAIN_STEWARD_BIN override, then PATH.
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf));
            let candidate = |dir: &Path| dir.join("steward-harness");
            let bin: PathBuf = std::env::var("BRAIN_STEWARD_BIN")
                .map(PathBuf::from)
                .ok()
                .or_else(|| {
                    exe_dir.and_then(|d| {
                        let p = candidate(&d);
                        p.exists().then_some(p)
                    })
                })
                .or_else(|| which_path("steward-harness"))
                .ok_or_else(|| {
                    "steward-harness binary not found — build tools/steward-harness and install beside `brain` (or set BRAIN_STEWARD_BIN)".to_string()
                })?;
            let mut cmd_line = serde_json::json!({
                "cmd": "crank",
                "run_id": run.parse::<i64>().map_err(|_| "run id must be an integer")?,
            });
            if let Some(steps) = rest.get(1) {
                cmd_line["max_steps"] = serde_json::json!(
                    steps
                        .parse::<u32>()
                        .map_err(|_| "steps must be a positive integer")?
                );
            }
            let mut child = std::process::Command::new(&bin)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("spawn {}: {e}", bin.display()))?;
            {
                use std::io::Write as _;
                let stdin = child.stdin.as_mut().expect("piped stdin captured above");
                writeln!(stdin, "{cmd_line}").map_err(|e| format!("write stdin: {e}"))?;
            }
            let out = child
                .wait_with_output()
                .map_err(|e| format!("wait harness: {e}"))?;
            let v: serde_json::Value =
                serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
            if json_out {
                return emit_json_ok("workflow", v);
            }
            println!(
                "crank run {run}: stopped_at={} steps_executed={}",
                v["stopped_at"], v["steps_executed"]
            );
            Ok(())
        }
        "note" => {
            let reask = rest.iter().any(|a| a == "--reask");
            let plain: Vec<&String> = rest.iter().filter(|a| !a.starts_with('-')).collect();
            let Some(run) = plain.first() else {
                return Err(usage.into());
            };
            let text: String = plain
                .get(1..)
                .unwrap_or(&[])
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<&str>>()
                .join(" ");
            if text.is_empty() || text.len() > 4000 {
                return Err("note text must be 1..=4000 chars".into());
            }
            let mut body = serde_json::json!({ "content": text });
            if reask {
                body["kind"] = serde_json::json!("reask");
            }
            post_json(&format!("/workflow/runs/{run}/notes"), body)?;
            if json_out {
                return emit_json_ok(
                    "workflow",
                    serde_json::json!({ "run_id": run, "kind": if reask { "reask" } else { "note" } }),
                );
            }
            println!(
                "note filed on run {run}{}",
                if reask { " (reask event recorded)" } else { "" }
            );
            Ok(())
        }
        "handoff" => {
            let Some(run) = rest.first() else {
                return Err(usage.into());
            };
            let resp = http::get(
                &base,
                &format!("/workflow/runs/{run}/handoff"),
                &[],
                token.as_deref(),
            )?;
            let v: serde_json::Value =
                serde_json::from_str(&resp.body).map_err(|e| format!("bad response: {e}"))?;
            if json_out {
                return emit_json_ok("workflow", v);
            }
            println!(
                "I-PASS handoff — run {} (domain {}) complete={}",
                v["run_id"], v["domain"], v["handoff_complete"]
            );
            for section in ["illness", "patient", "action", "situation", "safety"] {
                println!("\n[{}]", v[section]["title"].as_str().unwrap_or(section));
                for line in v[section]["lines"].as_array().unwrap_or(&vec![]) {
                    println!("  {}", line.as_str().unwrap_or(""));
                }
            }
            Ok(())
        }
        _ => Err(usage.into()),
    }
}

/// Minimal PATH lookup (no external crates).
fn which_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var("PATH").ok()?;
    paths
        .split(':')
        .map(|dir| PathBuf::from(dir).join(name))
        .find(|p| p.exists())
}

/// `brain kb build --domain <d> --out <dir>`: emit the public KB as a static
/// build artifact from the domain's `published` articles. Local-only (opens
/// the domain DB file directly — no server, no network path from the site).
fn cmd_kb(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let verb = require_positional(&positionals, "build")?;
    if verb != "build" {
        return Err(format!(
            "unknown kb verb '{verb}' (only 'build' is supported)"
        ));
    }
    let domain = flags
        .get("domain")
        .and_then(|o| o.clone())
        .ok_or("kb build requires --domain <d>")?;
    let out = flags
        .get("out")
        .and_then(|o| o.clone())
        .ok_or("kb build requires --out <dir>")?;
    let base_url = flags.get("base-url").and_then(|o| o.clone());
    let with_case_status = flags.contains_key("with-case-status");
    let locales: Vec<String> = flags
        .get("locales")
        .and_then(|o| o.clone())
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 12
                        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let db_path = if let Some(p) = flags.get("db").and_then(|o| o.clone()) {
        PathBuf::from(p)
    } else {
        brain_server::storage_layout::StorageLayout::detect()
            .map_err(|e| format!("storage layout: {e}"))?
            .domain_db(&domain)
            .map_err(|e| format!("domain: {e}"))?
    };
    if !db_path.exists() {
        return Err(format!("no database file at {}", db_path.display()));
    }

    let mut conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("open {}: {e}", db_path.display()))?;
    // Idempotent: guarantees the KCS columns exist on a pre-1.28.23 DB.
    brain_server::migration::run_migration(&mut conn, 1).map_err(|e| format!("migration: {e}"))?;

    let (articles, redirects) = brain_server::kb::collect_articles(&conn)
        .map_err(|e| format!("collect published articles: {e}"))?;
    if articles.is_empty() && !with_case_status {
        println!("no published articles in domain '{domain}' — nothing to build");
        return Ok(());
    }
    let mut status_count = 0usize;
    let mut opts = brain_server::kb::BuildOptions {
        with_case_status,
        locales: locales.clone(),
        ..Default::default()
    };
    if !locales.is_empty() {
        let translations = brain_server::kb::collect_translations(&conn)
            .map_err(|e| format!("collect translations: {e}"))?;
        println!(
            "translations: {} approved across {} locales",
            translations.len(),
            locales.len()
        );
        opts.translations = translations;
    }
    if with_case_status {
        // The build-time stamp is the honest static-freshness ceiling.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| format!("clock: {e}"))?
            .as_secs() as i64;
        let entries = brain_server::kb::collect_status_entries(&conn, now)
            .map_err(|e| format!("collect case-status refs: {e}"))?;
        status_count = entries.len();
        opts.status_entries = entries;
        // The complaint channel (ISO 10002 visibility): every status page
        // links how-to-complain.html, so the published complaints policy
        // MUST exist — a missing policy refuses the build loudly rather
        // than hosting links that lead nowhere.
        use rusqlite::OptionalExtension;
        let policy: Option<String> = conn
            .query_row(
                "SELECT content FROM knowledge WHERE source = ?1
                 ORDER BY id DESC LIMIT 1",
                [brain_server::kb::COMPLAINT_POLICY_SOURCE],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("read complaints policy: {e}"))?;
        let Some(policy_md) = policy else {
            return Err(
                "no published complaints policy (knowledge.source = 'complaint_policy'): \
                 publish one before building status pages — their footer links \
                 the how-to-complain page"
                    .into(),
            );
        };
        opts.complaint_policy_html = Some(brain_server::kb::render_policy_page(&policy_md));
    }
    let files =
        brain_server::kb::build_files_ext(&articles, &redirects, base_url.as_deref(), &opts);
    let n = brain_server::kb::write_artifact(std::path::Path::new(&out), &files)
        .map_err(|e| format!("write artifact: {e}"))?;
    println!(
        "kb built: {} articles (+{} redirect pages) → {out} ({n} files)",
        articles.len(),
        redirects.len()
    );
    if with_case_status {
        println!(
            "case-status pages: {status_count} (status/<ref>.json + .html; /status/ excluded from robots.txt and never in the sitemap)"
        );
    }
    println!(
        "verify what you host: sha256 each file against {out}/{}",
        brain_server::kb::MANIFEST_NAME
    );
    println!("sign before hosting: scripts/release-sign.sh <artifact.tar.gz>");
    Ok(())
}

/// `brain parcel export|import|ledger` — the operator surface for signed
/// knowledge parcels. Talks to the running server (the same governed paths
/// as `POST /parcels/export`, `POST /parcels/import`, `GET /parcels`), so
/// authz, screening, dedup, and the ledger stay server-side law.
fn cmd_parcel(args: &[String]) -> Result<(), String> {
    match args.first().map(|s| s.as_str()) {
        Some("export") => cmd_parcel_export(&args[1..]),
        Some("import") => cmd_parcel_import(&args[1..]),
        Some("ledger") => cmd_parcel_ledger(&args[1..]),
        _ => Err(
            "usage: brain parcel export --domain <d> [--since <ts>] --out <file>\n       brain parcel import --file <file> --domain <d> [--expected-signer <did>]\n       brain parcel ledger [--domain <d>]"
                .into(),
        ),
    }
}

fn cmd_parcel_export(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let domain = flags
        .get("domain")
        .and_then(|o| o.clone())
        .or_else(|| positionals.first().cloned())
        .ok_or("--domain is required")?;
    let out = flags
        .get("out")
        .and_then(|o| o.clone())
        .ok_or("--out is required (parcel file destination)")?;
    let mut body = serde_json::json!({ "domain": domain });
    if let Some(since) = flags.get("since").and_then(|o| o.clone()) {
        let ts: i64 = since
            .parse()
            .map_err(|_| "--since must be an epoch timestamp".to_string())?;
        body["since"] = serde_json::json!(ts);
    }
    let resp = post(
        &base_url(),
        "/parcels/export",
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
    std::fs::write(&out, resp.body).map_err(|e| format!("write {out}: {e}"))?;
    println!("wrote {out}");
    Ok(())
}

fn cmd_parcel_import(args: &[String]) -> Result<(), String> {
    let (_positionals, flags) = parse_flags(args)?;
    let file = flags
        .get("file")
        .and_then(|o| o.clone())
        .ok_or("--file is required (parcel file from export)")?;
    let domain = flags
        .get("domain")
        .and_then(|o| o.clone())
        .ok_or("--domain is required (receiving target domain)")?;
    let raw = std::fs::read_to_string(&file).map_err(|e| format!("read {file}: {e}"))?;
    let bundle: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("{file} is not a parcel JSON: {e}"))?;
    let parcel = bundle
        .get("parcel")
        .cloned()
        .ok_or("parcel file missing the 'parcel' object (run brain parcel export)")?;
    let mut body = serde_json::json!({ "domain": domain, "parcel": parcel });
    if let Some(signer) = flags.get("expected-signer").and_then(|o| o.clone()) {
        body["expected_signer"] = serde_json::json!(signer);
    }
    let resp = post(
        &base_url(),
        "/parcels/import",
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    println!("{}", resp.body);
    if resp.status != 200 {
        return Err(format!("server returned status {}", resp.status));
    }
    Ok(())
}

fn cmd_parcel_ledger(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let domain = flags
        .get("domain")
        .and_then(|o| o.clone())
        .or_else(|| positionals.first().cloned())
        .unwrap_or_else(|| "global".to_string());
    let q = vec![("domain".to_string(), url_encode(&domain))];
    let resp = get(&base_url(), "/parcels", &q, auth_token().as_deref())?;
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

/// `brain wfm-import <file.csv|file.json> [--domain D] [--dry-run]`:
/// the generic WFM import adapter (docs/wfm-seam.md). Format detection is
/// content-based — shifts rows POST to `/ops/shifts` with the server's own
/// validation and audit; skill rows become `crew_skills_update` proposals
/// (HITL), never direct registry writes.
fn cmd_wfm_import(args: &[String]) -> Result<(), String> {
    let (positionals, flags) = parse_flags(args)?;
    let file = positionals
        .first()
        .ok_or("usage: brain wfm-import <file.csv|file.json> [--domain D] [--dry-run]")?;
    let dry_run = flags.contains_key("dry-run");
    let raw = std::fs::read_to_string(file).map_err(|e| format!("read {file}: {e}"))?;

    // Content-based format detection: try shifts first, fall back to skills.
    // A file that parses as neither refuses loudly with both errors.
    let plan = match wfm_import::parse_shifts_csv(&raw)
        .or_else(|_| wfm_import::parse_shifts_json(&raw))
    {
        Ok(rows) => WfmPlan::Shifts(rows),
        Err(shift_err) => {
            let skill_parse = if file.ends_with(".json") {
                wfm_import::parse_skills_json(&raw)
            } else {
                wfm_import::parse_skills_csv(&raw).or_else(|_| wfm_import::parse_skills_json(&raw))
            };
            match skill_parse {
                Ok(rows) if !rows.is_empty() => WfmPlan::Skills(rows),
                _ => {
                    return Err(format!(
                        "{file} parsed as neither shifts nor skills ({shift_err})"
                    ));
                }
            }
        }
    };

    let domain_override = flags.get("domain").and_then(|o| o.clone());
    let mut posted = 0usize;
    let mut failed = 0usize;
    match plan {
        WfmPlan::Shifts(rows) => {
            for row in rows {
                let domain = domain_override.clone().unwrap_or(row.domain.clone());
                if domain.is_empty() {
                    return Err(format!(
                        "shift row for site '{}' has no domain and no --domain override",
                        row.site
                    ));
                }
                let body = serde_json::json!({
                    "domain": domain,
                    "site": row.site,
                    "tz": row.tz,
                    "start_epoch": row.start_epoch,
                    "end_epoch": row.end_epoch,
                    "overlap_minutes": row.overlap_minutes,
                    "roster": row.roster,
                });
                if !import_post("/ops/shifts", &body, dry_run, &mut posted, &mut failed)? {
                    continue;
                }
            }
        }
        WfmPlan::Skills(rows) => {
            for row in rows {
                let body = serde_json::json!({
                    "domain": domain_override.clone().unwrap_or_else(|| "global".into()),
                    "principal": row.principal,
                    "add": [row.skill],
                });
                if !import_post("/ops/skills", &body, dry_run, &mut posted, &mut failed)? {
                    continue;
                }
            }
        }
    }
    let verb = if dry_run { "would import" } else { "imported" };
    println!(
        "{verb} {posted} row(s) against seam {}; {failed} refused by the server",
        wfm_import::WFM_SCHEMA_VERSION
    );
    Ok(())
}

enum WfmPlan {
    Shifts(Vec<wfm_import::ImportedShift>),
    Skills(Vec<wfm_import::ImportedSkill>),
}

/// One import POST. Returns false when the server refused the row (the run
/// continues; the summary reports the refusals). Dry-run prints and counts.
fn import_post(
    path: &str,
    body: &serde_json::Value,
    dry_run: bool,
    posted: &mut usize,
    failed: &mut usize,
) -> Result<bool, String> {
    if dry_run {
        println!("POST {path} {}", body);
        *posted += 1;
        return Ok(true);
    }
    let resp = post(
        &base_url(),
        path,
        &[],
        "application/json",
        &body.to_string(),
        auth_token().as_deref(),
    )?;
    if resp.status == 200 || resp.status == 201 {
        *posted += 1;
        Ok(true)
    } else {
        *failed += 1;
        eprintln!("refused {}: {}", path, truncate(&resp.body, 200));
        Ok(false)
    }
}
