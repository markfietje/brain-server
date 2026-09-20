//! Scoped subagent delegation: a child harness under the parent's ceiling.
//!
//! A subagent is a child [`AgentHarness`] over the SAME host (every child
//! act — RunStart, session events, RunEnd — lands in the same hash-chained
//! audit stream and the same run's session log), with a NARROWED
//! [`ExecutionEnv`] (capability subtraction, never addition: the child's
//! powers are the intersection of what the spec asks and what the parent
//! already had) and a SUBSET of the parent's tools (the registry's
//! presentation alignment means what the child is shown is exactly what it
//! can run). Budget reservations stop new calls at the boundary; a crossing
//! provider call can overshoot its reservation and its actual usage still counts.
//!
//! The child's session narrative is written with a `child:<name>:` kind
//! prefix so replay distinguishes it from the parent's; the delegation
//! itself appends one parent-visible `subagent_result` event carrying the
//! child's final answer. What this deliberately does NOT do: nested fibers
//! (the plugin kernel's declared ceiling), capability addition, or a
//! parallel identity — the child rides the parent's principal and audit
//! chain, full stop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use brain_engine_sdk::env::{ExecutionEnv, ToolDef};
use brain_engine_sdk::harness::AgentHarness;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::Pool;
use crate::agentloop::hooks::LoopHooks;
use crate::agentloop::provider::{LlmProvider, Usage};
use crate::agentloop::run_loop::{ExchangeReceipt, LoopConfig, LoopDriver, LoopError, RunOutcome};
use crate::workflow::host::SqliteWorkflowHost;

/// Typed accounting refusal — measured usage is the only success; these are
/// the distinct failure classes, never disguised as canceled or
/// budget-exhausted outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccountingRefusal {
    /// Ledger state cannot safely authorize dispatch (poison).
    Unavailable,
    /// The reservation authority was released or dropped.
    Revoked,
    /// Structurally invalid authority (nesting depth, generation overflow).
    Invalid,
    /// A started call ended without MessageEnd: spend is unknown and the
    /// shared authority refuses further dispatch.
    Incomplete,
    /// Observed usage plus outstanding reservations reached the ceiling.
    Exhausted,
}

impl std::fmt::Display for AccountingRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "accounting_unavailable",
            Self::Revoked => "accounting_revoked",
            Self::Invalid => "accounting_invalid",
            Self::Incomplete => "accounting_incomplete",
            Self::Exhausted => "accounting_exhausted",
        })
    }
}

/// Shared exchange accounting, including outstanding child reservations.
/// Usage is observed at MessageEnd, not reconstructed from retry receipts.
///
/// Bounded fixed-depth authority: root (depth 0) → exchange view (depth 1)
/// → at most ONE child reservation level (depth 2); a deeper nesting
/// REFUSES structurally (`reserve_child` on a non-root). Every view and
/// child shares the ROOT's single dispatch permit (`Arc<Semaphore>` — one
/// provider call admitted at a time per authority). Lock order is child →
/// parent; each lock is short: no await, no callback, no recursion under
/// it. Slots are per-exchange allocations (never recycled), so no stale
/// handle can address recycled state; the reservation generation is checked
/// and refuses overflow.
#[derive(Clone, Debug)]
pub(crate) struct ExchangeBudget {
    limit: Option<u64>,
    // Lock order is child -> parent; no await or external callback under a lock.
    state: Arc<Mutex<BudgetState>>,
    parent: Option<Box<ExchangeBudget>>,
    /// Dispatch authority shared with the owning reservation; a root
    /// budget is its own authority.
    authority: Option<Arc<Authority>>,
    /// The ROOT-owned single dispatch permit, shared by every view/child of
    /// this authority tree.
    dispatch: Arc<Semaphore>,
    /// Structural nesting bound: 0 = root authority, 1 = exchange view or
    /// child reservation, 2 = a child's own exchange view (leaf).
    depth: u8,
}

#[derive(Debug, Default)]
struct BudgetState {
    usage: Usage,
    reserved: u64,
    parent_reserved: u64,
    /// Started call ended without MessageEnd — spend unknown.
    incomplete: bool,
    /// Admitted-but-unaccounted dispatch attempts (the admission record).
    in_flight: u64,
    /// Reservation slot generation: checked increment per reservation
    /// minted from this node; overflow refuses instead of wrapping.
    generation: u64,
}

/// Dispatch authority shared between a reservation and its cloned views.
#[derive(Debug)]
struct Authority(AtomicBool);

impl Authority {
    fn live_new() -> Self {
        Self(AtomicBool::new(true))
    }
    fn live(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl ExchangeBudget {
    pub(crate) fn new(limit: Option<u64>) -> Self {
        Self {
            limit,
            state: Arc::new(Mutex::new(BudgetState::default())),
            parent: None,
            authority: None,
            dispatch: Arc::new(Semaphore::new(1)),
            depth: 0,
        }
    }

    /// Fresh exchange view over this authority: local counters start empty
    /// (a reused driver's next exchange inherits no accidental usage) and the
    /// requested policy ceiling is enforced — a supplied authority narrows,
    /// never widens. Usage propagates to the authority.
    pub(crate) fn fresh_exchange(&self, requested: Option<u64>) -> ExchangeGuard {
        let limit = match (requested, self.limit) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (requested, authority) => requested.or(authority),
        };
        ExchangeGuard {
            budget: ExchangeBudget {
                limit,
                state: Arc::new(Mutex::new(BudgetState::default())),
                parent: Some(Box::new(self.clone())),
                authority: self.authority.clone(),
                dispatch: Arc::clone(&self.dispatch),
                depth: self.depth.saturating_add(1),
            },
            _reservation: None,
        }
    }

    /// The single root-owned dispatch permit, shared by every view/child of
    /// this authority tree.
    pub(crate) fn dispatch_permit(&self) -> Arc<Semaphore> {
        Arc::clone(&self.dispatch)
    }

    /// Step 2 of the admission algorithm: wait for the root-owned permit,
    /// cancellation-aware. `Ok(None)` = canceled while waiting (no provider
    /// work); `Err` = the authority closed the permit (typed unavailability).
    pub(crate) async fn acquire_dispatch(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, AccountingRefusal> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Ok(None),
            acquired = self.dispatch.clone().acquire_owned() => match acquired {
                Ok(permit) => Ok(Some(permit)),
                Err(_) => Err(AccountingRefusal::Unavailable),
            },
        }
    }

    /// Step 3: ONE short ledger admission — under the state mutexes
    /// (child → parent, bounded depth, no await) validate liveness, poison,
    /// unknown spend and ceilings atomically and record the attempt.
    pub(crate) fn admit(&self) -> Result<Admission, AccountingRefusal> {
        // Ancestors first (bounded chain, lock order child → parent).
        let mut cur = self.parent.as_deref();
        while let Some(node) = cur {
            if node.state.is_poisoned() {
                return Err(AccountingRefusal::Unavailable);
            }
            let incomplete = node.state.lock().map(|s| s.incomplete).unwrap_or(false);
            if incomplete {
                return Err(AccountingRefusal::Incomplete);
            }
            if node.revoked() {
                return Err(AccountingRefusal::Revoked);
            }
            cur = node.parent.as_deref();
        }
        if self.revoked() {
            return Err(AccountingRefusal::Revoked);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| AccountingRefusal::Unavailable)?;
        if state.incomplete {
            return Err(AccountingRefusal::Incomplete);
        }
        if self
            .limit
            .is_some_and(|limit| state.usage.total().saturating_add(state.reserved) >= limit)
        {
            return Err(AccountingRefusal::Exhausted);
        }
        state.in_flight = state.in_flight.saturating_add(1);
        Ok(Admission {
            budget: self.clone(),
        })
    }

