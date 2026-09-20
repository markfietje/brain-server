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
//! two different consumers, two different tables, one audit chain. A terminal
//! exchange receipt certifies both writes completed. A partial write retains the
//! claim and REFUSES retry; there is no automatic projection repair or tool replay.
//! Tool
//! execution rides the SDK registry (`ToolRegistry` presentation/lookup/
//! execution alignment) over an injected [`ExecutionEnv`] — the loop never
//! spawns a process or opens a file itself.

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use brain_engine_sdk::env::{ExecutionEnv, ToolDef, ToolRegistry};
use brain_engine_sdk::harness::{AgentHarness, HarnessError, TurnSnapshot, TurnToken};
use brain_engine_sdk::host::{AuditKind, AuditStatus, WorkflowHost};
use tokio_util::sync::CancellationToken;

use crate::Pool;
use crate::agentloop::hooks::{
    BeforeCompaction, BeforeStart, LOOP_BEFORE_COMPACTION, LOOP_BEFORE_INPUT, LOOP_BEFORE_START,
    LoopHooks,
};
use crate::agentloop::provider::{
    LlmProvider, ProviderError, ProviderRequest, StreamEvent, ToolCall, Usage,
};
use crate::agentloop::subagents::{ExchangeBudget, ExchangeGuard};
use crate::workflow::host::SqliteWorkflowHost;
use crate::workflow::session_log;
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
    pub budget_accounting: Option<crate::agentloop::subagents::ExchangeBudget>,
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
            budget_accounting: None,
        }
    }
}

/// How a loop run ended. Cancellation and the turn cap are OUTCOMES, not
/// errors — the caller acted within contract in both cases.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// Durable exchange result. The text is read from this exchange's exact
/// assistant key, never from a capped or label-based history search.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExchangeReceipt {
    pub outcome: RunOutcome,
    pub final_text: String,
    pub exchange_id: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExchangeEnd {
    version: u32,
    outcome: RunOutcome,
    assistant_key: Option<String>,
}

struct Exchange<'a> {
    id: i64,
    owner: &'a str,
    invocation: &'a str,
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
    /// A constructor-injected hook policy refused the boundary — carries
    /// the event name and a bounded reason, never payload content.
    Hook(String),
}

impl std::fmt::Display for LoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoopError::Harness(m) => write!(f, "harness: {m}"),
            LoopError::Provider(e) => write!(f, "provider: {e}"),
            LoopError::Persist(m) => write!(f, "persist: {m}"),
            LoopError::Hook(m) => write!(f, "hook: {m}"),
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
    host: Arc<SqliteWorkflowHost>,
    harness: Arc<AgentHarness<SqliteWorkflowHost>>,
    provider: Arc<dyn LlmProvider>,
    registry: Arc<ToolRegistry>,
    env: ExecutionEnv,
    tools: Vec<ToolDef>,
    config: LoopConfig,
    /// Constructor-injected policy boundaries (C1): before-start, before-
    /// input, before-compaction. `pass_through()` everywhere no operator
    /// policy is supplied — never env-driven.
    hooks: LoopHooks,
    /// Prefix on session-event kinds so a child loop's narrative is
    /// distinguishable from the parent's in the same run log
    /// (`child:<name>:`); empty for the parent loop.
    session_prefix: String,
    retry_spec: String,
    policy_snapshot: Option<(String, String, Vec<String>, Vec<String>)>,
    #[cfg(test)]
    receipt_pause: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    /// Test-only deterministic seam: entered-notified after the compaction
    /// append, committed-notified after COMMIT, latch released by the test.
    #[cfg(test)]
    compaction_pause: Option<CompactionPause>,
}

/// Test-only compaction boundary seam (see `LoopDriver::compaction_pause`).
#[cfg(test)]
type CompactionPause = (
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
    Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
);

/// Token-owned settlement: armed only by a successful owned start; the single
/// explicit attempt disarms it even when that attempt fails or unwinds — the
/// token is consumed BEFORE any host work — so detached callbacks can never
/// run twice and Drop cannot retry an already-attempted settlement. Drop
/// cleans up only an otherwise-unattempted turn, through a bounded
/// try-acquire path that never blocks the destructor on a contended lock.
struct TurnSettlement {
    harness: Arc<AgentHarness<SqliteWorkflowHost>>,
    /// THE single ownership view. `Some` while the turn is unsettled;
    /// consumed by the one settlement attempt (success, failure, unwind,
    /// cancellation, or Drop).
    token: Option<TurnToken>,
    /// Fixed bounded status of the last settlement attempt — part of a small
    /// closed vocabulary, never backend text, never a replacement for the
    /// primary error.
    secondary: Option<&'static str>,
}

impl TurnSettlement {
    fn arm(harness: Arc<AgentHarness<SqliteWorkflowHost>>, token: TurnToken) -> Self {
        TurnSettlement {
            token: Some(token),
            harness,
            secondary: None,
        }
    }

    /// The single explicit settlement attempt. Consumes the token before any
    /// host work (structural disarm, including unwind during host code); a
    /// failed attempt is never retried by Drop.
    fn settle(&mut self, finish: bool) -> Result<(), LoopError> {
        let Some(token) = self.token.take() else {
            self.secondary = Some("already");
            return Ok(());
        };
        let result = if finish {
            self.harness.finish_turn(token)
        } else {
            self.harness.abort_turn(token)
        };
        self.secondary = Some(if result.is_ok() { "settled" } else { "refused" });
        result.map_err(LoopError::from)
    }

    /// Owned mid-turn cancellation: aborts THIS turn through the armed token,
    /// then the caller journals the marker. Never a generic abort.
    fn cancel(&mut self) -> Result<(), LoopError> {
        self.settle(false)
    }

    /// Owned message-end under the turn token (delivery is token-checked).
    fn token_message_end(&self, payload: &str, key: &str) -> Result<(), HarnessError> {
        let token = self.token.as_ref().ok_or(HarnessError::Unavailable)?;
        self.harness.message_end_owned(token, payload, key)
    }

    fn complete<T>(&mut self, result: Result<T, LoopError>) -> Result<T, LoopError> {
        if result.is_err() {
            let _ = self.settle(false);
        }
        result
    }

    /// The fixed bounded secondary status, for diagnostics that must not
    /// carry backend text.
    fn secondary_status(&self) -> Option<&'static str> {
        self.secondary
    }
}

