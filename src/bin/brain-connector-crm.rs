//! `brain-connector-crm` — support cases in from Zendesk, Salesforce, and
//! Genesys Cloud, one binary, `--source`-selected.
//!
//! ```sh
//! brain-connector-crm \
//!   --source zendesk \
//!   --config     ~/.config/brain-server/connectors/zendesk-acme.json \
//!   --checkpoint ~/.openclaw/workspace/brain.db
//! ```
//!
//! Loop-once-per-invocation — operator-cranked via cron, exactly like
//! `brain-connector-gh`. No background worker; the supervisor stays unwired.
//!
//! Config shapes (0600 JSON; secrets ride in separate 0600 `*_file`s):
//! - zendesk    `{subdomain, email, api_token_file}`
//! - salesforce `{instance_url, client_id, client_secret_file, api_version?}`
//! - genesys    `{region, client_id, client_secret_file, worktype?, org_id?}`
//!
//! Cursors persist in the connector's own state file beside the config
//! (`crm-state-{source}-{org}.json`), never inside brain-server. The
//! case↔run linkage lands in brain-server's `crm_cases` table (idempotent by
//! `case_ref`). Poll cadence floor is 300s — cron recipes in
//! `docs/deployment.md`.

#![cfg(feature = "connector-crm")]

use anyhow::{Context, Result};
use brain_server::connector::crm::{self, BrainSink, VendorTransport};
use std::collections::HashMap;

const DEFAULT_URL: &str = "http://127.0.0.1:8765";

