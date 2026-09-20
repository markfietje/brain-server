//! The mediated exec bridge — the loop's ONLY process path.
//!
//! The kernel's exec mediation (argv0 canonicalize-and-refuse, the danger
//! screen incl. pipe-to-shell tripwires, the deadline kill) was hardened
//! while dormant, pinned unwired until this line. This module is THE
//! wiring: the loop's `exec` tool routes every process invocation through
//! `hostcalls::build`'s mediated dispatch — never a private spawn path,
//! never the raw seam. The dormancy pin's deletion and the live-mediation
//! pins below landed in the same commit as this call, by design: wiring
//! without deleting the pin fails the old law; deleting without live
//! replacement pins would be a self-vacating guard.
//!
//! What this deliberately does NOT do: no shell — the tool takes an argv
//! ARRAY (the model cannot smuggle metacharacters past the array boundary),
//! the allowlist stays operator env truth (`BRAIN_ENGINE_EXEC_ALLOWLIST`,
//! empty = deny ALL exec, fail-closed), and denials surface as typed tool
//! errors the model can see, never as silent no-ops. The runner-supplied
//! `ExecutionEnv` is consumed BEFORE any mediation work — a loop (parent or
//! delegated child) whose env denies process, or whose non-empty command
//! list excludes argv0, is refused at the bridge; an empty command list
//! stays the parent-loop posture where the operator allowlist is the trust
//! anchor.

use std::sync::Arc;

use brain_engine_sdk::env::{EnvError, ToolDef};

use crate::workflow::host::SqliteWorkflowHost;

/// Build the loop's `exec` tool over the mediated hostcall path. The
/// dispatch context is assembled ONCE per tool (handler registration is
/// per-context); every invocation then rides the four-step pipeline
/// (interceptor → canonicalize → capability check → handler) — but only
/// AFTER the loop's own env gate: capability subtraction delegated to a
/// child must hold at the seam the child actually reaches, not just at the
/// operator allowlist.
pub(crate) fn mediated_exec_tool(host: Arc<SqliteWorkflowHost>, engine: &str) -> ToolDef {
    let ctx = crate::workflow::hostcalls::build(host, engine);
    ToolDef::new(
        "exec",
        "run one allowlisted command (argv array) through the mediated hostcall path",
        r#"{"argv":["string"]}"#,
        move |env, input| {
            // Fixed text, no payload echo: a process-denying env refuses
            // before anything about the request is examined.
            if !env.allow_process {
                return Err(EnvError::Denied(
                    "process execution disabled by environment".into(),
                ));
            }
            let body = argv_body(input)?;
            // Non-empty loop-level command list: exact-match on argv[0]. An
            // empty list is the parent-loop posture — no loop-level command
            // restriction; the operator allowlist remains the trust anchor.
            if !env.allowed_commands.is_empty() {
                let argv0 = argv_head(&body)?;
                if !env.allowed_commands.iter().any(|c| c == &argv0) {
                    return Err(EnvError::Denied(format!(
                        "command not permitted by loop environment allowlist: {argv0}"
                    )));
                }
            }
            ctx.dispatch("exec", "loop-exec", &body)
                .map_err(|e| EnvError::Denied(e.to_string()))
        },
    )
}

