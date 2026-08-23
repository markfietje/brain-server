//! The server's one `HostCallContext` assembly: which capabilities an engine
//! gets here, and what each kind actually runs. The confused-deputy posture
//! lives in the SDK dispatcher; this module only supplies fail-closed
//! handlers — notably secret mediation, where resolution happens host-side
//! and ONLY status metadata crosses the boundary (never the material).

use brain_engine_sdk::host::{AuditKind, AuditStatus, WorkflowHost};
use brain_engine_sdk::hostcall::{HostCallContext, exec_mediation};
use brain_engine_sdk::trust::{EngineOverride, ExtensionPolicy, HostCallKind};
use std::sync::Arc;

use crate::workflow::host::SqliteWorkflowHost;

/// Hard cap on captured exec stdout/stderr and HTTP bodies (each stream).
const EFFECT_OUTPUT_CAP: usize = 64 * 1024;
/// Exec wall-clock bound (the SDK `Budget` default effective timeout; the
/// per-op budget seam lands with the GUI crank).
const EXEC_TIMEOUT_SECS: u64 = 30;

/// Operator env: whitespace-separated argv0 prefixes an engine may execute.
/// Empty/absent = deny ALL engine exec (fail-closed).
pub(crate) fn exec_allowlist() -> Vec<String> {
    word_list(&std::env::var("BRAIN_ENGINE_EXEC_ALLOWLIST").unwrap_or_default())
}

/// Operator env: host names an engine may reach over HTTPS (loopback may use
/// plain http). Empty/absent = no egress (fail-closed).
pub(crate) fn http_allowlist() -> Vec<String> {
    let raw = std::env::var("BRAIN_ENGINE_HTTP_ALLOWLIST").unwrap_or_default();
    word_list(&raw)
}