impl Drop for TurnSettlement {
    fn drop(&mut self) {
        // Bounded synchronous cleanup — NOT a crash protocol and never an
        // unbounded destructor wait: try-acquire the harness; on contention
        // or host refusal fail closed (claim retained, no Idle assertion,
        // fixed status). Only an otherwise-unattempted settlement lands here.
        let Some(token) = self.token.take() else {
            return;
        };
        let verdict = if self.harness.try_abort_turn(token).is_ok() {
            "drop_settled"
        } else {
            "drop_refused"
        };
        self.secondary = Some(verdict);
    }
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
        hooks: LoopHooks,
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
            host,
            harness,
            provider,
            registry: Arc::new(registry),
            env,
            tools,
            config,
            hooks,
            session_prefix: session_prefix.to_string(),
            retry_spec: String::new(),
            policy_snapshot: None,
            #[cfg(test)]
            receipt_pause: None,
            #[cfg(test)]
            compaction_pause: None,
        }
    }

    /// Bind caller-level invocation policy in addition to the effective tools/env.
    pub(crate) fn set_retry_spec(&mut self, spec: String) {
        self.retry_spec = spec;
    }

    /// Bind the immutable harness configuration for pre-work GDL identity.
    /// Actual snapshots are checked before provider work; mutation fails closed.
    pub(crate) fn bind_policy(
        &mut self,
        model: &str,
        system: &str,
        tools: Vec<String>,
        resources: Vec<String>,
    ) {
        self.policy_snapshot = Some((model.into(), system.into(), tools, resources));
    }

    pub(crate) fn policy_identity(&self) -> String {
        self.policy_hash(Some(
            self.policy_snapshot
                .as_ref()
                .expect("bind_policy is required before policy_identity"),
        ))
    }

    fn policy_matches(&self, snapshot: &TurnSnapshot) -> bool {
        self.policy_snapshot
            .as_ref()
            .is_none_or(|(model, system, tools, resources)| {
                model == snapshot.model()
                    && system == snapshot.system_prompt()
                    && tools == snapshot.tools()
                    && resources == snapshot.resources()
            })
    }

    /// One durable trace row per hook deny/steer (C4): coarse fields only —
    /// the event name and a FIXED kernel reason. Listener text never enters
    /// the chain (no input or payload echo).
    fn audit_hook(&self, event: &'static str, status: AuditStatus, detail: &'static str) {
        self.host
            .audit(AuditKind::Workflow, "loop-hooks", event, status, detail);
    }

    /// Waterfall policy at a failing boundary: deny ⇒ one Denied audit row
    /// with a fixed kernel reason, then `LoopError::Hook`. The deadline
    /// denial keeps its own fixed reason.
    async fn policy_gate<E>(
        &self,
        event: &'static str,
        payload: &E,
        detail: &'static str,
    ) -> Result<(), LoopError>
    where
        E: Any + Clone + Send + 'static,
    {
        if let Err(denied) = self.hooks.run_policy(event, payload).await {
            let fixed: &'static str =
                if denied.reason == crate::agentloop::hooks::HOOK_DEADLINE_REASON {
                    "hook deadline exceeded"
                } else {
                    detail
                };
            self.audit_hook(event, AuditStatus::Denied, fixed);
            return Err(LoopError::Hook(denied.to_string()));
        }
        Ok(())
    }

    /// Drive the multi-turn loop for one user input against `run_id`'s
    /// session log: input → context → stream → tool-exec, repeated until the
    /// model ends a turn tool-free or a bound (cap/cancel) stops the loop.
    ///
    /// Convenience calls always create a NEW exchange; explicit retries must
    /// use run_turns_keyed with a caller-persisted request key.
    pub(crate) async fn run_turns(
        &self,
        run_id: i64,
        input: &str,
        cancel: &CancellationToken,
    ) -> Result<RunOutcome, LoopError> {
        Ok(self
            .run_turns_keyed(run_id, &uuid::Uuid::new_v4().to_string(), input, cancel)
            .await?
            .outcome)
    }

    pub(crate) async fn run_turns_keyed(
        &self,
        run_id: i64,
        request_key: &str,
        input: &str,
        cancel: &CancellationToken,
    ) -> Result<ExchangeReceipt, LoopError> {
        let owner = uuid::Uuid::new_v4().to_string();
        self.run_invocation(
            run_id,
            request_key,
            input,
            &owner,
            cancel,
            true,
            |_| Ok(Vec::new()),
            None,
        )
        .await
    }

    /// GDL owns the whole case claim. Errors/drops deliberately leave it held.
    pub(crate) async fn run_turns_owned(
        &self,
        run_id: i64,
        request_key: &str,
        input: &str,
        owner: &str,
        cancel: &CancellationToken,
    ) -> Result<ExchangeReceipt, LoopError> {
        self.run_turns_owned_projected(
            run_id,
            request_key,
            input,
            owner,
            cancel,
            |_| Ok(Vec::new()),
        )
        .await
    }

    /// The projection runs inside the receipt/finalization transaction. Never
    /// finalize a child invocation before its parent-visible result is durable.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_turns_owned_projected<F>(
        &self,
        run_id: i64,
        request_key: &str,
        input: &str,
        owner: &str,
        cancel: &CancellationToken,
        projection: F,
    ) -> Result<ExchangeReceipt, LoopError>
    where
        F: FnOnce(&ExchangeReceipt) -> Result<Vec<(String, String, String)>, LoopError>
            + Send
            + 'static,
    {
        self.run_invocation(
            run_id,
            request_key,
            input,
            owner,
            cancel,
            false,
            projection,
            None,
        )
        .await
    }

    /// Fresh-only budget hook: `budget` runs ONLY on the fresh-admission path
    /// (after the receipt check), so an exact retry neither dispatches, debits,
    /// nor transiently reserves capacity. The returned guard owns the
    /// exchange's accounting (and any child reservation) for the whole turn.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_turns_owned_projected_budgeted<F, B>(
        &self,
        run_id: i64,
        request_key: &str,
        input: &str,
        owner: &str,
        cancel: &CancellationToken,
        projection: F,
        budget: B,
    ) -> Result<ExchangeReceipt, LoopError>
    where
        F: FnOnce(&ExchangeReceipt) -> Result<Vec<(String, String, String)>, LoopError>
            + Send
            + 'static,
        B: FnOnce() -> Result<ExchangeGuard, LoopError> + Send + 'static,
    {
        self.run_invocation(
            run_id,
            request_key,
            input,
            owner,
            cancel,
            false,
            projection,
            Some(Box::new(budget)),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_invocation<F>(
        &self,
        run_id: i64,
        request_key: &str,
        input: &str,
        owner: &str,
        cancel: &CancellationToken,
        own_claim: bool,
        projection: F,
        budget_factory: Option<Box<dyn FnOnce() -> Result<ExchangeGuard, LoopError> + Send>>,
    ) -> Result<ExchangeReceipt, LoopError>
    where
        F: FnOnce(&ExchangeReceipt) -> Result<Vec<(String, String, String)>, LoopError>
            + Send
            + 'static,
    {
        let invocation = uuid::Uuid::new_v4().to_string();
        let pool = self.pool.clone();
        let owned = owner.to_string();
        let token = invocation.clone();
        // C2 before_start: a deny closes the whole owned run BEFORE the
        // claim, the invocation row, and any provider work — nothing was
        // claimed, nothing to unwind, one coarse audit row as the trace.
        self.policy_gate(
            LOOP_BEFORE_START,
            &BeforeStart {
                run_id,
                owner: owned.clone(),
            },
            "loop start denied by hook policy",
        )
        .await?;
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(persist)?;
            let mut tx = WorkflowTx::begin(&mut conn).map_err(persist)?;
            if own_claim {
                session_log::acquire(tx.tx(), run_id, &owned).map_err(persist)?;
            }
            session_log::begin_invocation(tx.tx(), run_id, &owned, &token).map_err(persist)?;
            tx.commit().map_err(persist)?;
            Ok::<_, LoopError>(())
        })
        .await
        .map_err(persist)??;
        // Nothing external has started yet. Only the admission-refusal path
        // below may safely finalize and release; await/drop errors never do.
        let (snapshot, token) = self.harness.start_turn(run_id)?;
        let mut settlement = TurnSettlement::arm(Arc::clone(&self.harness), token);
        let policy_matches = self.policy_matches(&snapshot);
        settlement.settle(true)?;
        drop(settlement);
        // C2 before_input: after the claim gate, BEFORE admission and any
        // provider call. An allow is followed by the ordered steer of the
        // pending input; a deny refuses the exchange with the claim
        // retained (the tree's error-path law) and one audit row.
        let mut pending_input = input.to_string();
        self.policy_gate(
            LOOP_BEFORE_INPUT,
            &pending_input,
            "loop input denied by hook policy",
        )
        .await?;
        let pre_steer = pending_input.clone();
        if let Err(steer_refused) = self.hooks.steer_input(&mut pending_input) {
            self.audit_hook(
                LOOP_BEFORE_INPUT,
                AuditStatus::Denied,
                "loop input steer refused past payload cap",
            );
            return Err(LoopError::Hook(steer_refused.to_string()));
        }
        if pending_input != pre_steer {
            // One durable Ok row per ACTUAL application — a dispatch that
            // left the input byte-identical writes nothing.
            self.audit_hook(
                LOOP_BEFORE_INPUT,
                AuditStatus::Ok,
                "loop input steered by hook listener",
            );
        }
        let fingerprint = self.fingerprint(&pending_input, &snapshot);
        let pool = self.pool.clone();
        let key = request_key.to_string();
        let owned = owner.to_string();
        let token = invocation.clone();
        let (created, id) = tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(persist)?;
            let mut tx = WorkflowTx::begin(&mut conn).map_err(persist)?;
            session_log::verify_invocation(tx.tx(), run_id, &owned, &token).map_err(persist)?;
            let admission = if policy_matches {
                session_log::admit_exchange(tx.tx(), run_id, &owned, &key, &fingerprint)
            } else {
                Err(rusqlite::Error::InvalidParameterName(
                    "bound harness policy drift".into(),
                ))
            };
            match admission {
                Ok(admission) => {
                    tx.commit().map_err(persist)?;
                    Ok(admission)
                }
                Err(error) => {
                    // Roll back ANY admission writes, then finish only this
                    // known pre-work invocation; never release uncertain work.
                    drop(tx);
                    let mut tx = WorkflowTx::begin(&mut conn).map_err(persist)?;
                    session_log::finish_invocation(tx.tx(), run_id, &owned, &token)
                        .map_err(persist)?;
                    if own_claim {
                        session_log::release(tx.tx(), run_id, &owned).map_err(persist)?;
                    }
                    tx.commit().map_err(persist)?;
                    Err(persist(error))
                }
            }
        })
        .await
        .map_err(persist)??;
        let exchange = Exchange {
            id,
            owner,
            invocation: &invocation,
        };
        if !created {
            return self.receipt(run_id, &exchange, own_claim, projection).await;
        }
        // Fresh admission only: acquire the exchange's accounting authority
        // (and any child reservation) here, before any provider dispatch. A
        // refused authority fails the invocation instead of dispatching.
        let budget_guard = if let Some(factory) = budget_factory {
            factory()?
        } else {
            self.config
                .budget_accounting
                .as_ref()
                .map(|supplied| supplied.fresh_exchange(self.config.token_budget))
                .unwrap_or_else(|| ExchangeGuard::root(self.config.token_budget))
        };
        let budget = budget_guard.budget().clone();
        let (outcome, assistant_key) = self
            .drive_exchange(run_id, &pending_input, &exchange, cancel, &budget)
            .await?;
        let end = serde_json::to_string(&ExchangeEnd {
            version: 1,
            outcome,
            assistant_key,
        })
        .map_err(persist)?;
        self.append_owned(
            run_id,
            &exchange,
            vec![(
                "control:exchange_done".into(),
                end,
                format!("control:exchange_done:{id}"),
            )],
        )
        .await?;
        #[cfg(test)]
        if let Some((entered, resume)) = &self.receipt_pause {
            entered.notify_one();
            resume.notified().await;
        }
        self.receipt(run_id, &exchange, own_claim, projection).await
    }

    async fn drive_exchange(
        &self,
        run_id: i64,
        input: &str,
        exchange: &Exchange<'_>,
        cancel: &CancellationToken,
        budget: &ExchangeBudget,
    ) -> Result<(RunOutcome, Option<String>), LoopError> {
        let salt = exchange.id;
        let mut final_key = None;
        let mut events: Vec<(String, String, String)> = Vec::new();
        events.push((
            self.kind("user"),
            input.to_string(),
            format!("{}run{run_id}:u{salt}:0", self.session_prefix),
        ));
        self.append_owned(run_id, exchange, events).await?;

        for turn in 1..=self.config.max_turns {
            if cancel.is_cancelled() {
                return Ok((
                    self.journal_canceled(run_id, exchange, turn).await?,
                    final_key,
                ));
            }
            if budget.exhausted() {
                return Ok((
                    RunOutcome::BudgetExceeded {
                        turns: turn - 1,
                        usage: budget.usage(),
                    },
                    final_key,
                ));
            }
            // ── compaction admission (Idle boundary, just-before-call) ────
            if self.config.compaction.just_before_call
                && let Compacted::Canceled = self
                    .maybe_compact(run_id, exchange, turn, cancel, budget)
                    .await?
            {
                return Ok((
                    self.journal_canceled(run_id, exchange, turn).await?,
                    final_key,
                ));
            }
            if cancel.is_cancelled() {
                return Ok((
                    self.journal_canceled(run_id, exchange, turn).await?,
                    final_key,
                ));
            }
            if budget.exhausted() {
                return Ok((
                    RunOutcome::BudgetExceeded {
                        turns: turn - 1,
                        usage: budget.usage(),
                    },
                    final_key,
                ));
            }
            // ── steps 1-2: input → context (snapshot + replayed history) ──
            self.check_owner(run_id, exchange.owner).await?;
            let (snapshot, token) = self.harness.start_turn(run_id)?;
            let mut settlement = TurnSettlement::arm(Arc::clone(&self.harness), token);
            let result = async {
                if !self.policy_matches(&snapshot) {
                    return Err(persist("bound harness policy drift"));
                }
                if cancel.is_cancelled() {
                    return Ok(TurnEnd::Canceled);
                }
                let history = self.replay(run_id, exchange.id).await?;
                if cancel.is_cancelled() {
                    return Ok(TurnEnd::Canceled);
                }
                let request = self.build_request(&snapshot, &history)?;

                // ── step 3: stream ──────────────────────────────────────────
                let assistant = match self.stream_turn(request, cancel, budget).await? {
                    Streamed::Turn(a) => a,
                    Streamed::Canceled => return Ok(TurnEnd::Canceled),
                    Streamed::BudgetExhausted => return Ok(TurnEnd::Exhausted),
                };

                crate::agentloop::context::validate_calls(&assistant.tool_calls)
                    .map_err(persist)?;

                // Persist the assistant message: the harness's message_end (outbox
                // + its queue discipline) AND the session narrative, then settle
                // the harness turn.
                let assistant_json = assistant_to_json(&assistant);
                self.check_owner(run_id, exchange.owner).await?;
                settlement
                    .token_message_end(
                        &assistant_json,
                        &format!("run{run_id}:exchange:{salt}:asst:t{turn}"),
                    )
                    .map_err(LoopError::from)?;
                let assistant_key = format!("{}run{run_id}:a{salt}:t{turn}", self.session_prefix);
                self.append_owned(
                    run_id,
                    exchange,
                    vec![(
                        self.kind("assistant"),
                        assistant_json.clone(),
                        assistant_key.clone(),
                    )],
                )
                .await?;
                settlement.settle(true)?;
                Ok(TurnEnd::Done(assistant, assistant_key))
            }
            .await;
            let completed = settlement.complete(result)?;
            if matches!(completed, TurnEnd::Canceled) {
                // Mid-turn cancellation: settle THIS turn through the armed
                // token (one owned abort), then journal the marker. The
                // settlement is consumed, so Drop cannot attempt a second.
                settlement.cancel()?;
                drop(settlement);
                return Ok((
                    self.journal_canceled(run_id, exchange, turn).await?,
                    final_key,
                ));
            }
            drop(settlement);
            let TurnEnd::Done(assistant, assistant_key) = completed else {
                return Ok((
                    RunOutcome::BudgetExceeded {
                        turns: turn - 1,
                        usage: budget.usage(),
                    },
                    final_key,
                ));
            };
            final_key = Some(assistant_key);
            let usage = budget.usage();
            if budget.exhausted() {
                return Ok((RunOutcome::BudgetExceeded { turns: turn, usage }, final_key));
            }

            if assistant.tool_calls.is_empty() {
                return Ok((RunOutcome::Completed { turns: turn, usage }, final_key));
            }

            // ── step 4: tool-exec, results surfaced back as session events ─
            for (index, call) in assistant.tool_calls.iter().enumerate() {
                if cancel.is_cancelled() {
                    return Ok((
                        self.journal_canceled(run_id, exchange, turn).await?,
                        final_key,
                    ));
                }
                let intent = format!("run{run_id}:exchange:{salt}:tool:{turn}:{index}");
                self.append_owned(
                    run_id,
                    exchange,
                    vec![(
                        "control:tool_intent".into(),
                        serde_json::to_string(call).map_err(persist)?,
                        intent.clone(),
                    )],
                )
                .await?;
                let output = self.execute_tool(call, cancel).await?;
                if matches!(output, ToolOutput::Canceled) {
                    return Ok((
                        self.journal_canceled(run_id, exchange, turn).await?,
                        final_key,
                    ));
                }
                let ok = !matches!(output, ToolOutput::Failed(_));
                let body = match output {
                    ToolOutput::Done(body) | ToolOutput::Failed(body) => body,
                    ToolOutput::Canceled => unreachable!("handled above"),
                };
                let payload = tool_result_json(call, ok, &body, self.config.tool_output_cap);
                self.append_owned(
                    run_id,
                    exchange,
                    vec![
                        (
                            self.kind("tool_result"),
                            payload,
                            format!("{}run{run_id}:x{salt}:t{turn}:{index}", self.session_prefix),
                        ),
                        (
                            "control:tool_done".into(),
                            "{}".into(),
                            format!("{intent}:done"),
                        ),
                    ],
                )
                .await?;
            }
            // ── step 5: loop — next turn consumes the results via replay ──
        }
        Ok((
            RunOutcome::TurnCapReached {
                turns: self.config.max_turns,
                usage: budget.usage(),
            },
            final_key,
        ))
    }

    /// Compaction admission at the loop top (the harness is Idle between
    /// turns — the structural gate's own law). Rides the SDK's pressure
    /// policy, produces the summary via the loop's OWN provider, appends
    /// the single `compaction` event. The log is never rewritten.
    async fn maybe_compact(
        &self,
        run_id: i64,
        exchange: &Exchange<'_>,
        turn: u32,
        cancel: &CancellationToken,
        budget: &ExchangeBudget,
    ) -> Result<Compacted, LoopError> {
        if cancel.is_cancelled() {
            return Ok(Compacted::Canceled);
        }
        if budget.exhausted() {
            return Ok(Compacted::No);
        }
        let events = self.replay(run_id, exchange.id).await?;
        let effective = crate::agentloop::compaction::reconstruct(&events).map_err(persist)?;
        let Some(split) =
            crate::agentloop::compaction::admit(effective, exchange.id).map_err(persist)?
        else {
            return Ok(Compacted::No);
        };
        // C2 before_compaction: a cycle is genuinely pending (admit returned
        // Some under real pressure) — the policy may skip it. Deny ⇒ audited
        // skip, NO summary provider call; the loop continues and pressure
        // re-evaluates on the next exchange (bounded by the existing caps).
        let pressure = crate::agentloop::compaction::message_tokens(
            &crate::agentloop::context::with_summary(split.summary.as_deref(), &split.head)
                .map_err(persist)?,
        )
        .map_err(persist)?;
        if let Err(denied) = self
            .hooks
            .run_policy(
                LOOP_BEFORE_COMPACTION,
                &BeforeCompaction {
                    run_id,
                    pressure_tokens: pressure,
                },
            )
            .await
        {
            let fixed: &'static str =
                if denied.reason == crate::agentloop::hooks::HOOK_DEADLINE_REASON {
                    "hook deadline exceeded"
                } else {
                    "compaction skipped by hook policy"
                };
            self.audit_hook(LOOP_BEFORE_COMPACTION, AuditStatus::Denied, fixed);
            return Ok(Compacted::No);
        }
        // The structural gate: `compact()` is Idle-only by harness law, so
        // this call both performs the admission and pins the phase boundary.
        self.harness.compact()?;
        let request = ProviderRequest {
            system_prompt: crate::agentloop::compaction::COMPACTION_SYSTEM_PROMPT.into(),
            messages: crate::agentloop::context::with_summary(
                split.summary.as_deref(),
                &split.head,
            )
            .map_err(persist)?,
            tools: Vec::new(),
        };
        crate::agentloop::context::check_request(&request).map_err(persist)?;
        let summary = match self.stream_summary(request, cancel, budget).await? {
            Streamed::Turn(s) => s,
            Streamed::Canceled => return Ok(Compacted::Canceled),
            Streamed::BudgetExhausted => return Ok(Compacted::No),
        };
        let payload =
            crate::agentloop::compaction::admission_json(&summary, &split).map_err(persist)?;
        if cancel.is_cancelled() {
            return Ok(Compacted::Canceled);
        }
        self.append_compaction(run_id, exchange, turn, payload, cancel)
            .await
    }

    async fn append_compaction(
        &self,
        run_id: i64,
        exchange: &Exchange<'_>,
        turn: u32,
        payload: String,
        cancel: &CancellationToken,
    ) -> Result<Compacted, LoopError> {
        let pool = self.pool.clone();
        let owner = exchange.owner.to_string();
        let kind = self.kind("compaction");
        let key = format!("run{run_id}:exchange:{}:compact:{turn}", exchange.id);
        let cancel = cancel.clone();
        #[cfg(test)]
        let compaction_pause = self.compaction_pause.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(persist)?;
            let mut tx = WorkflowTx::begin(&mut conn).map_err(persist)?;
            session_log::verify_owner(tx.tx(), run_id, &owner).map_err(persist)?;
            if cancel.is_cancelled() {
                return Ok(Compacted::Canceled);
            }
            session_log::append(
                tx.tx(),
                run_id,
                &kind,
                &payload,
                &key,
                chrono::Utc::now().timestamp(),
            )
            .map_err(persist)?;
            #[cfg(test)]
            if let Some((entered, _committed, latch)) = &compaction_pause {
                entered.notify_one();
                let (lock, wake) = &**latch;
                let guard = lock.lock().unwrap();
                let _guard = wake
                    .wait_timeout_while(guard, Duration::from_secs(5), |released| !*released)
                    .unwrap();
            }
            if cancel.is_cancelled() {
                return Ok(Compacted::Canceled);
            }
            // Commit is the linearization point. Cancellation after this check
            // does not undo committed evidence; earlier cancellation rolls back.
            tx.commit().map_err(persist)?;
            #[cfg(test)]
            if let Some((_entered, committed, _latch)) = &compaction_pause {
                committed.notify_one();
            }
            Ok(Compacted::Yes)
        })
        .await
        .map_err(persist)?
    }

    /// Stream a summary call to its text (deltas folded, no tools possible).
    async fn stream_summary(
        &self,
        request: ProviderRequest,
        cancel: &CancellationToken,
        budget: &ExchangeBudget,
    ) -> Result<Streamed<String>, LoopError> {
        if cancel.is_cancelled() {
            return Ok(Streamed::Canceled);
        }
        let streamed = self
            .admitted_stream(
                request,
                cancel,
                budget,
                String::new,
                |event, text| match event {
                    StreamEvent::TextDelta(delta) => {
                        if delta.len()
                            > crate::agentloop::compaction::SUMMARY_CAP.saturating_sub(text.len())
                        {
                            return Err(persist(
                                crate::agentloop::context::ContextError::SummaryLimit,
                            ));
                        }
                        text.push_str(&delta);
                        Ok(())
                    }
                    StreamEvent::ToolCallDelta { .. } => Err(persist("context_summary_tool_call")),
                    StreamEvent::MessageStart | StreamEvent::MessageEnd { .. } => Ok(()),
                },
            )
            .await?;
        match streamed {
            Streamed::Turn(text) => Ok(Streamed::Turn(
                crate::agentloop::compaction::shape_summary(&text).map_err(persist)?,
            )),
            other => Ok(other),
        }
    }

    /// THE one provider-dispatch seam. Serves `stream_turn` AND
    /// `stream_summary`; delegated children dispatch through their own
    /// LoopDriver, whose seam is this same helper. No alternate dispatch
    /// path exists. Algorithm (D-contract): (1) awaited preparation finished
    /// by the caller without ledger locks or permits; (2) cancellation-aware
    /// wait on the single root-owned RAII permit shared by all
    /// views/children; (3) short ledger admission under the budget mutex —
    /// liveness, poison, unknown spend and ceilings validated atomically,
    /// the attempt recorded, no await under the lock; (4) cancellation
    /// rechecked immediately before the synchronous `provider.stream` call,
    /// no await/callback between; (5) the permit AND the admission are held
    /// through received-MessageEnd accounting or failure/cancel/drop — never
    /// during tool execution or child dispatch. A started call that ends
    /// without a received MessageEnd marks the shared authority
    /// accounting-incomplete (spend unknown) and refuses further dispatch.
    async fn admitted_stream<T>(
        &self,
        request: ProviderRequest,
        cancel: &CancellationToken,
        budget: &ExchangeBudget,
        init: impl FnOnce() -> T,
        mut step: impl FnMut(StreamEvent, &mut T) -> Result<(), LoopError>,
    ) -> Result<Streamed<T>, LoopError> {
        // (2) Wait for the single root-owned permit, cancellation-aware.
        let permit = match budget.acquire_dispatch(cancel).await {
            Ok(Some(permit)) => permit,
            Ok(None) => return Ok(Streamed::Canceled),
            Err(_) => {
                return Err(persist(
                    "dispatch permit closed: accounting authority terminalized",
                ));
            }
        };
        // (3) Short ledger admission; no await under the lock.
        let admission = match budget.admit() {
            Ok(admission) => admission,
            Err(crate::agentloop::subagents::AccountingRefusal::Exhausted) => {
                return Ok(Streamed::BudgetExhausted);
            }
            Err(refusal) => return Err(persist(format!("dispatch refused: {refusal}"))),
        };
        // (4) Recheck cancellation with no await before the provider call.
        if cancel.is_cancelled() {
            drop(admission);
            drop(permit);
            return Ok(Streamed::Canceled);
        }
        // (5) The synchronous provider seam itself.
        let mut rx = match self.provider.stream(request) {
            Ok(rx) => rx,
            Err(e) => {
                drop(admission);
                drop(permit);
                return Err(LoopError::Provider(e));
            }
        };
        let mut acc = init();
        let outcome = loop {
            let event = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    // Started, no MessageEnd: spend unknown.
                    budget.mark_incomplete();
                    break Ok(None);
                }
                ev = rx.recv() => ev,
            };
            let Some(event) = event else {
                // Channel closed without MessageEnd: broken provider
                // contract — spend unknown, never an empty turn.
                budget.mark_incomplete();
                break Err(LoopError::Provider(ProviderError::Unavailable(
                    "stream ended without MessageEnd".into(),
                )));
            };
            match event {
                Ok(StreamEvent::MessageEnd { stop_reason, usage }) => {
                    // Recorded ONCE, before any validation/shaping/
                    // persistence of the accumulated turn.
                    budget.record(usage);
                    // Fold the end into the accumulator (turn usage). A fold
                    // error cannot unrecord it: receipt of the end is fact.
                    if let Err(e) = step(StreamEvent::MessageEnd { stop_reason, usage }, &mut acc) {
                        break Err(e);
                    }
                    break Ok(Some(()));
                }
                Ok(other) => {
                    if let Err(e) = step(other, &mut acc) {
                        // The call started; without MessageEnd its spend is
                        // unknown — fail loud AND refuse further dispatch.
                        budget.mark_incomplete();
                        break Err(e);
                    }
                }
                Err(e) => {
                    budget.mark_incomplete();
                    break Err(LoopError::Provider(e));
                }
            }
        };
        drop(admission);
        drop(permit);
        match outcome {
            Ok(Some(())) => Ok(Streamed::Turn(acc)),
            Ok(None) => Ok(Streamed::Canceled),
            Err(e) => Err(e),
        }
    }

    /// Between-turn cancellation: journals the canceled marker ONLY. The
    /// harness is Idle at every call site of this helper; it never settles,
    /// aborts, or otherwise touches any turn — a foreign Active turn (e.g.
    /// started from a deferred callback) stays untouched.
    async fn journal_canceled(
        &self,
        run_id: i64,
        exchange: &Exchange<'_>,
        turn: u32,
    ) -> Result<RunOutcome, LoopError> {
        let salt = exchange.id;
        self.append_owned(
            run_id,
            exchange,
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
        history: &[crate::agentloop::context::ContextEvent],
    ) -> Result<ProviderRequest, LoopError> {
        let effective = crate::agentloop::compaction::reconstruct(history).map_err(persist)?;
        let messages =
            crate::agentloop::context::with_summary(effective.summary.as_deref(), &effective.tail)
                .map_err(persist)?;
        let request = ProviderRequest {
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
        };
        crate::agentloop::context::check_request(&request).map_err(persist)?;
        Ok(request)
    }

    /// Stream one turn to completion, folding deltas into an
    /// [`AssistantTurn`]. Cancellation at ANY await returns
    /// [`Streamed::Canceled`] with the receiver dropped (the provider's
    /// sender ends by its own contract). Dispatch rides the ONE admission
    /// seam ([`Self::admitted_stream`]).
    async fn stream_turn(
        &self,
        request: ProviderRequest,
        cancel: &CancellationToken,
        budget: &ExchangeBudget,
    ) -> Result<Streamed<AssistantTurn>, LoopError> {
        if cancel.is_cancelled() {
            return Ok(Streamed::Canceled);
        }
        self.admitted_stream(
            request,
            cancel,
            budget,
            || AssistantTurn {
                text: String::new(),
                tool_calls: Vec::new(),
                usage: Usage::default(),
            },
            |event, turn| match event {
                StreamEvent::TextDelta(delta) => {
                    turn.text.push_str(&delta);
                    Ok(())
                }
                StreamEvent::ToolCallDelta {
                    id,
                    name,
                    arguments_delta,
                } => {
                    if let Some(existing) = turn.tool_calls.iter_mut().find(|c| c.id == id) {
                        existing.arguments_json.push_str(&arguments_delta);
                    } else {
                        turn.tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments_json: arguments_delta,
                        });
                    }
                    Ok(())
                }
                StreamEvent::MessageStart => Ok(()),
                StreamEvent::MessageEnd { usage, .. } => {
                    turn.usage = usage;
                    Ok(())
                }
            },
        )
        .await
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
        if cancel.is_cancelled() {
            return Ok(ToolOutput::Canceled);
        }
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
            // Cutting the wait does not settle the thread. Never record a
            // tool_done marker or release ownership for indeterminate work.
            Err(_elapsed) => {
                return Err(LoopError::Persist(
                    "tool timed out; indeterminate work retains claim".into(),
                ));
            }
            Ok(Ok(Ok(body))) => ToolOutput::Done(body),
            Ok(Ok(Err(e))) => ToolOutput::Failed(e.to_string()),
            Ok(Err(_join_err)) => {
                return Err(persist(
                    "tool task join failed; indeterminate work retains claim",
                ));
            }
        })
    }

    /// Replay the session window on a pooled connection (reads never touch
    /// the write lane).
    async fn replay(
        &self,
        run_id: i64,
        exchange_id: i64,
    ) -> Result<Vec<crate::agentloop::context::ContextEvent>, LoopError> {
        let pool = self.pool.clone();
        let cap = self.config.session_replay_cap;
        let child_exchange = (!self.session_prefix.is_empty()).then_some(exchange_id);
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| LoopError::Persist(format!("pool: {e}")))?;
            let events = session_log::scoped_compaction_replay(&conn, run_id, cap, child_exchange)
                .map_err(|error| {
                    if error.to_string().contains("context_compaction_boundary") {
                        persist("context_compaction_boundary")
                    } else {
                        persist("context_scope_refused")
                    }
                })?;
            let effective = crate::agentloop::compaction::reconstruct(&events).map_err(persist)?;
            let live = |e: &&crate::agentloop::context::ContextEvent| {
                e.exchange_id == Some(exchange_id) && e.row.kind != "compaction"
            };
            if !effective
                .tail
                .iter()
                .filter(live)
                .any(|e| e.row.kind == "user")
                || events.iter().filter(live).count() != effective.tail.iter().filter(live).count()
            {
                return Err(persist("context_compaction_tail_budget"));
            }
            Ok(events)
        })
        .await
        .map_err(|e| LoopError::Persist(format!("replay join failed: {e}")))?
    }

    /// The session-event kind under this loop's prefix (parent: `user`;
    /// child: `child:<name>:user`).
    fn kind(&self, k: &'static str) -> String {
        format!("{}{k}", self.session_prefix)
    }

    fn fingerprint(&self, input: &str, snapshot: &TurnSnapshot) -> String {
        let actual = (
            snapshot.model().to_string(),
            snapshot.system_prompt().to_string(),
            snapshot.tools().to_vec(),
            snapshot.resources().to_vec(),
        );
        format!("{}:{}", self.policy_hash(Some(&actual)), hash_text(input))
    }

    fn policy_hash(&self, snapshot: Option<&(String, String, Vec<String>, Vec<String>)>) -> String {
        let tools: Vec<_> = self
            .tools
            .iter()
            .map(|t| serde_json::json!([t.name, t.description, t.schema_json]))
            .collect();
        let spec = serde_json::json!({
            "version": 2, "scope": self.session_prefix,
                        "context_contract": crate::agentloop::context::CONTEXT_CONTRACT,
            "invocation_spec": self.retry_spec,
            "provider": self.provider.name(), "harness": snapshot,
            "tools": tools,
            "read_only": self.env.read_only, "process": self.env.allow_process,
            "root": self.env.root, "commands": self.env.allowed_commands,
            "max_turns": self.config.max_turns, "replay_cap": self.config.session_replay_cap,
            "output_cap": self.config.tool_output_cap, "timeout": format!("{:?}", self.config.tool_timeout),
            "budget": self.config.token_budget,
            "budget_contract": "exchange-usage-v1",
            "compaction": [self.config.compaction.just_before_call, self.config.compaction.tool_result_clearing, self.config.compaction.selective_retention]
        });
        // The opaque FsSeam and provider implementation cannot be fingerprinted;
        // the caller must rotate its request key when either implementation changes.
        hash_text(&spec.to_string())
    }

    async fn claim(&self, run_id: i64, owner: &str, acquire: bool) -> Result<(), LoopError> {
        let pool = self.pool.clone();
        let owner = owner.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(persist)?;
            let mut tx = WorkflowTx::begin(&mut conn).map_err(persist)?;
            if acquire {
                session_log::acquire(tx.tx(), run_id, &owner).map_err(persist)?;
            } else {
                session_log::release(tx.tx(), run_id, &owner).map_err(persist)?;
            }
            tx.commit().map_err(persist)?;
            Ok(())
        })
        .await
        .map_err(persist)?
    }

    async fn check_owner(&self, run_id: i64, owner: &str) -> Result<(), LoopError> {
        append_session_events_owned(&self.pool, run_id, owner, Vec::new()).await
    }

    async fn append_owned(
        &self,
        run_id: i64,
        exchange: &Exchange<'_>,
        events: Vec<(String, String, String)>,
    ) -> Result<(), LoopError> {
        append_session_events_owned(&self.pool, run_id, exchange.owner, events).await
    }

    async fn receipt<F>(
        &self,
        run_id: i64,
        exchange: &Exchange<'_>,
        own_claim: bool,
        projection: F,
    ) -> Result<ExchangeReceipt, LoopError>
    where
        F: FnOnce(&ExchangeReceipt) -> Result<Vec<(String, String, String)>, LoopError>
            + Send
            + 'static,
    {
        let pool = self.pool.clone();
        let owner = exchange.owner.to_string();
        let id = exchange.id;
        let prefix = self.session_prefix.clone();
        let invocation = exchange.invocation.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(persist)?;
            let mut tx = WorkflowTx::begin(&mut conn).map_err(persist)?;
            session_log::verify_invocation(tx.tx(), run_id, &owner, &invocation)
                .map_err(persist)?;
            if !session_log::tools_quiescent(tx.tx(), run_id).map_err(persist)? {
                return Err(persist("indeterminate tool work retains claim"));
            }
            let row = session_log::exact(tx.tx(), run_id, &format!("control:exchange_done:{id}"))
                .map_err(persist)?
                .ok_or_else(|| persist("interrupted exchange requires explicit recovery"))?;
            let end: ExchangeEnd = serde_json::from_str(&row.payload_json)
                .map_err(|_| persist("corrupt exchange receipt"))?;
            if row.kind != "control:exchange_done" || end.version != 1 {
                return Err(persist("unsupported exchange receipt"));
            }
            let turns = match &end.outcome {
                RunOutcome::Completed { turns, .. }
                | RunOutcome::TurnCapReached { turns, .. }
                | RunOutcome::BudgetExceeded { turns, .. } => Some(*turns),
                RunOutcome::Canceled => None,
            };
            let final_text = match end.assistant_key {
                Some(key) => {
                    let start = format!("{prefix}run{run_id}:a{id}:t");
                    let turn = key.strip_prefix(&start).and_then(|t| t.parse::<u32>().ok());
                    if turn.is_none_or(|t| t == 0) || turns.is_some_and(|t| Some(t) != turn) {
                        return Err(persist("receipt assistant identity mismatch"));
                    }
                    let assistant = session_log::exact(tx.tx(), run_id, &key)
                        .map_err(persist)?
                        .ok_or_else(|| persist("receipt assistant missing"))?;
                    if assistant.kind != format!("{prefix}assistant") {
                        return Err(persist("receipt assistant kind mismatch"));
                    }
                    text_of(&assistant.payload_json)
                        .ok_or_else(|| persist("corrupt receipt assistant"))?
                }
                None if turns.is_none_or(|t| t == 0) => String::new(),
                None => return Err(persist("receipt assistant reference missing")),
            };
            let receipt = ExchangeReceipt {
                outcome: end.outcome,
                final_text,
                exchange_id: id,
            };
            for (kind, payload, key) in projection(&receipt)? {
                session_log::append(
                    tx.tx(),
                    run_id,
                    &kind,
                    &payload,
                    &key,
                    chrono::Utc::now().timestamp(),
                )
                .map_err(persist)?;
            }
            session_log::finish_invocation(tx.tx(), run_id, &owner, &invocation)
                .map_err(persist)?;
            if own_claim {
                session_log::release(tx.tx(), run_id, &owner).map_err(persist)?;
            }
            tx.commit().map_err(persist)?;
            Ok(receipt)
        })
        .await
        .map_err(persist)?
    }
}