    /// A started call ended WITHOUT MessageEnd: its spend is unknown. Mark
    /// the shared authority accounting-incomplete and close the dispatch
    /// permit so waiters wake into a typed refusal — never an invented
    /// zero or MAX.
    pub(crate) fn mark_incomplete(&self) {
        let close = |budget: &ExchangeBudget| {
            if let Ok(mut state) = budget.state.lock() {
                state.incomplete = true;
            }
            budget.dispatch.close();
        };
        close(self);
        let mut cur = self.parent.as_deref();
        while let Some(node) = cur {
            close(node);
            cur = node.parent.as_deref();
        }
    }

    pub(crate) fn usage(&self) -> Usage {
        // Poison is not permission to fabricate: report what was actually
        // recorded. A checked API must still refuse to certify this state.
        self.state
            .lock()
            .map(|state| state.usage)
            .unwrap_or_else(|error| error.into_inner().usage)
    }

    pub(crate) fn exhausted(&self) -> bool {
        if self.revoked() {
            return true;
        }
        let own = match self.state.lock() {
            Ok(state) => {
                state.incomplete
                    || self.limit.is_some_and(|limit| {
                        state.usage.total().saturating_add(state.reserved) >= limit
                    })
            }
            Err(_) => true,
        };
        own || self.parent_actual_exhausted()
    }

    /// Parent ACTUAL exhaustion stops children; the parent's own outstanding
    /// reservation does not — a fully allocated child keeps its allocation.
    fn parent_actual_exhausted(&self) -> bool {
        let mut cur = self.parent.as_deref();
        while let Some(node) = cur {
            let incomplete = node.state.lock().map(|s| s.incomplete).unwrap_or(true);
            if node.state.is_poisoned()
                || incomplete
                || node.revoked()
                || node
                    .limit
                    .is_some_and(|limit| node.usage().total() >= limit)
            {
                return true;
            }
            cur = node.parent.as_deref();
        }
        false
    }

    fn revoked(&self) -> bool {
        self.authority
            .as_ref()
            .is_some_and(|authority| !authority.live())
    }

    pub(crate) fn record(&self, usage: Usage) {
        self.record_reserved(usage, 0);
    }

    fn record_reserved(&self, usage: Usage, consumed: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.reserved = state.reserved.saturating_sub(consumed);
        state.usage.input_tokens = state.usage.input_tokens.saturating_add(usage.input_tokens);
        state.usage.output_tokens = state
            .usage
            .output_tokens
            .saturating_add(usage.output_tokens);
        if let Some(parent) = &self.parent {
            let consumed = usage
                .input_tokens
                .saturating_add(usage.output_tokens)
                .min(state.parent_reserved);
            state.parent_reserved = state.parent_reserved.saturating_sub(consumed);
            parent.record_reserved(usage, consumed);
        }
    }

    /// Reserve capacity for ONE delegated child. Structural bound: only a
    /// ROOT authority (depth 0) reserves — deeper nesting refuses. The
    /// reservation slot is generation-tagged with a checked increment;
    /// overflow refuses instead of wrapping.
    pub(crate) fn reserve_child(
        &self,
        requested: u64,
    ) -> Result<ChildReservation, AccountingRefusal> {
        if self.depth != 0 {
            return Err(AccountingRefusal::Invalid);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| AccountingRefusal::Unavailable)?;
        let generation = state
            .generation
            .checked_add(1)
            .ok_or(AccountingRefusal::Invalid)?;
        state.generation = generation;
        if state.incomplete {
            return Err(AccountingRefusal::Incomplete);
        }
        let ceiling = if self.revoked() {
            return Err(AccountingRefusal::Revoked);
        } else {
            self.limit.map_or(requested, |limit| {
                requested.min(
                    limit
                        .saturating_sub(
                            state
                                .usage
                                .input_tokens
                                .saturating_add(state.usage.output_tokens),
                        )
                        .saturating_sub(state.reserved),
                )
            })
        };
        // An uncapped parent observes usage but needs no capacity reservation.
        let reserved = if self.limit.is_some() { ceiling } else { 0 };
        state.reserved = state.reserved.saturating_add(reserved);
        drop(state);
        let authority = Arc::new(Authority::live_new());
        Ok(ChildReservation {
            authority: Arc::clone(&authority),
            budget: ExchangeBudget {
                limit: Some(ceiling),
                state: Arc::new(Mutex::new(BudgetState {
                    parent_reserved: reserved,
                    ..BudgetState::default()
                })),
                parent: Some(Box::new(self.clone())),
                authority: Some(authority),
                dispatch: Arc::clone(&self.dispatch),
                depth: self.depth.saturating_add(1),
            },
        })
    }
}

/// A recorded dispatch admission. RAII: dropping it releases the in-flight
/// accounting record (the SEMAPHORE permit is held separately by the
/// dispatch helper through MessageEnd accounting).
pub(crate) struct Admission {
    budget: ExchangeBudget,
}

impl Drop for Admission {
    fn drop(&mut self) {
        if let Ok(mut state) = self.budget.state.lock() {
            state.in_flight = state.in_flight.saturating_sub(1);
        }
    }
}

/// Exchange-scoped accounting authority. Holding it keeps a child reservation
/// (if any) alive; the inner view can be cloned for accounting but cannot
/// keep dispatch authority alive past the guard's release.
pub(crate) struct ExchangeGuard {
    budget: ExchangeBudget,
    _reservation: Option<ChildReservation>,
}

impl ExchangeGuard {
    /// A root guard with no parent authority: the driver's own ledger.
    pub(crate) fn root(requested: Option<u64>) -> Self {
        Self {
            budget: ExchangeBudget::new(requested),
            _reservation: None,
        }
    }

    /// Adopt a child reservation: the guard's lifetime IS the reservation's.
    pub(crate) fn reserved(reservation: ChildReservation) -> Self {
        let budget = reservation.budget();
        Self {
            budget,
            _reservation: Some(reservation),
        }
    }

    pub(crate) fn budget(&self) -> &ExchangeBudget {
        &self.budget
    }
}

/// Only this non-cloneable guard releases capacity. Cloned accounting views
/// share its remaining reservation; actual usage is never refunded on release.
pub(crate) struct ChildReservation {
    authority: Arc<Authority>,
    budget: ExchangeBudget,
}

impl ChildReservation {
    /// Cloneable accounting view; clones share the reservation's remaining
    /// capacity but lose dispatch authority the moment it is released.
    pub(crate) fn budget(&self) -> ExchangeBudget {
        self.budget.clone()
    }

    /// Single release: revokes dispatch authority and returns unused
    /// capacity once. Observed usage is never refunded.
    pub(crate) fn release(&mut self) {
        if !self.authority.0.swap(false, Ordering::SeqCst) {
            return;
        }
        let remaining = std::mem::take(
            &mut self
                .budget
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .parent_reserved,
        );
        if let Some(parent) = &self.budget.parent {
            let mut parent_state = parent
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            parent_state.reserved = parent_state.reserved.saturating_sub(remaining);
        }
    }
}

impl Drop for ChildReservation {
    fn drop(&mut self) {
        self.release();
    }
}

/// What a subagent spec ASKS to keep. Asking never grants: the child env is
/// the intersection of this with the parent's actual powers.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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
/// child's explicit ask narrows). A disjoint ask — non-empty ask, non-empty
/// parent list, empty intersection — grants NO command, and an empty list is
/// the SDK gate's unrestricted posture, so the process grant dies with it:
/// denied execution, never an unrestricted empty vector.
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
    let disjoint_ask =
        !caps.commands.is_empty() && !parent.allowed_commands.is_empty() && commands.is_empty();
    ExecutionEnv {
        fs: Arc::clone(&parent.fs),
        read_only: parent.read_only || !caps.write,
        allow_process: parent.allow_process && caps.process && !disjoint_ask,
        root: parent.root.clone(),
        allowed_commands: commands,
    }
}

