//! The agent-harness lifecycle (pi-shaped port; semantics ported, no code
//! copied): phases with structural gates, defensive turn snapshots, and a
//! pending-write queue flushed deterministically at save-points.
//!
//! Invariants: a turn snapshot is an owned clone — setters affect only the
//! *next* snapshot, never an in-flight turn. Structural operations
//! (`compact`, `set_leaf_id`, tree navigation) require
//! [`Phase::Idle`]; steering/abort/config stay legal mid-turn. Pending
//! session writes drain in FIFO order strictly after the `message_end`
//! persistence that triggered the flush.

use crate::host::{AuditKind, AuditStatus, HostError, WorkflowHost};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Harness phase; the gate for structural operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Running,
    Compact,
    /// Settlement failed; retained state is diagnostic, never retry authority.
    Failed,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Running => "running",
            Phase::Compact => "compact",
            Phase::Failed => "failed",
        }
    }
}

/// Harness failure vocabulary.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HarnessError {
    /// A structural op arrived while not [`Phase::Idle`].
    PhaseBusy { op: &'static str, phase: Phase },
    /// The operation is unavailable in the harness's current phase.
    PhaseUnavailable { op: &'static str, phase: Phase },
    /// Run operation attempted from a non-main lane.
    LaneNotMain { op: &'static str },
    /// The host refused or failed a write.
    Host(String),
    /// Shared state cannot safely authorize an operation.
    Unavailable,
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessError::PhaseBusy { op, phase } => {
                write!(f, "`{op}` requires idle phase (current {})", phase.as_str())
            }
            HarnessError::PhaseUnavailable { op, phase } => {
                write!(f, "`{op}` unavailable in phase {}", phase.as_str())
            }
            HarnessError::LaneNotMain { op } => {
                write!(f, "`{op}` is a run operation; non-main lanes may only read")
            }
            HarnessError::Host(m) => write!(f, "host: {m}"),
            HarnessError::Unavailable => write!(f, "harness unavailable"),
        }
    }
}

impl std::error::Error for HarnessError {}

/// Defensive copy of everything a turn depends on. Getters return clones;
/// nothing hands out interior references to in-flight config.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSnapshot {
    model: String,
    system_prompt: String,
    tools: Vec<String>,
    resources: Vec<String>,
}

impl TurnSnapshot {
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }
    /// Tool names in registration order.
    pub fn tools(&self) -> &[String] {
        &self.tools
    }
    pub fn resources(&self) -> &[String] {
        &self.resources
    }

    /// Assemble the prompt for this turn (system prompt + tool schema names).
    pub fn prompt(&self) -> String {
        let mut p = self.system_prompt.clone();
        if !self.tools.is_empty() {
            p.push_str("\n\ntools: ");
            p.push_str(&self.tools.join(", "));
        }
        p
    }
}

/// A session write queued without an id; the host assigns ids on flush.
#[derive(Debug, Clone, PartialEq)]
struct EntryWithoutId {
    topic: String,
    payload_json: String,
    idempotency_key: String,
}

/// Opaque, unforgeable handle bound to one harness turn. A stale or
/// cross-harness token cannot mutate a later turn even with the same run ID.
/// The retained allocation identity survives moves and prevents reuse while
/// any token remains alive; the generation distinguishes successive turns.
#[derive(Debug, Clone)]
pub struct TurnToken {
    harness: Arc<()>,
    generation: u64,
}

impl<H: WorkflowHost> AgentHarness<H> {
    /// Owned start: checked RunStart evidence commits BEFORE a usable turn
    /// exists; success returns the snapshot plus an unforgeable turn token
    /// minted under the same lock that publishes Running. A failed start
    /// creates no armed guard and no token.
    pub fn start_turn(&self, run_id: i64) -> Result<(TurnSnapshot, TurnToken), HarnessError> {
        self.start(run_id, "start_turn")
    }

    fn start(
        &self,
        run_id: i64,
        op: &'static str,
    ) -> Result<(TurnSnapshot, TurnToken), HarnessError> {
        let mut g = self.checked_lock()?;
        if g.phase != Phase::Idle {
            return Err(HarnessError::PhaseBusy { op, phase: g.phase });
        }
        // Checked increment BEFORE any host work: overflow refuses with no
        // audit written, no phase change, no snapshot.
        let generation = g
            .turn_token
            .checked_add(1)
            .ok_or(HarnessError::Host("turn_token_overflow".into()))?;
        let snap = g.config.snapshot();
        // Unknown errors and unwinds cannot certify an unused start.
        g.phase = Phase::Failed;
        if let Err(error) = g.host.audit_settlement(
            AuditKind::Workflow,
            "harness",
            &format!("run:{run_id}"),
            AuditStatus::Ok,
            "RunStart",
        ) {
            if matches!(
                error,
                HostError::SettlementRefused | HostError::Busy | HostError::NotFound
            ) {
                g.phase = Phase::Idle;
            }
            return Err(HarnessError::Host(error.to_string()));
        }
        g.run_id = run_id;
        g.phase = Phase::Running;
        g.snapshot = Some(snap.clone());
        g.turn_token = generation;
        let token = TurnToken {
            harness: Arc::clone(&self.identity),
            generation,
        };
        Ok((snap, token))
    }

    /// Verify token ownership under the CALLER's held lock, so the check is
    /// atomic with the operation that follows it. A stale/cross-harness token
    /// mutates nothing.
    fn verify_token(
        g: &Inner<H>,
        token: &TurnToken,
        identity: &Arc<()>,
    ) -> Result<(), HarnessError> {
        if Arc::ptr_eq(&token.harness, identity)
            && token.generation == g.turn_token
            && g.phase == Phase::Running
        {
            Ok(())
        } else {
            Err(HarnessError::Host("stale_turn_token".into()))
        }
    }

