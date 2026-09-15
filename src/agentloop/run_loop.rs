//! The 5-step run loop: input → context → stream → tool-exec → loop.
//!
//! This is the driver the SDK's harness lifecycle was built to be driven by:
//! each provider call is one harness TURN (`start_run` snapshots config →
//! stream → `message_end` persists the assistant message and drains queued
//! writes → `finish_run` settles and audits `RunEnd`). Structural ops
//! (compaction included) stay Idle-only by harness law; the loop only runs.
//!
//! Cancellation contract: every await selects on the caller's
//! [`CancellationToken`]. On cancel the loop calls `harness.abort()` (the
//! same settlement path as finish — cleanup is not a special case), appends
//! a `canceled` session event, and returns [`RunOutcome::Canceled`]. No
//! detached tasks: the provider's sender ends when the receiver drops (the
//! seam's own contract), tool work rides `spawn_blocking` awaited under the
//! same select.
//!
//! Persistence contract: the loop's session narrative lands in
//! `agent_session_events` (append-only, audited in-tx, exactly-once by
//! idempotency key) while the harness's own queue drains into the outbox —
//! two different consumers, two different tables, one audit chain. Tool
//! execution rides the SDK registry (`ToolRegistry` presentation/lookup/
//! execution alignment) over an injected [`ExecutionEnv`] — the loop never
//! spawns a process or opens a file itself.

use std::sync::Arc;
use std::time::Duration;

use brain_engine_sdk::env::{ExecutionEnv, ToolDef, ToolRegistry};
use brain_engine_sdk::harness::{AgentHarness, HarnessError, TurnSnapshot};
use tokio_util::sync::CancellationToken;

use crate::Pool;
use crate::agentloop::provider::{
    ChatMessage, LlmProvider, ProviderError, ProviderRequest, Role, StreamEvent, ToolCall, Usage,
};
use crate::workflow::host::SqliteWorkflowHost;
use crate::workflow::session_log::{self, SessionEventRow};
use crate::workflow::tx::WorkflowTx;

/// Turn bound (bounds law): a loop that never stops asking for tools is
/// stopped here, loudly, as [`RunOutcome::TurnCapReached`].
pub(crate) const DEFAULT_MAX_TURNS: u32 = 8;

/// Tool output bound (bounds law): what a tool returns is truncated to this
/// before it enters the session narrative or model context — the loop's
/// context budget is finite and pinned.
pub(crate) const TOOL_OUTPUT_CAP: usize = 16 * 1024;

/// Tool wall-clock bound: a tool that never returns is cut here and the
/// failure is surfaced to the model as a tool error (hang-proofing).
pub(crate) const TOOL_TIMEOUT: Duration = Duration::from_secs(30);

/// Loop knobs, all pinned by defaults; callers narrow, never widen past the
/// consts above (tests assert the pins).
#[derive(Debug, Clone)]
pub(crate) struct LoopConfig {
    pub max_turns: u32,
    pub session_replay_cap: usize,
    pub tool_output_cap: usize,
    pub tool_timeout: Duration,
    pub compaction: crate::agentloop::compaction::CompactionPolicy,
    /// Cumulative provider-token ceiling for this loop (None = uncapped;
    /// subagents delegate with Some). Crossing it stops the loop loudly.
    pub token_budget: Option<u64>,
}

impl Default for LoopConfig {
    fn default() -> Self {
        LoopConfig {
            max_turns: DEFAULT_MAX_TURNS,
            session_replay_cap: session_log::REPLAY_CAP,
            tool_output_cap: TOOL_OUTPUT_CAP,
            tool_timeout: TOOL_TIMEOUT,
            compaction: crate::agentloop::compaction::DEFAULT_COMPACTION_POLICY,
            token_budget: None,
        }
    }
}

/// How a loop run ended. Cancellation and the turn cap are OUTCOMES, not
/// errors — the caller acted within contract in both cases.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RunOutcome {
    /// The model ended a turn with no tool calls; `turns` counts completed
    /// provider turns, `usage` sums the provider's own accounting.
    Completed { turns: u32, usage: Usage },
    /// The caller's token fired; the harness was aborted (audit `RunEnd`
    /// `denied`), a `canceled` session event was appended.
    Canceled,
    /// Every turn asked for tools until the cap; `turns` == the cap.
    TurnCapReached { turns: u32, usage: Usage },
    /// The cumulative token budget was crossed; the loop stopped loudly
    /// at the turn boundary (a subagent's share, typically).
    BudgetExceeded { turns: u32, usage: Usage },
}

/// Loop failure vocabulary — infrastructure and contract breaches, loud.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LoopError {
    /// The harness refused (phase gate breach — a driver bug, never noise).
    Harness(String),
    /// The provider failed before or mid-stream.
    Provider(ProviderError),
    /// A session-store append failed (SQL, pool, or bounds refusal).
    Persist(String),
}