/// One delegation request.
#[derive(Debug, Clone, serde::Serialize)]
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
    /// Cumulative pre-dispatch ceiling; one provider call may overshoot.
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
/// tools, bounded calls. Appends a parent-visible `subagent_result` event.
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
    let owner = uuid::Uuid::new_v4().to_string();
    change_claim(pool, run_id, &owner, true).await?;
    let result = delegate_owned(
        pool,
        host,
        parent_env,
        parent_tools,
        provider,
        run_id,
        spec,
        &uuid::Uuid::new_v4().to_string(),
        &owner,
        cancel,
    )
    .await?;
    change_claim(pool, run_id, &owner, false).await?;
    Ok(result)
}

/// The parent holds its case claim across delegation and projection. The
/// invocation key is stable for retry, not the human-readable child name.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn delegate_owned(
    pool: &Pool,
    host: &Arc<SqliteWorkflowHost>,
    parent_env: &ExecutionEnv,
    parent_tools: &[ToolDef],
    provider: Arc<dyn LlmProvider>,
    run_id: i64,
    spec: &SubagentSpec,
    invocation_key: &str,
    owner: &str,
    cancel: &CancellationToken,
) -> Result<SubagentOutcome, LoopError> {
    let parent_budget = ExchangeBudget::new(None);
    delegate_owned_budgeted(
        pool,
        host,
        parent_env,
        parent_tools,
        provider,
        run_id,
        spec,
        invocation_key,
        owner,
        cancel,
        &parent_budget,
    )
    .await
}

/// Explicit parent-accounting seam. The dynamic reservation is runtime state,
/// never a retry-policy input: exact receipts dispatch nothing and debit nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn delegate_owned_budgeted(
    pool: &Pool,
    host: &Arc<SqliteWorkflowHost>,
    parent_env: &ExecutionEnv,
    parent_tools: &[ToolDef],
    provider: Arc<dyn LlmProvider>,
    run_id: i64,
    spec: &SubagentSpec,
    invocation_key: &str,
    owner: &str,
    cancel: &CancellationToken,
    parent_budget: &ExchangeBudget,
) -> Result<SubagentOutcome, LoopError> {
    // Fresh-only reservation: the guard is created lazily AFTER the owned
    // admission distinguishes fresh/retry, so an exact receipt retry neither
    // dispatches, debits, nor transiently reserves parent capacity.
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
    let prefix = format!("child:{}:", spec.name);
    let mut child = LoopDriver::new(
        pool.clone(),
        Arc::clone(host),
        child_harness,
        provider,
        tools,
        narrowed_env(parent_env, &spec.caps),
        LoopConfig {
            max_turns: spec.max_turns,
            token_budget: Some(spec.token_budget),
            ..LoopConfig::default()
        },
        &prefix,
        LoopHooks::pass_through(),
    );
    child.set_retry_spec(
        serde_json::to_string(spec)
            .map_err(|_| LoopError::Persist("child_spec_encoding".into()))?,
    );
    child.bind_policy("child", &spec.system_prompt, Vec::new(), Vec::new());
    let projection_spec = spec.clone();
    let budget_spec = spec.clone();
    let budget_parent = parent_budget.clone();
    let receipt = child
        .run_turns_owned_projected_budgeted(
            run_id,
            &format!("child:{invocation_key}"),
            &spec.task,
            owner,
            cancel,
            move |receipt| {
                let (_, mut follow_up) = child_result(&projection_spec, receipt);
                follow_up["exchange_id"] = serde_json::json!(receipt.exchange_id);
                Ok(vec![(
                    "subagent_result".into(),
                    follow_up.to_string(),
                    format!(
                        "run{run_id}:exchange:{}:subagent_result",
                        receipt.exchange_id
                    ),
                )])
            },
            // Runs only on the FRESH path, after admission: the reservation
            // exists exactly for the exchange that will dispatch work. A
            // refusal (terminalized authority) fails the invocation before
            // any dispatch — never an unaccounted exchange.
            move || {
                budget_parent
                    .reserve_child(budget_spec.token_budget)
                    .map(ExchangeGuard::reserved)
                    .map_err(|refusal| {
                        LoopError::Persist(format!("budget refused admission: {refusal}"))
                    })
            },
        )
        .await?;
    Ok(child_result(spec, &receipt).0)
}

