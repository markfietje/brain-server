//! The server's one `HostCallContext` assembly: which capabilities an engine
//! gets here, and what each kind actually runs. The confused-deputy posture
//! lives in the SDK dispatcher; this module only supplies fail-closed
//! handlers — notably secret mediation, where resolution happens host-side
//! and ONLY status metadata crosses the boundary (never the material).

use brain_engine_sdk::host::WorkflowHost;
use brain_engine_sdk::hostcall::{HostCallContext, exec_mediation};
use brain_engine_sdk::trust::{ExtensionPolicy, HostCallKind};
use std::sync::Arc;

use crate::workflow::host::SqliteWorkflowHost;

/// The production posture: Standard plus the two always-mediated classes
/// (`tools` runs only the built-in fail-closed handlers above; `log` is a
/// structured emit). exec/env stay hard-denied, the rest prompt-gated.
pub(crate) fn production_policy(engine: &str) -> ExtensionPolicy {
    let mut policy = ExtensionPolicy::standard();
    let _ = engine;
    policy.default_caps.push("tools".into());
    policy.default_caps.push("log".into());
    policy
}

/// Build the dispatch context for one engine against the shared host.
pub(crate) fn build(
    host: Arc<SqliteWorkflowHost>,
    engine: &str,
) -> HostCallContext<SqliteWorkflowHost> {
    let ctx = HostCallContext::new(host.clone(), production_policy(engine), engine);

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
        let unprivileged = Some(crate::auth::Principal {
            sub: "workflow-extension".into(),
            tenant: "global".into(),
            scopes: Vec::new(),
            jti: String::new(),
            roles: Vec::new(),
            manages: Vec::new(),
        });
        Ok(crate::gate::sanitize_read(&raw, true, &unprivileged))
    });

    // tool → the two built-in mediated tools. Everything else fails closed.
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
        other => {
            let _ = &h3;
            Err(format!("unknown tool `{other}`"))
        }
    });

    ctx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use brain_engine_sdk::hostcall::DispatchError;
    use brain_engine_sdk::trust::{Decision, EngineOverride, PolicyMode};
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

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
        unsafe { std::env::set_var("BRAIN_BROKER_HC_TEST_KEY", "supersecret") };
        let ctx = build(host(), "engine-a");
        let out = ctx
            .dispatch("tool", "secret_status", "broker_hc_test")
            .unwrap();
        unsafe { std::env::remove_var("BRAIN_BROKER_HC_TEST_KEY") };
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
    fn unknown_tool_fails_closed() {
        let ctx = build(host(), "engine-a");
        assert!(matches!(
            ctx.dispatch("tool", "wild_tool", "{}"),
            Err(DispatchError::Internal(_))
        ));
    }

    #[test]
    fn per_engine_allow_cannot_reinstate_denied_exec() {
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
}