impl std::fmt::Display for LoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoopError::Harness(m) => write!(f, "harness: {m}"),
            LoopError::Provider(e) => write!(f, "provider: {e}"),
            LoopError::Persist(m) => write!(f, "persist: {m}"),
        }
    }
}

impl std::error::Error for LoopError {}

impl From<HarnessError> for LoopError {
    fn from(e: HarnessError) -> Self {
        LoopError::Harness(e.to_string())
    }
}

impl From<ProviderError> for LoopError {
    fn from(e: ProviderError) -> Self {
        LoopError::Provider(e)
    }
}

/// One assembled assistant turn (deltas concatenated in arrival order).
#[derive(Debug, Clone, PartialEq)]
struct AssistantTurn {
    text: String,
    tool_calls: Vec<ToolCall>,
    usage: Usage,
}

/// The loop driver: one provider, one harness, one tool set, one session.
/// Constructed kernel-side with every dependency injected — no env knobs,
/// no globals, no provider construction in here.
pub(crate) struct LoopDriver {
    pool: Pool,
    _host: Arc<SqliteWorkflowHost>,
    harness: Arc<AgentHarness<SqliteWorkflowHost>>,
    provider: Arc<dyn LlmProvider>,
    registry: Arc<ToolRegistry>,
    env: ExecutionEnv,
    tools: Vec<ToolDef>,
    config: LoopConfig,
    /// Prefix on session-event kinds so a child loop's narrative is
    /// distinguishable from the parent's in the same run log
    /// (`child:<name>:`); empty for the parent loop.
    session_prefix: String,
}