    /// Owned message-end: delivery only under a valid current-turn token.
    pub fn message_end_owned(
        &self,
        token: &TurnToken,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<(), HarnessError> {
        let mut g = self.checked_lock()?;
        Self::verify_token(&g, token, &self.identity)?;
        g.enqueue_locked("message_end", payload_json, idempotency_key)?;
        g.drain_locked()
    }

    /// Owned save-point: drain pending writes under a valid token.
    pub fn save_point_owned(&self, token: &TurnToken) -> Result<usize, HarnessError> {
        let mut g = self.checked_lock()?;
        Self::verify_token(&g, token, &self.identity)?;
        let n = g.pending.len();
        g.drain_locked()?;
        Ok(n)
    }

    /// Owned finish: token verified atomically with the settlement under one
    /// lock; checked RunEnd commits before Idle; token consumed.
    pub fn finish_turn(&self, token: TurnToken) -> Result<(), HarnessError> {
        self.settle_owned(Some(&token), "RunEnd:finished", AuditStatus::Ok)
    }

    /// Owned abort: same settlement path as finish; token consumed.
    pub fn abort_turn(&self, token: TurnToken) -> Result<(), HarnessError> {
        self.settle_owned(Some(&token), "RunEnd:aborted", AuditStatus::Denied)
    }

    /// Bounded abort for destructor-time cleanup: try-acquires the harness
    /// lock, never blocking the caller. On contention this returns
    /// [`HarnessError::Unavailable`] with no state change — fail closed: the
    /// claim is retained, no Idle is published, and the caller records a
    /// fixed status. The host calls inside still honor configured database
    /// waits; a test deadline is not an OS bound.
    pub fn try_abort_turn(&self, token: TurnToken) -> Result<(), HarnessError> {
        self.try_settle(Some(&token), "RunEnd:aborted", AuditStatus::Denied)
    }

    /// Bounded finish: same contract as [`Self::try_abort_turn`].
    pub fn try_finish_turn(&self, token: TurnToken) -> Result<(), HarnessError> {
        self.try_settle(Some(&token), "RunEnd:finished", AuditStatus::Ok)
    }

    /// The single settlement core under an already-held lock. Terminalizes
    /// BEFORE host code (an error or unwind never re-arms a turn), then
    /// returns the detached callbacks to run outside all locks.
    fn settle_under_lock(
        g: &mut Inner<H>,
        token: Option<&TurnToken>,
        identity: &Arc<()>,
        detail: &str,
        status: AuditStatus,
    ) -> Result<Vec<DeferredOp>, HarnessError> {
        if let Some(token) = token {
            Self::verify_token(g, token, identity)?;
        }
        if g.phase == Phase::Idle {
            return Ok(Vec::new());
        }
        g.require_running("settle")?;
        // Terminalize before host code: an error or unwind never re-arms a turn.
        g.phase = Phase::Failed;
        g.drain_locked()?;
        let run_id = g.run_id;
        g.host
            .audit_settlement(
                AuditKind::Workflow,
                "harness",
                &format!("run:{run_id}"),
                status,
                detail,
            )
            .map_err(|e| HarnessError::Host(e.to_string()))?;
        g.phase = Phase::Idle;
        g.snapshot = None;
        Ok(std::mem::take(&mut g.deferred_idle))
    }

    fn run_detached(ops: Vec<DeferredOp>, panicked: &AtomicBool) {
        for op in ops {
            // One panicking callback must not discard the rest; the default
            // panic hook still reports the payload (documented external-hook
            // ceiling: third-party callback payloads are not confidential).
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(op)).is_err() {
                panicked.store(true, Ordering::Relaxed);
            }
        }
    }

    fn settle_owned(
        &self,
        token: Option<&TurnToken>,
        detail: &str,
        status: AuditStatus,
    ) -> Result<(), HarnessError> {
        let ops = {
            let mut g = self.checked_lock()?;
            Self::settle_under_lock(&mut g, token, &self.identity, detail, status)?
        };
        Self::run_detached(ops, &self.callback_panicked);
        Ok(())
    }

    /// Bounded settlement: try-acquire only; contention refuses with
    /// [`HarnessError::Unavailable`] before any host work.
    fn try_settle(
        &self,
        token: Option<&TurnToken>,
        detail: &str,
        status: AuditStatus,
    ) -> Result<(), HarnessError> {
        let ops = {
            let mut g = self
                .inner
                .try_lock()
                .map_err(|_| HarnessError::Unavailable)?;
            Self::settle_under_lock(&mut g, token, &self.identity, detail, status)?
        };
        Self::run_detached(ops, &self.callback_panicked);
        Ok(())
    }

    /// Drop cleanup for an otherwise-unattempted owned settlement.
    /// Synchronous best effort, NOT a crash protocol: on abort/process loss
    /// this does not run. A failed cleanup attempt is not retried here.
    pub fn settle_on_drop(&self, token: TurnToken) {
        let _ = self.abort_turn(token);
    }
}

type DeferredOp = Box<dyn FnOnce() + Send>;

/// The harness's shared mutable state, guarded by one mutex.
struct Inner<H: WorkflowHost> {
    host: Arc<H>,
    config: Config,
    phase: Phase,
    snapshot: Option<TurnSnapshot>,
    pending: VecDeque<EntryWithoutId>,
    follow_ups: Vec<String>,
    deferred_idle: Vec<DeferredOp>,
    run_id: i64,
    /// Monotone harness-local turn identity. Same run ID is NOT ownership:
    /// only the token returned by a successful owned start may settle that
    /// turn. Checked increment; exhaustion refuses instead of wrapping.
    turn_token: u64,
}

struct Config {
    model: String,
    system_prompt: String,
    tools: Vec<String>,
    resources: Vec<String>,
}

impl Config {
    fn snapshot(&self) -> TurnSnapshot {
        // Defensive copies everywhere: the snapshot never aliases config.
        TurnSnapshot {
            model: self.model.clone(),
            system_prompt: self.system_prompt.clone(),
            tools: self.tools.clone(),
            resources: self.resources.clone(),
        }
    }
}

/// The harness over one host. `Send + Sync` via the inner mutex; hosts are
/// shared as `Arc<dyn WorkflowHost>`.
pub struct AgentHarness<H: WorkflowHost> {
    identity: Arc<()>,
    callback_panicked: AtomicBool,
    inner: Mutex<Inner<H>>,
}

impl<H: WorkflowHost> AgentHarness<H> {
    pub fn new(host: Arc<H>, model: &str, system_prompt: &str) -> Self {
        AgentHarness {
            identity: Arc::new(()),
            callback_panicked: AtomicBool::new(false),
            inner: Mutex::new(Inner {
                host,
                config: Config {
                    model: model.to_string(),
                    system_prompt: system_prompt.to_string(),
                    tools: Vec::new(),
                    resources: Vec::new(),
                },
                phase: Phase::Idle,
                snapshot: None,
                pending: VecDeque::new(),
                follow_ups: Vec::new(),
                deferred_idle: Vec::new(),
                run_id: 0,
                turn_token: 0,
            }),
        }
    }

    // -- configuration (next-turn semantics) --------------------------------

    pub fn set_model(&self, model: &str) -> Result<(), HarnessError> {
        self.checked_lock()?.config.model = model.to_string();
        Ok(())
    }
    pub fn set_system_prompt(&self, prompt: &str) -> Result<(), HarnessError> {
        self.checked_lock()?.config.system_prompt = prompt.to_string();
        Ok(())
    }
    pub fn add_tool(&self, name: &str) -> Result<(), HarnessError> {
        self.checked_lock()?.config.tools.push(name.to_string());
        Ok(())
    }
    pub fn add_resource(&self, name: &str) -> Result<(), HarnessError> {
        self.checked_lock()?.config.resources.push(name.to_string());
        Ok(())
    }