fn hash_text(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("v2:{hex}")
}

fn persist(error: impl std::fmt::Display) -> LoopError {
    LoopError::Persist(error.to_string())
}

pub(crate) async fn append_session_events_owned(
    pool: &Pool,
    run_id: i64,
    owner: &str,
    events: Vec<(String, String, String)>,
) -> Result<(), LoopError> {
    let pool = pool.clone();
    let owner = owner.to_string();
    tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(persist)?;
        let mut tx = WorkflowTx::begin(&mut conn).map_err(persist)?;
        session_log::verify_owner(tx.tx(), run_id, &owner).map_err(persist)?;
        for (kind, payload, key) in events {
            session_log::append(
                tx.tx(),
                run_id,
                &kind,
                &payload,
                &key,
                chrono::Utc::now().timestamp(),
            )
            .map_err(persist)?;
        }
        tx.commit().map_err(persist)?;
        Ok(())
    })
    .await
    .map_err(persist)?
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

/// How one in-turn attempt ended (before the loop-level outcome decision).
enum TurnEnd {
    Done(AssistantTurn, String),
    Canceled,
    /// Admission refused between the loop-top check and dispatch.
    Exhausted,
}

/// Terminal state of one streamed turn.
#[derive(Debug, Clone, PartialEq)]
enum Streamed<T> {
    Turn(T),
    Canceled,
    /// Admission refused: the ceiling was crossed between the loop-top
    /// check and dispatch. The admitted call never ran.
    BudgetExhausted,
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
    use crate::agentloop::provider::{
        ChatMessage, LoopbackProvider, scripted_text, scripted_text_then_tool,
    };
    use crate::audit::verify_chain;
    use crate::config;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::session_log::SessionEventRow;
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
        fixture_hooks_with(script, config, LoopHooks::pass_through())
    }

    fn fixture_hooks_with(
        script: Vec<Vec<StreamEvent>>,
        config: LoopConfig,
        hooks: LoopHooks,
    ) -> Fixture {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = crate::pool::SqliteConnectionManager::file(tmp.path());
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
            hooks,
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

    fn reopened_driver(
        f: &Fixture,
        provider: Arc<dyn LlmProvider>,
        config: LoopConfig,
    ) -> LoopDriver {
        reopened_driver_hooks(f, provider, config, LoopHooks::pass_through())
    }

    fn reopened_driver_hooks(
        f: &Fixture,
        provider: Arc<dyn LlmProvider>,
        config: LoopConfig,
        hooks: LoopHooks,
    ) -> LoopDriver {
        let pool: Pool = r2d2::Pool::builder()
            .max_size(2)
            .build(crate::pool::SqliteConnectionManager::file(f.tmp.path()))
            .unwrap();
        let host = Arc::new(SqliteWorkflowHost::new(pool.clone()));
        let harness = Arc::new(AgentHarness::new(
            host.clone(),
            "test-model",
            "you are a steward",
        ));
        LoopDriver::new(
            pool,
            host,
            harness,
            provider,
            vec![brain_engine_sdk::env::create_read_tool()],
            f.driver.env.clone(),
            config,
            "",
            hooks,
        )
    }

    /// Deny-at-start policy: refuses every owned entry (test helper).
    fn deny_start_hooks() -> LoopHooks {
        let hooks = brain_engine_sdk::events::Hooks::new();
        hooks
            .on::<BeforeStart, _>(LOOP_BEFORE_START, "test-deny", |_| {
                brain_engine_sdk::events::Verdict::Deny("start refused by test policy".into())
            })
            .unwrap();
        LoopHooks::new(Arc::new(hooks))
    }

    /// Deny-at-input policy: refuses every exchange input (test helper).
    fn deny_input_hooks() -> LoopHooks {
        let hooks = brain_engine_sdk::events::Hooks::new();
        hooks
            .on::<String, _>(LOOP_BEFORE_INPUT, "test-deny", |_| {
                brain_engine_sdk::events::Verdict::Deny("input refused by test policy".into())
            })
            .unwrap();
        LoopHooks::new(Arc::new(hooks))
    }

    fn durable_counts(f: &Fixture) -> (i64, i64) {
        let conn = Connection::open(f.tmp.path()).unwrap();
        let audits: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_session_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        (audits, events)
    }

    /// Every session kind for run 1, INCLUDING the `control:*` rows that
    /// `replay` deliberately filters out (claims, invocations live there).
    fn all_kinds(f: &Fixture) -> Vec<String> {
        let conn = Connection::open(f.tmp.path()).unwrap();
        let mut stmt = conn
            .prepare("SELECT kind FROM agent_session_events WHERE run_id=1 ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn audit_rows_detail(f: &Fixture, detail: &str) -> i64 {
        let conn = Connection::open(f.tmp.path()).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM audit_events WHERE detail_hash=?1",
            rusqlite::params![crate::audit::hash(detail)],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// C2/C4: a before-start deny refuses the whole owned run BEFORE the
    /// claim, the invocation row, and any provider work; exactly one coarse
    /// audit row is the durable trace.
    #[test]
    fn denied_before_start_refuses_the_run_before_claim() {
        let f = fixture_hooks_with(
            vec![scripted_text("must never run")],
            LoopConfig::default(),
            deny_start_hooks(),
        );
        let refused = rt()
            .block_on(f.driver.run_turns(1, "hello", &CancellationToken::new()))
            .unwrap_err();
        assert!(
            matches!(refused, LoopError::Hook(_)),
            "the refusal is a named hook denial: {refused}"
        );
        assert_eq!(
            f.provider.requests().len(),
            0,
            "no provider work on a start deny"
        );
        let kinds: Vec<String> = all_kinds(&f);
        assert!(
            !kinds.iter().any(|k| k == "control:claim"),
            "no claim taken"
        );
        assert!(
            !kinds.iter().any(|k| k == "control:invocation"),
            "no invocation row"
        );
        // Row pins ride the exact fixed detail: under compliance-pack the
        // host additionally extends the chain with its own decision rows
        // (status Ok, detail "decision:<hash>"), so actor/status counts are
        // NOT feature-stable — the kernel's own decision count is.
        assert_eq!(audit_rows_detail(&f, "loop start denied by hook policy"), 1);
    }

    /// C2/C4: a before-input deny refuses the exchange BEFORE admission and
    /// provider work; the claim is retained per the tree's error-path law
    /// ("errors/drops deliberately leave it held"), and exactly one audit
    /// row records the denial with a fixed kernel reason.
    #[test]
    fn denied_before_input_refuses_the_exchange_and_prevents_provider_work() {
        let f = fixture_hooks_with(
            vec![scripted_text("must never run")],
            LoopConfig::default(),
            deny_input_hooks(),
        );
        let refused = rt()
            .block_on(f.driver.run_turns(1, "hello", &CancellationToken::new()))
            .unwrap_err();
        assert!(
            matches!(refused, LoopError::Hook(_)),
            "the refusal is a named hook denial: {refused}"
        );
        assert_eq!(f.provider.requests().len(), 0, "provider count 0 on deny");
        let kinds: Vec<String> = all_kinds(&f);
        assert!(
            kinds.iter().any(|k| k == "control:claim"),
            "the claim WAS taken (the boundary sits after the claim gate)"
        );
        assert!(
            kinds.iter().any(|k| k == "control:invocation"),
            "the invocation row was begun (claim-gate side)"
        );
        assert!(
            !kinds.iter().any(|k| k == "user"),
            "the exchange was never admitted"
        );
        assert_eq!(audit_rows_detail(&f, "loop input denied by hook policy"), 1);
    }

    /// C5: steer touches the pending input of the CURRENT exchange only.
    /// Exchange 1 (unmarked) reaches the provider byte-identical and writes
    /// no steer row; exchange 2 (marked) is steered exactly once; the
    /// system prompt is byte-identical across both (C6's law).
    #[test]
    fn steer_rewrites_only_the_marked_input_of_the_current_exchange() {
        let hooks = brain_engine_sdk::events::Hooks::new();
        hooks
            .on_mutate::<String, _>(LOOP_BEFORE_INPUT, "test-steer", |input: &mut String| {
                if input.contains("[steer]") {
                    *input = input.replace("[steer]", "[steered]");
                }
            })
            .unwrap();
        let f = fixture_hooks_with(
            vec![scripted_text("one"), scripted_text("two")],
            LoopConfig::default(),
            LoopHooks::new(Arc::new(hooks)),
        );
        let cancel = CancellationToken::new();
        rt().block_on(async {
            f.driver.run_turns(1, "plain input", &cancel).await.unwrap();
            f.driver
                .run_turns(1, "marked [steer] input", &cancel)
                .await
                .unwrap();
        });
        let requests = f.provider.requests();
        assert_eq!(requests.len(), 2);
        // The context projection frames user rows as
        // "Source: user input\n{shaped}" — the pin is on the input bytes.
        let saw_user = |r: &ProviderRequest, input: &str| {
            let framed = format!("Source: user input\n{input}");
            r.messages
                .iter()
                .any(|m| matches!(m, ChatMessage::User { text: t } if t.contains(&framed)))
        };
        assert!(
            saw_user(&requests[0], "plain input"),
            "exchange 1 reaches the provider byte-identical"
        );
        assert!(
            saw_user(&requests[1], "marked [steered] input"),
            "exchange 2 is steered exactly once"
        );
        assert!(
            !requests[1]
                .messages
                .iter()
                .any(|m| m.text().contains("[steer]")),
            "the unsteered marker never reaches the provider"
        );
        assert_eq!(
            requests[0].system_prompt, requests[1].system_prompt,
            "steer never touches the system prompt"
        );
        assert_eq!(
            audit_rows_detail(&f, "loop input steered by hook listener"),
            1,
            "exactly one steer application row (exchange 2 only)"
        );
    }

    /// C2/C4: a before-compaction deny skips THIS cycle — no summary
    /// provider call, no compaction row — and the loop completes anyway;
    /// exactly one coarse audit row records the skip.
    #[test]
    fn denied_before_compaction_skips_the_cycle_and_the_loop_completes() {
        let hooks = brain_engine_sdk::events::Hooks::new();
        hooks
            .on::<BeforeCompaction, _>(LOOP_BEFORE_COMPACTION, "test-deny", |_| {
                brain_engine_sdk::events::Verdict::Deny("no compaction".into())
            })
            .unwrap();
        let f = fixture_hooks_with(
            vec![
                scripted_text("summary would have been here"),
                scripted_text("done"),
            ],
            LoopConfig::default(),
            LoopHooks::new(Arc::new(hooks)),
        );
        seed_compaction_pressure(&f, 25, 4_000, "pressure");
        let outcome = rt().block_on(f.driver.run_turns(1, "current", &CancellationToken::new()));
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed { .. })),
            "the loop continues past a compaction deny: {outcome:?}"
        );
        let requests = f.provider.requests();
        assert_eq!(requests.len(), 1, "the summary provider call never happens");
        assert_ne!(
            requests[0].system_prompt,
            crate::agentloop::compaction::COMPACTION_SYSTEM_PROMPT,
            "the single request is the MAIN exchange, not a summary call"
        );
        assert!(
            !session_kinds(&f).iter().any(|(_, k)| k == "compaction"),
            "no compaction row is stored"
        );
        assert_eq!(
            audit_rows_detail(&f, "compaction skipped by hook policy"),
            1
        );
    }

    /// C6: the harness system prompt is fixed at construction and rides
    /// every MAIN lane byte-identically — first start, resume (a reopened
    /// driver over the same store), and the post-compaction exchange. The
    /// summary call is the one lane with its own fixed prompt. Because the
    /// prompt is EXACTLY the construction snapshot on every main request,
    /// no plan/strip content can ever appear in it (the dynamic tail is
    /// input-borne by the assembler law pinned in `workflow::gdl`).
    #[test]
    fn system_prompt_is_byte_identical_across_resume_and_post_compaction() {
        // Summary calls consume script entries too; seed enough for however
        // many compactions the seeded pressure produces.
        let f = fixture(vec![
            scripted_text("answer 1"),
            scripted_text("answer 2"),
            scripted_text("answer 3"),
            scripted_text("answer 4"),
            scripted_text("answer 5"),
            scripted_text("answer 6"),
        ]);
        seed_compaction_pressure(&f, 25, 4_000, "pressure");
        let cancel = CancellationToken::new();
        rt().block_on(async {
            f.driver
                .run_turns(1, "exchange one", &cancel)
                .await
                .unwrap();
            let reopened = reopened_driver(&f, f.provider.clone(), LoopConfig::default());
            reopened
                .run_turns(1, "exchange two", &cancel)
                .await
                .unwrap();
            reopened
                .run_turns(1, "exchange three", &cancel)
                .await
                .unwrap();
        });
        let requests = f.provider.requests();
        let compaction_prompt = crate::agentloop::compaction::COMPACTION_SYSTEM_PROMPT;
        assert!(
            requests
                .iter()
                .any(|r| r.system_prompt == compaction_prompt),
            "a compaction actually ran under the seeded pressure"
        );
        let mains: Vec<&ProviderRequest> = requests
            .iter()
            .filter(|r| r.system_prompt != compaction_prompt)
            .collect();
        assert!(
            mains.len() >= 3,
            "three main exchanges (however many summary calls): {}",
            mains.len()
        );
        for request in &mains {
            assert_eq!(
                request.system_prompt, "you are a steward",
                "the construction snapshot rides every main lane byte-identically"
            );
        }
    }

    /// C7: live dispatch writes NOTHING durable — no audit row, no session
    /// row — while a deadline denial through the DRIVER writes exactly one
    /// coarse audit row with the fixed deadline reason (deny-closed, no
    /// provider work).
    #[test]
    fn live_dispatch_leaves_no_trace_and_deadline_denial_writes_one_row() {
        let f = fixture(vec![scripted_text("unused")]);
        let hooks = brain_engine_sdk::events::Hooks::new();
        hooks
            .on::<String, _>(LOOP_BEFORE_INPUT, "observer", |_| {
                brain_engine_sdk::events::Verdict::Allow
            })
            .unwrap();
        let live = LoopHooks::new(Arc::new(hooks));
        let durable_before = durable_counts(&f);
        rt().block_on(async {
            // The allow path's report is re-emitted LIVE: in-memory only.
            live.run_policy(LOOP_BEFORE_INPUT, &"observed".to_string())
                .await
                .unwrap();
        });
        assert_eq!(
            durable_counts(&f),
            durable_before,
            "live dispatch leaves no durable trace"
        );

        // Deadline denial through the driver: the pinned 500 ms deadline
        // fires before this listener's slow verdict; the late Allow is
        // discarded, the run is refused, and ONE durable row records it.
        let hooks = brain_engine_sdk::events::Hooks::new();
        hooks
            .on::<String, _>(LOOP_BEFORE_INPUT, "slow-listener", |_| {
                std::thread::sleep(Duration::from_millis(700));
                brain_engine_sdk::events::Verdict::Allow
            })
            .unwrap();
        let f2 = fixture_hooks_with(
            vec![scripted_text("must never run")],
            LoopConfig::default(),
            LoopHooks::new(Arc::new(hooks)),
        );
        let refused = rt()
            .block_on(f2.driver.run_turns(1, "hello", &CancellationToken::new()))
            .unwrap_err();
        assert!(
            matches!(refused, LoopError::Hook(_)),
            "a deadline expiry denies closed: {refused}"
        );
        assert!(f2.provider.requests().is_empty());
        assert_eq!(audit_rows_detail(&f2, "hook deadline exceeded"), 1);
    }

    /// B4: the single root-owned permit serializes dispatch at the ONE
    /// admission seam. The first call pauses INSIDE the provider holding
    /// the permit; the second waits cancellation-aware; canceling the
    /// waiter returns Canceled with zero provider work of its own, and
    /// releasing the first lets it complete and record its usage.
    #[test]
    fn b4_permit_serializes_dispatch_and_canceled_wait_releases() {
        use crate::agentloop::provider::{ProviderError, StopReason};
        struct Paused {
            entered: Arc<tokio::sync::Notify>,
            release: Arc<tokio::sync::Notify>,
            calls: std::sync::atomic::AtomicUsize,
        }
        impl LlmProvider for Paused {
            fn name(&self) -> &str {
                "paused"
            }
            fn stream(
                &self,
                _: ProviderRequest,
            ) -> Result<
                tokio::sync::mpsc::Receiver<Result<StreamEvent, ProviderError>>,
                ProviderError,
            > {
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.entered.notify_one();
                let release = Arc::clone(&self.release);
                tokio::spawn(async move {
                    release.notified().await;
                    let _ = tx
                        .send(Ok(StreamEvent::MessageEnd {
                            stop_reason: StopReason::EndTurn,
                            usage: Usage {
                                input_tokens: 5,
                                output_tokens: 2,
                            },
                        }))
                        .await;
                });
                Ok(rx)
            }
        }
        let f = fixture(vec![]);
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let paused = Arc::new(Paused {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let driver = reopened_driver(&f, paused.clone(), LoopConfig::default());
        let budget = ExchangeBudget::new(Some(1000));
        let runtime = rt();
        runtime.block_on(async {
            let request = |n: String| ProviderRequest {
                system_prompt: n,
                messages: vec![],
                tools: vec![],
            };
            let cancel_first = CancellationToken::new();
            let first = driver.stream_turn(request("first".to_string()), &cancel_first, &budget);
            tokio::pin!(first);
            let deadline = tokio::time::sleep(Duration::from_secs(5));
            tokio::pin!(deadline);
            tokio::select! {
                _ = &mut deadline => panic!("first dispatch never entered the provider"),
                result = &mut first => panic!("first returned while paused: {result:?}"),
                _ = entered.notified() => {}
            }
            // The permit is HELD by the paused first call. The second waits;
            // canceling its wait returns Canceled with NO provider work.
            let cancel_second = CancellationToken::new();
            let second = driver.stream_turn(request("second".to_string()), &cancel_second, &budget);
            tokio::pin!(second);
            let mut parked = false;
            for _ in 0..50 {
                tokio::task::yield_now().await;
                if paused.calls.load(std::sync::atomic::Ordering::SeqCst) == 1 {
                    parked = true;
                    break;
                }
            }
            assert!(parked, "the second dispatch must be parked on the permit");
            cancel_second.cancel();
            let second_outcome = (&mut second).await.expect("canceled waiter resolves");
            let second_outcome = if matches!(second_outcome, Streamed::Canceled) {
                second_outcome
            } else {
                panic!("the canceled waiter must return Canceled");
            };
            assert!(matches!(second_outcome, Streamed::Canceled));
            assert_eq!(
                paused.calls.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "the canceled waiter never dispatched"
            );
            // Release the first: it completes and records its usage once.
            release.notify_one();
            let first_outcome = (&mut first).await.expect("first resolves after release");
            assert!(matches!(first_outcome, Streamed::Turn(_)));
            assert_eq!(budget.usage().total(), 7);
        });
    }

    /// C1: cancellation before replay/build and before any dispatch yields
    /// ZERO corresponding work: no provider request, no tool execution, and
    /// only the canceled marker lands.
    #[test]
    fn c1_cancel_before_dispatch_zero_work() {
        let f = fixture(vec![scripted_text("never reached")]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let outcome = rt()
            .block_on(f.driver.run_turns(1, "doomed input", &cancel))
            .unwrap();
        assert_eq!(outcome, RunOutcome::Canceled);
        assert!(f.provider.requests().is_empty(), "no provider request");
        assert!(f.fs.execs.lock().unwrap().is_empty(), "no tool execution");
        let conn = f.pool.get().unwrap();
        let intents: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind='control:tool_intent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(intents, 0, "no tool intents");
    }

    /// C2: deterministic between-tools boundary — the first tool completes
    /// and its result is recorded; the SECOND intent/runner never exist.
    #[test]
    fn c2_cancel_between_tools_second_intent_absent() {
        let mut f = fixture(vec![
            crate::agentloop::provider::scripted_tool_call("c1", "cancels", "{}"),
            crate::agentloop::provider::scripted_tool_call("c2", "read", "a.txt"),
            scripted_text("unreached"),
        ]);
        let cancel = CancellationToken::new();
        let second_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let tools = vec![
            {
                let cancel = cancel.clone();
                ToolDef::new(
                    "cancels",
                    "cancels before the second intent",
                    "{}",
                    move |_, _| {
                        cancel.cancel();
                        Ok("first done".into())
                    },
                )
            },
            {
                let second_ran = Arc::clone(&second_ran);
                ToolDef::new("read", "must never run", "{}", move |_, _| {
                    second_ran.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok("body".into())
                })
            },
        ];
        let mut registry = ToolRegistry::new();
        for tool in &tools {
            registry.register(tool.clone()).unwrap();
        }
        f.driver.registry = Arc::new(registry);
        f.driver.tools = tools;
        // The cancel fires while the FIRST runner executes: its result is
        // never recorded (completion unknown), the second intent/runner
        // never exist, and the uncertain claim retains ownership loudly —
        // no fabricated Canceled receipt.
        let outcome = rt().block_on(f.driver.run_turns(1, "two tools", &cancel));
        assert!(outcome.is_err(), "indeterminate work retains the claim");
        assert!(
            outcome
                .unwrap_err()
                .to_string()
                .contains("indeterminate tool work")
        );
        assert!(
            !second_ran.load(std::sync::atomic::Ordering::SeqCst),
            "the second runner never executed"
        );
        let conn = f.pool.get().unwrap();
        let intents: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind='control:tool_intent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(intents, 1, "the second intent never appended");
        let results: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind IN ('tool_result','control:tool_done')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(results, 0, "no fabricated first result");
    }

    /// C3: drop/abort of the run future while a blocking tool is scheduled —
    /// the runner is released and JOINED, and no late result or tool_done
    /// ever enters the narrative; the uncertain claim is retained.
    #[test]
    fn c3_dropped_future_with_blocking_tool_no_late_result() {
        let mut f = fixture(vec![crate::agentloop::provider::scripted_tool_call(
            "c1", "wait", "{}",
        )]);
        let entered = Arc::new(tokio::sync::Notify::new());
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let latch = Arc::new((StdMutex::new(false), std::sync::Condvar::new()));
        let tool = {
            let entered = entered.clone();
            let finished = finished.clone();
            let latch = latch.clone();
            ToolDef::new("wait", "benign blocking fixture", "{}", move |_, _| {
                entered.notify_one();
                let (lock, wake) = &*latch;
                let guard = lock.lock().unwrap();
                let (_guard, _) = wake
                    .wait_timeout_while(guard, Duration::from_secs(5), |released| !*released)
                    .unwrap();
                finished.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok("late".into())
            })
        };
        let mut registry = ToolRegistry::new();
        registry.register(tool.clone()).unwrap();
        f.driver.registry = Arc::new(registry);
        f.driver.tools = vec![tool];
        let runtime = rt();
        let cancel = CancellationToken::new();
        runtime.block_on(async {
            let work = f.driver.run_turns_keyed(1, "c3", "wait", &cancel);
            tokio::pin!(work);
            tokio::select! {
                outcome = &mut work => panic!("returned before tool entered: {outcome:?}"),
                _ = tokio::time::timeout(Duration::from_secs(5), entered.notified()) => {}
            }
            // DROP the caller future while the runner lives (Pin<Box<_>>
            // has no Drop of its own to run; the point is the future never
            // polls again).
            let _dropped = work;
            // Release and join the runner (bounded latch, never a sleep).
            {
                let (lock, wake) = &*latch;
                *lock.lock().unwrap() = true;
                wake.notify_all();
            }
            let deadline = tokio::time::sleep(Duration::from_secs(5));
            tokio::pin!(deadline);
            while !finished.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::select! {
                    _ = &mut deadline => panic!("runner never finished"),
                    _ = tokio::task::yield_now() => {}
                }
            }
        });
        let conn = f.pool.get().unwrap();
        let late: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind IN ('control:tool_done','tool_result')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(late, 0, "no late tool result entered the narrative");
        assert!(
            !session_log::quiescent(&conn, 1).unwrap(),
            "the uncertain claim is retained"
        );
    }

    /// C4: cancellation-before-commit vs commit-before-cancel observed
    /// separately at the R3 compaction linearization point.
    #[test]
    fn c4_compaction_commit_vs_cancel_boundary_observed_separately() {
        // (a) Cancellation BEFORE commit: no compaction row survives.
        {
            let mut config = LoopConfig::default();
            config.compaction.just_before_call = true;
            let mut f = fixture_with(vec![scripted_text("brief"), scripted_text("done")], config);
            seed_compaction_pressure(&f, 25, 4_000, "pressure");
            let entered = Arc::new(tokio::sync::Notify::new());
            let committed = Arc::new(tokio::sync::Notify::new());
            let latch = Arc::new((StdMutex::new(false), std::sync::Condvar::new()));
            f.driver.compaction_pause = Some((entered.clone(), committed.clone(), latch.clone()));
            let cancel = CancellationToken::new();
            let runtime = rt();
            let work = f.driver.run_turns_keyed(1, "c4a", "current", &cancel);
            tokio::pin!(work);
            runtime.block_on(async {
                tokio::select! {
                    outcome = &mut work => panic!("returned before the pause: {outcome:?}"),
                    _ = tokio::time::timeout(Duration::from_secs(5), entered.notified()) => {}
                }
                cancel.cancel();
                {
                    let (lock, wake) = &*latch;
                    *lock.lock().unwrap() = true;
                    wake.notify_all();
                }
                let outcome = (&mut work).await.unwrap();
                assert_eq!(outcome.outcome, RunOutcome::Canceled);
            });
            let conn = f.pool.get().unwrap();
            let rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_session_events WHERE kind='compaction'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(rows, 0, "cancellation-before-commit leaves no row");
        }
        // (b) Commit BEFORE cancellation: committed evidence stands.
        {
            let mut config = LoopConfig::default();
            config.compaction.just_before_call = true;
            let mut f = fixture_with(vec![scripted_text("brief"), scripted_text("done")], config);
            seed_compaction_pressure(&f, 25, 4_000, "pressure");
            let entered = Arc::new(tokio::sync::Notify::new());
            let committed = Arc::new(tokio::sync::Notify::new());
            let latch = Arc::new((StdMutex::new(false), std::sync::Condvar::new()));
            f.driver.compaction_pause = Some((entered.clone(), committed.clone(), latch.clone()));
            let cancel = CancellationToken::new();
            let runtime = rt();
            let work = f.driver.run_turns_keyed(1, "c4b", "current", &cancel);
            tokio::pin!(work);
            runtime.block_on(async {
                tokio::select! {
                    outcome = &mut work => panic!("returned before the pause: {outcome:?}"),
                    _ = tokio::time::timeout(Duration::from_secs(5), entered.notified()) => {}
                }
                {
                    let (lock, wake) = &*latch;
                    *lock.lock().unwrap() = true;
                    wake.notify_all();
                }
                let committed_first = tokio::select! {
                    _outcome = &mut work => true,
                    _ = tokio::time::timeout(Duration::from_secs(5), committed.notified()) => false,
                };
                // Commit won; cancel afterward — the loop cancels but the
                // committed evidence is not undone.
                cancel.cancel();
                let outcome = (&mut work).await.unwrap();
                assert_eq!(outcome.outcome, RunOutcome::Canceled);
                let _ = committed_first;
            });
            let conn = f.pool.get().unwrap();
            let rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_session_events WHERE kind='compaction'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(rows, 1, "commit-before-cancel keeps the evidence");
        }
    }

    /// S1: synthetic secret markers must never reach APPLICATION diagnostics.
    /// The provider/backend payload carries the marker; the loop's own
    /// diagnostic surface (persisted session/audit/outbox rows — there are
    /// no log call sites in the loop) must be free of it, while the typed
    /// primary error retains the payload INTERNALLY for equality (the
    /// panic-hook ceiling — a panic hook may print payloads — is documented
    /// and does not apply to these paths).
    #[test]
    fn s1_secret_markers_absent_from_app_diagnostics() {
        use crate::agentloop::provider::{ProviderError, ProviderRequest, StreamEvent};
        const MARKER: &str = "synthetic-secret-payload-marker";
        struct Leaky;
        impl LlmProvider for Leaky {
            fn name(&self) -> &str {
                "leaky"
            }
            fn stream(
                &self,
                _: ProviderRequest,
            ) -> Result<
                tokio::sync::mpsc::Receiver<Result<StreamEvent, ProviderError>>,
                ProviderError,
            > {
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                std::thread::spawn(move || {
                    let _ = tx.blocking_send(Ok(StreamEvent::MessageStart));
                    let _ = tx.blocking_send(Ok(StreamEvent::TextDelta("prose ".into())));
                    let _ = tx.blocking_send(Err(ProviderError::Unavailable(format!(
                        "backend said {MARKER}"
                    ))));
                });
                Ok(rx)
            }
        }
        let f = fixture(vec![]);
        let driver = reopened_driver(&f, Arc::new(Leaky), LoopConfig::default());
        let runtime = rt();
        let error = runtime
            .block_on(driver.run_turns(1, "input", &CancellationToken::new()))
            .unwrap_err();
        eprintln!("DEBUG s1 error: {error}");
        // Internal retention: the typed primary error carries the payload.
        assert!(
            error.to_string().contains(MARKER),
            "primary values are retained internally for equality"
        );
        // Application diagnostics: every persisted row is marker-free.
        let conn = f.pool.get().unwrap();
        for (table, column) in [
            ("agent_session_events", "payload_json"),
            ("outbox", "payload_json"),
        ] {
            let hits: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {column} LIKE '%{MARKER}%'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(hits, 0, "{table}.{column} leaked the marker");
        }
        let observer = Connection::open(f.tmp.path()).unwrap();
        // Audit rows store hashes only — no raw lifecycle text at all.
        let raw: i64 = observer
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash = '' OR target_hash = ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, 0, "audit evidence stores hashes, not payloads");
    }

    #[test]
    fn r4_provider_start_error_settles() {
        rt().block_on(async {
            let f = fixture(vec![]);
            let error = f
                .driver
                .run_turns(1, "task", &CancellationToken::new())
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                LoopError::Provider(ProviderError::Unavailable(_))
            ));
            assert_eq!(
                f.driver.harness.phase(),
                brain_engine_sdk::harness::Phase::Idle
            );
        });
    }

    /// RED against the former generic-abort `cancel_settle` (foreign turn
    /// was aborted: phase Idle) → GREEN with `journal_canceled`: a foreign
    /// turn Active on the harness stays untouched by between-turn
    /// cancellation, and only the marker lands.
    #[test]
    fn between_turn_cancel_journals_only() {
        let f = fixture(vec![]);
        let runtime = rt();
        runtime.block_on(async {
            f.driver.claim(1, "fixture-owner", true).await.unwrap();
            let (_, foreign) = f.driver.harness.start_turn(1).unwrap();
            let exchange = Exchange {
                id: 1,
                owner: "fixture-owner",
                invocation: "invocation",
            };
            let outcome = f.driver.journal_canceled(1, &exchange, 1).await.unwrap();
            assert_eq!(outcome, RunOutcome::Canceled);
            assert_eq!(
                f.driver.harness.phase(),
                brain_engine_sdk::harness::Phase::Running,
                "between-turn cancellation must not abort a foreign turn"
            );
            f.driver.harness.abort_turn(foreign).unwrap();
        });
    }

    /// Mid-turn cancellation settles the OWN turn through the armed token —
    /// exactly one `RunEnd:aborted` — then the marker lands; the consumed
    /// settlement means Drop adds nothing.
    #[test]
    fn cancel_mid_stream_uses_owned_settlement_once() {
        let big_turn: Vec<StreamEvent> = (0..100_000)
            .map(|i| StreamEvent::TextDelta(format!("d{i} ")))
            .collect();
        let f = fixture(vec![big_turn]);
        let runtime = rt();
        let cancel = CancellationToken::new();
        let work = f.driver.run_turns(1, "long turn", &cancel);
        tokio::pin!(work);
        runtime.block_on(async {
            let deadline = tokio::time::sleep(Duration::from_secs(5));
            tokio::pin!(deadline);
            let mut fired = false;
            let outcome = loop {
                // Cancel deterministically once the provider is mid-stream.
                if !fired && !f.provider.requests().is_empty() {
                    cancel.cancel();
                    fired = true;
                }
                tokio::select! {
                    _ = &mut deadline => panic!("never settled within bound"),
                    outcome = &mut work => break outcome,
                    _ = tokio::task::yield_now() => {}
                }
            };
            assert_eq!(outcome.unwrap(), RunOutcome::Canceled);
        });
        let conn = f.pool.get().unwrap();
        let aborted = crate::audit::hash("RunEnd:aborted");
        let finished = crate::audit::hash("RunEnd:finished");
        let aborted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash=?1",
                rusqlite::params![aborted],
                |r| r.get(0),
            )
            .unwrap();
        let finished: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash=?1",
                rusqlite::params![finished],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(aborted, 1, "exactly one owned aborted end");
        assert_eq!(finished, 1, "policy-snapshot pair settled finished");
        assert_eq!(
            f.driver.harness.phase(),
            brain_engine_sdk::harness::Phase::Idle
        );
    }

    /// A failed explicit settlement attempt is disarmed BEFORE host code: the
    /// fault is removed after the failure, yet neither Drop nor a retry lands
    /// the missing end — the claim stays terminal.
    #[test]
    fn disarm_on_failed_attempt_no_drop_retry() {
        let f = fixture(vec![scripted_text("done")]);
        let runend_hash = crate::audit::hash("RunEnd:finished");
        let conn = f.pool.get().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE fixture_attempts(n INTEGER); INSERT INTO fixture_attempts VALUES (0);
             CREATE TRIGGER fail_first_runend BEFORE INSERT ON audit_events
             WHEN NEW.detail_hash='{runend_hash}' AND (SELECT n FROM fixture_attempts) = 0
             BEGIN UPDATE fixture_attempts SET n = 1; SELECT RAISE(ABORT, 'fixture_runend'); END;"
        ))
        .unwrap();
        drop(conn);
        let runtime = rt();
        let error = runtime
            .block_on(f.driver.run_turns(1, "input", &CancellationToken::new()))
            .unwrap_err();
        // The checked writer classifies the INSERT failure with a FIXED
        // refusal ("settlement refused" since the Slice-3 classification —
        // verified rollback = confirmed no-commit), never backend text; that
        // fixed classification is the primary error the driver surfaces.
        assert!(
            error.to_string().contains("settlement refused"),
            "the original audit failure is the primary error: {error}"
        );
        assert_eq!(
            f.driver.harness.phase(),
            brain_engine_sdk::harness::Phase::Failed,
            "failed settlement terminalizes; the guard is disarmed"
        );
        // The fault allowed the SECOND attempt; that no second attempt ran is
        // proven by the absent RunEnd row and the refused restart.
        let conn = f.pool.get().unwrap();
        let runend: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash IN (?1, ?2)",
                rusqlite::params![
                    crate::audit::hash("RunEnd:finished"),
                    crate::audit::hash("RunEnd:aborted")
                ],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(runend, 0, "Drop must not retry the failed attempt");
        assert!(
            runtime
                .block_on(f.driver.run_turns(1, "retry", &CancellationToken::new()))
                .is_err()
        );
    }

    /// Drop cleanup is synchronous with a declared bound: the destructor's
    /// own waits are the configured host waits (this stack's SQLite
    /// `busy_timeout=5000` from migration + r2d2 checkout), never an
    /// unbounded mutex wait. With the database write lock held behind a
    /// joined barrier, the destructor completes fail-closed within that
    /// declared bound — claim retained, no Idle published. Measured bound,
    /// not a hard OS guarantee.
    #[test]
    fn drop_with_contended_lock_fails_closed_within_bound() {
        let f = fixture(vec![]);
        let runtime = rt();
        runtime.block_on(async {
            let (_, token) = f.driver.harness.start_turn(1).unwrap();
            // Hold the database behind a real lock the destructor's host
            // write cannot take; its wait is the configured busy_timeout.
            let holder = Connection::open(f.tmp.path()).unwrap();
            holder.execute_batch("BEGIN IMMEDIATE").unwrap();
            let harness = Arc::clone(&f.driver.harness);
            let started = tokio::time::Instant::now();
            let task = tokio::task::spawn_blocking(move || {
                let settlement = TurnSettlement::arm(harness, token);
                drop(settlement);
            });
            let completed = tokio::time::timeout(Duration::from_secs(10), task).await;
            assert!(
                completed.is_ok(),
                "the destructor must complete within the declared bound"
            );
            completed.unwrap().unwrap();
            let elapsed = started.elapsed();
            assert_eq!(
                f.driver.harness.phase(),
                brain_engine_sdk::harness::Phase::Failed,
                "fail-closed terminal state, never an Idle assertion"
            );
            assert!(
                f.driver.harness.snapshot().is_some(),
                "the claim is retained for diagnosis"
            );
            // The observed wait is bounded: at most the configured 5s SQLite
            // busy wait plus checkout; it completes and it fails closed.
            assert!(
                elapsed <= Duration::from_secs(10),
                "measured destructor bound: {elapsed:?}"
            );
            holder.execute_batch("ROLLBACK").unwrap();
        });
    }

    /// A successful settlement followed by a receipt-projection failure
    /// retains the claim WITHOUT fabricating a second (aborted) end.
    #[test]
    fn no_fabricated_abort_after_projection_failure() {
        let f = fixture(vec![scripted_text("settled turn")]);
        f.pool
            .get()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_projection BEFORE INSERT ON agent_session_events
                 WHEN NEW.kind='subagent_result'
                 BEGIN SELECT RAISE(ABORT, 'fixture_projection'); END;",
            )
            .unwrap();
        let runtime = rt();
        let cancel = CancellationToken::new();
        runtime.block_on(f.driver.claim(1, "owner", true)).unwrap();
        let result = runtime.block_on(f.driver.run_turns_owned_projected(
            1,
            "projection",
            "input",
            "owner",
            &cancel,
            |receipt| {
                Ok(vec![(
                    "subagent_result".into(),
                    "projection".into(),
                    format!("projection:{}", receipt.exchange_id),
                )])
            },
        ));
        assert!(result.is_err(), "projection refusal surfaces");
        let conn = f.pool.get().unwrap();
        let aborted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash=?1",
                rusqlite::params![crate::audit::hash("RunEnd:aborted")],
                |r| r.get(0),
            )
            .unwrap();
        let finished: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash=?1",
                rusqlite::params![crate::audit::hash("RunEnd:finished")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            aborted, 0,
            "no fabricated aborted end after projection failure"
        );
        assert_eq!(finished, 2, "policy pair + turn pair ended exactly once");
        // The claim is retained: the interrupted invocation stays unfinished,
        // so a NEW invocation (even the exact same request key) is refused —
        // no recovery route, no fabricated completion.
        f.pool
            .get()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_projection")
            .unwrap();
        let retry = runtime.block_on(f.driver.run_turns_owned_projected(
            1,
            "projection",
            "input",
            "owner",
            &cancel,
            |receipt| {
                Ok(vec![(
                    "subagent_result".into(),
                    "projection".into(),
                    format!("projection:{}", receipt.exchange_id),
                )])
            },
        ));
        assert!(
            retry.unwrap_err().to_string().contains("retains ownership"),
            "the unfinished invocation retains ownership across retries"
        );
        assert_eq!(f.provider.requests().len(), 1, "no re-dispatch");
    }

    /// The exact original failure is the primary error on every path — never
    /// re-wrapped, never replaced by secondary settlement text.
    #[test]
    fn primary_error_equality_start_midstream_persistence() {
        let runtime = rt();
        // Start failure: the RunStart audit refusal surfaces as the primary.
        let f = fixture(vec![scripted_text("unused")]);
        let runstart_hash = crate::audit::hash("RunStart");
        f.pool
            .get()
            .unwrap()
            .execute_batch(&format!(
                "CREATE TRIGGER fail_runstart BEFORE INSERT ON audit_events
             WHEN NEW.detail_hash='{runstart_hash}'
             BEGIN SELECT RAISE(ABORT, 'fixture_runstart'); END;"
            ))
            .unwrap();
        let error = runtime
            .block_on(f.driver.run_turns(1, "input", &CancellationToken::new()))
            .unwrap_err();
        let fresh = AgentHarness::new(
            Arc::clone(&f.driver.host),
            "test-model",
            "you are a steward",
        );
        let direct = fresh.start_turn(1).unwrap_err();
        assert_eq!(
            error,
            LoopError::Harness(direct.to_string()),
            "start failure: exact original error, no re-wrapping"
        );
        // Midstream persistence failure: the append refusal is the primary.
        let f = fixture(vec![scripted_text("assistant bytes")]);
        f.pool
            .get()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_assistant BEFORE INSERT ON agent_session_events
             WHEN NEW.kind='assistant' BEGIN SELECT RAISE(ABORT, 'fixture_assistant'); END;",
            )
            .unwrap();
        let error = runtime
            .block_on(f.driver.run_turns(1, "input", &CancellationToken::new()))
            .unwrap_err();
        assert!(
            matches!(&error, LoopError::Persist(m) if m.contains("fixture_assistant")),
            "midstream failure: the original refusal is the primary: {error}"
        );
        let conn = f.pool.get().unwrap();
        let aborted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash=?1",
                rusqlite::params![crate::audit::hash("RunEnd:aborted")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(aborted, 1, "one aborted end when cleanup succeeds");
    }

    #[test]
    fn r4_zero_budget_stops_before_dispatch() {
        rt().block_on(async {
            let f = fixture_with(
                vec![scripted_text("unused")],
                LoopConfig {
                    token_budget: Some(0),
                    ..LoopConfig::default()
                },
            );
            let result = f
                .driver
                .run_turns(1, "task", &CancellationToken::new())
                .await
                .unwrap();
            assert_eq!(
                result,
                RunOutcome::BudgetExceeded {
                    turns: 0,
                    usage: Usage::default()
                }
            );
            assert!(f.provider.requests().is_empty());
        });
    }

    #[test]
    fn r4_summary_usage_stops_next_ordinary_call() {
        rt().block_on(async {
            let f = fixture_with(
                vec![scripted_text("brief"), scripted_text("unused")],
                LoopConfig {
                    token_budget: Some(10),
                    ..LoopConfig::default()
                },
            );
            seed_compaction_pressure(&f, 8, 60_000, "seed");
            let result = f
                .driver
                .run_turns(1, "task", &CancellationToken::new())
                .await
                .unwrap();
            assert_eq!(
                result,
                RunOutcome::BudgetExceeded {
                    turns: 0,
                    usage: Usage {
                        input_tokens: 10,
                        output_tokens: 4
                    }
                }
            );
            assert_eq!(f.provider.requests().len(), 1);
        });
    }

    #[test]
    fn scoped_context_preserves_tool_conversation_in_actual_request() {
        rt().block_on(async {
            let f = fixture(vec![
                scripted_text_then_tool("Read the fixture", "context-call-1", "read", "a.txt"),
                scripted_text("Finished"),
            ]);
            f.driver
                .run_turns_keyed(
                    1,
                    "context-request",
                    "Inspect the fixture",
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            let requests = f.provider.requests();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].system_prompt, requests[1].system_prompt);
            assert_eq!(requests[1].messages.len(), 3);
            let assistant = format!("{:?}", requests[1].messages[1]);
            assert!(
                assistant.contains("context-call-1"),
                "actual follow-up request lost the assistant tool-call identity"
            );
            assert!(
                assistant.contains("a.txt"),
                "tool arguments must survive replay"
            );
            let result = format!("{:?}", requests[1].messages[2]);
            assert!(
                result.contains("context-call-1"),
                "result must retain call identity"
            );
            assert!(result.contains("steward file body"));
            assert!(
                !result.contains("role: User"),
                "tool output is not human input"
            );
            let rows = session_log::replay(&f.pool.get().unwrap(), 1, 500).unwrap();
            let stored = rows.iter().find(|row| row.kind == "tool_result").unwrap();
            let payload: serde_json::Value = serde_json::from_str(&stored.payload_json).unwrap();
            assert_eq!(payload["id"], "context-call-1");
            assert_eq!(payload["ok"], true);
            assert_eq!(payload["output"], "steward file body");
            assert!(f.fs.execs.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn r1_same_owner_cannot_enter_or_release_before_receipt_projection() {
        let mut f = fixture(vec![scripted_text("child reply")]);
        let runtime = rt();
        let entered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        f.driver.receipt_pause = Some((entered.clone(), resume.clone()));
        let second_provider =
            LoopbackProvider::new("loopback", vec![scripted_text("must not run")]);
        let second = reopened_driver(&f, second_provider.clone(), LoopConfig::default());
        runtime.block_on(async {
            f.driver.claim(1, "parent", true).await.unwrap();
            let cancel = CancellationToken::new();
            let first = f.driver.run_turns_owned_projected(1, "parent:first", "task", "parent", &cancel, |receipt| {
                Ok(vec![("subagent_result".into(), receipt.final_text.clone(), format!("child-result:{}", receipt.exchange_id))])
            });
            tokio::pin!(first);
            tokio::select! {
                result = &mut first => panic!("returned before receipt pause: {result:?}"),
                signal = tokio::time::timeout(Duration::from_secs(5), entered.notified()) => signal.unwrap(),
            }
            // exchange_done already exists, but neither receipt nor child
            // projection is finalized. Even the same owner must stay excluded.
            let terminal: i64 = f.pool.get().unwrap().query_row("SELECT count(*) FROM agent_session_events WHERE kind='control:exchange_done'", [], |r| r.get(0)).unwrap();
            assert_eq!(terminal, 1);
            assert!(second.run_turns_owned(1, "parent:second", "task", "parent", &cancel).await.is_err());
            assert!(second_provider.requests().is_empty());
            assert!(second.claim(1, "parent", false).await.is_err());
            {
                let mut conn = f.pool.get().unwrap();
                let mut tx = WorkflowTx::begin(&mut conn).unwrap();
                assert!(session_log::check_idle(tx.tx(), 1).is_err());
            }
            resume.notify_one();
            let receipt = first.await.unwrap();
            let row = session_log::exact(&f.pool.get().unwrap(), 1, &format!("child-result:{}", receipt.exchange_id)).unwrap().unwrap();
            assert_eq!(row.payload_json, "child reply");
            second.claim(1, "parent", false).await.unwrap();
        });
    }

    #[test]
    fn r1_policy_identity_is_idle_pure_and_tracks_all_bound_policy() {
        let mut f = fixture(vec![scripted_text("unused")]);
        f.driver
            .bind_policy("test-model", "you are a steward", Vec::new(), Vec::new());
        let initial = f.driver.policy_identity();
        assert_eq!(initial, f.driver.policy_identity());
        assert!(f.provider.requests().is_empty());
        assert_eq!(
            f.pool
                .get()
                .unwrap()
                .query_row("SELECT count(*) FROM audit_events", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        f.driver.config.max_turns += 1;
        assert_ne!(initial, f.driver.policy_identity());
        f.driver.config.max_turns -= 1;
        assert_eq!(initial, f.driver.policy_identity());
        f.driver.set_retry_spec("different spec".into());
        assert_ne!(initial, f.driver.policy_identity());
        f.driver.set_retry_spec(String::new());
        f.driver.bind_policy(
            "other-model",
            "other-system",
            vec!["tool".into()],
            vec!["resource".into()],
        );
        assert_ne!(initial, f.driver.policy_identity());
    }

    #[test]
    fn r1_mismatch_then_correct_retry_does_not_strand_claim() {
        let f = fixture(vec![scripted_text("original")]);
        let runtime = rt();
        let cancel = CancellationToken::new();
        let original = runtime
            .block_on(f.driver.run_turns_keyed(1, "stable", "input", &cancel))
            .unwrap();
        assert!(
            runtime
                .block_on(f.driver.run_turns_keyed(1, "stable", "wrong", &cancel))
                .is_err()
        );
        let retried = runtime
            .block_on(f.driver.run_turns_keyed(1, "stable", "input", &cancel))
            .unwrap();
        assert_eq!(retried, original);
        assert_eq!(f.provider.requests().len(), 1);
    }

    #[test]
    fn r1_keyed_retry_reopens_and_returns_exact_old_assistant() {
        let f = fixture(vec![
            scripted_text_then_tool("read", "c", "read", "a.txt"),
            scripted_text("original"),
            scripted_text("newer"),
        ]);
        let runtime = rt();
        let cancel = CancellationToken::new();
        let first = runtime
            .block_on(f.driver.run_turns_keyed(1, "request", "read", &cancel))
            .unwrap();
        runtime
            .block_on(f.driver.run_turns_keyed(1, "other", "new", &cancel))
            .unwrap();
        for n in 0..session_log::REPLAY_CAP {
            session_log::append(
                &f.pool.get().unwrap(),
                1,
                "user",
                "seed",
                &format!("late:{n}"),
                1,
            )
            .unwrap();
        }
        let empty = LoopbackProvider::new("loopback", vec![]);
        let reopened = reopened_driver(&f, empty.clone(), LoopConfig::default());
        let before = session_kinds(&f);
        let second = runtime
            .block_on(reopened.run_turns_keyed(1, "request", "read", &cancel))
            .unwrap();
        assert_eq!(second, first);
        assert_eq!(second.final_text, "original");
        assert_eq!(session_kinds(&f), before);
        assert!(empty.requests().is_empty());
        let count: i64 = f
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM agent_session_events WHERE kind='tool_result'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn r1_retry_fingerprint_refuses_text_policy_and_prompt_changes() {
        for change in ["text", "policy", "prompt", "scope", "tools"] {
            let f = fixture(vec![scripted_text("original")]);
            let runtime = rt();
            let cancel = CancellationToken::new();
            runtime
                .block_on(f.driver.run_turns_keyed(1, "request", "input", &cancel))
                .unwrap();
            let empty = LoopbackProvider::new("loopback", vec![]);
            let mut other = reopened_driver(&f, empty.clone(), LoopConfig::default());
            let mut input = "input";
            match change {
                "text" => input = "different",
                "policy" => other.config.token_budget = Some(99),
                "prompt" => other.harness.set_system_prompt("different").unwrap(),
                "scope" => other.session_prefix = "child:different:".into(),
                "tools" => other.tools.clear(),
                _ => unreachable!(),
            }
            let before = session_kinds(&f);
            let error = runtime
                .block_on(other.run_turns_keyed(1, "request", input, &cancel))
                .unwrap_err();
            assert!(
                error.to_string().contains("fingerprint mismatch"),
                "{change}: {error}"
            );
            assert!(empty.requests().is_empty());
            assert_eq!(session_kinds(&f), before);
        }
    }

    #[test]
    fn r1_provider_paused_excludes_other_pool_and_drop_retains_claim() {
        struct Paused {
            entered: Arc<tokio::sync::Notify>,
            sender: StdMutex<Option<tokio::sync::mpsc::Sender<Result<StreamEvent, ProviderError>>>>,
        }
        impl LlmProvider for Paused {
            fn name(&self) -> &str {
                "paused"
            }
            fn stream(
                &self,
                _: ProviderRequest,
            ) -> Result<
                tokio::sync::mpsc::Receiver<Result<StreamEvent, ProviderError>>,
                ProviderError,
            > {
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                *self.sender.lock().unwrap() = Some(tx);
                self.entered.notify_one();
                Ok(rx)
            }
        }
        let f = fixture(vec![]);
        let entered = Arc::new(tokio::sync::Notify::new());
        let paused = Arc::new(Paused {
            entered: entered.clone(),
            sender: StdMutex::new(None),
        });
        let a = reopened_driver(&f, paused, LoopConfig::default());
        let empty = LoopbackProvider::new("loopback", vec![]);
        let b = reopened_driver(&f, empty.clone(), LoopConfig::default());
        let runtime = rt();
        runtime.block_on(async {
            let task = tokio::spawn(async move {
                a.run_turns_keyed(1, "a", "one", &CancellationToken::new())
                    .await
            });
            tokio::time::timeout(Duration::from_secs(5), entered.notified())
                .await
                .unwrap();
            let before = session_kinds(&f);
            let error = b
                .run_turns(1, "two", &CancellationToken::new())
                .await
                .unwrap_err();
            assert!(error.to_string().contains("already owned"));
            assert!(empty.requests().is_empty());
            assert_eq!(session_kinds(&f), before);
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(
                b.run_turns(1, "retry after drop", &CancellationToken::new())
                    .await
                    .is_err()
            );
            assert_eq!(session_kinds(&f), before);
        });
    }

    #[test]
    fn r1_tool_cancel_retains_claim_while_blocking_work_lives() {
        let mut f = fixture(vec![scripted_text_then_tool("waiting", "c", "wait", "{}")]);
        let entered = Arc::new(tokio::sync::Notify::new());
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let latch = Arc::new((StdMutex::new(false), std::sync::Condvar::new()));
        let tool = {
            let entered = entered.clone();
            let finished = finished.clone();
            let latch = latch.clone();
            ToolDef::new("wait", "benign blocking fixture", "{}", move |_, _| {
                entered.notify_one();
                let (lock, wake) = &*latch;
                let guard = lock.lock().unwrap();
                let (_guard, _) = wake
                    .wait_timeout_while(guard, Duration::from_secs(5), |released| !*released)
                    .unwrap();
                finished.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok("finished".into())
            })
        };
        let mut registry = ToolRegistry::new();
        registry.register(tool.clone()).unwrap();
        f.driver.registry = Arc::new(registry);
        f.driver.tools = vec![tool];
        let runtime = rt();
        let cancel = CancellationToken::new();
        runtime.block_on(async {
            let work = f.driver.run_turns_keyed(1, "tool", "wait", &cancel);
            tokio::pin!(work);
            tokio::select! {
                outcome = &mut work => panic!("returned before tool entered: {outcome:?}"),
                _ = tokio::time::timeout(Duration::from_secs(5), entered.notified()) => {}
            }
            cancel.cancel();
            let result = (&mut work).await;
            assert!(result.is_err());
            assert!(!finished.load(std::sync::atomic::Ordering::SeqCst));
            let other = reopened_driver(
                &f,
                LoopbackProvider::new("loopback", vec![]),
                LoopConfig::default(),
            );
            assert!(
                other
                    .run_turns(1, "new", &CancellationToken::new())
                    .await
                    .is_err()
            );
            assert_eq!(f.provider.requests().len(), 1);
            let (lock, wake) = &*latch;
            *lock.lock().unwrap() = true;
            wake.notify_all();
        });
        assert!(!session_log::quiescent(&f.pool.get().unwrap(), 1).unwrap());
    }

    #[test]
    fn r1_assistant_projection_failure_retains_claim_and_never_regenerates() {
        let f = fixture(vec![scripted_text("committed outbox bytes")]);
        f.pool.get().unwrap().execute_batch("CREATE TRIGGER fail_assistant BEFORE INSERT ON agent_session_events WHEN NEW.kind='assistant' BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        let runtime = rt();
        let cancel = CancellationToken::new();
        assert!(
            runtime
                .block_on(f.driver.run_turns_keyed(1, "request", "input", &cancel))
                .is_err()
        );
        f.pool
            .get()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_assistant")
            .unwrap();
        let empty = LoopbackProvider::new("loopback", vec![]);
        let other = reopened_driver(&f, empty.clone(), LoopConfig::default());
        assert!(
            runtime
                .block_on(other.run_turns_keyed(1, "request", "input", &cancel))
                .is_err()
        );
        assert!(empty.requests().is_empty());
        let conn = f.pool.get().unwrap();
        let outbox: i64 = conn
            .query_row(
                "SELECT count(*) FROM outbox WHERE topic='message_end'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(outbox, 1);
        let terminal: i64 = conn
            .query_row(
                "SELECT count(*) FROM agent_session_events WHERE kind='control:exchange_done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(terminal, 0);
        assert!(verify_chain(&conn));
    }

    #[test]
    fn r1_exchanges_beyond_replay_cap_keep_rows_and_outbox() {
        let f = fixture(vec![scripted_text("first"), scripted_text("second")]);
        for n in 0..session_log::REPLAY_CAP {
            session_log::append(
                &f.pool.get().unwrap(),
                1,
                "user",
                "seed",
                &format!("seed:{n}"),
                1,
            )
            .unwrap();
        }
        let runtime = rt();
        let cancel = CancellationToken::new();
        runtime
            .block_on(f.driver.run_turns(1, "one", &cancel))
            .unwrap();
        runtime
            .block_on(f.driver.run_turns(1, "two", &cancel))
            .unwrap();
        let conn = f.pool.get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM agent_session_events WHERE kind IN ('user','assistant')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, session_log::REPLAY_CAP as i64 + 4);
        let outbox: i64 = conn
            .query_row(
                "SELECT count(*) FROM outbox WHERE topic = 'message_end'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(outbox, 2);
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
                (4, "user".into()),
                (5, "assistant".into()),
                (7, "tool_result".into()),
                (9, "assistant".into()),
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
        assert!(requests[1].messages[2].text().contains("steward file body"));
        let roles: Vec<&str> = requests[1]
            .messages
            .iter()
            .map(|m| match m {
                ChatMessage::User { .. } => "user",
                ChatMessage::Assistant { .. } => "assistant",
                ChatMessage::ToolResult { .. } => "tool_result",
                ChatMessage::Delegation { .. } => "delegation",
                ChatMessage::Summary { .. } => "summary",
            })
            .collect();
        assert_eq!(
            roles,
            vec!["user", "assistant", "tool_result"],
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
        let mgr = crate::pool::SqliteConnectionManager::file(tmp);
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
            LoopHooks::pass_through(),
        )
    }

    #[test]
    fn unknown_tool_fails_closed_into_a_tool_result() {
        // The model hallucinates a tool name: the registry's fail-closed
        // refusal becomes a tool_result the model can see and correct from.
        let f = fixture(vec![
            crate::agentloop::provider::scripted_tool_call("c1", "nope", "{}"),
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
        // cuts the WAIT, not the work. The durable-identity line must retain
        // ownership and refuse further model/tool calls instead of certifying
        // it as a settled error.
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
        let mgr = crate::pool::SqliteConnectionManager::file(tmp.path());
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
                crate::agentloop::provider::scripted_tool_call("c1", "slow", "{}"),
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
            LoopHooks::pass_through(),
        );
        let cancel = CancellationToken::new();
        let runtime = rt();
        let error = runtime
            .block_on(driver.run_turns(1, "run the slow tool", &cancel))
            .unwrap_err();
        assert!(error.to_string().contains("indeterminate"));
        assert!(
            runtime
                .block_on(driver.run_turns(1, "must not run", &cancel))
                .is_err()
        );
        let conn = Connection::open(tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        assert!(!events.iter().any(|e| e.kind == "tool_result"));
        assert!(!session_log::quiescent(&conn, 1).unwrap());
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

    fn compaction_history(payload: &str) -> Vec<crate::agentloop::context::ContextEvent> {
        use crate::agentloop::context::ContextEvent;
        [
            (1, "user", "covered"),
            (2, "user", "current input"),
            (3, "compaction", payload),
        ]
        .into_iter()
        .map(|(seq, kind, payload)| ContextEvent {
            row: SessionEventRow {
                seq,
                kind: kind.into(),
                payload_json: payload.into(),
                created_at: 1,
            },
            exchange_id: None,
            turn: None,
        })
        .collect()
    }

    #[test]
    fn compaction_request_retains_projection_equal_current_input() {
        rt().block_on(async {
            let f = fixture(vec![scripted_text("done")]);
            let history = compaction_history(r#"{"summary":"brief","compacted_through_seq":1,"tail_from_seq":2,"version":2,"supersedes_seq":null,"tail_seqs":[2]}"#);
            let expected = crate::agentloop::context::project(&history[1..2]).unwrap();
            let snapshot = f.driver.harness.start_run(1).unwrap();
            let request = f.driver.build_request(&snapshot, &history).expect("validated compaction must continue");
            f.driver.stream_turn(request, &CancellationToken::new(), &ExchangeBudget::new(None)).await.unwrap();
            let requests = f.provider.requests();
            assert!(matches!(requests[0].messages[0], ChatMessage::Summary { .. }));
            assert_eq!(&requests[0].messages[1..], expected.as_slice());
        });
    }

    #[test]
    fn compaction_malformed_request_refuses_boundary_before_provider() {
        for payload in [
            r#"{"compacted_through_seq":1,"tail_from_seq":2}"#,
            r#"{"summary":"brief","compacted_through_seq":1,"tail_from_seq":99}"#,
            r#"{"summary":"brief","compacted_through_seq":1,"tail_from_seq":2,"version":99}"#,
            r#"{"summary":"brief","compacted_through_seq":1,"tail_from_seq":3,"version":2,"supersedes_seq":null,"tail_seqs":[3]}"#,
            r#"{"summary":"brief","compacted_through_seq":2,"tail_from_seq":3,"version":2,"supersedes_seq":null,"tail_seqs":[3]}"#,
        ] {
            let f = fixture(vec![]);
            let snapshot = f.driver.harness.start_run(1).unwrap();
            let error = f
                .driver
                .build_request(&snapshot, &compaction_history(payload))
                .unwrap_err();
            assert!(error.to_string().contains("context_compaction_boundary"));
            assert!(f.provider.requests().is_empty());
        }
    }

    fn seed_compaction_pressure(f: &Fixture, count: usize, width: usize, prefix: &str) {
        let mut conn = f.pool.get().unwrap();
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        for i in 0..count {
            session_log::append(
                tx.tx(),
                1,
                "user",
                &format!("{prefix}-{i} {}", "x".repeat(width)),
                &format!("{prefix}:{i}"),
                1,
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    #[test]
    fn compaction_replay_cap_cannot_evict_live_input() {
        rt().block_on(async {
            let f = fixture_with(
                vec![
                    scripted_text_then_tool("read", "id", "read", "a.txt"),
                    scripted_text("done"),
                ],
                LoopConfig {
                    session_replay_cap: 2,
                    ..LoopConfig::default()
                },
            );
            let result = f
                .driver
                .run_turns(1, "live input", &CancellationToken::new())
                .await;
            assert!(
                result.is_err(),
                "a complete tool group is not a substitute for the live user input"
            );
            assert_eq!(f.provider.requests().len(), 1);
        });
    }

    #[test]
    fn compaction_boundary_cannot_leave_a_visible_hole() {
        let f = fixture(vec![]);
        let mut history = compaction_history("unused");
        history[2].row.kind = "user".into();
        history.push(crate::agentloop::context::ContextEvent { row: SessionEventRow {
            seq: 4, kind: "compaction".into(), payload_json: r#"{"summary":"brief","version":2,"compacted_through_seq":1,"tail_from_seq":3,"supersedes_seq":null,"tail_seqs":[3]}"#.into(), created_at: 1,
        }, exchange_id: None, turn: None });
        let snapshot = f.driver.harness.start_run(1).unwrap();
        assert!(f.driver.build_request(&snapshot, &history).is_err());
        assert!(f.provider.requests().is_empty());
    }

    #[test]
    fn compaction_summary_hygiene_bounds_and_cancellation() {
        rt().block_on(async {
            for (summary, reason) in [
                (
                    "x".repeat(crate::agentloop::compaction::SUMMARY_CAP + 1),
                    "context_summary_limit",
                ),
                (
                    "password=synthetic-secret".into(),
                    "context_sensitive_content",
                ),
            ] {
                let f = fixture(vec![scripted_text(&summary)]);
                seed_compaction_pressure(&f, 25, 4_000, "pressure");
                let error = f
                    .driver
                    .run_turns(1, "current", &CancellationToken::new())
                    .await
                    .unwrap_err();
                assert!(error.to_string().contains(reason));
                assert!(!error.to_string().contains(&summary));
                assert_eq!(f.provider.requests().len(), 1);
                assert!(!session_kinds(&f).iter().any(|(_, k)| k == "compaction"));
            }
            let f = fixture(vec![scripted_text("unused")]);
            let cancel = CancellationToken::new();
            cancel.cancel();
            let request = ProviderRequest {
                system_prompt: "summary".into(),
                messages: vec![],
                tools: vec![],
            };
            assert!(matches!(
                f.driver
                    .stream_summary(request, &cancel, &ExchangeBudget::new(None))
                    .await
                    .unwrap(),
                Streamed::Canceled
            ));
            assert!(f.provider.requests().is_empty());
            assert!(session_kinds(&f).is_empty());
        });
    }

    #[test]
    fn compaction_shaped_summary_is_stored_and_reused_deterministically() {
        rt().block_on(async {
            let raw = format!("contact synthetic@example.test {}", crate::fence::FENCE_END);
            let f = fixture(vec![scripted_text(&raw), scripted_text("done")]);
            seed_compaction_pressure(&f, 25, 4_000, "pressure");
            f.driver
                .run_turns(1, "current", &CancellationToken::new())
                .await
                .unwrap();
            let requests = f.provider.requests();
            assert_eq!(requests.len(), 2);
            let conn = f.pool.get().unwrap();
            let rows = session_log::scoped_compaction_replay(&conn, 1, 500, None).unwrap();
            let summary = rows.iter().find(|e| e.row.kind == "compaction").unwrap();
            assert!(!summary.row.payload_json.contains("synthetic@example.test"));
            assert!(summary.row.payload_json.contains("redacted:email"));
            assert!(!summary.row.payload_json.contains(crate::fence::FENCE_END));
            let effective = crate::agentloop::compaction::reconstruct(&rows).unwrap();
            let again = crate::agentloop::context::with_summary(
                effective.summary.as_deref(),
                &effective.tail,
            )
            .unwrap();
            assert_eq!(again[0], requests[1].messages[0]);
            assert!(requests[1].messages[0].text().contains("unverified prose"));
        });
    }

    #[test]
    fn compaction_active_turn_gate_refuses_before_summary_dispatch() {
        rt().block_on(async {
            let f = fixture(vec![scripted_text("unused")]);
            seed_compaction_pressure(&f, 25, 4_000, "pressure");
            let owner = "active-test";
            f.driver.claim(1, owner, true).await.unwrap();
            f.driver.harness.start_run(1).unwrap();
            assert!(f.driver.harness.compact().is_err());
            assert!(f.provider.requests().is_empty());
            assert!(f.driver.harness.compact().is_err());
        });
    }

    #[test]
    fn compaction_empty_head_does_not_call_summary_provider() {
        rt().block_on(async {
            let f = fixture(vec![scripted_text("done")]);
            seed_compaction_pressure(&f, 17, 4_000, "empty-head");
            let outcome = f
                .driver
                .run_turns(1, "current", &CancellationToken::new())
                .await;
            let requests = f.provider.requests();
            assert_eq!(requests.len(), 1);
            assert!(
                !requests[0].system_prompt.contains("session compactor"),
                "empty head must not trigger summary work"
            );
            assert!(outcome.is_ok());
        });
    }

    #[test]
    fn compaction_second_summary_consumes_prior_brief_and_unsummarized_tail() {
        rt().block_on(async {
            let f = fixture(vec![
                scripted_text("first brief"),
                scripted_text("first answer"),
                scripted_text("second brief"),
                scripted_text("second answer"),
            ]);
            seed_compaction_pressure(&f, 25, 4_000, "first-batch");
            let first = f
                .driver
                .run_turns_keyed(1, "first", "first current", &CancellationToken::new())
                .await;
            assert!(
                first.is_ok(),
                "first continuation must reach the ordinary request"
            );
            let first_requests = f.provider.requests();
            let covered = first_requests[0].messages.clone();
            seed_compaction_pressure(&f, 25, 4_000, "second-batch");
            f.driver
                .run_turns_keyed(1, "second", "second current", &CancellationToken::new())
                .await
                .unwrap();
            let requests = f.provider.requests();
            assert_eq!(requests.len(), 4);
            assert_eq!(requests[2].messages[0], requests[1].messages[0]);
            assert!(matches!(
                requests[2].messages[0],
                ChatMessage::Summary { .. }
            ));
            for message in covered {
                assert!(!requests[2].messages.contains(&message));
            }
            let mut reconstructed = requests[2].messages[1..].to_vec();
            reconstructed.extend_from_slice(&requests[3].messages[1..]);
            for message in &requests[1].messages[1..] {
                assert!(
                    reconstructed.contains(message),
                    "retained history must not disappear at the next compaction"
                );
            }
        });
    }

    #[test]
    fn loop_compacts_under_pressure_via_its_own_provider() {
        // The former compaction-refusal pin now proves validated continuation
        // and privacy parity at both actual provider requests.
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = crate::pool::SqliteConnectionManager::file(tmp.path());
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
                let payload = format!(
                    "synthetic@example.test {} {}",
                    crate::fence::FENCE_END,
                    "x".repeat(4_000)
                );
                session_log::append(
                    wtx.tx(),
                    1,
                    "user",
                    &payload,
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
            LoopHooks::pass_through(),
        );
        let cancel = CancellationToken::new();
        rt().block_on(driver.run_turns(1, "continue", &cancel))
            .unwrap();
        let requests = provider.requests();
        assert_eq!(
            requests.len(),
            2,
            "summary plus validated ordinary continuation"
        );
        // The actual summary call carries shaped data, not raw private content.
        assert!(requests[0].system_prompt.contains("session compactor"));
        assert!(requests[0].tools.is_empty());
        assert!(!requests[0].messages.is_empty());
        for message in &requests[0].messages {
            assert!(!message.text().contains("synthetic@example.test"));
            assert!(message.text().contains("redacted:email"));
            assert!(!message.text().contains(crate::fence::FENCE_END));
        }
        // The log was never rewritten: every seeded row still replays, the
        // compaction event appended after them.
        let conn = Connection::open(tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        assert_eq!(
            events.len(),
            27,
            "24 seeded + user + compaction + ordinary assistant"
        );
        assert_eq!(events[24].kind, "user");
        assert_eq!(events[25].kind, "compaction");
        assert!(events[0].payload_json.contains("synthetic@example.test"));
        let payload: serde_json::Value = serde_json::from_str(&events[25].payload_json).unwrap();
        assert_eq!(payload["summary"], serde_json::json!("the compact brief"));
        // The verbatim-tail budget (20k tokens) keeps the newest ~20 events;
        // the head ends where the tail begins — assert the recorded boundary
        // against the replay, not a magic number.
        let through = payload["compacted_through_seq"].as_i64().unwrap();
        let from = payload["tail_from_seq"].as_i64().unwrap();
        assert!(from > through);
        let scoped = session_log::scoped_replay(&conn, 1, 500, None).unwrap();
        let tail: Vec<_> = scoped
            .iter()
            .filter(|e| e.row.seq >= from && e.row.seq < events[25].seq)
            .cloned()
            .collect();
        let expected = crate::agentloop::context::project(&tail).unwrap();
        assert_eq!(&requests[1].messages[1..], expected.as_slice());
        assert_eq!(
            requests[1].messages.last().unwrap().text(),
            "Source: user input\ncontinue"
        );
        assert!(matches!(
            requests[1].messages[0],
            ChatMessage::Summary { .. }
        ));
        assert_eq!(payload["version"], 2);
        assert!(verify_chain(&conn), "compaction writes stay chain-verified");
    }
}