impl LoopDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pool: Pool,
        host: Arc<SqliteWorkflowHost>,
        harness: Arc<AgentHarness<SqliteWorkflowHost>>,
        provider: Arc<dyn LlmProvider>,
        tools: Vec<ToolDef>,
        env: ExecutionEnv,
        config: LoopConfig,
        session_prefix: &str,
    ) -> Self {
        let mut registry = ToolRegistry::new();
        for def in &tools {
            // Duplicate registration is a construction-time contract breach;
            // fail loud by refusing the whole driver at first execute would
            // be late — so register here and let the first execute fail
            // closed on the SDK's own duplicate refusal instead.
            let _ = registry.register(def.clone());
        }
        LoopDriver {
            pool,
            _host: host,
            harness,
            provider,
            registry: Arc::new(registry),
            env,
            tools,
            config,
            session_prefix: session_prefix.to_string(),
        }
    }

    /// Drive the multi-turn loop for one user input against `run_id`'s
    /// session log: input → context → stream → tool-exec, repeated until the
    /// model ends a turn tool-free or a bound (cap/cancel) stops the loop.
    ///
    /// Callable more than once per run (the GDL phase machine drives one
    /// exchange per phase): every idempotency key this call draws carries a
    /// per-call salt — the session length observed BEFORE this call's first
    /// append — so the exactly-once guard cannot silently no-op a later
    /// call's events onto an earlier call's keys (the collision the GDL
    /// phase machine found: the original run loop assumed one call per
    /// run).
    pub(crate) async fn run_turns(
        &self,
        run_id: i64,
        input: &str,
        cancel: &CancellationToken,
    ) -> Result<RunOutcome, LoopError> {
        let salt = self.replay(run_id).await?.len();
        let mut events: Vec<(String, String, String)> = Vec::new();
        events.push((
            self.kind("user"),
            input.to_string(),
            format!("{}run{run_id}:u{salt}:0", self.session_prefix),
        ));
        self.append_events(run_id, events).await?;

        let mut usage = Usage::default();
        for turn in 1..=self.config.max_turns {
            // ── compaction admission (Idle boundary, just-before-call) ────
            if self.config.compaction.just_before_call
                && let Compacted::Canceled = self.maybe_compact(run_id, cancel).await?
            {
                return self.cancel_settle(run_id, turn).await;
            }
            // ── steps 1-2: input → context (snapshot + replayed history) ──
            let snapshot = self.harness.start_run(run_id)?;
            let history = self.replay(run_id).await?;
            let request = self.build_request(&snapshot, &history);

            // ── step 3: stream ──────────────────────────────────────────
            let assistant = match self.stream_turn(request, cancel).await? {
                Streamed::Turn(a) => a,
                Streamed::Canceled => return self.cancel_settle(run_id, turn).await,
            };

            // Persist the assistant message: the harness's message_end (outbox
            // + its queue discipline) AND the session narrative, then settle
            // the harness turn.
            let assistant_json = assistant_to_json(&assistant);
            self.harness
                .message_end(&assistant_json, &format!("run{run_id}:asst:t{turn}"))?;
            self.append_events(
                run_id,
                vec![(
                    self.kind("assistant"),
                    assistant_json.clone(),
                    format!("{}run{run_id}:a{salt}:t{turn}", self.session_prefix),
                )],
            )
            .await?;
            self.harness.finish_run()?;
            usage.input_tokens += assistant.usage.input_tokens;
            usage.output_tokens += assistant.usage.output_tokens;
            if let Some(budget) = self.config.token_budget
                && usage.total() > budget
            {
                return Ok(RunOutcome::BudgetExceeded { turns: turn, usage });
            }

            if assistant.tool_calls.is_empty() {
                return Ok(RunOutcome::Completed { turns: turn, usage });
            }

            // ── step 4: tool-exec, results surfaced back as session events ─
            for call in &assistant.tool_calls {
                let output = self.execute_tool(call, cancel).await?;
                if matches!(output, ToolOutput::Canceled) {
                    return self.cancel_settle(run_id, turn).await;
                }
                let ok = !matches!(output, ToolOutput::Failed(_));
                let body = match output {
                    ToolOutput::Done(body) | ToolOutput::Failed(body) => body,
                    ToolOutput::Canceled => unreachable!("handled above"),
                };
                let payload = tool_result_json(call, ok, &body, self.config.tool_output_cap);
                self.append_events(
                    run_id,
                    vec![(
                        self.kind("tool_result"),
                        payload,
                        format!(
                            "{}run{run_id}:x{salt}:t{turn}:{}",
                            self.session_prefix, call.id
                        ),
                    )],
                )
                .await?;
            }
            // ── step 5: loop — next turn consumes the results via replay ──
        }
        Ok(RunOutcome::TurnCapReached {
            turns: self.config.max_turns,
            usage,
        })
    }

    /// Compaction admission at the loop top (the harness is Idle between
    /// turns — the structural gate's own law). Rides the SDK's pressure
    /// policy, produces the summary via the loop's OWN provider, appends
    /// the single `compaction` event. The log is never rewritten.
    async fn maybe_compact(
        &self,
        run_id: i64,
        cancel: &CancellationToken,
    ) -> Result<Compacted, LoopError> {
        let events = self.replay(run_id).await?;
        let Some(split) = crate::agentloop::compaction::plan(&events) else {
            return Ok(Compacted::No);
        };
        // The structural gate: `compact()` is Idle-only by harness law, so
        // this call both performs the admission and pins the phase boundary.
        self.harness.compact()?;
        let request = ProviderRequest {
            system_prompt: crate::agentloop::compaction::COMPACTION_SYSTEM_PROMPT.into(),
            messages: crate::agentloop::compaction::summary_input(&split, self.config.compaction),
            tools: Vec::new(),
        };
        let summary = match self.stream_summary(request, cancel).await? {
            Streamed::Turn(s) => s,
            Streamed::Canceled => return Ok(Compacted::Canceled),
        };
        let payload = crate::agentloop::compaction::compaction_event_json(&summary, &split);
        self.append_events(
            run_id,
            vec![(
                self.kind("compaction"),
                payload,
                format!(
                    "run{run_id}:compact:{}",
                    split.head.last().map(|e| e.seq).unwrap_or(0)
                ),
            )],
        )
        .await?;
        Ok(Compacted::Yes)
    }

    /// Stream a summary call to its text (deltas folded, no tools possible).
    async fn stream_summary(
        &self,
        request: ProviderRequest,
        cancel: &CancellationToken,
    ) -> Result<Streamed<String>, LoopError> {
        let mut rx = self.provider.stream(request)?;
        let mut text = String::new();
        loop {
            let event = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(Streamed::Canceled),
                ev = rx.recv() => ev,
            };
            let Some(event) = event else {
                return Err(LoopError::Provider(ProviderError::Unavailable(
                    "summary stream ended without MessageEnd".into(),
                )));
            };
            match event? {
                StreamEvent::TextDelta(delta) => text.push_str(&delta),
                StreamEvent::MessageEnd { .. } => return Ok(Streamed::Turn(text)),
                StreamEvent::MessageStart | StreamEvent::ToolCallDelta { .. } => {}
            }
        }
    }

    /// Cancel settlement: abort the in-flight harness turn (same path as
    /// finish — the queue drains, `RunEnd` audits denied), append the
    /// `canceled` marker, return the outcome.
    async fn cancel_settle(&self, run_id: i64, turn: u32) -> Result<RunOutcome, LoopError> {
        let salt = self.replay(run_id).await?.len();
        self.harness.abort()?;
        self.append_events(
            run_id,
            vec![(
                self.kind("canceled"),
                format!(r#"{{"turn":{turn}}}"#),
                format!("{}run{run_id}:c{salt}:t{turn}", self.session_prefix),
            )],
        )
        .await?;
        Ok(RunOutcome::Canceled)
    }

    /// Prefix-stable request assembly: the snapshot's cache-stable system
    /// prefix first, then the context window — reshaped around the LATEST
    /// compaction event (summary leads, verbatim tail after) — with tool
    /// specs as the trailing semi-dynamic block.
    fn build_request(
        &self,
        snapshot: &TurnSnapshot,
        history: &[SessionEventRow],
    ) -> ProviderRequest {
        let (summary, tail) = crate::agentloop::compaction::context_window(history);
        let mut messages: Vec<ChatMessage> = Vec::new();
        if let Some(summary) = summary {
            messages.push(ChatMessage {
                role: Role::User,
                text: format!("context summary of earlier session: {summary}"),
            });
        }
        messages.extend(tail.iter().filter_map(|ev| match ev.kind.as_str() {
            "user" => Some(ChatMessage {
                role: Role::User,
                text: ev.payload_json.clone(),
            }),
            "assistant" => Some(ChatMessage {
                role: Role::Assistant,
                text: text_of(&ev.payload_json).unwrap_or_default(),
            }),
            "tool_result" => Some(ChatMessage {
                role: Role::User,
                text: format!(
                    "tool result: {}",
                    text_of(&ev.payload_json).unwrap_or_default()
                ),
            }),
            _ => None,
        }));
        ProviderRequest {
            system_prompt: snapshot.prompt(),
            messages,
            tools: self
                .tools
                .iter()
                .map(|t| crate::agentloop::provider::ToolSpec {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    schema_json: t.schema_json.clone(),
                })
                .collect(),
        }
    }

    /// Stream one turn to completion, folding deltas into an
    /// [`AssistantTurn`]. Cancellation at ANY await returns
    /// [`Streamed::Canceled`] with the receiver dropped (the provider's
    /// sender ends by its own contract).
    async fn stream_turn(
        &self,
        request: ProviderRequest,
        cancel: &CancellationToken,
    ) -> Result<Streamed<AssistantTurn>, LoopError> {
        let mut rx = self.provider.stream(request)?;
        let mut text = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        loop {
            let event = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Ok(Streamed::Canceled),
                ev = rx.recv() => ev,
            };
            let Some(event) = event else {
                // Channel closed without MessageEnd: a broken provider
                // contract — loud, never an empty turn.
                return Err(LoopError::Provider(ProviderError::Unavailable(
                    "stream ended without MessageEnd".into(),
                )));
            };
            match event? {
                StreamEvent::MessageStart => {}
                StreamEvent::TextDelta(delta) => text.push_str(&delta),
                StreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                } => {
                    if let Some(existing) = calls.iter_mut().find(|c| c.id == id) {
                        existing.arguments_json.push_str(&arguments_delta);
                    } else {
                        calls.push(ToolCall {
                            id,
                            name,
                            arguments_json: arguments_delta,
                        });
                    }
                }
                StreamEvent::MessageEnd { usage, .. } => {
                    return Ok(Streamed::Turn(AssistantTurn {
                        text,
                        tool_calls: calls,
                        usage,
                    }));
                }
            }
        }
    }

    /// Execute one tool call under the wall-clock bound, on the blocking
    /// pool (the SDK runners are sync by contract), fail-closed on unknown
    /// names (the registry's own law). Honest ceiling: `spawn_blocking`
    /// cannot be interrupted — a timed-out or cancelled call's runner may
    /// finish discarded in the background; its result never enters the
    /// session. Cancellation returns BEFORE waiting for the runner.
    async fn execute_tool(
        &self,
        call: &ToolCall,
        cancel: &CancellationToken,
    ) -> Result<ToolOutput, LoopError> {
        let registry = Arc::clone(&self.registry);
        let env = self.env.clone();
        let name = call.name.clone();
        let args = call.arguments_json.clone();
        let handle = tokio::task::spawn_blocking(move || registry.execute(&name, &env, &args));
        let joined = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(ToolOutput::Canceled),
            j = tokio::time::timeout(self.config.tool_timeout, handle) => j,
        };
        Ok(match joined {
            // Hang-proofing: the deadline cuts the wait, not the thread —
            // the failure is surfaced to the model as a tool error.
            Err(_elapsed) => ToolOutput::Failed(format!(
                "tool timed out after {:?}",
                self.config.tool_timeout
            )),
            Ok(Ok(Ok(body))) => ToolOutput::Done(body),
            Ok(Ok(Err(e))) => ToolOutput::Failed(e.to_string()),
            Ok(Err(join_err)) => ToolOutput::Failed(format!("tool task join failed: {join_err}")),
        })
    }

    /// Replay the session window on a pooled connection (reads never touch
    /// the write lane).
    async fn replay(&self, run_id: i64) -> Result<Vec<SessionEventRow>, LoopError> {
        let pool = self.pool.clone();
        let cap = self.config.session_replay_cap;
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| LoopError::Persist(format!("pool: {e}")))?;
            session_log::replay(&conn, run_id, cap).map_err(|e| LoopError::Persist(e.to_string()))
        })
        .await
        .map_err(|e| LoopError::Persist(format!("replay join failed: {e}")))?
    }

    /// The session-event kind under this loop's prefix (parent: `user`;
    /// child: `child:<name>:user`).
    fn kind(&self, k: &'static str) -> String {
        format!("{}{k}", self.session_prefix)
    }

    /// Append session events in ONE workflow transaction: all rows and all
    /// their audit rows commit together or not at all.
    async fn append_events(
        &self,
        run_id: i64,
        events: Vec<(String, String, String)>,
    ) -> Result<(), LoopError> {
        append_session_events(&self.pool, run_id, events).await
    }
}