    // -- turn lifecycle ------------------------------------------------------

    /// Start a run: capture the current config as this turn's immutable
    /// snapshot and audit `RunStart`. Mid-turn config changes do NOT touch it.
    pub fn start_run(&self, run_id: i64) -> Result<TurnSnapshot, HarnessError> {
        self.start(run_id, "start_run")
            .map(|(snapshot, _)| snapshot)
    }

    /// The in-flight snapshot (defensive clone), when a run is active.
    pub fn snapshot(&self) -> Option<TurnSnapshot> {
        self.lock().snapshot.clone()
    }

    pub fn phase(&self) -> Phase {
        self.lock().phase
    }

    /// Persist the assistant message end FIRST, then drain queued writes in
    /// FIFO order — the deterministic ordering invariant.
    pub fn message_end(
        &self,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<(), HarnessError> {
        let mut g = self.checked_lock()?;
        g.require_running("message_end")?;
        g.enqueue_locked("message_end", payload_json, idempotency_key)?;
        g.drain_locked()
    }

    /// Queue a session write without an id; flushed at save-points or on
    /// operation finish/failure cleanup.
    pub fn queue_write(
        &self,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<(), HarnessError> {
        let mut g = self.checked_lock()?;
        g.require_running("queue_write")?;
        g.pending.push_back(EntryWithoutId {
            topic: topic.to_string(),
            payload_json: payload_json.to_string(),
            idempotency_key: idempotency_key.to_string(),
        });
        Ok(())
    }

    /// Save-point: drain all pending writes now.
    pub fn save_point(&self) -> Result<usize, HarnessError> {
        let mut g = self.checked_lock()?;
        g.require_running("save_point")?;
        let n = g.pending.len();
        g.drain_locked()?;
        Ok(n)
    }

    /// Steer mid-turn: allowed while running; recorded for the provider loop.
    pub fn steer(&self, note: &str) -> Result<(), HarnessError> {
        let mut g = self.checked_lock()?;
        if g.phase != Phase::Running {
            return Err(HarnessError::PhaseBusy {
                op: "steer",
                phase: g.phase,
            });
        }
        g.follow_ups.push(note.to_string());
        Ok(())
    }

    /// Queue follow-up work; allowed mid-turn.
    pub fn follow_up(&self, note: &str) -> Result<(), HarnessError> {
        self.checked_lock()?.follow_ups.push(note.to_string());
        Ok(())
    }

    pub fn follow_ups(&self) -> Vec<String> {
        self.lock().follow_ups.clone()
    }

    /// Fixed, sticky diagnostic; no callback payload is retained here.
    pub fn callback_panicked(&self) -> bool {
        self.callback_panicked.load(Ordering::Relaxed)
    }

    /// Register work to execute exactly when the harness next reaches Idle.
    /// This is the facade's `runWhenIdle`: callers never poll raw internals.
    pub fn run_when_idle<F: FnOnce() + Send + 'static>(&self, f: F) -> Result<(), HarnessError> {
        self.checked_lock()?.deferred_idle.push(Box::new(f));
        Ok(())
    }

    /// Finish the run: final save-point, then back to Idle (running any
    /// deferred-idle work). Audits `RunEnd`.
    pub fn finish_run(&self) -> Result<(), HarnessError> {
        self.settle("RunEnd:finished", AuditStatus::Ok)
    }

    /// Abort mid-turn: same settlement path as finish (cleanup is not a
    /// special case). Audits `RunEnd` with the aborted status.
    pub fn abort(&self) -> Result<(), HarnessError> {
        self.settle("RunEnd:aborted", AuditStatus::Denied)
    }

    fn settle(&self, detail: &str, status: AuditStatus) -> Result<(), HarnessError> {
        self.settle_owned(None, detail, status)
    }

    // -- structural operations (Idle-only) -----------------------------------

    /// Idle-only structural gate. Verified (2026-09-18): this is a phase
    /// CHECK, not an exclusive reservation — nothing enters
    /// [`Phase::Compact`], so a caller that intends to hold the harness
    /// across async compaction work must not assume the phase stays
    /// reserved; the check only refuses while a turn is active. The loop
    /// driver calls this between turns, when the harness is Idle, and its
    /// R3 commit semantics live in the session log, not here.
    pub fn compact(&self) -> Result<(), HarnessError> {
        self.require_idle("compact")
    }

    pub fn set_leaf_id(&self, _leaf: &str) -> Result<(), HarnessError> {
        self.require_idle("set_leaf_id")
    }

    pub fn navigate_tree(&self, _node: &str) -> Result<(), HarnessError> {
        self.require_idle("navigate_tree")
    }

    fn require_idle(&self, op: &'static str) -> Result<(), HarnessError> {
        let g = self.checked_lock()?;
        if g.phase != Phase::Idle {
            return Err(HarnessError::PhaseBusy { op, phase: g.phase });
        }
        Ok(())
    }

    /// A read-only view handle for a lane. Only the main lane runs.
    pub fn lane_handle<'a>(&'a self, lane: &'a AgentLane) -> LaneHandle<'a, H> {
        LaneHandle {
            harness: self,
            lane,
        }
    }

    fn checked_lock(&self) -> Result<std::sync::MutexGuard<'_, Inner<H>>, HarnessError> {
        self.inner.lock().map_err(|_| HarnessError::Unavailable)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner<H>> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// The harness's current generation, for token-identity tests only.
    #[cfg(test)]
    fn current_generation(&self) -> u64 {
        self.lock().turn_token
    }
}

/// A named lane. The main lane drives runs; side lanes only observe.
#[derive(Debug, Clone)]
pub struct AgentLane {
    name: String,
    main: bool,
}

impl AgentLane {
    pub fn main() -> Self {
        AgentLane {
            name: "main".into(),
            main: true,
        }
    }
    pub fn side(name: &str) -> Self {
        AgentLane {
            name: name.to_string(),
            main: false,
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn is_main(&self) -> bool {
        self.main
    }
}

/// Read-delegating handle onto the harness for non-main lanes: reads flow
/// through the session view; every run op is rejected (`LaneNotMain`).
pub struct LaneHandle<'a, H: WorkflowHost> {
    harness: &'a AgentHarness<H>,
    lane: &'a AgentLane,
}

impl<H: WorkflowHost> LaneHandle<'_, H> {
    /// Read view: phase + snapshot are safe for any lane.
    pub fn view(&self) -> (Phase, Option<TurnSnapshot>) {
        (self.harness.phase(), self.harness.snapshot())
    }

