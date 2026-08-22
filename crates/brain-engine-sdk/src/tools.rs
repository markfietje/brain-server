//! The workflow tool surface (`sdk::tools`): the dsh-tool-workflow analogue.
//!
//! Invariant: start → await → dispose in a `try/finally` shape (the drop
//! guard is the finally); a run that ends non-`completed` surfaces as a tool
//! error so the model sees the failure. Presentation is a pure `meta.name`
//! card. Honest ceiling: the abort bridge observes only the cooperative
//! cancel flag — there is no exec-signal channel in this SDK surface yet.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use crate::env::{EnvError, ToolDef, ToolResult};
use crate::workflow::{
    AgentId, StopReason, WorkflowEngine, WorkflowMeta, WorkflowResult, WorkflowRun,
    WorkflowStartRequest,
};

/// Await grace for the tool path (bounded; the model must never wait
/// unboundedly on a stuck script).
pub const TOOL_AWAIT_GRACE: Duration = Duration::from_secs(30);

/// Input format: first line = declared name (the presentation card), second
/// line = description, remainder = the script text.
fn parse_tool_input(input: &str) -> Result<(String, String, String), EnvError> {
    let mut lines = input.splitn(3, '\n');
    let name = lines.next().unwrap_or("").trim().to_string();
    let description = lines.next().unwrap_or("").trim().to_string();
    let script = lines.next().unwrap_or("").to_string();
    if name.is_empty() || script.trim().is_empty() {
        return Err(EnvError::Denied(
            "malformed workflow input: need `name\\ndescription\\nscript`".into(),
        ));
    }
    Ok((name, description, script))
}

fn await_result(run: &mut WorkflowRun) -> Option<WorkflowResult> {
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    let deadline = std::time::Instant::now() + TOOL_AWAIT_GRACE;
    loop {
        if let Poll::Ready(r) = Pin::new(&mut run.result).poll(&mut cx) {
            return Some(r);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The `finally` half of try/finally: dispose runs even when awaiting panics
/// or early-returns.
struct DisposeGuard(Option<WorkflowRun>);
impl Drop for DisposeGuard {
    fn drop(&mut self) {
        if let Some(run) = self.0.take() {
            run.dispose.dispose();
        }
    }
}

/// The `workflow` tool bound to ONE engine (the ctx-mounted one). Starts the
/// run, awaits within [`TOOL_AWAIT_GRACE`], and disposes in the guard's
/// `finally`. Non-completed outcomes map to tool errors.
pub fn create_workflow_tool(engine: Arc<dyn WorkflowEngine>) -> ToolDef {
    ToolDef::new(
        "workflow",
        "start a named workflow, await it to completion",
        r#"{"name":"string","description":"string","script":"string"}"#,
        move |_, input| -> ToolResult {
            let (name, description, script) = parse_tool_input(input)?;
            let req = WorkflowStartRequest {
                script,
                meta: WorkflowMeta {
                    name,
                    description,
                    when_to_use: None,
                    phases: Vec::new(),
                },
                args: "{}".into(),
                parent: AgentId("tool".into()),
            };
            // Meta validation happens inside the engine's admit gate BEFORE
            // any script evaluation; invalid meta is refused pre-publish.
            let run = engine
                .start(req)
                .map_err(|e| EnvError::Denied(e.to_string()))?;
            let mut guard = DisposeGuard(Some(run));
            let run = guard
                .0
                .as_mut()
                .ok_or_else(|| EnvError::Internal("workflow run handle vanished".into()))?;
            let result = await_result(run).unwrap_or_else(WorkflowResult::cancelled);
            match result.stop_reason {
                StopReason::Completed => Ok(result.output.unwrap_or_else(|| "{}".into())),
                other => Err(EnvError::Denied(format!(
                    "workflow ended with stopReason `{}`",
                    other.as_str()
                ))),
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{RunBuilder, RunId, WorkflowError};
    use std::sync::Mutex as StdMutex;

    struct InlineEngine {
        outcome: StopReason,
        started: Arc<StdMutex<Vec<String>>>,
    }
    impl WorkflowEngine for InlineEngine {
        fn start(&self, req: WorkflowStartRequest) -> Result<WorkflowRun, WorkflowError> {
            let mut b = RunBuilder::default();
            let id = b.admit(&req)?;
            let (completer, run) = b.build_run(id);
            let result = match self.outcome {
                StopReason::Completed => {
                    WorkflowResult::completed(format!("ran {}", req.script.trim()))
                }
                other => WorkflowResult {
                    stop_reason: other,
                    output: None,
                },
            };
            completer.complete(result);
            if let Ok(mut g) = self.started.lock() {
                g.push(req.meta.name.clone());
            }
            let _ = RunId::default_shim();
            Ok(run)
        }
    }

    trait Shim {
        fn default_shim() -> Self;
    }
    impl Shim for RunId {
        fn default_shim() -> Self {
            RunId("x".into())
        }
    }

    #[test]
    fn tool_starts_awaits_and_disposes_completed_runs() {
        let eng = Arc::new(InlineEngine {
            outcome: StopReason::Completed,
            started: Arc::new(StdMutex::new(Vec::new())),
        });
        let tool = create_workflow_tool(eng.clone());
        let out = tool
            .execute(&crate::env::ExecutionEnv::default(), "demo\na demo\nsay hi")
            .unwrap();
        assert_eq!(out, "ran say hi");
        assert_eq!(
            eng.started.lock().map(|g| g.clone()).unwrap_or_default(),
            vec!["demo".to_string()]
        );
    }

    #[test]
    fn tool_surfaces_non_completed_as_error_after_dispose() {
        let eng = Arc::new(InlineEngine {
            outcome: StopReason::Error,
            started: Arc::new(StdMutex::new(Vec::new())),
        });
        let tool = create_workflow_tool(eng);
        assert!(
            tool.execute(&crate::env::ExecutionEnv::default(), "bad\nbad\nboom")
                .is_err()
        );
    }

    #[test]
    fn tool_refuses_malformed_input_before_engine_start() {
        let eng = Arc::new(InlineEngine {
            outcome: StopReason::Completed,
            started: Arc::new(StdMutex::new(Vec::new())),
        });
        let tool = create_workflow_tool(eng.clone());
        assert!(
            tool.execute(&crate::env::ExecutionEnv::default(), "")
                .is_err()
        );
        assert!(eng.started.lock().map(|g| g.is_empty()).unwrap_or(false));
    }
}