/// Append session events in one workflow transaction on a pool — the shared
/// writer for the parent loop and the subagent delegation surface.
pub(crate) async fn append_session_events(
    pool: &Pool,
    run_id: i64,
    events: Vec<(String, String, String)>,
) -> Result<(), LoopError> {
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = pool
            .get()
            .map_err(|e| LoopError::Persist(format!("pool: {e}")))?;
        let mut wtx =
            WorkflowTx::begin(&mut conn).map_err(|e| LoopError::Persist(e.to_string()))?;
        let now = chrono::Utc::now().timestamp();
        for (kind, payload, key) in events {
            session_log::append(wtx.tx(), run_id, &kind, &payload, &key, now)
                .map_err(|e| LoopError::Persist(e.to_string()))?;
        }
        wtx.commit()
            .map_err(|e| LoopError::Persist(e.to_string()))?;
        Ok(())
    })
    .await
    .map_err(|e| LoopError::Persist(format!("append join failed: {e}")))?
}

/// Terminal state of one streamed turn.
enum Streamed<T> {
    Turn(T),
    Canceled,
}

/// Outcome of a loop-top compaction check.
enum Compacted {
    /// Pressure under threshold, or a compaction landed.
    No,
    Yes,
    Canceled,
}

enum ToolOutput {
    Done(String),
    Failed(String),
    Canceled,
}