    pub fn start_run(&self, _run_id: i64) -> Result<TurnSnapshot, HarnessError> {
        Err(self.reject("start_run"))
    }
    pub fn compact(&self) -> Result<(), HarnessError> {
        Err(self.reject("compact"))
    }
    pub fn set_leaf_id(&self, _leaf: &str) -> Result<(), HarnessError> {
        Err(self.reject("set_leaf_id"))
    }
    pub fn abort(&self) -> Result<(), HarnessError> {
        Err(self.reject("abort"))
    }

    fn reject(&self, op: &'static str) -> HarnessError {
        if self.lane.is_main() {
            HarnessError::Host(format!("internal: `{op}` must route through the harness"))
        } else {
            HarnessError::LaneNotMain { op }
        }
    }
}

impl<H: WorkflowHost> Inner<H> {
    fn require_running(&self, op: &'static str) -> Result<(), HarnessError> {
        if self.phase != Phase::Running {
            return Err(HarnessError::PhaseUnavailable {
                op,
                phase: self.phase,
            });
        }
        Ok(())
    }

    fn enqueue_locked(
        &self,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<(), HarnessError> {
        let created = self
            .host
            .enqueue_settlement(self.run_id, topic, payload_json, idempotency_key)
            .map_err(|e| HarnessError::Host(e.to_string()))?;
        if !created {
            // Replay receipt: the entry already exists; nothing further.
            return Ok(());
        }
        Ok(())
    }

    fn drain_locked(&mut self) -> Result<(), HarnessError> {
        while let Some(entry) = self.pending.front() {
            self.enqueue_locked(&entry.topic, &entry.payload_json, &entry.idempotency_key)?;
            self.pending.pop_front();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{AuditKind, AuditStatus};
    use std::sync::Mutex as StdMutex;

    /// Recording host: captures enqueues + audit rows in call order.
    #[derive(Default)]
    struct TapeHost {
        log: StdMutex<Vec<String>>,
        fail_key: StdMutex<Option<String>>,
        fail_audit: StdMutex<bool>,
        audit_error: StdMutex<Option<crate::host::HostError>>,
        delivered: StdMutex<Vec<(i64, String, String, String)>>,
        panic_audit: StdMutex<bool>,
    }
    impl TapeHost {
        fn record(&self, s: String) {
            if let Ok(mut g) = self.log.lock() {
                g.push(s);
            }
        }
        fn calls(&self) -> Vec<String> {
            self.log.lock().map(|g| g.clone()).unwrap_or_default()
        }
    }
    impl WorkflowHost for TapeHost {
        fn tx(&self) -> Result<crate::host::HostTx, crate::host::HostError> {
            unreachable!()
        }
        fn enqueue(
            &self,
            run_id: i64,
            topic: &str,
            payload: &str,
            key: &str,
        ) -> Result<bool, crate::host::HostError> {
            self.record(format!("enqueue:{topic}:{key}:{run_id}"));
            if self.fail_key.lock().unwrap().as_deref() == Some(key) {
                return Err(crate::host::HostError::Internal("injected_delivery".into()));
            }
            let mut delivered = self.delivered.lock().unwrap();
            if delivered.iter().any(|entry| entry.3 == key) {
                return Ok(false);
            }
            delivered.push((run_id, topic.into(), payload.into(), key.into()));
            Ok(true)
        }
        fn cas(&self, _: i64, _: i64, _: &str) -> Result<(), crate::host::CasError> {
            Ok(())
        }
        fn load_state(&self, _: i64) -> Result<Option<(String, i64)>, crate::host::HostError> {
            Ok(None)
        }
        fn audit(&self, k: AuditKind, actor: &str, target: &str, s: AuditStatus, d: &str) {
            self.record(format!(
                "audit:{}/{}/{}:{}/{}",
                k.as_str(),
                actor,
                target,
                s.as_str(),
                d
            ));
        }
        fn enqueue_settlement(
            &self,
            run_id: i64,
            topic: &str,
            payload: &str,
            key: &str,
        ) -> Result<bool, crate::host::HostError> {
            self.record(format!("settle_enqueue:{topic}:{key}:{run_id}"));
            if self.fail_key.lock().unwrap().as_deref() == Some(key) {
                return Err(crate::host::HostError::Internal("injected_delivery".into()));
            }
            let mut delivered = self.delivered.lock().unwrap();
            if delivered.iter().any(|entry| entry.3 == key) {
                return Ok(false);
            }
            delivered.push((run_id, topic.into(), payload.into(), key.into()));
            Ok(true)
        }
        #[allow(clippy::panic)] // deliberate unwind fixture (test host)
        fn audit_settlement(
            &self,
            k: AuditKind,
            actor: &str,
            target: &str,
            s: AuditStatus,
            d: &str,
        ) -> Result<(), crate::host::HostError> {
            self.record(format!(
                "settle_audit:{}/{}/{}:{}/{}",
                k.as_str(),
                actor,
                target,
                s.as_str(),
                d
            ));
            if *self.panic_audit.lock().unwrap() {
                panic!("synthetic host unwind inside settlement");
            }
            if let Some(error) = self.audit_error.lock().unwrap().clone() {
                return Err(error);
            }
            if *self.fail_audit.lock().unwrap() {
                return Err(crate::host::HostError::SettlementRefused);
            }
            Ok(())
        }
    }

    #[test]
    fn identity_survives_harness_move() {
        let h = Box::new(harness());
        let (_, token) = h.start_turn(7).unwrap();
        let moved = *h;
        moved
            .message_end_owned(&token, "assistant", "moved")
            .unwrap();
        moved.finish_turn(token).unwrap();
        assert_eq!(moved.phase(), Phase::Idle);
    }

    #[test]
    fn legacy_start_invalidates_retained_owned_token() {
        for next_run in [7, 8] {
            let tape = Arc::new(TapeHost::default());
            let h = AgentHarness::new(tape.clone(), "m", "s");
            let (_, token) = h.start_turn(7).unwrap();
            h.finish_turn(token.clone()).unwrap();
            h.start_run(next_run).unwrap();
            let calls = tape.calls();
            assert!(h.message_end_owned(&token, "stale", "stale").is_err());
            assert!(h.save_point_owned(&token).is_err());
            assert!(h.abort_turn(token).is_err());
            assert_eq!(tape.calls(), calls);
            h.finish_run().unwrap();
        }
    }

    /// O1 residual: the plan's dedicated fixture shape — a retained token is
    /// refused after a LATER turn on the SAME run ID and on a NEW run ID,
    /// with zero host calls in both cases.
    #[test]
    fn o1_retained_token_after_later_turn_same_run_id_and_new_run_id() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        let (_, retained) = h.start_turn(7).unwrap();
        h.finish_turn(retained.clone()).unwrap();
        // Later turn on the SAME run ID: same run is not ownership.
        let (_, current) = h.start_turn(7).unwrap();
        let calls = tape.calls();
        assert!(h.message_end_owned(&retained, "{}", "stale-k").is_err());
        assert!(h.save_point_owned(&retained).is_err());
        assert!(h.finish_turn(retained.clone()).is_err());
        assert!(h.abort_turn(retained.clone()).is_err());
        assert_eq!(
            tape.calls(),
            calls,
            "stale token on the same run ID performs zero host calls"
        );
        h.finish_turn(current).unwrap();
        // Later turn on a DIFFERENT run ID.
        let (_, next) = h.start_turn(8).unwrap();
        let calls = tape.calls();
        assert!(h.message_end_owned(&retained, "{}", "stale-k2").is_err());
        assert!(h.save_point_owned(&retained).is_err());
        assert!(h.finish_turn(retained.clone()).is_err());
        assert!(h.abort_turn(retained).is_err());
        assert_eq!(
            tape.calls(),
            calls,
            "stale token on a new run ID performs zero host calls"
        );
        h.finish_turn(next).unwrap();
        assert_eq!(h.phase(), Phase::Idle);
    }

    #[test]
    fn overflow_preserves_all_start_state() {
        for legacy in [false, true] {
            let tape = Arc::new(TapeHost::default());
            let h = AgentHarness::new(tape.clone(), "m", "s");
            h.lock().turn_token = u64::MAX;
            let result = if legacy {
                h.start_run(7).map(|_| ())
            } else {
                h.start_turn(7).map(|_| ())
            };
            assert!(result.is_err());
            assert_eq!(h.lock().run_id, 0);
            assert_eq!(h.current_generation(), u64::MAX);
            assert_eq!(h.phase(), Phase::Idle);
            assert!(h.snapshot().is_none());
            assert!(tape.calls().is_empty());
        }
    }

    #[test]
    fn failed_start_publishes_no_usable_turn() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        *tape.fail_audit.lock().unwrap() = true;
        let err = h.start_run(7).unwrap_err();
        assert_eq!(err, HarnessError::Host("settlement refused".into()));
        assert_eq!(
            h.phase(),
            Phase::Idle,
            "failed start must not publish Running"
        );
        assert!(
            h.snapshot().is_none(),
            "no snapshot without committed start"
        );
        assert!(
            tape.delivered.lock().unwrap().is_empty(),
            "no delivery before start"
        );
        // Recovery: once the host accepts lifecycle evidence, the harness works.
        *tape.fail_audit.lock().unwrap() = false;
        h.start_run(7).unwrap();
        h.finish_run().unwrap();
    }

    #[test]
    fn failed_end_retains_state_and_refuses_restart_without_callbacks() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        let (_, token) = h.start_turn(7).unwrap();
        let snap = h.snapshot().unwrap();
        h.queue_write("residual", "{}", "kr").unwrap();
        let ran = Arc::new(StdMutex::new(false));
        {
            let ran = Arc::clone(&ran);
            h.run_when_idle(move || {
                if let Ok(mut g) = ran.lock() {
                    *g = true;
                }
            })
            .unwrap();
        }
        *tape.fail_audit.lock().unwrap() = true;
        assert!(h.finish_run().is_err(), "failed RunEnd must surface");
        assert_eq!(h.phase(), Phase::Failed, "terminal failed settlement");
        assert_eq!(
            h.snapshot().as_ref(),
            Some(&snap),
            "snapshot retained for diagnosis"
        );
        assert!(
            !*ran.lock().unwrap(),
            "callbacks must not run on failed settlement"
        );
        assert!(
            matches!(h.start_run(8), Err(HarnessError::PhaseBusy { .. })),
            "non-reusable failure state refuses a new turn"
        );
        *tape.fail_audit.lock().unwrap() = false;
        let calls = tape.calls();
        assert!(h.finish_run().is_err());
        assert!(h.abort().is_err());
        assert!(h.finish_turn(token.clone()).is_err());
        assert!(h.abort_turn(token.clone()).is_err());
        assert!(h.message_end_owned(&token, "late", "late").is_err());
        assert!(h.save_point_owned(&token).is_err());
        assert!(h.message_end("late", "legacy").is_err());
        assert!(h.save_point().is_err());
        assert_eq!(tape.calls(), calls);
        assert!(!*ran.lock().unwrap());
    }