fn child_result(
    spec: &SubagentSpec,
    receipt: &ExchangeReceipt,
) -> (SubagentOutcome, serde_json::Value) {
    match &receipt.outcome {
        RunOutcome::Completed { turns, usage } => {
            let summary = receipt.final_text.clone();
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
    }
}

async fn change_claim(
    pool: &Pool,
    run_id: i64,
    owner: &str,
    acquire: bool,
) -> Result<(), LoopError> {
    let pool = pool.clone();
    let owner = owner.to_string();
    tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(|e| LoopError::Persist(e.to_string()))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| LoopError::Persist(e.to_string()))?;
        let result = if acquire {
            crate::workflow::session_log::acquire(tx.tx(), run_id, &owner)
        } else {
            crate::workflow::session_log::release(tx.tx(), run_id, &owner)
        };
        result.map_err(|e| LoopError::Persist(e.to_string()))?;
        tx.commit().map_err(|e| LoopError::Persist(e.to_string()))?;
        Ok(())
    })
    .await
    .map_err(|e| LoopError::Persist(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentloop::provider::{ChatMessage, DelegationOutcome, ToolResultStatus};
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
    fn r4_reserved_child_stops_at_parent_actual_ceiling() {
        let parent = ExchangeBudget::new(Some(100));
        let child = parent.reserve_child(100).unwrap();
        assert!(parent.exhausted());
        assert!(
            !child.budget.exhausted(),
            "the child's own reservation must remain usable"
        );
        parent.record(Usage {
            input_tokens: 60,
            output_tokens: 40,
        });
        assert_eq!(parent.usage().total(), 100);
        assert_eq!(child.budget.usage(), Usage::default());
        assert!(
            child.budget.exhausted(),
            "parent actual exhaustion must stop an otherwise unused child"
        );
    }

    #[test]
    fn r4_released_child_handle_cannot_authorize_dispatch() {
        let parent = ExchangeBudget::new(Some(100));
        let mut child = parent.reserve_child(80).unwrap();
        let surviving = child.budget.clone();
        surviving.record(Usage {
            input_tokens: 7,
            output_tokens: 3,
        });
        child.release();
        child.release();
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
        assert_eq!(parent.usage().total(), 10);
        assert_eq!(surviving.usage().total(), 10);
        assert!(
            surviving.exhausted(),
            "released authority must refuse through surviving accounting handles"
        );
    }

    #[test]
    fn r4_dropped_child_handle_cannot_authorize_dispatch() {
        let parent = ExchangeBudget::new(None);
        let child = parent.reserve_child(80).unwrap();
        let surviving = child.budget.clone();
        assert!(!surviving.exhausted());
        drop(child);
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
        assert_eq!(parent.usage(), Usage::default());
        assert!(
            surviving.exhausted(),
            "uncapped parents still require a live child reservation authority"
        );
    }

    #[test]
    fn r4_poison_refuses_without_fabricating_observed_usage() {
        let budget = ExchangeBudget::new(Some(100));
        let observed = Usage {
            input_tokens: 7,
            output_tokens: 3,
        };
        budget.record(observed);
        let state = Arc::clone(&budget.state);
        assert!(
            std::thread::spawn(move || {
                let _guard = state.lock().unwrap();
                panic!("synthetic_budget_poison");
            })
            .join()
            .is_err()
        );
        assert!(budget.state.is_poisoned());
        assert!(budget.exhausted());
        // The current infallible API can only expose the truthful-counter half
        // of the contract; a checked API must instead return unavailable.
        assert_eq!(
            budget.state.lock().unwrap_err().into_inner().usage,
            observed
        );
        assert_eq!(budget.usage(), observed, "poison is not measured spend");
    }

    #[test]
    fn r4_supplied_uncapped_accounting_cannot_widen_zero_policy() {
        let d = deleg();
        let parent = ExchangeBudget::new(None);
        let provider = LoopbackProvider::new("loopback", vec![scripted_text("must not run")]);
        let driver = LoopDriver::new(
            d.pool.clone(),
            Arc::clone(&d.host),
            Arc::new(AgentHarness::new(
                Arc::clone(&d.host),
                "parent",
                "budget fixture",
            )),
            provider.clone(),
            Vec::new(),
            d.env.clone(),
            LoopConfig {
                token_budget: Some(0),
                budget_accounting: Some(parent.clone()),
                ..LoopConfig::default()
            },
            "",
            LoopHooks::pass_through(),
        );
        let outcome = rt()
            .block_on(driver.run_turns(1, "synthetic task", &CancellationToken::new()))
            .unwrap();
        assert!(
            provider.requests().is_empty(),
            "supplied uncapped accounting must not replace the requested zero ceiling"
        );
        assert_eq!(parent.usage(), Usage::default());
        assert!(matches!(
            outcome,
            RunOutcome::BudgetExceeded { turns: 0, usage } if usage == Usage::default()
        ));
    }

    #[test]
    fn r4_exact_child_retry_never_transiently_reserves_parent_capacity() {
        use std::future::{Future, poll_fn};
        use std::task::Poll;

        let d = deleg();
        let runtime = rt();
        runtime.block_on(async {
            let cancel = CancellationToken::new();
            let provider = LoopbackProvider::new("loopback", vec![scripted_text("first")]);
            let parent = ExchangeBudget::new(Some(100));
            let spec = spec("scout", &[], 80);
            change_claim(&d.pool, 1, "parent", true).await.unwrap();
            let first = delegate_owned_budgeted(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                provider.clone(),
                1,
                &spec,
                "transient",
                "parent",
                &cancel,
                &parent,
            )
            .await
            .unwrap();
            parent.record(Usage {
                input_tokens: 6,
                output_tokens: 0,
            });
            let before = parent.usage();
            assert_eq!(before.total(), 20);
            assert_eq!(parent.state.lock().unwrap().reserved, 0);

            // Exhaust this fixture's four-connection pool before first poll.
            // The owned-admission worker cannot complete until we release it.
            let held: Vec<_> = (0..4).map(|_| d.pool.get().unwrap()).collect();
            let mut retry = Box::pin(delegate_owned_budgeted(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                provider.clone(),
                1,
                &spec,
                "transient",
                "parent",
                &cancel,
                &parent,
            ));
            let first_poll = poll_fn(|cx| Poll::Ready(retry.as_mut().poll(cx))).await;
            let during = parent.state.lock().unwrap().reserved;
            drop(held);
            // Finish even on the red path; never strand a blocked pool worker
            // behind an assertion panic or manufacture an incomplete retry.
            let result = match first_poll {
                Poll::Pending => retry.await,
                Poll::Ready(result) => result,
            };
            assert_eq!(result.unwrap(), first);
            assert_eq!(provider.requests().len(), 1);
            assert_eq!(parent.usage(), before);
            assert_eq!(parent.state.lock().unwrap().reserved, 0);
            assert_eq!(
                during, 0,
                "receipt-only retry transiently reserved capacity"
            );
        });
    }

    #[test]
    fn r4_child_reservation_accounts_actual_and_releases_once() {
        let parent = ExchangeBudget::new(Some(100));
        parent.record(Usage {
            input_tokens: 10,
            output_tokens: 0,
        });
        let mut child = parent.reserve_child(80).unwrap();
        assert_eq!(child.budget.limit, Some(80));
        let sibling = parent.reserve_child(80).unwrap();
        assert_eq!(sibling.budget.limit, Some(10));
        assert!(parent.exhausted());
        child.budget.record(Usage {
            input_tokens: 12,
            output_tokens: 8,
        });
        assert_eq!(parent.usage().total(), 30);
        child.release();
        child.release();
        drop(child);
        assert_eq!(parent.state.lock().unwrap().reserved, 10);
        drop(sibling);
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
        assert_eq!(parent.usage().total(), 30);
        assert!(!parent.exhausted());
    }

    #[test]
    fn r4_child_drop_releases_without_refunding_received_usage() {
        let parent = ExchangeBudget::new(Some(20));
        {
            let child = parent.reserve_child(15).unwrap();
            child.budget.record(Usage {
                input_tokens: 12,
                output_tokens: 18,
            });
            assert!(child.budget.exhausted());
            assert_eq!(parent.usage().total(), 30);
        }
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
        assert!(parent.exhausted());
        assert_eq!(parent.reserve_child(10).unwrap().budget.limit, Some(0));
    }

    #[test]
    fn r4_usage_saturates_and_uncapped_parent_observes_children() {
        let parent = ExchangeBudget::new(None);
        let child = parent.reserve_child(100).unwrap();
        child.budget.record(Usage {
            input_tokens: u64::MAX,
            output_tokens: u64::MAX,
        });
        child.budget.record(Usage {
            input_tokens: 1,
            output_tokens: 1,
        });
        assert_eq!(
            parent.usage(),
            Usage {
                input_tokens: u64::MAX,
                output_tokens: u64::MAX
            }
        );
        assert!(!parent.exhausted());
        drop(child);
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
    }

    #[test]
    fn r4_budgeted_child_retry_ignores_changed_parent_capacity() {
        let d = deleg();
        let runtime = rt();
        let cancel = CancellationToken::new();
        let provider = LoopbackProvider::new("loopback", vec![scripted_text("first")]);
        let parent = ExchangeBudget::new(Some(100));
        let spec = spec("scout", &[], 1000);
        runtime
            .block_on(change_claim(&d.pool, 1, "parent", true))
            .unwrap();
        let first = runtime
            .block_on(delegate_owned_budgeted(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                provider.clone(),
                1,
                &spec,
                "one",
                "parent",
                &cancel,
                &parent,
            ))
            .unwrap();
        assert_eq!(parent.usage().total(), 14);
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
        parent.record(Usage {
            input_tokens: 86,
            output_tokens: 0,
        });
        assert!(parent.exhausted());
        let retry = runtime
            .block_on(delegate_owned_budgeted(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                provider.clone(),
                1,
                &spec,
                "one",
                "parent",
                &cancel,
                &parent,
            ))
            .unwrap();
        assert_eq!(retry, first);
        assert_eq!(parent.usage().total(), 100);
        assert_eq!(provider.requests().len(), 1);
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
    }

    #[test]
    fn r4_child_received_usage_survives_validation_error() {
        let d = deleg();
        let runtime = rt();
        let parent = ExchangeBudget::new(Some(1000));
        let provider = LoopbackProvider::new(
            "loopback",
            vec![scripted_text_then_tool("invalid call", "", "read", "{}")],
        );
        runtime
            .block_on(change_claim(&d.pool, 1, "parent", true))
            .unwrap();
        let result = runtime.block_on(delegate_owned_budgeted(
            &d.pool,
            &d.host,
            &d.env,
            &[],
            provider,
            1,
            &spec("scout", &[], 1000),
            "invalid",
            "parent",
            &CancellationToken::new(),
            &parent,
        ));
        assert!(result.is_err());
        assert!(parent.usage().input_tokens > 0);
        assert!(parent.usage().output_tokens > 0);
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
    }

    #[test]
    fn r4_child_parent_ceiling_limits_new_calls() {
        let d = deleg();
        let runtime = rt();
        let parent = ExchangeBudget::new(Some(10));
        let provider = LoopbackProvider::new("loopback", vec![scripted_text("crossing")]);
        runtime
            .block_on(change_claim(&d.pool, 1, "parent", true))
            .unwrap();
        let result = runtime
            .block_on(delegate_owned_budgeted(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                provider.clone(),
                1,
                &spec("scout", &[], 1000),
                "limited",
                "parent",
                &CancellationToken::new(),
                &parent,
            ))
            .unwrap();
        assert!(matches!(result, SubagentOutcome::BudgetExceeded { .. }));
        assert_eq!(parent.usage().total(), 14);
        let stopped = runtime
            .block_on(delegate_owned_budgeted(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                provider.clone(),
                1,
                &spec("scout", &[], 1000),
                "stopped",
                "parent",
                &CancellationToken::new(),
                &parent,
            ))
            .unwrap();
        assert!(matches!(stopped, SubagentOutcome::BudgetExceeded { .. }));
        assert_eq!(parent.usage().total(), 14);
        assert_eq!(provider.requests().len(), 1);
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
    }

    #[test]
    fn r4_child_cancel_after_received_turn_keeps_usage() {
        use crate::agentloop::provider::{ProviderError, ProviderRequest, StreamEvent};
        struct CancelNext {
            inner: Arc<LoopbackProvider>,
            calls: std::sync::atomic::AtomicUsize,
            cancel: CancellationToken,
        }
        impl LlmProvider for CancelNext {
            fn name(&self) -> &str {
                self.inner.name()
            }
            fn stream(
                &self,
                request: ProviderRequest,
            ) -> Result<
                tokio::sync::mpsc::Receiver<Result<StreamEvent, ProviderError>>,
                ProviderError,
            > {
                if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
                    self.cancel.cancel();
                }
                self.inner.stream(request)
            }
        }
        let d = deleg();
        let runtime = rt();
        let cancel = CancellationToken::new();
        let parent = ExchangeBudget::new(Some(1000));
        let first_script = scripted_text_then_tool("read", "one", "read", r#"{"path":"a.txt"}"#);
        let expected = match first_script.last().unwrap() {
            StreamEvent::MessageEnd { usage, .. } => *usage,
            _ => panic!("fixture_missing_usage"),
        };
        let provider = Arc::new(CancelNext {
            inner: LoopbackProvider::new("loopback", vec![first_script, scripted_text("canceled")]),
            calls: std::sync::atomic::AtomicUsize::new(0),
            cancel: cancel.clone(),
        });
        runtime
            .block_on(change_claim(&d.pool, 1, "parent", true))
            .unwrap();
        let result = runtime
            .block_on(delegate_owned_budgeted(
                &d.pool,
                &d.host,
                &d.env,
                &[brain_engine_sdk::env::create_read_tool()],
                provider,
                1,
                &spec("scout", &["read"], 1000),
                "cancel",
                "parent",
                &cancel,
                &parent,
            ))
            .unwrap();
        assert_eq!(result, SubagentOutcome::Canceled);
        assert_eq!(parent.usage(), expected);
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
    }

    /// B5: a started call that ends WITHOUT MessageEnd has unknown spend —
    /// the shared authority is marked accounting-incomplete, refuses further
    /// dispatch, and never invents a zero or MAX total.
    #[test]
    fn b5_missing_message_end_marks_authority_incomplete_and_refuses() {
        use crate::agentloop::provider::{ProviderError, ProviderRequest, StreamEvent};
        struct NoEnd;
        impl LlmProvider for NoEnd {
            fn name(&self) -> &str {
                "no-end"
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
                    // Deliberately NEVER sends a MessageEnd: drop the sender
                    // so the channel closes without one (unknown-spend path).
                    std::mem::drop(tx);
                });
                Ok(rx)
            }
        }
        let d = deleg();
        let parent = ExchangeBudget::new(Some(1000));
        rt().block_on(change_claim(&d.pool, 1, "parent", true))
            .unwrap();
        let outcome = rt().block_on(delegate_owned_budgeted(
            &d.pool,
            &d.host,
            &d.env,
            &[],
            Arc::new(NoEnd),
            1,
            &spec("scout", &[], 900),
            "no-end",
            "parent",
            &CancellationToken::new(),
            &parent,
        ));
        let outcome = match outcome {
            Ok(_) => panic!("the broken stream must fail"),
            Err(error) => error,
        };
        assert!(
            outcome.to_string().contains("MessageEnd"),
            "the broken stream is the failure: {outcome}"
        );
        // Unknown spend: no invented zero or MAX on the shared authority.
        assert_eq!(parent.usage(), Usage::default());
        // Further dispatch on the SAME authority refuses (typed).
        assert_eq!(
            parent.admit().err(),
            Some(AccountingRefusal::Incomplete),
            "the authority refuses new calls after unknown spend; state={:?}",
            parent.state.lock().unwrap()
        );
        // A fresh RUN, live invocation, healthy provider: the ONLY refusal
        // left is the terminalized authority itself.
        {
            let conn = d.pool.get().unwrap();
            conn.execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 2, 2)",
                [],
            )
            .unwrap();
        }
        rt().block_on(change_claim(&d.pool, 2, "second", true))
            .unwrap();
        let next = rt()
            .block_on(delegate_owned_budgeted(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                LoopbackProvider::new("loopback", vec![scripted_text("must not run")]),
                2,
                &spec("scout", &[], 900),
                "after",
                "second",
                &CancellationToken::new(),
                &parent,
            ))
            .expect_err("dispatch after unknown spend refuses");
        assert!(
            next.to_string().contains("accounting_incomplete"),
            "the authority itself refuses: {next}"
        );
    }

    /// B6: structural bounds — a non-root authority cannot reserve (depth),
    /// and generation overflow refuses instead of wrapping.
    #[test]
    fn b6_structural_depth_and_generation_overflow_refuse() {
        let root = ExchangeBudget::new(Some(100));
        let child = root.reserve_child(80).unwrap();
        // The child IS itself a non-root authority: a grandchild refuses.
        assert_eq!(
            child.budget.reserve_child(10).err(),
            Some(AccountingRefusal::Invalid),
            "deeper nesting is rejected structurally"
        );
        // Generation overflow refuses the reservation.
        let fresh = ExchangeBudget::new(Some(100));
        fresh.state.lock().unwrap().generation = u64::MAX;
        assert_eq!(
            fresh.reserve_child(10).err(),
            Some(AccountingRefusal::Invalid),
            "generation overflow refuses instead of wrapping"
        );
        assert_eq!(fresh.state.lock().unwrap().reserved, 0, "no capacity taken");
    }

    /// B6: repeated exchanges reclaim their bounded state — after every
    /// exchange the root holds no reservation and no in-flight record, and
    /// the shared permit's reference count returns to baseline (no cycles,
    /// no driver-lifetime map).
    #[test]
    fn b6_bounded_slots_reclaim_across_repeated_exchanges() {
        let d = deleg();
        let runtime = rt();
        let cancel = CancellationToken::new();
        let parent = ExchangeBudget::new(Some(100_000));
        let permit = parent.dispatch_permit();
        let baseline = Arc::strong_count(&permit);
        let provider = LoopbackProvider::new(
            "loopback",
            (0..12)
                .map(|i| scripted_text(&format!("answer {i}")))
                .collect(),
        );
        runtime
            .block_on(change_claim(&d.pool, 1, "parent", true))
            .unwrap();
        for n in 0..12 {
            let outcome = runtime
                .block_on(delegate_owned_budgeted(
                    &d.pool,
                    &d.host,
                    &d.env,
                    &[],
                    Arc::clone(&provider) as Arc<dyn LlmProvider>,
                    1,
                    &spec("scout", &[], 900),
                    &format!("exchange-{n}"),
                    "parent",
                    &cancel,
                    &parent,
                ))
                .unwrap();
            assert!(matches!(outcome, SubagentOutcome::Completed { .. }));
            // After each exchange: root slot empty, nothing in flight, no
            // reservation leaked, permit references reclaimed.
            assert_eq!(parent.state.lock().unwrap().reserved, 0);
            assert_eq!(parent.state.lock().unwrap().in_flight, 0);
            assert_eq!(
                Arc::strong_count(&permit),
                baseline,
                "exchange {n}: bounded reclamation"
            );
        }
        // Dropping the root authority drops the permit (no cycle keeps it).
        drop(parent);
        assert_eq!(Arc::strong_count(&permit), 1);
    }

    /// R2: usage received by an admitted call still debits after the
    /// reservation was released, and the in-flight record is what keeps the
    /// slot observable until accounting finishes.
    #[test]
    fn r2_inflight_usage_after_release_still_debits() {
        let parent = ExchangeBudget::new(Some(100));
        let mut child = parent.reserve_child(80).unwrap();
        let admitted = child.budget.admit().unwrap();
        assert_eq!(child.budget.state.lock().unwrap().in_flight, 1);
        child.release();
        child.release();
        // The released reservation revokes dispatch...
        assert!(child.budget.exhausted(), "released authority refuses");
        // ...but the admitted call's subsequently received usage still
        // debits every applicable aggregate — never refunded, never lost.
        admitted.budget.record(Usage {
            input_tokens: 9,
            output_tokens: 6,
        });
        drop(admitted);
        assert_eq!(parent.usage().total(), 15);
        assert_eq!(parent.state.lock().unwrap().in_flight, 0);
        assert_eq!(parent.state.lock().unwrap().reserved, 0);
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

    #[test]
    fn narrowed_env_denies_execution_on_disjoint_command_sets() {
        let fs: Arc<dyn FsSeam> = Arc::new(MemFs::default());
        let parent = ExecutionEnv {
            fs: Arc::clone(&fs),
            read_only: false,
            allow_process: true,
            root: "/srv".into(),
            allowed_commands: vec!["ls".into(), "cat".into()],
        };
        // An explicit ask that intersects the parent's list NOWHERE
        // grants no command at all — and the SDK gate reads an empty list as
        // UNRESTRICTED, so the silent [] must not keep a process grant
        // attached. Denied execution, never unrestricted.
        let child = narrowed_env(&parent, &caps(true, true, &["curl"]));
        assert!(
            !child.allow_process,
            "empty ask∩parent intersection denies execution outright"
        );
        assert!(
            child.allowed_commands.is_empty(),
            "no command is granted: the list stays empty"
        );
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
    fn scoped_context_child_receives_task_without_parent_history() {
        let d = deleg();
        session_log::append(
            &d.pool.get().unwrap(),
            1,
            "user",
            "parent-private-marker",
            "parent-seed",
            1,
        )
        .unwrap();
        let provider = LoopbackProvider::new("loopback", vec![scripted_text("child summary")]);
        rt().block_on(delegate(
            &d.pool,
            &d.host,
            &d.env,
            &[],
            provider.clone(),
            1,
            &spec("scout", &[], 1000),
            &CancellationToken::new(),
        ))
        .unwrap();
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .messages
                .iter()
                .any(|message| message.text().contains("read a.txt and report")),
            "actual child request omitted its explicit task"
        );
        assert!(
            requests[0]
                .messages
                .iter()
                .all(|message| !message.text().contains("parent-private-marker")),
            "child received implicit parent history"
        );
    }

    #[test]
    fn r1_same_owner_delegation_paused_provider_blocks_entry_and_release() {
        struct Paused {
            entered: Arc<tokio::sync::Notify>,
            sender: StdMutex<
                Option<
                    tokio::sync::mpsc::Sender<
                        Result<
                            crate::agentloop::provider::StreamEvent,
                            crate::agentloop::provider::ProviderError,
                        >,
                    >,
                >,
            >,
        }
        impl LlmProvider for Paused {
            fn name(&self) -> &str {
                "paused-child"
            }
            fn stream(
                &self,
                _: crate::agentloop::provider::ProviderRequest,
            ) -> Result<
                tokio::sync::mpsc::Receiver<
                    Result<
                        crate::agentloop::provider::StreamEvent,
                        crate::agentloop::provider::ProviderError,
                    >,
                >,
                crate::agentloop::provider::ProviderError,
            > {
                let (sender, receiver) = tokio::sync::mpsc::channel(1);
                *self.sender.lock().unwrap() = Some(sender);
                self.entered.notify_one();
                Ok(receiver)
            }
        }
        let d = deleg();
        let runtime = rt();
        let entered = Arc::new(tokio::sync::Notify::new());
        let paused = Arc::new(Paused {
            entered: entered.clone(),
            sender: StdMutex::new(None),
        });
        let other = LoopbackProvider::new("loopback", vec![scripted_text("must not run")]);
        let spec = spec("scout", &[], 1000);
        let cancel = CancellationToken::new();
        runtime.block_on(async {
            change_claim(&d.pool, 1, "parent", true).await.unwrap();
            let first = delegate_owned(&d.pool, &d.host, &d.env, &[], paused, 1, &spec, "first", "parent", &cancel);
            tokio::pin!(first);
            tokio::select! {
                result = &mut first => panic!("returned before stream pause: {result:?}"),
                signal = tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified()) => signal.unwrap(),
            }
            assert!(delegate_owned(&d.pool, &d.host, &d.env, &[], other.clone(), 1, &spec, "second", "parent", &cancel).await.is_err());
            assert!(other.requests().is_empty());
            assert!(change_claim(&d.pool, 1, "parent", false).await.is_err());
            cancel.cancel();
            assert_eq!(first.await.unwrap(), SubagentOutcome::Canceled);
            change_claim(&d.pool, 1, "parent", false).await.unwrap();
        });
    }

    #[test]
    fn r1_owned_child_retry_uses_exact_invocation_and_keeps_parent_claim() {
        let d = deleg();
        let runtime = rt();
        let cancel = CancellationToken::new();
        let provider = LoopbackProvider::new(
            "loopback",
            vec![scripted_text("first"), scripted_text("second")],
        );
        runtime
            .block_on(change_claim(&d.pool, 1, "parent", true))
            .unwrap();
        let spec = spec("scout", &[], 1000);
        let first = runtime
            .block_on(delegate_owned(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                provider.clone(),
                1,
                &spec,
                "one",
                "parent",
                &cancel,
            ))
            .unwrap();
        runtime
            .block_on(delegate_owned(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                provider.clone(),
                1,
                &spec,
                "two",
                "parent",
                &cancel,
            ))
            .unwrap();
        let retry = runtime
            .block_on(delegate_owned(
                &d.pool,
                &d.host,
                &d.env,
                &[],
                provider.clone(),
                1,
                &spec,
                "one",
                "parent",
                &cancel,
            ))
            .unwrap();
        assert_eq!(retry, first);
        assert_eq!(provider.requests().len(), 2);
        assert!(
            runtime
                .block_on(change_claim(&d.pool, 1, "other", true))
                .is_err()
        );
        runtime
            .block_on(change_claim(&d.pool, 1, "parent", false))
            .unwrap();
        let conn = d.pool.get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM agent_session_events WHERE kind='subagent_result'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn r1_same_name_children_keep_distinct_results() {
        let d = deleg();
        let provider =
            LoopbackProvider::new("loopback", vec![scripted_text("one"), scripted_text("two")]);
        let runtime = rt();
        let cancel = CancellationToken::new();
        for expected in ["one", "two"] {
            let outcome = runtime
                .block_on(delegate(
                    &d.pool,
                    &d.host,
                    &d.env,
                    &[],
                    provider.clone(),
                    1,
                    &spec("scout", &[], 1000),
                    &cancel,
                ))
                .unwrap();
            assert!(
                matches!(outcome, SubagentOutcome::Completed { summary, .. } if summary == expected)
            );
        }
        let conn = d.pool.get().unwrap();
        let events = session_log::replay(&conn, 1, 500).unwrap();
        let results: Vec<_> = events
            .iter()
            .filter(|e| e.kind == "subagent_result")
            .collect();
        assert_eq!(results.len(), 2);
        for (row, expected) in results.iter().zip(["one", "two"]) {
            let value: serde_json::Value = serde_json::from_str(&row.payload_json).unwrap();
            assert_eq!(value["summary"], expected);
        }
    }

    fn r2_parent_driver(d: &Deleg, provider: Arc<LoopbackProvider>) -> LoopDriver {
        LoopDriver::new(
            d.pool.clone(),
            Arc::clone(&d.host),
            Arc::new(AgentHarness::new(
                Arc::clone(&d.host),
                "parent",
                "parent policy marker",
            )),
            provider,
            vec![brain_engine_sdk::env::create_read_tool()],
            d.env.clone(),
            LoopConfig::default(),
            "",
            LoopHooks::pass_through(),
        )
    }

    fn r2_framed(source: &str, text: &str) -> String {
        crate::fence::wrap_fenced(&format!("Source: {source}\n{text}"))
    }

    #[test]
    fn r2_same_name_children_requests_keep_distinct_private_tasks() {
        let d = deleg();
        let runtime = rt();
        let provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text("first outcome"),
                scripted_text("second outcome"),
            ],
        );
        session_log::append(
            &d.pool.get().unwrap(),
            1,
            "user",
            "parent private input",
            "r2-parent-seed",
            1,
        )
        .unwrap();
        let tasks = ["first child private task", "second child private task"];
        for (task, summary) in tasks.iter().zip(["first outcome", "second outcome"]) {
            let mut child = spec("scout", &[], 1_000);
            child.task = (*task).into();
            let outcome = runtime
                .block_on(delegate(
                    &d.pool,
                    &d.host,
                    &d.env,
                    &[],
                    provider.clone(),
                    1,
                    &child,
                    &CancellationToken::new(),
                ))
                .unwrap();
            assert!(
                matches!(outcome, SubagentOutcome::Completed { summary: text, .. } if text == summary)
            );
        }
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].system_prompt, requests[1].system_prompt);
        for (request, task) in requests.iter().zip(tasks) {
            assert_eq!(
                request.messages,
                vec![ChatMessage::User {
                    text: format!("Source: user input\n{task}"),
                }],
                "same display name must not select another invocation's task or outcome"
            );
            assert!(request.tools.is_empty());
            let encoded = serde_json::to_string(request).unwrap();
            assert!(!encoded.contains("parent private input"));
            assert!(!encoded.contains("first outcome"));
        }
    }

    #[test]
    fn r2_child_tool_followup_preserves_typed_group_and_masks_synthetic_pii() {
        let d = deleg();
        let raw = "child private tool body synthetic@example.test";
        d.env.fs.write("a.txt", raw).unwrap();
        let provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text_then_tool("child private reasoning", "read-one", "read", "a.txt"),
                scripted_text("public child outcome"),
            ],
        );
        let child = spec("scout", &["read"], 1_000);
        let outcome = rt()
            .block_on(delegate(
                &d.pool,
                &d.host,
                &d.env,
                &[brain_engine_sdk::env::create_read_tool()],
                provider.clone(),
                1,
                &child,
                &CancellationToken::new(),
            ))
            .unwrap();
        assert!(
            matches!(outcome, SubagentOutcome::Completed { summary, .. } if summary == "public child outcome")
        );
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].system_prompt, requests[1].system_prompt);
        assert_eq!(requests[0].tools, requests[1].tools);
        assert_eq!(requests[0].tools.len(), 1);
        assert_eq!(requests[0].tools[0].name, "read");
        assert_eq!(
            requests[0].messages,
            vec![ChatMessage::User {
                text: format!("Source: user input\n{}", child.task),
            }]
        );
        let [
            task,
            ChatMessage::Assistant { text, tool_calls },
            ChatMessage::ToolResult {
                call_id,
                original_id,
                name,
                status,
                output,
                truncated,
            },
        ] = requests[1].messages.as_slice()
        else {
            panic!("child follow-up must be task, typed assistant call, typed result");
        };
        assert_eq!(task, &requests[0].messages[0]);
        assert_eq!(
            text,
            &r2_framed("historical assistant prose", "child private reasoning")
        );
        assert_eq!(tool_calls.len(), 1);
        let call = &tool_calls[0];
        assert_eq!(call.original_id, "read-one");
        assert_eq!(call.name, "read");
        assert_eq!(call.arguments_json, "a.txt");
        assert_ne!(call.id, call.original_id);
        assert_eq!(call_id, &call.id);
        assert_eq!(original_id, &call.original_id);
        assert_eq!(name, &call.name);
        assert_eq!(*status, ToolResultStatus::Success);
        assert!(!truncated);
        assert!(output.contains("Source: tool output"));
        assert!(output.contains("child private tool body"));
        assert!(output.contains("redacted:email"));
        assert!(
            !serde_json::to_string(&requests[1])
                .unwrap()
                .contains("example.test")
        );
        let rows = session_log::replay(&d.pool.get().unwrap(), 1, session_log::REPLAY_CAP).unwrap();
        let stored = rows
            .iter()
            .find(|row| row.kind == "child:scout:tool_result")
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&stored.payload_json).unwrap();
        assert_eq!(
            payload["output"], raw,
            "projection must not rewrite durable evidence"
        );
    }

    #[test]
    fn r2_parent_request_sees_only_projected_child_outcomes() {
        let d = deleg();
        let runtime = rt();
        let parent_provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text("parent earlier answer"),
                scripted_text("parent final answer"),
            ],
        );
        let parent = r2_parent_driver(&d, parent_provider.clone());
        let cancel = CancellationToken::new();
        runtime
            .block_on(parent.run_turns(1, "parent earlier task", &cancel))
            .unwrap();
        let child_provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text_then_tool("first private reasoning", "read-one", "read", "first.txt"),
                scripted_text("first projected outcome"),
                scripted_text_then_tool(
                    "second private reasoning",
                    "read-one",
                    "read",
                    "second.txt",
                ),
                scripted_text("second projected outcome"),
            ],
        );
        for (task, path, body) in [
            ("first private task", "first.txt", "first private tool body"),
            (
                "second private task",
                "second.txt",
                "second private tool body",
            ),
        ] {
            d.env.fs.write(path, body).unwrap();
            let mut child = spec("scout", &["read"], 1_000);
            child.task = task.into();
            runtime
                .block_on(delegate(
                    &d.pool,
                    &d.host,
                    &d.env,
                    &[brain_engine_sdk::env::create_read_tool()],
                    child_provider.clone(),
                    1,
                    &child,
                    &cancel,
                ))
                .unwrap();
        }
        assert_eq!(child_provider.requests().len(), 4);
        let rows = session_log::replay(&d.pool.get().unwrap(), 1, session_log::REPLAY_CAP).unwrap();
        let ids: Vec<i64> = rows
            .iter()
            .filter(|row| row.kind == "subagent_result")
            .map(|row| {
                let payload: serde_json::Value = serde_json::from_str(&row.payload_json).unwrap();
                payload["exchange_id"].as_i64().unwrap()
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        runtime
            .block_on(parent.run_turns(1, "parent later task", &cancel))
            .unwrap();
        let requests = parent_provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].system_prompt, requests[1].system_prompt);
        assert_eq!(
            requests[1].messages,
            vec![
                ChatMessage::User {
                    text: "Source: user input\nparent earlier task".into()
                },
                ChatMessage::Assistant {
                    text: r2_framed("historical assistant prose", "parent earlier answer"),
                    tool_calls: vec![],
                },
                ChatMessage::Delegation {
                    exchange_id: ids[0],
                    name: "scout".into(),
                    outcome: DelegationOutcome::Completed,
                    summary: r2_framed("delegated child output", "first projected outcome"),
                },
                ChatMessage::Delegation {
                    exchange_id: ids[1],
                    name: "scout".into(),
                    outcome: DelegationOutcome::Completed,
                    summary: r2_framed("delegated child output", "second projected outcome"),
                },
                ChatMessage::User {
                    text: "Source: user input\nparent later task".into()
                },
            ]
        );
        let encoded = serde_json::to_string(&requests[1]).unwrap();
        for private in [
            "private task",
            "private reasoning",
            "private tool body",
            "first.txt",
            "second.txt",
            "read-one",
        ] {
            assert!(
                !encoded.contains(private),
                "child internals leaked to parent: {private}"
            );
        }
    }

    #[test]
    fn r2_sibling_flood_cannot_evict_child_task_or_parent_history() {
        let d = deleg();
        let runtime = rt();
        let cancel = CancellationToken::new();
        let parent_provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text("retained parent answer"),
                scripted_text("done"),
            ],
        );
        let parent = r2_parent_driver(&d, parent_provider.clone());
        runtime
            .block_on(parent.run_turns(1, "retained parent task", &cancel))
            .unwrap();
        let flood_count = session_log::REPLAY_CAP + 1;
        assert!(flood_count > 500);
        {
            let mut conn = d.pool.get().unwrap();
            let tx = conn.transaction().unwrap();
            for index in 0..flood_count {
                session_log::append(
                    &tx,
                    1,
                    "child:scout:user",
                    "legacy sibling private marker",
                    &format!("r2-legacy-sibling-{index}"),
                    1,
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        let child_provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text_then_tool(
                    "selected child reasoning",
                    "selected-read",
                    "read",
                    "a.txt",
                ),
                scripted_text("selected child outcome"),
            ],
        );
        let child = spec("scout", &["read"], 1_000);
        runtime
            .block_on(delegate(
                &d.pool,
                &d.host,
                &d.env,
                &[brain_engine_sdk::env::create_read_tool()],
                child_provider.clone(),
                1,
                &child,
                &cancel,
            ))
            .unwrap();
        let child_requests = child_provider.requests();
        assert_eq!(child_requests.len(), 2);
        assert_eq!(child_requests[0].messages.len(), 1);
        assert_eq!(child_requests[1].messages.len(), 3);
        for request in &child_requests {
            assert_eq!(
                request.messages[0],
                ChatMessage::User {
                    text: format!("Source: user input\n{}", child.task),
                }
            );
            let encoded = serde_json::to_string(request).unwrap();
            assert!(!encoded.contains("legacy sibling private marker"));
            assert!(!encoded.contains("retained parent"));
        }
        runtime
            .block_on(parent.run_turns(1, "requested parent task", &cancel))
            .unwrap();
        let requests = parent_provider.requests();
        assert_eq!(requests.len(), 2);
        let [
            old_task,
            old_answer,
            ChatMessage::Delegation {
                name,
                outcome,
                summary,
                ..
            },
            new_task,
        ] = requests[1].messages.as_slice()
        else {
            panic!("sibling rows must not consume the parent's selected replay cap");
        };
        assert_eq!(
            old_task,
            &ChatMessage::User {
                text: "Source: user input\nretained parent task".into()
            }
        );
        assert_eq!(
            old_answer,
            &ChatMessage::Assistant {
                text: r2_framed("historical assistant prose", "retained parent answer"),
                tool_calls: vec![],
            }
        );
        assert_eq!(name, "scout");
        assert_eq!(*outcome, DelegationOutcome::Completed);
        assert_eq!(
            summary,
            &r2_framed("delegated child output", "selected child outcome")
        );
        assert_eq!(
            new_task,
            &ChatMessage::User {
                text: "Source: user input\nrequested parent task".into()
            }
        );
        assert!(
            !serde_json::to_string(&requests[1])
                .unwrap()
                .contains("legacy sibling private marker")
        );
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

    /// Composition proof (the delegating-child face of the env gate;
    /// the pre-fix failure modes — the child's exec RUNNING, or an
    /// operator-layer denial text — were proven RED at the unit slices): a
    /// child whose caps subtract process reaches the mediated exec tool
    /// through the SAME wiring as the parent, and the bridge refuses BEFORE
    /// any mediation dispatch — zero spawn, the denial rides back as the
    /// child's typed tool_result, and the parent's audit chain carries it.
    #[test]
    fn delegated_child_env_denies_exec_before_dispatch() {
        let d = deleg();
        let _g = crate::test_support::lock_env();
        unsafe {
            std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo");
        }
        // Parent-loop posture: process granted, no loop-level command list —
        // the operator allowlist is the trust anchor for the PARENT.
        let parent_env = ExecutionEnv {
            fs: Arc::new(MemFs::default()),
            read_only: false,
            allow_process: true,
            root: "/".into(),
            allowed_commands: vec![],
        };
        let mut s = spec("runner", &["exec"], 1_000);
        s.caps = caps(false, false, &[]); // the child's process ask is subtracted
        let provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text_then_tool(
                    "trying exec",
                    "e1",
                    "exec",
                    r#"["/bin/echo","child-ran"]"#,
                ),
                scripted_text("exec was refused, as designed"),
            ],
        );
        let cancel = CancellationToken::new();
        let tools = vec![crate::agentloop::exec::mediated_exec_tool(
            Arc::clone(&d.host),
            "agentloop",
        )];
        let outcome = rt()
            .block_on(delegate(
                &d.pool,
                &d.host,
                &parent_env,
                &tools,
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
            .find(|e| e.kind == "child:runner:tool_result")
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&tool_event.payload_json).unwrap();
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(
            payload["output"]
                .as_str()
                .unwrap()
                .contains("process execution disabled by environment"),
            "the child sees the env denial, not an operator-layer message: {}",
            payload["output"]
        );
        assert!(
            verify_chain(&conn),
            "the child's pre-dispatch denial lands in the verified parent chain"
        );
    }

    /// The disjoint-ask sibling: parent commands ["ls"], child asks ["curl"]
    /// — the narrowing rule caps the child at `allow_process: false` (empty
    /// intersection is never an unrestricted empty vector) and the bridge's
    /// env gate enforces it. Denied by the composed fix, end to end.
    #[test]
    fn delegated_child_disjoint_command_ask_denies_exec_before_dispatch() {
        let d = deleg();
        let _g = crate::test_support::lock_env();
        unsafe {
            std::env::set_var("BRAIN_ENGINE_EXEC_ALLOWLIST", "/bin/echo");
        }
        let parent_env = ExecutionEnv {
            fs: Arc::new(MemFs::default()),
            read_only: false,
            allow_process: true,
            root: "/".into(),
            allowed_commands: vec!["ls".into()],
        };
        let mut s = spec("prober", &["exec"], 1_000);
        s.caps = caps(true, true, &["curl"]); // non-empty ask, disjoint from the parent's
        let provider = LoopbackProvider::new(
            "loopback",
            vec![
                scripted_text_then_tool(
                    "trying curl",
                    "c1",
                    "exec",
                    r#"["curl","-s","http://example.invalid"]"#,
                ),
                scripted_text("curl was refused, as designed"),
            ],
        );
        let cancel = CancellationToken::new();
        let tools = vec![crate::agentloop::exec::mediated_exec_tool(
            Arc::clone(&d.host),
            "agentloop",
        )];
        let outcome = rt()
            .block_on(delegate(
                &d.pool,
                &d.host,
                &parent_env,
                &tools,
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
            .find(|e| e.kind == "child:prober:tool_result")
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&tool_event.payload_json).unwrap();
        assert_eq!(payload["ok"], serde_json::json!(false));
        assert!(
            payload["output"]
                .as_str()
                .unwrap()
                .contains("process execution disabled by environment"),
            "the disjoint ask narrows to no-execution, enforced at the bridge: {}",
            payload["output"]
        );
        assert!(
            verify_chain(&conn),
            "the composed denial lands in the verified parent chain"
        );
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