fn assistant_to_json(a: &AssistantTurn) -> String {
    serde_json::json!({
        "text": a.text,
        "tool_calls": a.tool_calls,
        "usage": {
            "input_tokens": a.usage.input_tokens,
            "output_tokens": a.usage.output_tokens,
        },
    })
    .to_string()
}

fn tool_result_json(call: &ToolCall, ok: bool, body: &str, cap: usize) -> String {
    let (truncated, kept) = if body.len() > cap {
        (
            true,
            format!("{}\n[truncated at {cap} bytes]", &body[..cap]),
        )
    } else {
        (false, body.to_string())
    };
    serde_json::json!({
        "id": call.id,
        "name": call.name,
        "ok": ok,
        "truncated": truncated,
        "output": kept,
    })
    .to_string()
}

fn text_of(payload_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(payload_json)
        .ok()?
        .get("text")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentloop::provider::{LoopbackProvider, scripted_text, scripted_text_then_tool};
    use crate::audit::verify_chain;
    use crate::config;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use brain_engine_sdk::env::{DenyAll, EnvError, FsSeam};
    use rusqlite::Connection;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    /// In-memory seam for loop tests: real reads, recorded execs.
    #[derive(Default)]
    struct MemFs {
        files: StdMutex<HashMap<String, String>>,
        execs: StdMutex<Vec<String>>,
    }
    impl FsSeam for MemFs {
        fn read(&self, path: &str) -> Result<String, EnvError> {
            self.files
                .lock()
                .ok()
                .and_then(|g| g.get(path).cloned())
                .ok_or_else(|| EnvError::NotFound(path.into()))
        }
        fn write(&self, path: &str, content: &str) -> Result<(), EnvError> {
            if let Ok(mut g) = self.files.lock() {
                g.insert(path.into(), content.into());
            }
            Ok(())
        }
        fn exec(&self, command: &str) -> Result<String, EnvError> {
            if let Ok(mut g) = self.execs.lock() {
                g.push(command.into());
            }
            Ok(format!("ran {command}"))
        }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    struct Fixture {
        driver: LoopDriver,
        provider: Arc<LoopbackProvider>,
        fs: Arc<MemFs>,
        tmp: tempfile::NamedTempFile,
        pool: Pool,
    }

    fn fixture(script: Vec<Vec<StreamEvent>>) -> Fixture {
        fixture_with(script, LoopConfig::default())
    }

    fn fixture_with(script: Vec<Vec<StreamEvent>>, config: LoopConfig) -> Fixture {
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
        let harness = Arc::new(AgentHarness::new(
            host.clone(),
            "test-model",
            "you are a steward",
        ));
        let provider = LoopbackProvider::new("loopback", script);
        let fs = Arc::new(MemFs::default());
        fs.write("a.txt", "steward file body").ok();
        let env = ExecutionEnv {
            fs: fs.clone() as Arc<dyn FsSeam>,
            read_only: true,
            allow_process: false,
            root: "/".into(),
            allowed_commands: vec![],
        };
        let driver = LoopDriver::new(
            pool.clone(),
            host,
            harness,
            provider.clone(),
            vec![brain_engine_sdk::env::create_read_tool()],
            env,
            config,
            "",
        );
        Fixture {
            driver,
            provider,
            fs,
            tmp,
            pool,
        }
    }

    fn session_kinds(f: &Fixture) -> Vec<(i64, String)> {
        let conn = Connection::open(f.tmp.path()).unwrap();
        session_log::replay(&conn, 1, session_log::REPLAY_CAP)
            .unwrap()
            .into_iter()
            .map(|r| (r.seq, r.kind))
            .collect()
    }

    #[test]
    fn five_step_loop_end_to_end() {
        let f = fixture(vec![
            scripted_text_then_tool("reading the file", "c1", "read", "a.txt"),
            scripted_text("done: steward file body"),
        ]);
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(f.driver.run_turns(1, "read a.txt for me", &cancel))
            .unwrap();
        assert_eq!(
            outcome,
            RunOutcome::Completed {
                turns: 2,
                usage: Usage {
                    input_tokens: 20,
                    output_tokens: 10
                }
            }
        );
        // The session narrative: user, assistant(tool ask), tool_result,
        // assistant(final) — in seq order, nothing else.
        assert_eq!(
            session_kinds(&f),
            vec![
                (1, "user".into()),
                (2, "assistant".into()),
                (3, "tool_result".into()),
                (4, "assistant".into()),
            ]
        );
        // The tool actually ran through the env seam and its body persisted.
        let conn = Connection::open(f.tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let tool_payload: serde_json::Value =
            serde_json::from_str(&events[2].payload_json).unwrap();
        assert_eq!(tool_payload["ok"], serde_json::json!(true));
        assert!(
            tool_payload["output"]
                .as_str()
                .unwrap()
                .contains("steward file body")
        );
        // The harness's own message_end rows landed in the outbox (one per
        // turn) — the two-consumer persistence contract.
        let outbox_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE topic = 'message_end' AND run_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(outbox_rows, 2);
        assert!(verify_chain(&conn), "the audit chain still verifies");
        // Context assembly: turn 2's request replayed the history — user
        // input, tool ask, tool result — before the final answer.
        let requests = f.provider.requests();
        assert_eq!(requests.len(), 2);
        let roles: Vec<&str> = requests[1]
            .messages
            .iter()
            .map(|m| m.role.as_str())
            .collect();
        assert_eq!(
            roles,
            vec!["user", "assistant", "user"],
            "turn 2 saw input + tool ask + tool result"
        );
    }

    #[test]
    fn turn_cap_is_pinned_and_stops_the_loop() {
        assert_eq!(DEFAULT_MAX_TURNS, 8);
        // Every turn asks for another read; the script is long enough that
        // only the cap stops the loop.
        let always_tool: Vec<Vec<StreamEvent>> = (0..DEFAULT_MAX_TURNS + 2)
            .map(|_| scripted_text_then_tool("again", "c", "read", "a.txt"))
            .collect();
        let f = fixture(always_tool);
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(f.driver.run_turns(1, "keep reading", &cancel))
            .unwrap();
        assert!(matches!(
            outcome,
            RunOutcome::TurnCapReached {
                turns: DEFAULT_MAX_TURNS,
                ..
            }
        ));
        assert_eq!(f.provider.requests().len() as u32, DEFAULT_MAX_TURNS);
    }

    #[test]
    fn cancel_before_the_first_stream_settles_loud() {
        let f = fixture(vec![scripted_text("never reached")]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let outcome = rt()
            .block_on(f.driver.run_turns(1, "doomed input", &cancel))
            .unwrap();
        assert_eq!(outcome, RunOutcome::Canceled);
        let kinds = session_kinds(&f);
        assert_eq!(
            kinds.last().map(|(_, k)| k.as_str()),
            Some("canceled"),
            "a canceled run appends the marker event"
        );
    }

    #[test]
    fn cancel_mid_stream_drops_clean() {
        // A turn far larger than the channel bound: the stream cannot
        // complete before the test's cancel fires, so the mid-stream arm is
        // exercised deterministically.
        let big_turn: Vec<StreamEvent> = (0..100_000)
            .map(|i| StreamEvent::TextDelta(format!("d{i} ")))
            .collect();
        let provider = LoopbackProvider::new("loopback", vec![big_turn]);
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // The spawned task must OWN everything it touches (rt.spawn is
        // 'static), so the driver is built standalone over this temp file
        // and shared with the test frame only via Arc + the tmp path.
        let driver = Arc::new(fixture_driver_for_spawn(provider.clone(), tmp.path()));
        let cancel = CancellationToken::new();
        let task_token = cancel.clone();
        let rt = rt();
        let handle = {
            let driver = Arc::clone(&driver);
            rt.spawn(async move { driver.run_turns(1, "long turn", &task_token).await })
        };
        // Everything below runs ON the runtime (current-thread): the spawned
        // loop task only makes progress while block_on drives it, so the
        // wait-yield-cancel sequence and the sender-drain are async waits.
        let outcome = rt.block_on(async {
            let deadline = tokio::time::sleep(std::time::Duration::from_secs(5));
            tokio::pin!(deadline);
            while provider.requests().is_empty() {
                // The provider is asked the moment the loop's first turn
                // reaches stream(); until then keep the runtime pumping.
                tokio::select! {
                    _ = &mut deadline => panic!("provider was never asked"),
                    _ = tokio::task::yield_now() => {}
                }
            }
            // A few yields of mid-stream consumption, then cancel.
            for _ in 0..50 {
                tokio::task::yield_now().await;
            }
            cancel.cancel();
            handle.await.unwrap().unwrap()
        });
        assert_eq!(outcome, RunOutcome::Canceled);
        // The sender drained after the receiver drop (no detached producer).
        rt.block_on(async {
            let deadline = tokio::time::sleep(std::time::Duration::from_secs(5));
            tokio::pin!(deadline);
            while provider.live_senders() > 0 {
                tokio::select! {
                    _ = &mut deadline => panic!("sender outlived the dropped receiver"),
                    _ = tokio::task::yield_now() => {}
                }
            }
        });
        let conn = Connection::open(tmp.path()).unwrap();
        assert!(
            verify_chain(&conn),
            "abort settlement left the audit chain intact"
        );
    }

    /// A driver owned entirely by the cancel test's spawned task: its own
    /// pool + host over the shared temp file, so nothing the task touches
    /// borrows from the test frame.
    fn fixture_driver_for_spawn(
        provider: Arc<LoopbackProvider>,
        tmp: &std::path::Path,
    ) -> LoopDriver {
        register_sqlite_vec();
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(tmp);
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
        let harness = Arc::new(AgentHarness::new(host.clone(), "m", "s"));
        LoopDriver::new(
            pool,
            host,
            harness,
            provider,
            vec![brain_engine_sdk::env::create_read_tool()],
            ExecutionEnv {
                fs: Arc::new(DenyAll),
                read_only: true,
                allow_process: false,
                root: "/".into(),
                allowed_commands: vec![],
            },
            LoopConfig::default(),
            "",
        )
    }

    #[test]
    fn unknown_tool_fails_closed_into_a_tool_result() {
        // The model hallucinates a tool name: the registry's fail-closed
        // refusal becomes a tool_result the model can see and correct from.
        let f = fixture(vec![
            crate::agentloop::provider::scripted_tool_call("c1", "nope", "x"),
            scripted_text("recovered"),
        ]);
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(f.driver.run_turns(1, "try a bogus tool", &cancel))
            .unwrap();
        assert!(
            matches!(outcome, RunOutcome::Completed { turns: 2, .. }),
            "the loop continues past a refused tool: {outcome:?}"
        );
        let conn = Connection::open(f.tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let refused: Vec<&SessionEventRow> =
            events.iter().filter(|e| e.kind == "tool_result").collect();
        assert_eq!(refused.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&refused[0].payload_json).unwrap();
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(payload["output"].as_str().unwrap().contains("unknown tool"));
    }

    #[test]
    fn tool_output_is_truncated_at_the_cap() {
        let body = "x".repeat(64 * 1024);
        let f = fixture_with(
            vec![
                scripted_text_then_tool("big read", "c1", "read", "big.txt"),
                scripted_text("done"),
            ],
            LoopConfig {
                tool_output_cap: 1024,
                ..LoopConfig::default()
            },
        );
        f.fs.write("big.txt", &body).ok();
        let cancel = CancellationToken::new();
        rt().block_on(f.driver.run_turns(1, "read the big one", &cancel))
            .unwrap();
        let conn = Connection::open(f.tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let payload: serde_json::Value = events
            .iter()
            .find(|e| e.kind == "tool_result")
            .map(|e| serde_json::from_str(&e.payload_json).unwrap())
            .unwrap();
        assert_eq!(payload["truncated"], serde_json::json!(true));
        assert!(
            payload["output"]
                .as_str()
                .unwrap()
                .contains("[truncated at 1024 bytes]")
        );
        assert!(payload["output"].as_str().unwrap().len() < 1100);
    }

    #[test]
    fn tool_deadline_cuts_the_wait_and_surfaces_the_error() {
        // A tool whose runner blocks past the configured deadline: the loop
        // cuts the WAIT (hang-proofing) and the model sees the timeout.
        let slow = brain_engine_sdk::env::ToolDef::new(
            "slow",
            "a tool that outlives its deadline",
            r#"{"x":"string"}"#,
            |_env, _input| {
                std::thread::sleep(Duration::from_millis(300));
                Ok("late".into())
            },
        );
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
        let harness = Arc::new(AgentHarness::new(host.clone(), "m", "s"));
        let provider = LoopbackProvider::new(
            "loopback",
            vec![
                crate::agentloop::provider::scripted_tool_call("c1", "slow", "x"),
                scripted_text("done"),
            ],
        );
        let driver = LoopDriver::new(
            pool,
            host,
            harness,
            provider,
            vec![slow],
            ExecutionEnv {
                fs: Arc::new(DenyAll),
                read_only: true,
                allow_process: false,
                root: "/".into(),
                allowed_commands: vec![],
            },
            LoopConfig {
                tool_timeout: Duration::from_millis(30),
                ..LoopConfig::default()
            },
            "",
        );
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(driver.run_turns(1, "run the slow tool", &cancel))
            .unwrap();
        assert!(matches!(outcome, RunOutcome::Completed { turns: 2, .. }));
        let conn = Connection::open(tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let payload: serde_json::Value = events
            .iter()
            .find(|e| e.kind == "tool_result")
            .map(|e| serde_json::from_str(&e.payload_json).unwrap())
            .unwrap();
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(payload["output"].as_str().unwrap().contains("timed out"));
    }

    #[test]
    fn loop_config_defaults_are_pinned() {
        let c = LoopConfig::default();
        assert_eq!(c.max_turns, 8);
        assert_eq!(c.session_replay_cap, 500);
        assert_eq!(c.tool_output_cap, 16 * 1024);
        assert_eq!(c.tool_timeout, Duration::from_secs(30));
        assert_eq!(
            c.token_budget, None,
            "the parent loop is uncapped; children cap"
        );
    }

    #[test]
    fn loop_compacts_under_pressure_via_its_own_provider() {
        // Seed a session already over the pressure line: 24 tool_result
        // events at ~1,000 tokens each (~24k window tokens ≥ 16k). The
        // script's FIRST turn is the compaction summary call; the second
        // is the turn's completion.
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
        {
            let mut conn = pool.get().unwrap();
            let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
            for i in 0..24 {
                let payload = serde_json::json!({"id": format!("c{i}"), "name": "read", "ok": true, "output": "x".repeat(4_000)});
                session_log::append(
                    wtx.tx(),
                    1,
                    "tool_result",
                    &payload.to_string(),
                    &format!("seed:{i}"),
                    i as i64,
                )
                .unwrap();
            }
            wtx.commit().unwrap();
        }
        let host = Arc::new(SqliteWorkflowHost::new(pool.clone()));
        let harness = Arc::new(AgentHarness::new(host.clone(), "m", "s"));
        let provider = LoopbackProvider::new(
            "loopback",
            vec![scripted_text("the compact brief"), scripted_text("done")],
        );
        let driver = LoopDriver::new(
            pool,
            host,
            harness,
            provider.clone(),
            vec![],
            ExecutionEnv {
                fs: Arc::new(DenyAll),
                read_only: true,
                allow_process: false,
                root: "/".into(),
                allowed_commands: vec![],
            },
            LoopConfig::default(),
            "",
        );
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(driver.run_turns(1, "continue", &cancel))
            .unwrap();
        assert!(
            matches!(outcome, RunOutcome::Completed { turns: 1, .. }),
            "one compacted turn completes: {outcome:?}"
        );
        let requests = provider.requests();
        assert_eq!(requests.len(), 2, "summary call + turn call, in order");
        // The summary call: compaction system prompt, no tools, the head's
        // tool-result bodies CLEARED (policy default), user turns retained.
        assert!(requests[0].system_prompt.contains("session compactor"));
        assert!(requests[0].tools.is_empty());
        assert!(requests[0].messages.iter().any(|m| {
            m.text
                .contains(crate::agentloop::compaction::CLEARED_TOOL_RESULT)
        }));
        // The turn call: context reshaped — the summary leads, and the
        // verbatim tail (not the head) follows.
        assert!(requests[1].messages[0].text.contains("the compact brief"));
        assert!(
            requests[1].messages.iter().all(
                |m| !m.text.contains("context summary") || m.text.contains("the compact brief")
            )
        );
        // The log was never rewritten: every seeded row still replays, the
        // compaction event appended after them.
        let conn = Connection::open(tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        assert_eq!(
            events.len(),
            27,
            "24 seeded + user + compaction + assistant"
        );
        assert_eq!(events[24].kind, "user");
        assert_eq!(events[25].kind, "compaction");
        assert_eq!(events[26].kind, "assistant");
        let payload: serde_json::Value = serde_json::from_str(&events[25].payload_json).unwrap();
        assert_eq!(payload["summary"], serde_json::json!("the compact brief"));
        // The verbatim-tail budget (20k tokens) keeps the newest ~20 events;
        // the head ends where the tail begins — assert the recorded boundary
        // against the replay, not a magic number.
        let through = payload["compacted_through_seq"].as_i64().unwrap();
        let from = payload["tail_from_seq"].as_i64().unwrap();
        assert_eq!(through, events[4].seq, "head = the oldest 5 events");
        assert_eq!(from, events[5].seq, "tail starts right after the head");
        assert!(from > through);
        assert!(verify_chain(&conn), "compaction writes stay chain-verified");
    }
}
