//! Scoped subagent delegation: a child harness under the parent's ceiling.
//!
//! A subagent is a child [`AgentHarness`] over the SAME host (every child
//! act — RunStart, session events, RunEnd — lands in the same hash-chained
//! audit stream and the same run's session log), with a NARROWED
//! [`ExecutionEnv`] (capability subtraction, never addition: the child's
//! powers are the intersection of what the spec asks and what the parent
//! already had) and a SUBSET of the parent's tools (the registry's
//! presentation alignment means what the child is shown is exactly what it
//! can run). The parent/child budget split is a hard token ceiling: a child
//! that crosses it stops loudly at the turn boundary, never silently.
//!
//! The child's session narrative is written with a `child:<name>:` kind
//! prefix so replay distinguishes it from the parent's; the delegation
//! itself appends one parent-visible `subagent_result` event carrying the
//! child's final answer. What this deliberately does NOT do: nested fibers
//! (the plugin kernel's declared ceiling), capability addition, or a
//! parallel identity — the child rides the parent's principal and audit
//! chain, full stop.

use std::sync::Arc;

use brain_engine_sdk::env::{ExecutionEnv, ToolDef};
use brain_engine_sdk::harness::AgentHarness;
use tokio_util::sync::CancellationToken;

use crate::Pool;
use crate::agentloop::provider::LlmProvider;
use crate::agentloop::run_loop::{
    LoopConfig, LoopDriver, LoopError, RunOutcome, append_session_events,
};
use crate::workflow::host::SqliteWorkflowHost;

/// What a subagent spec ASKS to keep. Asking never grants: the child env is
/// the intersection of this with the parent's actual powers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SubagentCaps {
    pub write: bool,
    pub process: bool,
    /// Command allowlist asked for (empty = inherit the parent's list).
    pub commands: Vec<String>,
}

/// Pure capability subtraction: build a child env that can never exceed the
/// parent's. Write needs BOTH the spec and a writable parent; process needs
/// BOTH the spec and a process-capable parent; commands intersect (an empty
/// ask inherits, an empty parent list is the parent's permissive posture the
/// child's explicit ask narrows).
pub(crate) fn narrowed_env(parent: &ExecutionEnv, caps: &SubagentCaps) -> ExecutionEnv {
    let commands = if caps.commands.is_empty() {
        parent.allowed_commands.clone()
    } else if parent.allowed_commands.is_empty() {
        caps.commands.clone()
    } else {
        caps.commands
            .iter()
            .filter(|c| parent.allowed_commands.contains(c))
            .cloned()
            .collect()
    };
    ExecutionEnv {
        fs: Arc::clone(&parent.fs),
        read_only: parent.read_only || !caps.write,
        allow_process: parent.allow_process && caps.process,
        root: parent.root.clone(),
        allowed_commands: commands,
    }
}

/// One delegation request.
#[derive(Debug, Clone)]
pub(crate) struct SubagentSpec {
    /// Stable label; rides the session kinds (`child:<name>:`) and the
    /// result event.
    pub name: String,
    /// The child's own system prompt (its harness config, not the parent's).
    pub system_prompt: String,
    /// The task, verbatim, as the child's user input.
    pub task: String,
    /// Tool names the child may see (subset of the parent's set).
    pub allowed_tools: Vec<String>,
    pub caps: SubagentCaps,
    pub max_turns: u32,
    /// Hard cumulative token ceiling for the whole child run.
    pub token_budget: u64,
}

/// How a delegation ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SubagentOutcome {
    /// The child finished a turn tool-free; `summary` is its final text.
    Completed {
        summary: String,
        usage: crate::agentloop::provider::Usage,
    },
    /// The child crossed its token budget; stopped loudly at the boundary.
    BudgetExceeded {
        usage: crate::agentloop::provider::Usage,
    },
    /// The child hit its turn cap without finishing.
    Capped {
        usage: crate::agentloop::provider::Usage,
    },
    /// The caller's token fired.
    Canceled,
}