fn main() -> Result<()> {
    let args = parse_argv().context("invalid argv")?;
    emit_log(
        "info",
        &format!("crm connector starting (source={})", args.source),
    );

    let raw = std::fs::read_to_string(&args.config)
        .with_context(|| format!("failed reading config {}", args.config.display()))?;
    let cfg: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&raw).context("config was not a JSON object")?;

    let http = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("brain-connector-crm/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")?;

    let brain_base = std::env::var("BRAIN_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let sink = crm::http::HttpBrainSink::new(&brain_base, auth_token(), "global")
        .context("failed to build brain sink")?;

    // Open the linkage DB read/write (the connector writes only `crm_cases`
    // rows; WAL keeps this safe alongside the server).
    let db_path = std::env::var("BRAIN_DB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| args.checkpoint.clone());
    let db = rusqlite::Connection::open(&db_path)
        .with_context(|| format!("failed to open checkpoint DB {}", db_path.display()))?;
    db.execute_batch("PRAGMA journal_mode=WAL;")
        .context("failed to enable WAL")?;

    let state_path = args.config.with_file_name(format!(
        "crm-state-{}-{}.json",
        args.source,
        org_label(&args.source, &cfg)
    ));
    let mut cursor: Option<String> = read_state(&state_path);

    let total: usize = match args.source.as_str() {
        "zendesk" => {
            let subdomain = cfg_str(&cfg, "subdomain")?;
            let email = cfg_str(&cfg, "email")?;
            let token = crm::http::read_secret_file(std::path::Path::new(&cfg_str(
                &cfg,
                "api_token_file",
            )?))?;
            let base = crm::zendesk::api_base(&subdomain)?;
            let t = crm::http::ReqwestTransport::new(
                http,
                dedup_hosts(vec![host_of(&base), host_of(&brain_base)]),
            );
            // First run starts from one week ago (epoch seconds); later runs
            // resume from the persisted opaque `after_cursor`.
            let start_time = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0))
            .saturating_sub(7 * 24 * 3600);
            let page = crm::zendesk::fetch_page(
                &t,
                &base,
                &crm::zendesk::basic_auth(&email, &token),
                cursor.as_deref(),
                start_time,
                &subdomain,
            )?;
            let mut n = 0;
            for case in &page.cases {
                deliver(&sink, &db, case)?;
                n += 1;
            }
            if let Some(c) = &page.after_cursor {
                write_state(&state_path, c)?;
            }
            n
        }
        "salesforce" => {
            let instance_url = cfg_str(&cfg, "instance_url")?;
            let client_id = cfg_str(&cfg, "client_id")?;
            let secret = crm::http::read_secret_file(std::path::Path::new(&cfg_str(
                &cfg,
                "client_secret_file",
            )?))?;
            let api_version = cfg
                .get("api_version")
                .and_then(|v| v.as_str())
                .unwrap_or("v62.0")
                .to_string();
            let base = crm::salesforce::api_base(&instance_url)?;
            let t = crm::http::ReqwestTransport::new(
                http,
                dedup_hosts(vec![host_of(&base), host_of(&brain_base)]),
            );
            // Client-credentials token fetch — fail-closed: an error aborts
            // before any case fetch carries credentials anywhere.
            let tok_resp = t.post_form(
                &format!("{base}/services/oauth2/token"),
                &crm::salesforce::token_form(&client_id, &secret),
                None,
            )?;
            let access_token = tok_resp
                .get("access_token")
                .and_then(|a| a.as_str())
                .context("token response missing access_token")?
                .to_string();
            let last: Option<String> = cursor.filter(|c| !c.is_empty());
            let url = crm::salesforce::query_url(&base, &api_version, last.as_deref());
            let body = t.get_json(&url, &format!("Bearer {access_token}"))?;
            let org = org_label("salesforce", &cfg);
            let (cases, next) = crm::salesforce::translate_page(&org, &body)?;
            let mut n = 0;
            for case in &cases {
                deliver(&sink, &db, case)?;
                n += 1;
            }
            // Persist the modstamp of the newest row seen; next sync resumes.
            if let Some(newest) = cases.iter().map(|c| c.updated_rev.clone()).max() {
                write_state(&state_path, &newest)?;
            } else if next.is_none() && last.is_none() {
                write_state(&state_path, "")?;
            }
            n
        }
        "genesys" => {
            let region = cfg_str(&cfg, "region")?;
            let client_id = cfg_str(&cfg, "client_id")?;
            let secret = crm::http::read_secret_file(std::path::Path::new(&cfg_str(
                &cfg,
                "client_secret_file",
            )?))?;
            let worktype = cfg
                .get("worktype")
                .and_then(|w| w.as_str())
                .unwrap_or("case");
            let base = crm::genesys::api_base(&region)?;
            let login = crm::genesys::login_base(&region)?;
            let t = crm::http::ReqwestTransport::new(
                http,
                dedup_hosts(vec![host_of(&base), host_of(&login), host_of(&brain_base)]),
            );
            let tok_resp = t.post_form(
                &format!("{login}/oauth/token"),
                &crm::salesforce::token_form(&client_id, &secret),
                None,
            )?;
            let access_token = tok_resp
                .get("access_token")
                .and_then(|a| a.as_str())
                .context("token response missing access_token")?
                .to_string();
            let auth = format!("Bearer {access_token}");
            let org = org_label("genesys", &cfg);
            let mut contacts: HashMap<String, String> = HashMap::new();
            let mut after: Option<String> = cursor.take();
            let mut n = 0;
            // The pagination loop is server-cursor-driven: bound it (and the
            // contact map) so a misbehaving endpoint cannot spin the
            // connector indefinitely. Exceeding the page cap resumes on the
            // next cron tick from the persisted cursor.
            const MAX_PAGES: usize = 50;
            let mut pages = 0usize;
            loop {
                pages += 1;
                if pages > MAX_PAGES {
                    tracing::warn!("genesys: page cap ({MAX_PAGES}) reached — resuming next tick");
                    break;
                }
                let url = crm::genesys::workitems_url(&base, worktype, after.as_deref());
                let body = t.get_json(&url, &auth)?;
                // Resolve customer identities through externalcontacts BEFORE
                // translation — only ids enter the map; the case stores their
                // salted hash. A failed lookup falls back to the raw
                // participant id (also hashed), never blocks the sync.
                if let Some(entities) = body.get("entities").and_then(|e| e.as_array()) {
                    for e in entities {
                        if let Some(cid) = e.get("externalContactId").and_then(|c| c.as_str()) {
                            if cid.is_empty() || contacts.contains_key(cid) {
                                continue;
                            }
                            // Vendor-controlled id: percent-encode before it
                            // touches a URL path (traversal/splitting hygiene).
                            let cid_enc: String = crm::salesforce::urlencode(cid);
                            if let Ok(rec) = t.get_json(
                                &format!("{base}/api/v2/externalcontacts/contacts/{cid_enc}"),
                                &auth,
                            ) {
                                let canon = rec.get("id").and_then(|i| i.as_str()).unwrap_or(cid);
                                contacts.insert(cid.to_string(), format!("canonical:{canon}"));
                            }
                        }
                    }
                }
                let (cases, next) = crm::genesys::translate_page(&org, &body, &contacts)?;
                for case in &cases {
                    deliver(&sink, &db, case)?;
                    n += 1;
                }
                match next {
                    Some(c) => after = Some(c),
                    None => break,
                }
            }
            n
        }
        other => anyhow::bail!("unknown --source {other:?}; expected zendesk|salesforce|genesys"),
    };

    emit_progress("default", total);
    emit_done();
    Ok(())
}