    /// O1/L2 residual: queued A(ok)+B(fail)+C at settlement → terminal
    /// `Phase::Failed`, A acknowledged once, B+C retained byte-exact, no
    /// callbacks, and every retry path refuses with zero host calls.
    #[test]
    fn terminal_settlement_drain_failure_retains_fifo_suffix() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        let (_, token) = h.start_turn(7).unwrap();
        for (topic, payload, key) in [
            ("a", "{\"a\":1}", "ka"),
            ("b", " b bytes ", "kb"),
            ("c", "[3]", "kc"),
        ] {
            h.queue_write(topic, payload, key).unwrap();
        }
        let callback_ran = Arc::new(StdMutex::new(false));
        {
            let callback_ran = Arc::clone(&callback_ran);
            h.run_when_idle(move || {
                if let Ok(mut g) = callback_ran.lock() {
                    *g = true;
                }
            })
            .unwrap();
        }
        *tape.fail_key.lock().unwrap() = Some("kb".into());
        assert!(
            h.finish_run().is_err(),
            "settlement drain failure must surface as the primary error"
        );
        assert_eq!(h.phase(), Phase::Failed, "terminal failed settlement");
        assert!(h.snapshot().is_some(), "snapshot retained for diagnosis");
        // A was acknowledged exactly once; B and C are retained byte-exact.
        assert_eq!(
            *tape.delivered.lock().unwrap(),
            vec![(7, "a".into(), "{\"a\":1}".into(), "ka".into())]
        );
        assert_eq!(
            h.lock()
                .pending
                .iter()
                .map(|e| (
                    e.topic.as_str(),
                    e.payload_json.as_str(),
                    e.idempotency_key.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![("b", " b bytes ", "kb"), ("c", "[3]", "kc")],
            "FIFO suffix retained byte-exact"
        );
        // Every retry path refuses with zero further host calls.
        let calls = tape.calls();
        assert!(h.finish_run().is_err());
        assert!(h.abort().is_err());
        assert!(h.finish_turn(token.clone()).is_err());
        assert!(h.abort_turn(token.clone()).is_err());
        assert!(h.message_end_owned(&token, "late", "late").is_err());
        assert!(h.save_point_owned(&token).is_err());
        assert!(h.message_end("late", "legacy").is_err());
        assert!(h.save_point().is_err());
        assert!(matches!(
            h.start_turn(8),
            Err(HarnessError::PhaseBusy {
                phase: Phase::Failed,
                ..
            })
        ));
        assert!(matches!(
            h.start_run(8),
            Err(HarnessError::PhaseBusy { .. })
        ));
        assert_eq!(tape.calls(), calls, "retry paths perform zero host calls");
        assert!(
            !*callback_ran.lock().unwrap(),
            "no callbacks on failed settlement"
        );
        // Retention is diagnostic, never retry authority: even with the
        // fault removed the failed harness refuses to settle.
        *tape.fail_key.lock().unwrap() = None;
        let calls = tape.calls();
        assert!(h.finish_run().is_err());
        assert_eq!(tape.calls(), calls);
    }

    #[test]
    fn stale_and_cross_harness_tokens_cannot_mutate() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        let (_, token_a) = h.start_turn(7).unwrap();
        let stale = token_a.clone();
        h.finish_turn(token_a).unwrap();
        let (_, token_b) = h.start_turn(8).unwrap();
        assert!(matches!(
            h.message_end_owned(&stale, "{}", "k"),
            Err(HarnessError::Host(ref m)) if m == "stale_turn_token"
        ));
        assert!(matches!(
            h.save_point_owned(&stale),
            Err(HarnessError::Host(_))
        ));
        assert!(matches!(h.finish_turn(stale), Err(HarnessError::Host(_))));
        // The current token still settles its own turn.
        h.message_end_owned(&token_b, "assistant", "km").unwrap();
        h.finish_turn(token_b).unwrap();
        // Cross-harness: same generation value, different harness identity.
        let other = AgentHarness::new(tape.clone(), "m", "s");
        let (_, token_other) = other.start_turn(9).unwrap();
        assert!(matches!(
            h.message_end_owned(&token_other, "{}", "k2"),
            Err(HarnessError::Host(_))
        ));
        // The token is valid for ITS OWN harness and settles that turn.
        other.finish_turn(token_other).unwrap();
        assert_eq!(other.phase(), Phase::Idle);
    }