/// Delegate one scoped task to a child loop: same host (same audit chain),
/// same run (same session log, kind-prefixed), narrowed env, filtered
/// tools, hard budget. Appends a parent-visible `subagent_result` event.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn delegate(
    pool: &Pool,
    host: &Arc<SqliteWorkflowHost>,
    parent_env: &ExecutionEnv,
    parent_tools: &[ToolDef],
    provider: Arc<dyn LlmProvider>,
    run_id: i64,
    spec: &SubagentSpec,
    cancel: &CancellationToken,
) -> Result<SubagentOutcome, LoopError> {
    let child_harness = Arc::new(AgentHarness::new(
        Arc::clone(host),
        "child",
        &spec.system_prompt,
    ));
    let tools: Vec<ToolDef> = parent_tools
        .iter()
        .filter(|t| spec.allowed_tools.contains(&t.name))
        .cloned()
        .collect();
    let config = LoopConfig {
        max_turns: spec.max_turns,
        token_budget: Some(spec.token_budget),
        ..LoopConfig::default()
    };
    let prefix = format!("child:{}:", spec.name);
    let child = LoopDriver::new(
        pool.clone(),
        Arc::clone(host),
        child_harness,
        provider,
        tools,
        narrowed_env(parent_env, &spec.caps),
        config,
        &prefix,
    );
    let outcome = child.run_turns(run_id, &spec.task, cancel).await?;
    let (result, follow_up) = match &outcome {
        RunOutcome::Completed { turns, usage } => {
            let summary = child_final_text(pool, run_id, &prefix).await?;
            (
                SubagentOutcome::Completed {
                    summary: summary.clone(),
                    usage: *usage,
                },
                serde_json::json!({"name": spec.name, "outcome": "completed", "turns": turns, "summary": summary}),
            )
        }
        RunOutcome::BudgetExceeded { turns, usage } => (
            SubagentOutcome::BudgetExceeded { usage: *usage },
            serde_json::json!({"name": spec.name, "outcome": "budget_exceeded", "turns": turns}),
        ),
        RunOutcome::TurnCapReached { turns, usage } => (
            SubagentOutcome::Capped { usage: *usage },
            serde_json::json!({"name": spec.name, "outcome": "capped", "turns": turns}),
        ),
        RunOutcome::Canceled => (
            SubagentOutcome::Canceled,
            serde_json::json!({"name": spec.name, "outcome": "canceled"}),
        ),
    };
    append_session_events(
        pool,
        run_id,
        vec![(
            "subagent_result".to_string(),
            follow_up.to_string(),
            format!("run{run_id}:subagent:{}", spec.name),
        )],
    )
    .await?;
    Ok(result)
}