/// Working directory engine exec is pinned to. Defaults to the server's
/// current directory (the domain data dir wiring arrives with Cockpit).
fn workdir() -> std::path::PathBuf {
    std::env::var("BRAIN_ENGINE_WORKDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        })
}

fn word_list(raw: &str) -> Vec<String> {
    raw.split_whitespace()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// argv0 admission: exact match, or a trailing-`/` entry acting as a
/// directory prefix (`/usr/bin/` admits `/usr/bin/ls`). No entry, no exec.
fn argv0_allowed(argv0: &str, allowlist: &[String]) -> bool {
    // Prefix admission is only sound on a normalized path: a `..` component
    // could let `/usr/bin/../sbin/evil` masquerade under the `/usr/bin/`
    // prefix. Refuse rather than canonicalize (symlinks stay out of scope).
    if argv0.split('/').any(|c| c == "..") {
        return false;
    }
    allowlist
        .iter()
        .any(|e| e == argv0 || (e.ends_with('/') && argv0.starts_with(e.as_str())))
}

fn loopback_host(host: &str) -> bool {
    let h = host.trim();
    // Bracketed IPv6 literal, optionally with a port (`[::1]:8080`).
    let bare = h
        .strip_prefix('[')
        .and_then(|r| r.split_once(']'))
        .map(|(inner, _)| inner)
        .unwrap_or_else(|| {
            // Plain host with at most ONE port separator (`127.0.0.1:8080`);
            // an IPv6 literal carries many colons and is kept whole.
            if h.matches(':').count() == 1 {
                h.split(':').next().unwrap_or(h)
            } else {
                h
            }
        });
    matches!(bare, "localhost" | "127.0.0.1" | "::1")
}

/// Audit-and-refuse for paths that have no resolved run yet (global tenant).
fn deny_simple(
    host: &SqliteWorkflowHost,
    engine: &str,
    who: &str,
    reason: &str,
) -> Result<String, String> {
    host.audit(
        AuditKind::Workflow,
        engine,
        who,
        AuditStatus::Denied,
        reason,
    );
    Err(reason.to_string())
}

/// URL construction for mediated egress: remote = HTTPS only; loopback may
/// speak plain http. Pure seam (the scheme law is pinned directly).
fn build_url(host_name: &str, path: &str) -> String {
    let scheme = if loopback_host(host_name) {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{host_name}{path}")
}

/// The unprivileged read principal handlers sanitize through — an engine's
/// mediated view is what the most restricted reader would see.
fn unprivileged() -> Option<crate::auth::Principal> {
    Some(crate::auth::Principal {
        sub: "workflow-extension".into(),
        tenant: "global".into(),
        scopes: Vec::new(),
        jti: String::new(),
        roles: Vec::new(),
        manages: Vec::new(),
    })
}

fn sanitized(json: &str) -> Result<String, String> {
    Ok(crate::gate::sanitize_read(json, true, &unprivileged()))
}

/// Production posture: Standard plus the two always-mediated classes
/// (`tools` runs only the built-in fail-closed handlers; `log` is a
/// structured emit). exec/env stay denied UNLESS the operator configured a
/// non-empty exec allowlist — then THIS engine gets the explicit per-engine
/// allow (global-deny removal + per-engine grant; every other engine still
/// falls through to Prompt, which server-side reads as Denied — there is no
/// interactive prompt without a human; Prompt == Denied until Witness wires
/// the GUI prompt path). ui prompts and therefore never runs; its handler
/// refuses with a named reason for exhaustiveness.
pub(crate) fn production_policy(engine: &str) -> ExtensionPolicy {
    let mut policy = ExtensionPolicy::standard();
    policy.default_caps.push("tools".into());
    policy.default_caps.push("log".into());
    if !exec_allowlist().is_empty() {
        policy.deny_caps.retain(|c| c != "exec");
        policy.per_engine.insert(
            engine.to_string(),
            EngineOverride {
                allow_caps: vec!["exec".into()],
                deny_caps: vec![],
            },
        );
    }
    policy
}

/// Build the dispatch context for one engine against the shared host.
pub(crate) fn build(
    host: Arc<SqliteWorkflowHost>,
    engine: &str,
) -> HostCallContext<SqliteWorkflowHost> {
    let ctx = HostCallContext::new(host.clone(), production_policy(engine), engine);
    register_handlers(&ctx, &host, engine);
    ctx
}

/// Register every mediated kind handler. Exhaustive over the closed
/// [`HostCallKind`] vocabulary — pinned by `hostcall_table_is_exhaustive`.
fn register_handlers(
    ctx: &HostCallContext<SqliteWorkflowHost>,
    host: &Arc<SqliteWorkflowHost>,
    engine: &str,
) {
    // log → structured emit (host-side tracing, no raw payload echo).
    ctx.set_handler(HostCallKind::Log, |name, body| {
        tracing::info!(target: "brain::workflow", engine_call = %name, "hostcall log");
        Ok(format!("logged:{name}:{}", body.len()))
    });

    // session → sanitized state view through the SDK session seam; the raw
    // state_json stays host-private.
    let h2 = host.clone();
    ctx.set_handler(HostCallKind::Session, move |name, _body| {
        let run_id: i64 = name.parse().map_err(|_| "invalid run key".to_string())?;
        let raw = h2
            .load_state(run_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "run not found".to_string())?
            .0;
        Ok(crate::gate::sanitize_read(&raw, true, &unprivileged()))
    });

    // exec → the SDK's exec_mediation contract behind an operator allowlist.
    let h_exec = host.clone();
    let engine_exec = engine.to_string();
    ctx.set_handler(HostCallKind::Exec, move |name, body| {
        run_mediated_exec(&h_exec, &engine_exec, name, body)
    });

    // http → deny-by-default egress mediation on the shared hardened client.
    let h_http = host.clone();
    let engine_http = engine.to_string();
    ctx.set_handler(HostCallKind::Http, move |_name, body| {
        run_mediated_http(&h_http, &engine_http, body)
    });

    // events → the outbox is the only event door; topics outside the
    // `workflow/*` namespace are refused (the alert bus stays server-owned).
    let h_events = host.clone();
    let engine_events = engine.to_string();
    ctx.set_handler(HostCallKind::Events, move |name, body| {
        run_mediated_event(&h_events, &engine_events, name, body)
    });

    // ui → an EXPLICIT refusal, not an absence. Unreachable under policy
    // today (Prompt == Denied server-side); registered so the dispatch table
    // is exhaustive over the closed vocabulary and any future policy change
    // fails loudly instead of Internal-erroring.
    ctx.set_handler(HostCallKind::Ui, |_name, _body| {
        Err("reserved: lands with Cockpit".to_string())
    });

    // tool → the built-in mediated tools. Everything else fails closed.
    let h3 = host.clone();
    ctx.set_handler(HostCallKind::Tool, move |name, body| match name {
        "secret_status" => {
            // Secret mediation: resolve host-side, publish STATUS only.
            let target = body.trim();
            if target.is_empty()
                || !target
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err("invalid secret name".into());
            }
            let configured = crate::secrets::resolve(target).is_ok();
            Ok(format!(r#"{{"configured":{configured}}}"#))
        }
        "mediated_exec" => {
            // Even where tools are granted, destructive commands are refused
            // before any process seam could exist.
            exec_mediation(body).map(|_| format!("accepted:{}", body.len()))
        }
        "knowledge_suggest" => run_knowledge_suggest(&h3, body),
        other => Err(format!("unknown tool `{other}`")),
    });
}

/// Exec mediation handler. Body shape: `{"argv": ["prog", "--flag", ...]}` —
/// argv only, never a shell line. Refusal paths audit `denied`; success
/// audits `ok`. Output is sanitized before it crosses back.
fn run_mediated_exec(
    host: &SqliteWorkflowHost,
    engine: &str,
    run: &str,
    body: &str,
) -> Result<String, String> {
    // `run:<id>`-shaped targets let the host chain resolve the run's domain
    // as the audit tenant (the substrate convention).
    let who = format!("workflow/hostcall/exec/run:{run}");
    match exec_effect(body) {
        Ok(out) => {
            let payload = serde_json::json!({
                "exit_code": out.exit_code,
                "stdout": String::from_utf8_lossy(&out.stdout),
                "stderr": String::from_utf8_lossy(&out.stderr),
            })
            .to_string();
            let result = sanitized(&payload)?;
            host.audit(
                AuditKind::Workflow,
                engine,
                &who,
                AuditStatus::Ok,
                "mediated exec",
            );
            Ok(result)
        }
        Err(reason) => {
            host.audit(
                AuditKind::Workflow,
                engine,
                &who,
                AuditStatus::Denied,
                &reason,
            );
            Err(reason)
        }
    }
}

struct ExecOutput {
    exit_code: i64,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Validate + run one allowlisted command. Pure-ish seam so tests hit the
/// refusal logic without spawning processes where possible.
fn exec_effect(body: &str) -> Result<ExecOutput, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "invalid exec payload".to_string())?;
    let mut argv: Vec<String> = v
        .get("argv")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .map(|x| x.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    if argv.is_empty() || argv.iter().any(String::is_empty) {
        return Err("exec requires a non-empty argv".into());
    }
    let argv0 = argv.remove(0);
    if !argv0_allowed(&argv0, &exec_allowlist()) {
        return Err("argv0 not in exec allowlist".into());
    }
    exec_mediation(&format!("{argv0} {}", argv.join(" ")))
        .map_err(|e| format!("dangerous command refused: {e}"))?;

    let cwd = workdir();
    if !cwd.is_dir() {
        return Err("workdir does not exist".into());
    }

    let mut cmd = std::process::Command::new(&argv0);
    cmd.args(&argv)
        .current_dir(&cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    // Drain stdout/stderr on threads so a chatty child can never wedge on a
    // full pipe while we poll for exit; each stream is capped at 64 KiB.
    use std::io::Read;
    fn drain<R: Read + Send + 'static>(
        pipe: Option<R>,
        cap: usize,
    ) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let mut chunk = [0u8; 8192];
                loop {
                    match p.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let take = n.min(cap.saturating_sub(buf.len()));
                            buf.extend_from_slice(&chunk[..take]);
                            // Past the cap: keep draining (discard) so the
                            // child cannot block, but record nothing more.
                        }
                    }
                }
            }
            buf
        })
    }
    let out_reader = drain(child.stdout.take(), EFFECT_OUTPUT_CAP);
    let err_reader = drain(child.stderr.take(), EFFECT_OUTPUT_CAP);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(EXEC_TIMEOUT_SECS);
    loop {
        match child.try_wait().map_err(|e| format!("wait failed: {e}"))? {
            Some(status) => {
                let stdout = out_reader
                    .join()
                    .map_err(|_| "stdout reader panicked".to_string())?;
                let stderr = err_reader
                    .join()
                    .map_err(|_| "stderr reader panicked".to_string())?;
                return Ok(ExecOutput {
                    exit_code: status.code().unwrap_or(-1) as i64,
                    stdout,
                    stderr,
                });
            }
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = out_reader.join();
                    let _ = err_reader.join();
                    return Err("exec exceeded time budget".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
}

/// Http mediation handler. Body shape: `{"host": "...", "path": "/..."}`.
/// Remote = HTTPS only; loopback hosts may use plain http. The client is the
/// shared hardened egress client (redirects refused, 5 s / 15 s bounds).
fn run_mediated_http(
    host: &SqliteWorkflowHost,
    engine: &str,
    body: &str,
) -> Result<String, String> {
    const WHO: &str = "workflow/hostcall/http";
    let deny = |reason: String| {
        host.audit(
            AuditKind::Workflow,
            engine,
            WHO,
            AuditStatus::Denied,
            &reason,
        );
        Err(reason)
    };
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return deny("invalid http payload".into()),
    };
    let host_name = v
        .get("host")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let path = v.get("path").and_then(|x| x.as_str()).unwrap_or("/");
    if host_name.is_empty()
        || !host_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':')
    {
        return deny("invalid host".into());
    }
    if !http_allowlist()
        .iter()
        .any(|e| e.eq_ignore_ascii_case(&host_name))
    {
        return deny("host not in http allowlist".into());
    }
    if !path.starts_with('/') {
        return deny("path must start with '/'".into());
    }
    let url = build_url(&host_name, path);
    let client = crate::webhook::egress_client();
    // The handler seam is sync; egress rides a throwaway current-thread
    // runtime (the webhook drain worker's posture).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    let sent = rt.block_on(async { client.get(&url).send().await });
    match sent {
        Ok(resp) => {
            // Refuse declared-oversized bodies BEFORE buffering them — a cap
            // applied after `bytes()` would still have read it all (memory
            // amplification via an allowlisted host). Bodies without a length
            // are caught by the post-read truncate instead.
            if resp
                .content_length()
                .is_some_and(|n| n as usize > EFFECT_OUTPUT_CAP)
            {
                return deny("body exceeds 64 KiB".into());
            }
            let status = resp.status().as_u16();
            match rt.block_on(resp.bytes()) {
                Ok(b) => {
                    let mut text = b.to_vec();
                    text.truncate(EFFECT_OUTPUT_CAP);
                    let payload =
                        serde_json::json!({"status": status, "body": String::from_utf8_lossy(&text)})
                            .to_string();
                    let result = sanitized(&payload)?;
                    host.audit(
                        AuditKind::Workflow,
                        engine,
                        WHO,
                        AuditStatus::Ok,
                        &host_name,
                    );
                    Ok(result)
                }
                Err(e) => deny(format!("read failed: {e}")),
            }
        }
        Err(e) => deny(format!("request failed: {e}")),
    }
}