    #[test]
    fn two_active_harnesses_reject_foreign_equal_generation() {
        let tape = Arc::new(TapeHost::default());
        let a = AgentHarness::new(tape.clone(), "m", "s");
        let b = AgentHarness::new(tape.clone(), "m", "s");
        let (_, ta) = a.start_turn(7).unwrap();
        let (_, tb) = b.start_turn(7).unwrap();
        assert_eq!(a.phase(), Phase::Running);
        assert_eq!(b.phase(), Phase::Running);
        assert_eq!(ta.generation, tb.generation);
        let calls = tape.calls();
        assert!(a.message_end_owned(&tb, "foreign", "x").is_err());
        assert!(b.save_point_owned(&ta).is_err());
        assert!(a.abort_turn(tb.clone()).is_err());
        assert!(b.finish_turn(ta.clone()).is_err());
        assert_eq!(calls, tape.calls());
        a.finish_turn(ta).unwrap();
        b.finish_turn(tb).unwrap();
    }

    #[test]
    fn ambiguous_start_is_terminal_and_publishes_nothing() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        *tape.audit_error.lock().unwrap() = Some(crate::host::HostError::Internal(
            "synthetic unknown commit".into(),
        ));
        assert!(h.start_turn(7).is_err());
        assert_eq!(h.phase(), Phase::Failed);
        assert!(h.snapshot().is_none());
        assert_eq!(h.lock().run_id, 0);
        assert_eq!(h.current_generation(), 0);
        *tape.audit_error.lock().unwrap() = None;
        let calls = tape.calls();
        assert!(h.start_run(8).is_err());
        assert!(h.start_turn(8).is_err());
        assert!(h.message_end("late", "late").is_err());
        assert_eq!(tape.calls(), calls);
    }

    #[test]
    fn queued_write_refused_without_running_turn() {
        let h = harness();
        assert!(h.queue_write("late", "payload", "key").is_err());
        assert!(h.lock().pending.is_empty());
    }

    #[test]
    fn retained_token_does_not_address_replacement_harness() {
        let original = harness();
        let (_, old) = original.start_turn(7).unwrap();
        drop(original);
        let replacement = harness();
        let (_, current) = replacement.start_turn(7).unwrap();
        assert_eq!(old.generation, current.generation);
        assert!(replacement.abort_turn(old).is_err());
        replacement.finish_turn(current).unwrap();
    }