fn deliver(sink: &dyn BrainSink, db: &rusqlite::Connection, case: &crm::CrmCase) -> Result<()> {
    let report = crm::deliver_case(sink, db, case)
        .with_context(|| format!("delivery failed for {}", case.case_ref()))?;
    emit_log(
        "info",
        &format!(
            "{} → run {} ({})",
            report.case_ref,
            report.run_id,
            report.topic_posted.unwrap_or_else(|| "replayed".into())
        ),
    );
    Ok(())
}

fn cfg_str(cfg: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<String> {
    cfg.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .with_context(|| format!("config missing string field {key:?}"))
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .map(|u| u.host_str().unwrap_or_default().to_ascii_lowercase())
        .unwrap_or_default()
}

fn dedup_hosts(mut v: Vec<String>) -> Vec<String> {
    v.retain(|h| !h.is_empty());
    v.sort();
    v.dedup();
    v
}

/// Stable per-org label used for the state-file name. Prefers the explicit
/// `org_id`; falls back to the source's primary identifier field.
fn org_label(source: &str, cfg: &serde_json::Map<String, serde_json::Value>) -> String {
    if let Some(o) = cfg.get("org_id").and_then(|v| v.as_str()) {
        return o.to_string();
    }
    match source {
        "zendesk" => cfg
            .get("subdomain")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string(),
        "salesforce" => cfg
            .get("instance_url")
            .and_then(|v| v.as_str())
            .map(host_of)
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "default".to_string()),
        _ => "default".to_string(),
    }
}

/// Read the persisted cursor (`{"cursor": "..."}` shape) from the state file.
fn read_state(path: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("cursor").and_then(|c| c.as_str()).map(str::to_string)
}

fn write_state(path: &std::path::Path, value: &str) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let mut f = std::fs::File::create(&tmp)
        .with_context(|| format!("cannot create state file {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(serde_json::json!({"cursor": value}).to_string().as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

struct ConnectorArgs {
    source: String,
    config: std::path::PathBuf,
    checkpoint: std::path::PathBuf,
}

fn parse_argv() -> Result<ConnectorArgs> {
    let mut source: Option<String> = None;
    let mut config: Option<std::path::PathBuf> = None;
    let mut checkpoint: Option<std::path::PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--source" => source = args.next(),
            "--config" => config = args.next().map(std::path::PathBuf::from),
            "--checkpoint" => checkpoint = args.next().map(std::path::PathBuf::from),
            "--help" | "-h" => {
                println!(
                    "usage: brain-connector-crm --source zendesk|salesforce|genesys --config <path> --checkpoint <path>"
                );
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argv {other:?}"),
        }
    }
    Ok(ConnectorArgs {
        source: source.context("missing --source zendesk|salesforce|genesys")?,
        config: config.context("missing --config <path>")?,
        checkpoint: checkpoint.context("missing --checkpoint <path>")?,
    })
}

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
    std::fs::read_to_string(std::path::Path::new(&home).join(".config/brain-server/auth-token"))
        .ok()
        .and_then(|s| first_token(&s))
}

// ── JSON-lines event emitters (connector → supervisor protocol) ─────────────

fn emit_log(level: &str, msg: &str) {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({"type":"log","level":level,"msg":msg}),
    );
    println!();
}
fn emit_progress(cursor: &str, count: usize) {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({"type":"progress","cursor":cursor,"count":count}),
    );
    println!();
}
fn emit_done() {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({"type":"done","report":{}}),
    );
    println!();
}