/// Events mediation handler: the outbox is the ONLY event door. Name carries
/// the run id; body is `{topic, payload, idempotency_key}`. Non-`workflow/*`
/// topics are refused (the alert bus stays server-owned).
fn run_mediated_event(
    host: &SqliteWorkflowHost,
    engine: &str,
    run: &str,
    body: &str,
) -> Result<String, String> {
    let run_id: i64 = match run.parse() {
        Ok(id) => id,
        Err(_) => return deny_simple(host, engine, "workflow/hostcall/events", "invalid run key"),
    };
    let who = format!("workflow/hostcall/events/run:{run_id}");
    let deny = |reason: String| {
        host.audit(
            AuditKind::Workflow,
            engine,
            &who,
            AuditStatus::Denied,
            &reason,
        );
        Err(reason)
    };
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return deny("invalid events payload".into()),
    };
    let topic = v.get("topic").and_then(|x| x.as_str()).unwrap_or_default();
    let payload = v.get("payload").and_then(|x| x.as_str()).unwrap_or("");
    let key = v
        .get("idempotency_key")
        .and_then(|x| x.as_str())
        .unwrap_or_default();
    let parent = v.get("parent_event_id").and_then(|x| x.as_i64());
    if !topic.starts_with("workflow/") || topic.len() > 128 {
        return deny("topic must be under workflow/*".into());
    }
    if payload.len() > EFFECT_OUTPUT_CAP {
        return deny("payload exceeds 64 KiB".into());
    }
    if key.is_empty() || key.len() > 256 {
        return deny("idempotency_key out of bounds".into());
    }
    match host.enqueue_with_parent(run_id, parent, topic, payload, key) {
        Ok((created, event_id)) => {
            host.audit(AuditKind::Workflow, engine, &who, AuditStatus::Ok, topic);
            // The id rides the receipt so the engine can parent its NEXT
            // emission without a second read.
            Ok(format!("enqueued:{created}:{event_id}"))
        }
        Err(e) => deny(format!("enqueue failed: {e}")),
    }
}