/// The child's final assistant text: the last `child:<name>:assistant`
/// event in the run's log (the parent's own narrative is never consulted).
async fn child_final_text(pool: &Pool, run_id: i64, prefix: &str) -> Result<String, LoopError> {
    let kind = format!("{prefix}assistant");
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool
            .get()
            .map_err(|e| LoopError::Persist(format!("pool: {e}")))?;
        crate::workflow::session_log::replay(
            &conn,
            run_id,
            crate::workflow::session_log::REPLAY_CAP,
        )
        .map_err(|e| LoopError::Persist(e.to_string()))
    })
    .await
    .map_err(|e| LoopError::Persist(format!("replay join failed: {e}")))?
    .map(|events| {
        events
            .iter()
            .rev()
            .find(|e| e.kind == kind)
            .and_then(|e| serde_json::from_str::<serde_json::Value>(&e.payload_json).ok())
            .and_then(|v| v.get("text")?.as_str().map(str::to_string))
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentloop::provider::{
        LoopbackProvider, Usage, scripted_text, scripted_text_then_tool,
    };
    use crate::audit::verify_chain;
    use crate::config;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::session_log;
    use brain_engine_sdk::env::{DenyAll, FsSeam};
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct MemFs(StdMutex<HashMap<String, String>>);
    impl FsSeam for MemFs {
        fn read(&self, p: &str) -> Result<String, brain_engine_sdk::env::EnvError> {
            self.0
                .lock()
                .ok()
                .and_then(|g| g.get(p).cloned())
                .ok_or_else(|| brain_engine_sdk::env::EnvError::NotFound(p.into()))
        }
        fn write(&self, p: &str, c: &str) -> Result<(), brain_engine_sdk::env::EnvError> {
            if let Ok(mut g) = self.0.lock() {
                g.insert(p.into(), c.into());
            }
            Ok(())
        }
        fn exec(&self, c: &str) -> Result<String, brain_engine_sdk::env::EnvError> {
            Ok(format!("ran {c}"))
        }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn caps(write: bool, process: bool, commands: &[&str]) -> SubagentCaps {
        SubagentCaps {
            write,
            process,
            commands: commands.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn narrowed_env_never_exceeds_the_parent() {
        let fs: Arc<dyn FsSeam> = Arc::new(MemFs::default());
        let parent = ExecutionEnv {
            fs: Arc::clone(&fs),
            read_only: false,
            allow_process: true,
            root: "/srv".into(),
            allowed_commands: vec!["ls".into(), "cat".into()],
        };
        // Asking for everything still lands at most at the parent's posture.
        let full = narrowed_env(&parent, &caps(true, true, &["ls", "cat", "curl"]));
        assert!(!full.read_only);
        assert!(full.allow_process);
        assert_eq!(
            full.allowed_commands,
            vec!["ls".to_string(), "cat".to_string()]
        );
        assert_eq!(full.root, "/srv", "same seam, same root");
        // A read-only parent stays read-only however loudly the child asks.
        let ro_parent = ExecutionEnv {
            read_only: true,
            ..parent.clone()
        };
        let asking = narrowed_env(&ro_parent, &caps(true, true, &[]));
        assert!(asking.read_only, "write cannot be granted upward");
        // A process-denying parent denies process.
        let no_exec = ExecutionEnv {
            allow_process: false,
            ..parent.clone()
        };
        let asking2 = narrowed_env(&no_exec, &caps(false, true, &[]));
        assert!(!asking2.allow_process, "process cannot be granted upward");
        // Empty ask inherits the parent's list verbatim.
        let inherit = narrowed_env(&parent, &caps(true, true, &[]));
        assert_eq!(inherit.allowed_commands, parent.allowed_commands);
    }

    struct Deleg {
        pool: Pool,
        host: Arc<SqliteWorkflowHost>,
        env: ExecutionEnv,
        tmp: tempfile::NamedTempFile,
    }

    fn deleg() -> Deleg {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(tmp.path());
        let pool: Pool = r2d2::Pool::builder().max_size(4).build(mgr).unwrap();
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
        let fs = Arc::new(MemFs::default());
        fs.write("a.txt", "child-visible body").ok();
        let env = ExecutionEnv {
            fs: fs as Arc<dyn FsSeam>,
            read_only: false,
            allow_process: false,
            root: "/".into(),
            allowed_commands: vec![],
        };
        Deleg {
            pool,
            host,
            env,
            tmp,
        }
    }

    fn spec(name: &str, tools: &[&str], budget: u64) -> SubagentSpec {
        SubagentSpec {
            name: name.into(),
            system_prompt: "you are the scout".into(),
            task: "read a.txt and report".into(),
            allowed_tools: tools.iter().map(|s| s.to_string()).collect(),
            caps: caps(false, false, &[]),
            max_turns: 4,
            token_budget: budget,
        }
    }

    #[test]
    fn child_completes_a_scoped_task_in_the_parents_chain() {
        let d = deleg();
        let provider = LoopbackProvider::new(
            "loopback",
            vec![scripted_text("scouted: child-visible body")],
        );
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(delegate(
                &d.pool,
                &d.host,
                &d.env,
                &[
                    brain_engine_sdk::env::create_read_tool(),
                    brain_engine_sdk::env::create_write_tool(),
                ],
                provider.clone(),
                1,
                &spec("scout", &["read"], 1_000),
                &cancel,
            ))
            .unwrap();
        assert_eq!(
            outcome,
            SubagentOutcome::Completed {
                summary: "scouted: child-visible body".into(),
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 4
                }
            }
        );
        // The child saw ONLY the read tool (subset presentation).
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].tools.len(), 1);
        assert_eq!(requests[0].tools[0].name, "read");
        // Same run, same log: child-prefixed narrative + parent-visible
        // result; the child harness's own audit rows are in the same chain.
        let conn = rusqlite::Connection::open(d.tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "child:scout:user",
                "child:scout:assistant",
                "subagent_result",
            ]
        );
        let result: serde_json::Value = serde_json::from_str(
            &events
                .iter()
                .find(|e| e.kind == "subagent_result")
                .unwrap()
                .payload_json,
        )
        .unwrap();
        assert_eq!(result["outcome"], serde_json::json!("completed"));
        assert_eq!(
            result["summary"],
            serde_json::json!("scouted: child-visible body")
        );
        assert!(
            verify_chain(&conn),
            "child acts land in the same verified chain"
        );
    }

    #[test]
    fn child_write_is_denied_when_the_caps_subtract_it() {
        let d = deleg();
        // Child asks to KEEP write but the caps subtract it (write=false).
        let mut s = spec("writer", &["write"], 1_000);
        s.caps = caps(false, false, &[]);
        let provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text_then_tool("trying to write", "w1", "write", "out.txt\nnope"),
                scripted_text("write was refused, as designed"),
            ],
        );
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(delegate(
                &d.pool,
                &d.host,
                &d.env,
                &[
                    brain_engine_sdk::env::create_read_tool(),
                    brain_engine_sdk::env::create_write_tool(),
                ],
                provider,
                1,
                &s,
                &cancel,
            ))
            .unwrap();
        assert!(matches!(outcome, SubagentOutcome::Completed { .. }));
        let conn = rusqlite::Connection::open(d.tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let tool_event = events
            .iter()
            .find(|e| e.kind == "child:writer:tool_result")
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&tool_event.payload_json).unwrap();
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(payload["output"].as_str().unwrap().contains("read-only"));
    }

    #[test]
    fn child_budget_trip_stops_loudly_at_the_boundary() {
        let d = deleg();
        // Budget 10; one scripted turn already reports 14 tokens used.
        let provider =
            LoopbackProvider::new("loopback", vec![scripted_text("spent the budget on this")]);
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(delegate(
                &d.pool,
                &d.host,
                &d.env,
                &[brain_engine_sdk::env::create_read_tool()],
                provider,
                1,
                &spec("frugal", &["read"], 10),
                &cancel,
            ))
            .unwrap();
        assert_eq!(
            outcome,
            SubagentOutcome::BudgetExceeded {
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 4
                }
            }
        );
        let conn = rusqlite::Connection::open(d.tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "child:frugal:user",
                "child:frugal:assistant",
                "subagent_result",
            ],
            "the boundary event lands after the child's turn, loud"
        );
        let result: serde_json::Value =
            serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
        assert_eq!(result["outcome"], serde_json::json!("budget_exceeded"));
    }

    #[test]
    fn denied_all_parent_still_delegates_fail_closed() {
        // A DenyAll parent: the child inherits the seam but nothing opens.
        let d = deleg();
        let env = ExecutionEnv {
            fs: Arc::new(DenyAll),
            read_only: true,
            allow_process: false,
            root: "/".into(),
            allowed_commands: vec![],
        };
        let provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text_then_tool("try", "t1", "read", "a.txt"),
                scripted_text("done"),
            ],
        );
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(delegate(
                &d.pool,
                &d.host,
                &env,
                &[brain_engine_sdk::env::create_read_tool()],
                provider,
                1,
                &spec("blind", &["read"], 1_000),
                &cancel,
            ))
            .unwrap();
        assert!(matches!(outcome, SubagentOutcome::Completed { .. }));
        let conn = rusqlite::Connection::open(d.tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let tool_event = events
            .iter()
            .find(|e| e.kind == "child:blind:tool_result")
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&tool_event.payload_json).unwrap();
        assert_eq!(payload["ok"], serde_json::json!(false));
    }
}
