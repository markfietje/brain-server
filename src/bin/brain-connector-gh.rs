//! `brain-connector-gh` — the reference connector for GitHub.
//!
//! Reads its config from `~/.config/brain-server/connectors/github-{instance}.json`
//! (per the connector-binary contract), backfills the configured repos'
//! issues into brain-server, and exits. Designed to be spawned + supervised
//! by the server's connector runner, but usable standalone:
//!
//! ```sh
//! brain-connector-gh \
//!   --config     ~/.config/brain-server/connectors/github-brain-server.json \
//!   --checkpoint ~/.openclaw/workspace/brain.db
//! ```
//!
//! The `--checkpoint` argv is the brain-server DB path — the connector opens
//! it read/write to persist cursors in `connector_checkpoints`. SQLite handles
//! concurrent access from the connector + brain-server safely (WAL mode).
//!
//! Env:
//!   BRAIN_URL         base URL of brain-server (default http://127.0.0.1:8765)
//!   BRAIN_TOKEN_FILE  path to a 0600 secret file (preferred over BRAIN_TOKEN)
//!   BRAIN_TOKEN       raw bearer token (dev convenience)
//!   BRAIN_DB_PATH     overrides the --checkpoint argv (dev convenience)
//!
//! This binary is feature-gated on `connector-github` because it pulls in
//! `reqwest` + `jsonwebtoken` — deps the server binary never has.

#![cfg(feature = "connector-github")]

use anyhow::{Context, Result};

use brain_server::connector::auth::AuthProvider;
use brain_server::connector::auth::github_app::{GitHubAppConfig, GitHubAppProvider};
use brain_server::connector::auth::store::CredentialStore;
use brain_server::connector::github::client::GitHubClient;
use brain_server::connector::github::{
    BackfillReport, backfill_issues_for_repo, reconcile_github_sources,
};

const DEFAULT_URL: &str = "http://127.0.0.1:8765";

fn main() -> Result<()> {
    let (config_path, checkpoint_path) = parse_argv().context("invalid argv")?;
    emit_log("info", "github connector starting");

    let config_path_str = config_path.to_string_lossy().to_string();
    emit_log("info", &format!("loading config from {config_path_str}"));

    let store: CredentialStore<GitHubAppConfig> =
        CredentialStore::load_from(&config_path).context("failed to load connector config")?;
    let gh_config = store.config().clone();
    let instance = derive_instance_name(&gh_config);

    // Open the checkpoint DB (brain-server's main DB). The connector writes
    // only to `connector_checkpoints`; SQLite's WAL mode keeps this safe
    // alongside brain-server's own writes.
    let db_path = std::env::var("BRAIN_DB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or(checkpoint_path);
    let db_path_str = db_path.to_string_lossy().to_string();
    emit_log("info", &format!("opening checkpoint DB at {db_path_str}"));
    let db = rusqlite::Connection::open(&db_path)
        .with_context(|| format!("failed to open checkpoint DB at {db_path_str}"))?;
    db.execute_batch("PRAGMA journal_mode=WAL;")
        .context("failed to enable WAL on checkpoint DB")?;

    // Resolve the connector instance id (creates the row if first run).
    let connector_id = resolve_connector_id(&db, &instance)?;
    emit_log(
        "info",
        &format!("connector instance '{instance}' resolved to id {connector_id}"),
    );

    // Build one reqwest client shared across the auth provider + REST client.
    // Connection pooling + a single TLS context.
    let http = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!("brain-connector-gh/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build reqwest client")?;

    let provider = GitHubAppProvider::new(gh_config.clone(), http.clone())?;
    let gh_client = GitHubClient::new(http.clone());

    emit_log("info", "fetching installation token");
    let token = provider
        .access_token()
        .context("failed to get installation token")?;
    emit_log("info", "installation token acquired (redacted)");

    let brain_base = std::env::var("BRAIN_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let brain_token = auth_token();

    // Parse the configured repos into (owner, repo) pairs. The config's
    // `repositories` field is `["owner/repo", ...]`.
    let repos = gh_config.repositories.clone();
    if repos.is_empty() {
        anyhow::bail!(
            "config has no `repositories` — nothing to backfill. \
             Populate the field with `owner/repo` strings."
        );
    }

    let mut total = 0usize;
    let mut all_walked_uris: Vec<String> = Vec::new();
    let mut reports: Vec<BackfillReport> = Vec::new();
    for full_name in &repos {
        let (owner, repo) = match full_name.split_once('/') {
            Some((o, r)) if !o.is_empty() && !r.is_empty() => (o, r),
            _ => {
                emit_log(
                    "warn",
                    &format!("skipping malformed repo name: '{full_name}'"),
                );
                continue;
            }
        };
        emit_log("info", &format!("backfilling {owner}/{repo}"));
        let report = match backfill_issues_for_repo(
            &db,
            &gh_client,
            &http,
            &token,
            &brain_base,
            brain_token.as_deref(),
            connector_id,
            owner,
            repo,
        ) {
            Ok(r) => r,
            Err(e) => {
                emit_error(&format!("backfill of {full_name} failed: {e:#}"), true);
                continue;
            }
        };
        total += report.ingested;
        all_walked_uris.extend(report.walked_uris.clone());
        reports.push(report);
    }

    // Reconcile AFTER all repos backfilled. The server's `sources::reconcile`
    // is kind-scoped, so passing the union of URIs across all configured repos
    // sweeps any GitHub source whose URI is no longer live (deleted issue,
    // uninstalled repo, transfer to another org). This is the convergence
    // path: even without webhooks (which are optional), periodic
    // sync runs converge to the truth.
    emit_log(
        "info",
        &format!("reconciling {} live URIs", all_walked_uris.len()),
    );
    match reconcile_github_sources(&http, &brain_base, brain_token.as_deref(), &all_walked_uris) {
        Ok(r) => {
            if r.deleted_sources > 0 || !r.orphan_uris.is_empty() {
                emit_log(
                    "info",
                    &format!(
                        "reconcile: {} source(s) retired, {} chunk(s) swept, {} orphan(s)",
                        r.deleted_sources,
                        r.deleted_chunks,
                        r.orphan_uris.len(),
                    ),
                );
            } else {
                emit_log("info", "reconcile: no drift detected");
            }
        }
        Err(e) => {
            emit_error(&format!("reconcile failed: {e:#}"), true);
        }
    }

    emit_progress("default", total);
    for r in &reports {
        let _ = serde_json::to_writer(std::io::stdout(), &r);
        println!();
    }
    emit_done();
    Ok(())
}

/// Parse the connector-binary contract argv: `--config <path>` + `--checkpoint <path>`.
fn parse_argv() -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let mut config_path: Option<std::path::PathBuf> = None;
    let mut checkpoint_path: Option<std::path::PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => config_path = args.next().map(std::path::PathBuf::from),
            "--checkpoint" => checkpoint_path = args.next().map(std::path::PathBuf::from),
            other => {
                emit_log("warn", &format!("ignoring unknown argv: {other}"));
            }
        }
    }
    let config_path = config_path.context("missing --config <path> argv")?;
    let checkpoint_path = checkpoint_path.context("missing --checkpoint <path> argv")?;
    Ok((config_path, checkpoint_path))
}