    #[test]
    fn refused_start_blocks_legacy_delivery() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        *tape.fail_audit.lock().unwrap() = true;
        assert!(h.start_run(7).is_err());
        let calls = tape.calls();
        assert!(h.message_end("late", "legacy").is_err());
        assert!(h.save_point().is_err());
        assert_eq!(calls, tape.calls());
        assert_eq!(h.lock().run_id, 0);
    }

    #[test]
    #[allow(clippy::panic)] // The unwind deliberately poisons the mutex.
    fn poisoned_harness_refuses_checked_start() {
        let h = harness();
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = h.inner.lock().unwrap();
            panic!("synthetic poison");
        }));
        assert!(poisoned.is_err());
        assert!(h.start_turn(7).is_err());
    }

    #[test]
    fn failed_start_creates_no_armed_guard() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        *tape.fail_audit.lock().unwrap() = true;
        assert!(h.start_turn(7).is_err());
        assert_eq!(h.current_generation(), 0, "no token minted on failed start");
        *tape.fail_audit.lock().unwrap() = false;
        let (_, token) = h.start_turn(7).unwrap();
        h.finish_turn(token).unwrap();
    }

    /// Slice 2 / O2 residual: a host panic inside the settlement path must
    /// leave no armed guard — the turn is terminalized before host code and
    /// every retry path refuses (the poisoned lock fails closed) with zero
    /// further host calls.
    #[test]
    #[allow(clippy::panic)] // The unwind deliberately poisons the mutex.
    fn unwind_during_settlement_leaves_no_armed_guard() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        let (_, token) = h.start_turn(7).unwrap();
        *tape.panic_audit.lock().unwrap() = true;
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = h.finish_turn(token.clone());
        }));
        assert!(unwound.is_err(), "the synthetic host panic must propagate");
        assert_eq!(h.phase(), Phase::Failed, "terminalized before host code");
        // No armed guard: retries refuse without a second host call.
        let calls = tape.calls();
        assert!(h.finish_turn(token.clone()).is_err());
        assert!(h.abort_turn(token).is_err());
        assert_eq!(tape.calls(), calls, "no duplicate settlement attempt");
    }

    /// Slice 2 / O4 residual: settlement on an already-Idle harness is a
    /// no-op — no duplicate RunEnd, zero host calls; the stale token refuses.
    #[test]
    fn no_duplicate_end_after_idle() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        let (_, token) = h.start_turn(7).unwrap();
        h.finish_turn(token.clone()).unwrap();
        let calls = tape.calls();
        assert!(h.finish_run().is_ok());
        assert!(h.abort().is_ok());
        assert!(h.finish_turn(token).is_err());
        assert_eq!(tape.calls(), calls, "no duplicate RunEnd after Idle");
    }

    #[test]
    fn token_overflow_refuses_instead_of_wrapping() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        // Exhaust the generation directly (same-module test access): the next
        // owned start must refuse, never wrap to generation 0/1.
        h.lock().turn_token = u64::MAX;
        assert!(
            matches!(
                h.start_turn(7),
                Err(HarnessError::Host(ref m)) if m == "turn_token_overflow"
            ),
            "exhaustion refuses"
        );
        assert_eq!(h.phase(), Phase::Idle, "failed start publishes nothing");
    }

    #[test]
    fn callback_panic_isolated_and_restart_from_callback_safe() {
        let tape = Arc::new(TapeHost::default());
        let h = Arc::new(AgentHarness::new(tape.clone(), "m", "s"));
        // Ordered trace: exact FIFO order, each callback exactly once, a
        // panic in the middle neither shifts nor duplicates the rest.
        let trace: Arc<StdMutex<Vec<&'static str>>> = Arc::new(StdMutex::new(Vec::new()));
        {
            let t1 = Arc::clone(&trace);
            h.run_when_idle(move || {
                if let Ok(mut g) = t1.lock() {
                    g.push("cb1");
                }
            })
            .unwrap();
            // Middle callback records its attempt, then panics.
            let t2 = Arc::clone(&trace);
            #[allow(clippy::panic)]
            h.run_when_idle(move || {
                if let Ok(mut g) = t2.lock() {
                    g.push("cb2-panicking");
                }
                panic!("synthetic callback panic");
            })
            .unwrap();
            let t3 = Arc::clone(&trace);
            h.run_when_idle(move || {
                if let Ok(mut g) = t3.lock() {
                    g.push("cb3");
                }
            })
            .unwrap();
            // A callback may start a later turn, including the same run ID: the
            // harness must not be locked or mid-settlement when callbacks run.
            let t4 = Arc::clone(&trace);
            let weak = Arc::downgrade(&h);
            h.run_when_idle(move || {
                let original = weak.upgrade().unwrap();
                let (_, token) = original.start_turn(7).unwrap();
                original.finish_turn(token).unwrap();
                if let Ok(mut g) = t4.lock() {
                    g.push("restart");
                }
            })
            .unwrap();
        }
        h.start_run(7).unwrap();
        h.finish_run().unwrap();
        assert_eq!(
            trace.lock().map(|g| g.clone()).unwrap_or_default(),
            vec!["cb1", "cb2-panicking", "cb3", "restart"],
            "exact FIFO once: the panic shifted or duplicated nothing"
        );
        assert!(
            h.callback_panicked(),
            "fixed panic diagnostic must be observable"
        );
        // The original harness is Idle and reusable after callback execution.
        assert_eq!(h.phase(), Phase::Idle);
        h.start_run(8).unwrap();
        h.abort().unwrap();
    }

    #[test]
    fn harness_lifecycle_uses_checked_host_methods() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        h.start_run(7).unwrap();
        h.message_end("assistant bytes", "km").unwrap();
        h.finish_run().unwrap();
        let calls = tape.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("settle_audit:") && c.contains("RunStart")),
            "RunStart must ride the checked lifecycle audit"
        );
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("settle_enqueue:") && c.contains(":km:")),
            "delivery must ride the checked enqueue"
        );
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("settle_audit:") && c.contains("RunEnd")),
            "RunEnd must ride the checked lifecycle audit"
        );
        assert!(
            !calls
                .iter()
                .any(|c| c.starts_with("enqueue:") || c.starts_with("audit:")),
            "no legacy void/unchecked lifecycle calls may remain: {:?}",
            calls
        );
    }

    fn harness() -> AgentHarness<TapeHost> {
        let h = AgentHarness::new(
            Arc::new(TapeHost::default()),
            "test-model",
            "you are a steward",
        );
        h.add_tool("read").unwrap();
        h
    }

    #[test]
    fn prompt_construction_includes_tools() {
        let h = harness();
        let snap = h.start_run(1).unwrap();
        assert_eq!(snap.model(), "test-model");
        assert_eq!(snap.tools(), ["read"]);
        assert!(snap.prompt().starts_with("you are a steward"));
        assert!(snap.prompt().contains("tools: read"));
    }

    #[test]
    fn snapshot_is_defensive_against_mid_turn_setters() {
        let h = harness();
        let snap = h.start_run(1).unwrap();
        h.set_model("next-model").unwrap();
        h.set_system_prompt("changed").unwrap();
        h.add_tool("bash").unwrap();
        // In-flight snapshot untouched...
        assert_eq!(h.snapshot().unwrap(), snap);
        assert_eq!(h.snapshot().unwrap().model(), "test-model");
        assert_eq!(h.snapshot().unwrap().tools().len(), 1);
        // ...next turn picks the changes up.
        h.finish_run().unwrap();
        let next = h.start_run(2).unwrap();
        assert_eq!(next.model(), "next-model");
        assert_eq!(next.tools(), ["read", "bash"]);
    }

    #[test]
    fn pending_write_ordering_after_message_end() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        h.start_run(7).unwrap();
        h.queue_write("summary", "{}", "k-summary").unwrap();
        h.queue_write("usage", "{}", "k-usage").unwrap();
        // message_end persists FIRST, then queued writes drain FIFO.
        h.message_end("{\"role\":\"assistant\"}", "k-msg").unwrap();
        let calls = tape.calls();
        let msg = calls
            .iter()
            .position(|c| c.contains("message_end"))
            .unwrap();
        assert!(calls[msg + 1..].len() >= 2, "drain happened");
        assert!(calls[msg + 1].contains("k-summary"));
        assert!(calls[msg + 2].contains("k-usage"));
        assert!(
            calls[..msg].iter().all(|c| !c.contains("enqueue")),
            "nothing flushed before message_end"
        );
    }

    #[test]
    fn pending_failure_retains_exact_fifo_suffix() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        h.start_run(7).unwrap();
        for (topic, payload, key) in [
            ("a", "{\"a\":1}", "ka"),
            ("b", " b bytes ", "kb"),
            ("c", "[3]", "kc"),
        ] {
            h.queue_write(topic, payload, key).unwrap();
        }
        let before = h.lock().pending.clone();
        *tape.fail_key.lock().unwrap() = Some("kb".into());
        assert_eq!(
            h.save_point(),
            Err(HarnessError::Host("internal: injected_delivery".into()))
        );
        assert_eq!(
            h.lock().pending.iter().cloned().collect::<Vec<_>>(),
            before.iter().skip(1).cloned().collect::<Vec<_>>()
        );
        assert!(!tape.calls().iter().any(|call| call.contains(":kc:")));
        *tape.fail_key.lock().unwrap() = None;
        assert_eq!(h.save_point().unwrap(), 2);
        assert_eq!(
            *tape.delivered.lock().unwrap(),
            vec![
                (7, "a".into(), "{\"a\":1}".into(), "ka".into()),
                (7, "b".into(), " b bytes ".into(), "kb".into()),
                (7, "c".into(), "[3]".into(), "kc".into()),
            ]
        );
        assert_eq!(h.save_point().unwrap(), 0);
    }

    #[test]
    fn assistant_failure_leaves_pending_and_false_receipt_acknowledges() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        h.start_run(7).unwrap();
        h.queue_write("pending", "exact bytes", "kp").unwrap();
        let before = h.lock().pending.clone();
        *tape.fail_key.lock().unwrap() = Some("km".into());
        assert!(h.message_end("assistant bytes", "km").is_err());
        assert_eq!(h.lock().pending, before);
        assert!(tape.delivered.lock().unwrap().is_empty());
        *tape.fail_key.lock().unwrap() = None;
        h.message_end("assistant bytes", "km").unwrap();
        h.queue_write("pending", "exact bytes", "kp").unwrap();
        assert_eq!(h.save_point().unwrap(), 1);
        assert_eq!(h.save_point().unwrap(), 0);
        let delivered = tape.delivered.lock().unwrap();
        assert_eq!(delivered.len(), 2);
        assert_eq!(delivered[0].1, "message_end");
        assert_eq!(delivered[1].1, "pending");
    }

    #[test]
    fn save_point_drains_and_refreshes_queue() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        h.start_run(3).unwrap();
        h.queue_write("a", "{}", "ka").unwrap();
        assert_eq!(h.save_point().unwrap(), 1);
        assert!(tape.calls().iter().any(|c| c.contains(":ka:")));
        // Queue is empty after the save-point; second flush is a no-op.
        assert_eq!(h.save_point().unwrap(), 0);
    }

    #[test]
    fn finish_and_abort_flush_residual_writes_once() {
        for (settle, status) in [("finish", "ok"), ("abort", "denied")] {
            let tape = Arc::new(TapeHost::default());
            let h = AgentHarness::new(tape.clone(), "m", "s");
            h.start_run(9).unwrap();
            h.queue_write("residual", "{}", "kr").unwrap();
            match settle {
                "finish" => h.finish_run().unwrap(),
                _ => h.abort().unwrap(),
            }
            let calls = tape.calls();
            assert!(
                calls
                    .iter()
                    .any(|c| c.contains(":kr:") && c.contains("enqueue"))
            );
            // Audit rows read `…/{target}:{status}/{detail}`.
            assert!(
                calls
                    .iter()
                    .any(|c| c.contains("RunStart") && c.contains(":ok/"))
            );
            assert!(
                calls
                    .iter()
                    .any(|c| c.contains("RunEnd") && c.contains(&format!(":{status}/")))
            );
            assert_eq!(h.phase(), Phase::Idle);
        }
    }

    #[test]
    fn phase_gates_reject_structural_ops_but_allow_steering() {
        let h = harness();
        // Idle allows structural ops.
        assert!(h.compact().is_ok());
        assert!(h.set_leaf_id("n1").is_ok());
        h.start_run(1).unwrap();
        let errs = [
            h.compact().unwrap_err(),
            h.set_leaf_id("n2").unwrap_err(),
            h.navigate_tree("n3").unwrap_err(),
            h.start_run(4).unwrap_err(),
        ];
        for err in errs {
            assert!(
                matches!(
                    err,
                    HarnessError::PhaseBusy {
                        phase: Phase::Running,
                        ..
                    }
                ),
                "expected PhaseBusy(Running), got {err}"
            );
        }
        // Steering/follow-up stay legal mid-turn.
        assert!(h.steer("slow down").is_ok());
        h.follow_up("check again").unwrap();
        assert_eq!(h.follow_ups(), ["slow down", "check again"]);
        h.abort().unwrap();
        // Steering once idle is refused.
        assert!(matches!(h.steer("x"), Err(HarnessError::PhaseBusy { .. })));
    }

    #[test]
    fn run_when_idle_defers_until_settlement_order() {
        let order: Arc<StdMutex<Vec<&'static str>>> = Arc::new(StdMutex::new(Vec::new()));
        let h = harness();
        {
            let o = order.clone();
            h.run_when_idle(move || {
                if let Ok(mut g) = o.lock() {
                    g.push("deferred-a");
                }
            })
            .unwrap();
        }
        {
            let o = order.clone();
            h.run_when_idle(move || {
                if let Ok(mut g) = o.lock() {
                    g.push("deferred-b");
                }
            })
            .unwrap();
        }
        h.start_run(5).unwrap();
        h.finish_run().unwrap();
        assert_eq!(
            order.lock().map(|g| g.clone()).unwrap_or_default(),
            vec!["deferred-a", "deferred-b"]
        );
        // Settlement order: RunEnd audited before deferred work runs.
        // (Guaranteed by construction: drain+audit happen under lock, then
        // deferred ops run after release.)
    }

    #[test]
    fn non_main_lanes_read_but_cannot_run() {
        let h = harness();
        let main = AgentLane::main();
        let side = AgentLane::side("observer");
        h.start_run(11).unwrap();

        let side_view = h.lane_handle(&side).view();
        assert_eq!(side_view.0, Phase::Running);
        assert!(side_view.1.is_some());

        assert!(matches!(
            h.lane_handle(&side).start_run(12),
            Err(HarnessError::LaneNotMain { .. })
        ));
        assert!(matches!(
            h.lane_handle(&side).compact(),
            Err(HarnessError::LaneNotMain { .. })
        ));
        assert!(matches!(
            h.lane_handle(&side).abort(),
            Err(HarnessError::LaneNotMain { .. })
        ));
        let _ = h.lane_handle(&main); // main lane exists for symmetry
        h.abort().unwrap();
    }
}