/// Normalize the model's tool arguments into the Exec handler's payload:
/// a bare JSON array becomes `{"argv": [...]}`; an object carrying `argv`
/// passes as-is; anything else is refused loudly (malformed, never guessed).
fn argv_body(input: &str) -> Result<String, EnvError> {
    let value: serde_json::Value = serde_json::from_str(input).map_err(|_| {
        EnvError::Denied(r#"malformed exec arguments: expected {"argv":["..."]}"#.into())
    })?;
    match value {
        serde_json::Value::Array(_) => Ok(serde_json::json!({ "argv": value }).to_string()),
        serde_json::Value::Object(ref map) if map.contains_key("argv") => Ok(input.to_string()),
        _ => Err(EnvError::Denied(
            r#"malformed exec arguments: expected {"argv":["..."]}"#.into(),
        )),
    }
}

/// argv[0] of an already-normalized exec body (`{"argv":[...]}`) — the head
/// the loop env allowlist exact-matches, mirroring the mediation's own
/// argv0 law. The body was just built or passed by `argv_body`, so a body
/// without a string head is malformed input, refused loudly.
fn argv_head(body: &str) -> Result<String, EnvError> {
    let malformed =
        || EnvError::Denied(r#"malformed exec arguments: expected {"argv":["..."]}"#.into());
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| malformed())?;
    value["argv"][0]
        .as_str()
        .map(str::to_string)
        .ok_or_else(malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentloop::hooks::LoopHooks;
    use crate::agentloop::provider::{LoopbackProvider, scripted_text, scripted_text_then_tool};
    use crate::agentloop::run_loop::{LoopConfig, LoopDriver, RunOutcome};
    use crate::audit::verify_chain;
    use crate::config;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::session_log;
    use brain_engine_sdk::env::{DenyAll, ExecutionEnv};
    use tokio_util::sync::CancellationToken;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// The parent-loop posture the mediation pins ride: process granted, no
    /// loop-level command restriction — the operator allowlist stays the
    /// trust anchor (checkpoint posture case 3). `ExecutionEnv::default()`
    /// is the deny-process posture, which the loop env gate now enforces;
    /// these pins subject is the OPERATOR law, not the env gate.
    fn parent_posture() -> ExecutionEnv {
        ExecutionEnv {
            fs: Arc::new(DenyAll),
            read_only: true,
            allow_process: true,
            root: "/".into(),
            allowed_commands: vec![],
        }
    }

    fn tool() -> ToolDef {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = crate::pool::SqliteConnectionManager::file(tmp.path());
        let pool: crate::Pool = r2d2::Pool::builder().max_size(2).build(mgr).unwrap();
        run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB).unwrap();
        mediated_exec_tool(Arc::new(SqliteWorkflowHost::new(pool)), "agentloop")
    }

    #[test]
    fn empty_allowlist_denies_all_exec_live() {
        let _g = crate::test_support::lock_env();
        unsafe {
            std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST");
        }
        let err = tool()
            .execute(&parent_posture(), r#"["/bin/echo","x"]"#)
            .unwrap_err();
        assert!(
            matches!(err, EnvError::Denied(ref m) if m.to_lowercase().contains("exec")),
            "no allowlist, no exec — fail closed: {err:?}"
        );
    }

    #[test]
    fn argv0_law_is_live_unallowlisted_binary_refused() {
        let _g = crate::test_support::lock_env();
        unsafe {
            std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo");
        }
        let err = tool()
            .execute(&parent_posture(), r#"["/usr/bin/id","-u"]"#)
            .unwrap_err();
        assert!(
            matches!(err, EnvError::Denied(ref m) if m.contains("allowlist")),
            "an honest binary outside the allowlist refuses: {err:?}"
        );
    }

    #[test]
    fn danger_screen_is_live_pipe_and_rm_refused() {
        let _g = crate::test_support::lock_env();
        unsafe {
            std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo");
        }
        for argv in [
            r#"["/bin/echo","; rm -rf /"]"#, // destructive-prefix smuggling in an arg
            r#"["/bin/echo","| bash"]"#,     // the pipe-to-shell family
        ] {
            let err = tool().execute(&parent_posture(), argv).unwrap_err();
            assert!(
                matches!(err, EnvError::Denied(ref m) if m.contains("dangerous command refused")),
                "the tripwire fires LIVE on {argv}: {err:?}"
            );
        }
        // Honest ceiling, pinned: argv-array exec runs NO shell, so a
        // substitution STRING as a literal argument is inert — the screen
        // passes it and echo prints it verbatim. The dangerous forms are
        // the ones the screen names (pipe-to-shell, destructive prefixes).
        let inert = tool()
            .execute(&parent_posture(), r#"["/bin/echo","$(curl x)"]"#)
            .unwrap();
        assert!(
            inert.contains("$(curl x)"),
            "literal args stay literal: {inert}"
        );
    }

    #[test]
    fn allowlisted_command_runs_and_malformed_input_refuses() {
        let _g = crate::test_support::lock_env();
        unsafe {
            std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo");
        }
        let out = tool()
            .execute(&parent_posture(), r#"["/bin/echo","mediated-ok"]"#)
            .unwrap();
        assert!(out.contains("mediated-ok"), "output rides back: {out}");
        // Object form passes through; garbage refuses loudly.
        let obj = tool().execute(&parent_posture(), r#"{"argv":["/bin/echo","object-form"]}"#);
        assert!(obj.unwrap().contains("object-form"));
        let err = tool().execute(&parent_posture(), "rm -rf /").unwrap_err();
        assert!(matches!(err, EnvError::Denied(ref m) if m.contains("malformed")));
        let err = tool()
            .execute(&parent_posture(), r#"{"cmd":"ls"}"#)
            .unwrap_err();
        assert!(matches!(err, EnvError::Denied(ref m) if m.contains("malformed")));
    }

    #[test]
    fn exec_bridge_enforces_loop_process_denial() {
        let _g = crate::test_support::lock_env();
        // Operator allowlist left EMPTY: the old (env-blind) path cannot
        // execute anything, so the only way the denial text can differ from
        // the operator's own argv0 refusal is the loop env gate firing first.
        unsafe {
            std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST");
        }
        let env = ExecutionEnv {
            fs: Arc::new(DenyAll),
            read_only: true,
            allow_process: false,
            root: "/".into(),
            allowed_commands: vec![],
        };
        let err = tool().execute(&env, r#"["/bin/echo","x"]"#).unwrap_err();
        assert!(
            matches!(err, EnvError::Denied(ref m) if m.contains("process execution disabled by environment")),
            "a process-denying loop env denies BEFORE mediation sees argv: {err:?}"
        );
    }

    #[test]
    fn exec_bridge_enforces_loop_command_allowlist() {
        let _g = crate::test_support::lock_env();
        unsafe {
            std::env::remove_var("BRAIN_ENGINE_EXEC_ALLOWLIST");
        }
        let env = ExecutionEnv {
            fs: Arc::new(DenyAll),
            read_only: true,
            allow_process: true,
            root: "/".into(),
            allowed_commands: vec!["/bin/ls".into()],
        };
        let err = tool().execute(&env, r#"["/bin/echo","x"]"#).unwrap_err();
        assert!(
            matches!(err, EnvError::Denied(ref m) if m.contains("loop environment allowlist")),
            "argv0 outside the loop env's command list refuses: {err:?}"
        );
    }

    /// THE security delta of the env gate, proven LIVE with the real binary (focused-only
    /// per the sanitized-run inventory; excluded from broad runs). The operator
    /// allowlisted /bin/echo; the loop env denies process — the loop env
    /// must win. RED today: the command RUNS.
    #[test]
    fn exec_bridge_process_denial_beats_operator_allowlist() {
        let _g = crate::test_support::lock_env();
        unsafe {
            std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo");
        }
        let env = ExecutionEnv {
            fs: Arc::new(DenyAll),
            read_only: true,
            allow_process: false,
            root: "/".into(),
            allowed_commands: vec![],
        };
        let err = tool()
            .execute(&env, r#"["/bin/echo","denied"]"#)
            .unwrap_err();
        assert!(
            matches!(err, EnvError::Denied(ref m) if m.contains("process execution disabled by environment")),
            "process denial beats a permissive operator allowlist: {err:?}"
        );
    }

    #[test]
    fn loop_exec_rides_the_mediated_hostcalls_end_to_end() {
        let _g = crate::test_support::lock_env();
        unsafe {
            std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo");
        }
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = crate::pool::SqliteConnectionManager::file(tmp.path());
        let pool: crate::Pool = r2d2::Pool::builder().max_size(4).build(mgr).unwrap();
        run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB).unwrap();
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
                [],
            )
            .unwrap();
        let host = Arc::new(SqliteWorkflowHost::new(pool.clone()));
        let harness = Arc::new(brain_engine_sdk::harness::AgentHarness::new(
            host.clone(),
            "m",
            "s",
        ));
        let provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text_then_tool(
                    "checking uptime",
                    "e1",
                    "exec",
                    r#"["/bin/echo","loop-exec-ran"]"#,
                ),
                scripted_text("done"),
            ],
        );
        // ONE host for the loop AND the tool: the mediation's audit rows
        // land in the same chain the loop writes.
        let exec_host = Arc::clone(&host);
        let driver = LoopDriver::new(
            pool,
            Arc::clone(&host),
            harness,
            provider,
            vec![mediated_exec_tool(exec_host, "agentloop")],
            ExecutionEnv {
                fs: Arc::new(DenyAll),
                read_only: true,
                allow_process: true,
                root: "/".into(),
                allowed_commands: vec![],
            },
            LoopConfig::default(),
            "",
            LoopHooks::pass_through(),
        );
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(driver.run_turns(1, "check via exec", &cancel))
            .unwrap();
        assert!(matches!(outcome, RunOutcome::Completed { turns: 2, .. }));
        let conn = rusqlite::Connection::open(tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let tool_event = events.iter().find(|e| e.kind == "tool_result").unwrap();
        let payload: serde_json::Value = serde_json::from_str(&tool_event.payload_json).unwrap();
        assert_eq!(payload["ok"], serde_json::json!(true));
        assert!(
            payload["output"]
                .as_str()
                .unwrap()
                .contains("loop-exec-ran")
        );
        assert!(
            verify_chain(&conn),
            "mediated exec writes land in the verified chain"
        );
    }
}