/// knowledge_suggest tool: the domain-scoped, quarantine-clean suggestion
/// read for `run_id`'s domain, sanitized. Body shape:
/// `{"run_id": <id>, "query": "..."}`.
fn run_knowledge_suggest(host: &SqliteWorkflowHost, body: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "invalid suggest payload".to_string())?;
    let run_id = v
        .get("run_id")
        .and_then(|x| x.as_i64())
        .ok_or_else(|| "run_id required".to_string())?;
    let q = v
        .get("query")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .trim();
    if q.is_empty() || q.len() > 512 {
        host.audit(
            AuditKind::Workflow,
            "engine",
            &format!("workflow/hostcall/tool/knowledge_suggest/run:{run_id}"),
            AuditStatus::Denied,
            "query out of bounds",
        );
        return Err("query out of bounds".into());
    }
    // The LIKE pattern escapes `%`/`_`/`\` so the query cannot inject
    // wildcards (data-shape class, same posture as the HTTP handler).
    let mut pattern = String::with_capacity(q.len() * 2);
    for c in q.chars() {
        match c {
            '%' | '_' | '\\' => {
                pattern.push('\\');
                pattern.push(c);
            }
            _ => pattern.push(c),
        }
    }
    // Substring match on both ends.
    pattern.insert(0, '%');
    pattern.push('%');
    let who = format!("workflow/hostcall/tool/knowledge_suggest/run:{run_id}");
    let hits = host.with_conn(|conn| {
        // Fail closed on an unknown run: a deleted run must not read as an
        // empty (ok) answer — resolve the domain explicitly first.
        let domain: String = conn
            .query_row(
                "SELECT domain FROM workflow_runs WHERE id = ?1",
                rusqlite::params![run_id],
                |r| r.get(0),
            )
            .map_err(|_| "run not found".to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT k.id, k.title, k.content FROM knowledge k \
                 WHERE k.domain = ?1 \
                   AND k.flagged = 0 \
                   AND (k.expires_at IS NULL OR k.expires_at >= ?2) \
                   AND k.content LIKE ?3 ESCAPE '\\' LIMIT 5",
            )
            .map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        let rows = stmt
            .query_map(rusqlite::params![domain, now, pattern], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, i64>(0)?,
                    "title": r.get::<_, String>(1)?,
                    "snippet": r.get::<_, String>(2)?.chars().take(200).collect::<String>(),
                }))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>();
        Ok(rows)
    })?;
    let payload = serde_json::Value::Array(hits).to_string();
    let result = sanitized(&payload)?;
    host.audit(
        AuditKind::Workflow,
        "engine",
        &who,
        AuditStatus::Ok,
        "suggest",
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use brain_engine_sdk::hostcall::DispatchError;
    use brain_engine_sdk::trust::{Decision, EngineOverride, PolicyMode};
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

    /// Env-mutating tests serialize on this lock (the compliance-test
    /// posture): env reads are process-global.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Poison-tolerant acquisition: a panicking sibling must not cascade
    /// PoisonErrors through every other env test (the CI failure mode).
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn host() -> Arc<SqliteWorkflowHost> {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(tmp.path());
        let pool = r2d2::Pool::builder().max_size(2).build(mgr).unwrap();
        run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB).unwrap();
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, status, created_at, updated_at)
                 VALUES ('acme', 'interview', '{\"note\":\"mail jane@example.com\"}', 'active', 1, 1)",
                [],
            )
            .unwrap();
        Arc::new(SqliteWorkflowHost::new(pool))
    }

    #[test]
    fn exec_and_env_are_denied_under_production_policy() {
        let _g = env_lock();
        unsafe { std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST") };
        let ctx = build(host(), "engine-a");
        assert!(matches!(
            ctx.dispatch("exec", "sh", "ls"),
            Err(DispatchError::Denied(_))
        ));
    }

    #[test]
    fn log_dispatch_runs_and_audits() {
        let ctx = build(host(), "engine-a");
        let out = ctx.dispatch("log", "boot", "hello").unwrap();
        assert!(out.starts_with("logged:boot:"));
        drop(ctx);
    }

    #[test]
    fn secret_mediation_never_publishes_material() {
        unsafe { std::env::set_var("BRAIN_BROKER_HC_TEST_KEY", "supersecret") }
        let ctx = build(host(), "engine-a");
        let out = ctx
            .dispatch("tool", "secret_status", "broker_hc_test")
            .unwrap();
        unsafe { std::env::remove_var("BRAIN_BROKER_HC_TEST_KEY") }
        assert_eq!(out, r#"{"configured":true}"#);
        assert!(!out.contains("supersecret"));

        let miss = ctx
            .dispatch("tool", "secret_status", "never_configured")
            .unwrap();
        assert_eq!(miss, r#"{"configured":false}"#);

        // Name-shape validation refuses injection-ish payloads.
        assert!(
            ctx.dispatch("tool", "secret_status", "bad name; rm -rf")
                .is_err()
        );
    }

    #[test]
    fn mediated_exec_refuses_destructive_commands() {
        let ctx = build(host(), "engine-a");
        assert!(ctx.dispatch("tool", "mediated_exec", "ls -l").is_ok());
        assert!(matches!(
            ctx.dispatch("tool", "mediated_exec", "rm -rf / --no-preserve-root"),
            Err(DispatchError::Internal(_))
        ));
    }

    #[test]
    fn session_view_is_sanitized() {
        let ctx = build(host(), "engine-a");
        let view = ctx.dispatch("session", "1", "").unwrap();
        assert!(!view.contains("jane@example.com"));
        assert!(view.contains("[redacted:"));
        assert_eq!(
            ctx.dispatch("session", "999", "").unwrap_err(),
            DispatchError::Internal("run not found".into())
        );
    }

    #[test]
    fn http_denied_by_default_and_allowlisted_host_passes() {
        let _g = env_lock();
        unsafe { std::env::remove_var("BRAIN_ENGINE_HTTP_ALLOWLIST") }
        let ctx = build(host(), "engine-a");
        assert!(
            ctx.dispatch("http", "fetch", r#"{"host":"example.com","path":"/"}"#)
                .is_err(),
            "deny-by-default: no allowlist, no egress"
        );

        // Allowlisted loopback host against a one-shot local HTTP server.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, BufReader, Write};
            let (stream, _) = listener.accept().unwrap();
            // Read the request to its header terminator BEFORE responding —
            // writing while the client is still sending races hyper into a
            // connection reset.
            let mut r = BufReader::new(stream);
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let mut w = r.into_inner();
            let _ =
                w.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nhi");
        });
        unsafe { std::env::set_var("BRAIN_ENGINE_HTTP_ALLOWLIST", format!("127.0.0.1:{port}")) }
        let ctx = build(host(), "engine-a");
        let out = ctx
            .dispatch(
                "http",
                "fetch",
                &format!(r#"{{"host":"127.0.0.1:{port}","path":"/x"}}"#),
            )
            .unwrap();
        assert!(out.contains("\"status\":200"), "got: {out}");
        server.join().unwrap();
        unsafe { std::env::remove_var("BRAIN_ENGINE_HTTP_ALLOWLIST") }
    }

    #[test]
    fn http_refuses_redirects_and_non_https_remote() {
        // The scheme law is structural: remote hosts are forced onto https,
        // loopback may speak plain http; redirects are refused by the shared
        // egress client (pinned in webhook tests) — this pins the URL law.
        assert_eq!(build_url("example.com", "/a"), "https://example.com/a");
        assert_eq!(build_url("localhost", "/a"), "http://localhost/a");
        assert_eq!(build_url("127.0.0.1:8080", "/b"), "http://127.0.0.1:8080/b");

        // Host-shape validation refuses injection-ish payloads outright, and
        // a non-allowlisted remote host is denied even when configured.
        let _g = env_lock();
        unsafe { std::env::set_var("BRAIN_ENGINE_HTTP_ALLOWLIST", "example.com") }
        let ctx = build(host(), "engine-a");
        for bad in [
            r#"{"host":"ex ample.com","path":"/"}"#,
            r#"{"host":"","path":"/"}"#,
            r#"{"host":"example.com","path":"no-slash"}"#,
            r#"{"host":"evil.com","path":"/"}"#,
        ] {
            assert!(ctx.dispatch("http", "f", bad).is_err(), "refused: {bad}");
        }
        unsafe { std::env::remove_var("BRAIN_ENGINE_HTTP_ALLOWLIST") }
    }

    #[test]
    fn events_handler_enforces_workflow_topic_prefix_and_size() {
        let h = host();
        let ctx = build(h.clone(), "engine-a");
        let ok = ctx
            .dispatch(
                "events",
                "1",
                r#"{"topic":"workflow/log","payload":"{\"a\":1}","idempotency_key":"k-ev-1"}"#,
            )
            .unwrap();
        assert_eq!(ok, "enqueued:true:1", "receipt carries the event id");
        // Replay by key is an idempotent no-op receipt (same id resolved).
        let replay = ctx
            .dispatch(
                "events",
                "1",
                r#"{"topic":"workflow/log","payload":"{}","idempotency_key":"k-ev-1"}"#,
            )
            .unwrap();
        assert_eq!(replay, "enqueued:false:1");

        // Non-workflow topics are refused — the alert bus stays server-owned.
        for bad_topic in ["alerts/page", "workflow", "", "../secrets"] {
            let body =
                format!(r#"{{"topic":"{bad_topic}","payload":"{{}}","idempotency_key":"k-x"}}"#);
            assert!(
                ctx.dispatch("events", "1", &body).is_err(),
                "topic refused: {bad_topic}"
            );
        }

        // Payloads over 64 KiB and missing keys are refused.
        let big_payload = "y".repeat(64 * 1024 + 1);
        let body = format!(
            r#"{{"topic":"workflow/log","payload":"{big_payload}","idempotency_key":"k-big"}}"#
        );
        assert!(ctx.dispatch("events", "1", &body).is_err());
        assert!(
            ctx.dispatch("events", "1", r#"{"topic":"workflow/x","payload":"{}"}"#)
                .is_err()
        );
    }

    #[test]
    fn ui_denied_with_named_reason() {
        // Under production policy ui prompts → server-side Denied (Prompt ==
        // Denied until Witness wires the GUI).
        let ctx = build(host(), "engine-a");
        let err = ctx.dispatch("ui", "dialog", "{}").unwrap_err();
        assert!(matches!(err, DispatchError::Denied(_)));

        // And even where policy would admit it, the handler names its refusal
        // (an explicit refusal, not an absence).
        let h = host();
        let open_ctx = HostCallContext::new(h.clone(), ExtensionPolicy::permissive(), "engine-a");
        register_handlers(&open_ctx, &h, "engine-a");
        let err = open_ctx.dispatch("ui", "dialog", "{}").unwrap_err();
        assert_eq!(err.to_string(), "internal: reserved: lands with Cockpit");
    }

    #[test]
    fn hostcall_table_is_exhaustive() {
        let ctx = build(host(), "engine-a");
        for wire in ["tool", "exec", "http", "session", "events", "ui", "log"] {
            assert!(
                ctx.has_handler(HostCallKind::parse(wire).unwrap()),
                "kind `{wire}` must have a registered handler"
            );
        }
    }

    #[test]
    fn dispatch_counter_increments_per_kind_and_report_carries_it() {
        let _g = env_lock();
        unsafe { std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "") }
        let ctx = build(host(), "engine-a");
        ctx.dispatch("log", "boot", "hello").unwrap();
        ctx.dispatch("log", "boot", "again").unwrap();
        assert!(ctx.dispatch("exec", "sh", "ls").is_err()); // denied counts too
        ctx.dispatch("tool", "secret_status", "nope_missing")
            .unwrap();
        let counters = ctx.counters();
        assert_eq!(
            counters.get(&("boot".to_string(), "log".to_string())),
            Some(&2)
        );
        assert_eq!(
            counters.get(&("sh".to_string(), "exec".to_string())),
            Some(&1),
            "denials tally"
        );
        assert!(counters.contains_key(&("secret_status".to_string(), "tool".to_string())));
        unsafe { std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST") }
    }

    #[test]
    fn knowledge_suggest_is_domain_scoped_and_sanitized() {
        let h = host();
        // Seed knowledge in TWO domains; only the run's domain may answer.
        h.with_conn(|c| -> Result<(), String> {
            c.execute(
                "INSERT INTO knowledge(domain, title, content, flagged, source) VALUES ('acme','Acme runbook','mail jane@example.com for access',0,'manual')",
                [],
            )
            .map_err(|e| e.to_string())?;
            c.execute(
                "INSERT INTO knowledge(domain, title, content, flagged, source) VALUES ('other','Other runbook','mail boss@other.example for help',0,'manual')",
                [],
            )
            .map_err(|e| e.to_string())?;
            c.execute(
                "INSERT INTO knowledge(domain, title, content, flagged, source) VALUES ('acme','Flagged acme','runbook secret sauce',1,'manual')",
                [],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();

        let ctx = build(h.clone(), "engine-a");
        let body = serde_json::json!({"run_id": 1, "query": "access"}).to_string();
        let out = ctx.dispatch("tool", "knowledge_suggest", &body).unwrap();
        assert!(out.contains("[redacted:"), "PII sanitized: {out}");
        assert!(!out.contains("jane@example.com"), "raw PII crossed: {out}");
        assert!(
            !out.contains("boss@other.example"),
            "cross-domain row leaked: {out}"
        );
        assert!(
            !out.contains("secret sauce"),
            "flagged (quarantined) row leaked: {out}"
        );

        // Query bounds fail closed.
        assert!(
            ctx.dispatch("tool", "knowledge_suggest", r#"{"run_id":1,"query":""}"#)
                .is_err()
        );
        let too_long = serde_json::json!({"run_id": 1, "query": "x".repeat(513)}).to_string();
        assert!(
            ctx.dispatch("tool", "knowledge_suggest", &too_long)
                .is_err()
        );

        // A missing run fails closed — never an empty (ok) answer.
        let missing = serde_json::json!({"run_id": 999, "query": "access"}).to_string();
        assert!(
            ctx.dispatch("tool", "knowledge_suggest", &missing)
                .unwrap_err()
                .to_string()
                .contains("run not found")
        );
    }

    #[test]
    fn hostcall_audits_resolve_the_run_domain_tenant() {
        let _g = env_lock();
        unsafe { std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo") };
        let h = host();
        let ctx = build(h.clone(), "engine-a");
        assert!(
            ctx.dispatch("exec", "1", r#"{"argv":["/bin/cat","x"]}"#)
                .is_err()
        );
        // Audit targets are stored hashed; assert via the resolved tenant.
        let rows: Vec<(String, String)> = h
            .with_conn(|c| -> Result<Vec<(String, String)>, String> {
                let mut stmt = c
                    .prepare(
                        "SELECT detail_hash, status FROM audit_events \
                         WHERE kind='workflow' AND tenant_id = 'acme' ORDER BY id",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(|e| e.to_string())?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            })
            .unwrap();
        // The dispatch gate audit (target `exec/1`, global tenant) plus the
        // handler's run-scoped denial — only the latter lands on `acme`.
        let acme_denials = rows.iter().filter(|(_, s)| s == "denied").count();
        assert_eq!(acme_denials, 1, "exactly one run-scoped denial audited");
        unsafe { std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST") };
    }

    #[test]
    fn unknown_tool_fails_closed() {
        let ctx = build(host(), "engine-a");
        assert!(matches!(
            ctx.dispatch("tool", "wild_tool", "{}"),
            Err(DispatchError::Internal(_))
        ));
    }

    #[test]
    fn per_engine_allow_cannot_reinstate_denied_exec() {
        let _g = env_lock();
        unsafe { std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST") }
        let mut policy = production_policy("locked-engine");
        policy.per_engine.insert(
            "locked-engine".into(),
            EngineOverride {
                allow_caps: vec!["exec".into(), "env".into()],
                deny_caps: vec![],
            },
        );
        // The global deny outranks a per-engine allow — always.
        assert_eq!(policy.decide("locked-engine", "exec"), Decision::Denied);
        assert_eq!(policy.decide("locked-engine", "env"), Decision::Denied);
        assert_eq!(policy.mode, PolicyMode::Prompt);
    }

    #[test]
    fn exec_denied_when_allowlist_empty() {
        let _g = env_lock();
        unsafe { std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST") }
        assert!(exec_allowlist().is_empty());
        let ctx = build(host(), "engine-a");
        let err = ctx.dispatch("exec", "1", r#"{"argv":["ls"]}"#).unwrap_err();
        assert!(matches!(err, DispatchError::Denied(_)));
        // An explicit empty string keeps the same posture.
        unsafe { std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "") }
        let ctx = build(host(), "engine-a");
        assert!(matches!(
            ctx.dispatch("exec", "1", r#"{"argv":["ls"]}"#),
            Err(DispatchError::Denied(_))
        ));
        unsafe { std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST") }
    }

    #[test]
    fn exec_runs_only_allowlisted_argv0_with_cwd_and_timeout() {
        let _g = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("BRAIN_ENGINE_WORKDIR", tmp.path()) }
        unsafe { std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo /bin/ls") }
        let ctx = build(host(), "engine-a");

        // Allowlisted argv0 runs (cwd pinned — `ls` lists the empty tempdir).
        let out = ctx
            .dispatch("exec", "1", r#"{"argv":["/bin/ls","-a"]}"#)
            .unwrap();
        assert!(out.contains("\"exit_code\":0"), "got: {out}");

        // Un-allowlisted argv0 is refused.
        let err = ctx
            .dispatch("exec", "1", r#"{"argv":["/bin/cat","/etc/hostname"]}"#)
            .unwrap_err();
        assert!(err.to_string().contains("not in exec allowlist"));

        // Destructive content is refused even when allowlisted.
        let err = ctx
            .dispatch("exec", "1", r#"{"argv":["/bin/echo","rm -rf /"]}"#)
            .unwrap_err();
        assert!(err.to_string().contains("dangerous command refused"));

        // Malformed payloads fail loudly.
        assert!(ctx.dispatch("exec", "1", "not json").is_err());
        assert!(
            ctx.dispatch("exec", "1", r#"{"argv":[]}"#).is_err(),
            "empty argv refused"
        );
        unsafe { std::env::remove_var("BRAIN_ENGINE_WORKDIR") }
        unsafe { std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST") }
    }

    #[test]
    fn exec_output_is_sanitized_and_capped() {
        let _g = env_lock();
        unsafe { std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo") }
        let ctx = build(host(), "engine-a");
        // An email address in the output must cross the boundary redacted.
        let out = ctx
            .dispatch(
                "exec",
                "1",
                r#"{"argv":["/bin/echo","mail jane@example.com now"]}"#,
            )
            .unwrap();
        assert!(!out.contains("jane@example.com"), "PII crossed raw: {out}");
        assert!(out.contains("[redacted:"), "sanitized form: {out}");

        // Oversized output is capped at the 64 KiB bound per stream. The
        // volume comes from STDOUT (`cat` on a big file), never from argv —
        // Linux rejects a single argument over ~128 KiB with E2BIG, which
        // made this test platform-dependent rather than a real cap probe.
        let big_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(big_file.path(), vec![b'x'; 200_000]).unwrap();
        unsafe { std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo /bin/cat") }
        let ctx = build(host(), "engine-a");
        let body = serde_json::json!({
            "argv": ["/bin/cat", big_file.path().to_str().unwrap()],
        })
        .to_string();
        let out = ctx.dispatch("exec", "1", &body).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let stdout_len = v["stdout"].as_str().map(|s| s.len()).unwrap_or(0);
        assert!(stdout_len <= 64 * 1024, "stdout not capped: {stdout_len}");
        unsafe { std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST") }
    }
}