/// Derive a stable instance name from the config. Used as the disambiguator
/// in `connectors.instance` (e.g. `github-brain-server` for one configured repo).
fn derive_instance_name(config: &GitHubAppConfig) -> String {
    if config.repositories.len() == 1 {
        config.repositories[0].replace('/', "_")
    } else {
        // Multi-repo: hash the sorted set so the name is stable regardless
        // of the order the user listed repos in. Keeps the connectors row
        // stable across config edits that only reorder.
        let mut sorted = config.repositories.clone();
        sorted.sort();
        let joined = sorted.join(",");
        let hash = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(joined.as_bytes()));
        format!("multi-{hash}")
    }
}

/// Look up the connectors row for this instance, creating it if missing.
fn resolve_connector_id(db: &rusqlite::Connection, instance: &str) -> Result<i64> {
    let config_json = "{}".to_string(); // we don't store dynamic state here
    db.execute(
        "INSERT INTO connectors (kind, instance, config_json, state) \
         VALUES ('github', ?1, ?2, 'running') \
         ON CONFLICT(kind, instance) DO UPDATE SET state = 'running', updated_at = CURRENT_TIMESTAMP",
        rusqlite::params![instance, config_json],
    )?;
    let id: i64 = db.query_row(
        "SELECT id FROM connectors WHERE kind = 'github' AND instance = ?1",
        rusqlite::params![instance],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// Resolve the bearer token for brain-server auth. Same ladder as `brain` /
/// `mcp` / the stub connector — duplicated rather than shared to keep
/// `bin_common` surface tiny. Token files may carry multiple rotation slots;
/// send exactly one (see `bin_common::http::first_token`).
fn auth_token() -> Option<String> {
    fn first_token(raw: &str) -> Option<String> {
        raw.split_whitespace().next().map(str::to_string)
    }
    if let Ok(path) = std::env::var("BRAIN_TOKEN_FILE")
        && let Ok(s) = std::fs::read_to_string(path.trim())
        && let Some(t) = first_token(&s)
    {
        return Some(t);
    }
    if let Ok(t) = std::env::var("BRAIN_TOKEN")
        && let Some(t) = first_token(&t)
    {
        return Some(t);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let default_path = std::path::Path::new(&home).join(".config/brain-server/auth-token");
    std::fs::read_to_string(&default_path)
        .ok()
        .and_then(|s| first_token(&s))
}

// ── JSON-lines event emitters (connector → supervisor protocol) ─────────────

fn emit_log(level: &str, msg: &str) {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({
            "type": "log",
            "level": level,
            "msg": msg,
        }),
    );
    println!();
}
fn emit_progress(cursor: &str, count: usize) {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({
            "type": "progress",
            "cursor": cursor,
            "count": count,
        }),
    );
    println!();
}
fn emit_done() {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({
            "type": "done",
            "report": {},
        }),
    );
    println!();
}
fn emit_error(msg: &str, retry: bool) {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({
            "type": "error",
            "msg": msg,
            "retry": retry,
        }),
    );
    println!();
}
