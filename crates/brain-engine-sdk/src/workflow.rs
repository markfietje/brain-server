//! The WorkflowEngine seam: dsh-workflow-shaped, one engine per context.
//!
//! Invariants: meta is validated as DATA before any script evaluation (a
//! script is never evaluated to obtain its own metadata); a [`WorkflowRun`]'s
//! `result` never rejects — failure resolves to `stop_reason: error|cancelled`;
//! cancel and dispose settle within bounded grace even when the underlying
//! script never settles (the handle force-completes, the engine-level mirror
//! of force-terminating the worker). Events are observe-only DATA SNAPSHOTS:
//! id + meta clones, never the live run. Honest ceiling: script trust equals
//! bash trust; the worker thread is a serialization boundary, not a security
//! boundary.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll, Waker};
use std::time::Duration;

/// Canonical JSON carried as text; hosts own parsing/validation at their
/// boundaries (two-layer envelope rule). The SDK stays dependency-free.
pub type Json = String;

/// Default bounded grace for cancel/dispose settlement.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(5);

/// Run-concurrency ceiling: `maxTotalAgents`. A start that would exceed it is
/// refused, never queued unbounded.
pub const MAX_TOTAL_AGENTS: usize = 16;

// -- identity ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunId(pub String);

// -- meta (validated as data) ------------------------------------------------

/// Declared workflow metadata. Validated BEFORE any script evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowMeta {
    pub name: String,
    pub description: String,
    /// When to surface this workflow as an agent option (observer vocabulary).
    pub when_to_use: Option<String>,
    /// Observer vocabulary only: `phase()` matches a title here; no execution
    /// structure is implied.
    pub phases: Vec<String>,
}

/// Meta-validation failure vocabulary (data-shaped, before any eval).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetaError {
    EmptyName,
    NameTooLong,
    DescriptionTooLong,
    TooManyPhases,
    EmptyPhaseName,
}

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetaError::EmptyName => write!(f, "meta name empty"),
            MetaError::NameTooLong => write!(f, "meta name exceeds 128 chars"),
            MetaError::DescriptionTooLong => write!(f, "description exceeds 1024 chars"),
            MetaError::TooManyPhases => write!(f, "more than 32 phases"),
            MetaError::EmptyPhaseName => write!(f, "empty phase name"),
        }
    }
}

impl Error for MetaError {}

impl WorkflowMeta {
    /// Validate as data. Bounds keep prompt cards and event payloads bounded.
    pub fn validate(&self) -> Result<(), MetaError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(MetaError::EmptyName);
        }
        if name.len() > 128 {
            return Err(MetaError::NameTooLong);
        }
        if self.description.len() > 1024 {
            return Err(MetaError::DescriptionTooLong);
        }
        if self.phases.len() > 32 {
            return Err(MetaError::TooManyPhases);
        }
        if self.phases.iter().any(|p| p.trim().is_empty()) {
            return Err(MetaError::EmptyPhaseName);
        }
        Ok(())
    }
}

// -- request / result / error -----------------------------------------------

#[derive(Debug, Clone)]
pub struct WorkflowStartRequest {
    pub script: String,
    pub meta: WorkflowMeta,
    pub args: Json,
    pub parent: AgentId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopReason {
    Completed,
    Error,
    Cancelled,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            StopReason::Completed => "completed",
            StopReason::Error => "error",
            StopReason::Cancelled => "cancelled",
        }
    }
}

/// Terminal run outcome. Failure is a VALUE here — the future never yields
/// `Err`, so consumers cannot panic on unwrap.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct WorkflowResult {
    pub stop_reason: StopReason,
    /// Absent on `workflow/end` snapshots and on cancelled runs.
    pub output: Option<Json>,
}

impl WorkflowResult {
    pub fn completed(output: Json) -> Self {
        WorkflowResult {
            stop_reason: StopReason::Completed,
            output: Some(output),
        }
    }
    pub fn error() -> Self {
        WorkflowResult {
            stop_reason: StopReason::Error,
            output: None,
        }
    }
    pub fn cancelled() -> Self {
        WorkflowResult {
            stop_reason: StopReason::Cancelled,
            output: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkflowError {
    /// Refused before publish: meta failed data validation.
    MetaInvalid(MetaError),
    /// Capability/budget refusal (e.g. over `MAX_TOTAL_AGENTS`).
    Denied(String),
    Internal(String),
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkflowError::MetaInvalid(e) => write!(f, "meta invalid: {e}"),
            WorkflowError::Denied(m) => write!(f, "denied: {m}"),
            WorkflowError::Internal(m) => write!(f, "internal: {m}"),
        }
    }
}

impl Error for WorkflowError {}

// -- shared run state --------------------------------------------------------

struct RunShared {
    result: Option<WorkflowResult>,
    children: usize,
    cancelled: bool,
    waker: Option<Waker>,
}

fn lock<T>(g: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // Poisoned lock = recover the inner value (events.rs precedent): a
    // panicking listener must not wedge the run's settlement path.
    g.lock().unwrap_or_else(|p| p.into_inner())
}

type Shared = Arc<Mutex<RunShared>>;

/// Engine-side half of a started run: resolve it exactly once.
pub struct RunCompleter {
    shared: Shared,
}

impl RunCompleter {
    /// Resolve the run. First call wins; later calls are ignored (once
    /// semantics), so double-settlement cannot fork the outcome.
    pub fn complete(&self, result: WorkflowResult) {
        let mut s = lock(&self.shared);
        if s.result.is_none() {
            s.result = Some(result);
        }
        if let Some(w) = s.waker.take() {
            w.wake();
        }
    }

    /// Child-run accounting feeding dispose quiescence.
    pub fn child_started(&self) {
        lock(&self.shared).children += 1;
    }
    pub fn child_finished(&self) {
        let mut s = lock(&self.shared);
        s.children = s.children.saturating_sub(1);
    }
}

/// Never-rejecting once-future over the terminal result.
pub struct WorkflowResultFuture {
    shared: Shared,
}

impl Future for WorkflowResultFuture {
    type Output = WorkflowResult;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let mut s = lock(&self.shared);
        match &s.result {
            Some(r) => Poll::Ready(r.clone()),
            None => {
                s.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

// -- handles -----------------------------------------------------------------

/// Cooperative cancellation signal plus the bounded-settle guarantee.
#[derive(Clone)]
pub struct CancelHandle {
    shared: Shared,
    grace: Duration,
}

impl CancelHandle {
    /// Non-blocking cooperative signal. Engines observe it via
    /// [`CancelHandle::is_cancelled`].
    pub fn cancel(&self) {
        lock(&self.shared).cancelled = true;
    }

    pub fn is_cancelled(&self) -> bool {
        lock(&self.shared).cancelled
    }

    /// Override the settle grace (tests use small values).
    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// Cancel AND guarantee settlement within `grace`: wait for the engine,
    /// then force-complete as cancelled (the worker was presumed dead).
    pub fn cancel_blocking(self) -> WorkflowResult {
        {
            let mut s = lock(&self.shared);
            s.cancelled = true;
        }
        let deadline = std::time::Instant::now() + self.grace;
        while std::time::Instant::now() < deadline {
            {
                let s = lock(&self.shared);
                if let Some(r) = &s.result {
                    return r.clone();
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let completer = RunCompleter {
            shared: Arc::clone(&self.shared),
        };
        completer.complete(WorkflowResult::cancelled());
        WorkflowResult::cancelled()
    }
}

/// `cancel + bounded settle + child quiescence`, never hangs.
pub struct DisposeHandle {
    shared: Shared,
    grace: Duration,
}

impl DisposeHandle {
    /// Override the settle grace (tests use small values).
    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// Dispose: cooperative-cancel, wait bounded grace for the result AND
    /// child quiescence, then force-complete as cancelled. Never hangs past
    /// `grace`.
    pub fn dispose(self) -> WorkflowResult {
        let cancel = CancelHandle {
            shared: Arc::clone(&self.shared),
            grace: self.grace,
        };
        let _ = cancel.cancel_blocking();
        let deadline = std::time::Instant::now() + self.grace;
        loop {
            {
                let s = lock(&self.shared);
                if s.children == 0 {
                    break;
                }
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        WorkflowResult::cancelled()
    }
}

// -- run ---------------------------------------------------------------------

/// Holder-owned run handle. Dropping a run does NOT block; call
/// `dispose.dispose()` explicitly for the quiescing teardown.
pub struct WorkflowRun {
    pub id: RunId,
    pub result: WorkflowResultFuture,
    pub cancel: CancelHandle,
    pub dispose: DisposeHandle,
}

// -- the seam ------------------------------------------------------------------

/// One engine per context: the host mounts at most one; a second mount
/// replaces the first via config (Cordis mount, never parallel providers).
pub trait WorkflowEngine: Send + Sync {
    /// Validate-as-data then start. Implementations must refuse
    /// ([`WorkflowError::MetaInvalid`]) BEFORE evaluating `script`.
    fn start(&self, req: WorkflowStartRequest) -> Result<WorkflowRun, WorkflowError>;
}

/// Shared scaffolding for engine implementations: validate meta as data,
/// enforce the concurrency bound, and hand back the holder-owned run.
pub struct RunBuilder {
    next_id: u64,
    active: usize,
    max_total_agents: usize,
}

impl Default for RunBuilder {
    fn default() -> Self {
        RunBuilder {
            next_id: 0,
            active: 0,
            max_total_agents: MAX_TOTAL_AGENTS,
        }
    }
}

impl RunBuilder {
    /// Pre-publish gate every engine calls first: meta validation + bound.
    /// Returns the allocated [`RunId`] on success.
    pub fn admit(&mut self, req: &WorkflowStartRequest) -> Result<RunId, WorkflowError> {
        req.meta.validate().map_err(WorkflowError::MetaInvalid)?;
        if self.active >= self.max_total_agents {
            return Err(WorkflowError::Denied(format!(
                "maxTotalAgents ({}) reached",
                self.max_total_agents
            )));
        }
        self.active += 1;
        self.next_id += 1;
        Ok(RunId(format!("wf-{}", self.next_id)))
    }

    pub fn released(&mut self) {
        self.active = self.active.saturating_sub(1);
    }

    pub fn build_run(&self, id: RunId) -> (Arc<RunCompleter>, WorkflowRun) {
        let shared: Shared = Arc::new(Mutex::new(RunShared {
            result: None,
            children: 0,
            cancelled: false,
            waker: None,
        }));
        let completer = Arc::new(RunCompleter {
            shared: Arc::clone(&shared),
        });
        let run = WorkflowRun {
            id,
            result: WorkflowResultFuture {
                shared: Arc::clone(&shared),
            },
            cancel: CancelHandle {
                shared: Arc::clone(&shared),
                grace: DEFAULT_GRACE,
            },
            dispose: DisposeHandle {
                shared,
                grace: DEFAULT_GRACE,
            },
        };
        (completer, run)
    }
}

// -- event snapshots (observe-only DATA) --------------------------------------

/// `workflow/start` snapshot: id + meta clone, never the live run or script.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowStartedSnapshot {
    pub id: RunId,
    pub name: String,
    pub parent: String,
}

/// `workflow/phase` snapshot (`phase()` matches a declared title).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowPhaseSnapshot {
    pub id: String,
    pub phase: String,
}

/// `workflow/log` snapshot (bounded line).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowLogSnapshot {
    pub id: String,
    pub line: String,
}

/// `workflow/agent-start` / `agent-end` snapshots.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowAgentSnapshot {
    pub id: String,
    pub agent: String,
}

/// `workflow/end` snapshot — omits the result VALUE by design.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowEndSnapshot {
    pub id: String,
    pub stop_reason: &'static str,
}

/// Publish the observe-only lifecycle events through the contained emitter.
/// Each listener receives a cloned payload and panics are contained there
/// (dsh containment mirror); a throwing subscriber cannot starve later ones.
pub mod events {
    use super::{
        RunId, WorkflowAgentSnapshot, WorkflowEndSnapshot, WorkflowLogSnapshot,
        WorkflowPhaseSnapshot, WorkflowStartedSnapshot,
    };
    use crate::events::Hooks;

    pub const START: &str = "workflow/start";
    pub const PHASE: &str = "workflow/phase";
    pub const LOG: &str = "workflow/log";
    pub const AGENT_START: &str = "workflow/agent-start";
    pub const AGENT_END: &str = "workflow/agent-end";
    pub const END: &str = "workflow/end";

    pub fn started(hooks: &Hooks, id: &RunId, name: &str, parent: &str) {
        hooks.emit(
            START,
            &WorkflowStartedSnapshot {
                id: id.clone(),
                name: name.to_string(),
                parent: parent.to_string(),
            },
        );
    }
    pub fn phase(hooks: &Hooks, id: &RunId, phase: &str) {
        hooks.emit(
            PHASE,
            &WorkflowPhaseSnapshot {
                id: id.0.clone(),
                phase: phase.to_string(),
            },
        );
    }
    pub fn log(hooks: &Hooks, id: &RunId, line: &str) {
        hooks.emit(
            LOG,
            &WorkflowLogSnapshot {
                id: id.0.clone(),
                line: line.to_string(),
            },
        );
    }
    pub fn agent_start(hooks: &Hooks, id: &RunId, agent: &str) {
        hooks.emit(
            AGENT_START,
            &WorkflowAgentSnapshot {
                id: id.0.clone(),
                agent: agent.to_string(),
            },
        );
    }
    pub fn agent_end(hooks: &Hooks, id: &RunId, agent: &str) {
        hooks.emit(
            AGENT_END,
            &WorkflowAgentSnapshot {
                id: id.0.clone(),
                agent: agent.to_string(),
            },
        );
    }
    pub fn end(hooks: &Hooks, id: &RunId, stop_reason: &'static str) {
        hooks.emit(
            END,
            &WorkflowEndSnapshot {
                id: id.0.clone(),
                stop_reason,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Hooks;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    /// Test engine: validates meta via the shared admit gate, then either
    /// completes inline or (script contains "hang") never settles.
    struct TestEngine {
        builder: StdMutex<RunBuilder>,
        script_evaluated: std::sync::atomic::AtomicBool,
    }
    impl Default for TestEngine {
        fn default() -> Self {
            TestEngine {
                builder: StdMutex::new(RunBuilder::default()),
                script_evaluated: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }
    impl WorkflowEngine for TestEngine {
        fn start(&self, req: WorkflowStartRequest) -> Result<WorkflowRun, WorkflowError> {
            let id = {
                let mut b = self.builder.lock().unwrap_or_else(|p| p.into_inner());
                b.admit(&req)?
            };
            let (completer, run) = {
                let b = self.builder.lock().unwrap_or_else(|p| p.into_inner());
                b.build_run(id)
            };
            if !req.script.contains("hang") {
                // The engine observes cancel only between "steps"; a normal
                // script completes.
                completer.complete(WorkflowResult::completed("done".into()));
                let mut b = self.builder.lock().unwrap_or_else(|p| p.into_inner());
                b.released();
            } else if req.script.contains("cancel-aware") {
                let c = completer.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(5));
                    c.complete(WorkflowResult::cancelled());
                });
            }
            self.script_evaluated
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(run)
        }
    }

    fn req(script: &str) -> WorkflowStartRequest {
        WorkflowStartRequest {
            script: script.into(),
            meta: WorkflowMeta {
                name: "demo".into(),
                description: "a demo workflow".into(),
                when_to_use: None,
                phases: vec!["intake".into()],
            },
            args: "{}".into(),
            parent: AgentId("parent-1".into()),
        }
    }

    #[test]
    fn meta_invalid_refused_before_any_script_evaluation() {
        let eng = TestEngine::default();
        let mut bad = req("print hi");
        bad.meta.name = "  ".into();
        assert!(matches!(
            eng.start(bad),
            Err(WorkflowError::MetaInvalid(MetaError::EmptyName))
        ));
        assert!(
            !eng.script_evaluated
                .load(std::sync::atomic::Ordering::SeqCst),
            "the script must never be evaluated to obtain/validate meta"
        );
    }

    #[test]
    fn result_never_rejects_failure_is_a_value() {
        let mut b = RunBuilder::default();
        let r = req("boom");
        let id = b.admit(&r).unwrap();
        let (completer, run) = b.build_run(id);
        completer.complete(WorkflowResult::error());
        let out = futures_block_on(run.result);
        assert_eq!(out.stop_reason, StopReason::Error);
        assert!(out.output.is_none());
    }

    fn futures_block_on(f: impl Future<Output = WorkflowResult>) -> WorkflowResult {
        // Minimal block-on for tests: poll with a real thread-park waker.
        std::pin::pin!(f)
            .now_or_never_poll()
            .expect("test future should resolve")
    }

    trait NowOrNeverPoll {
        type Out;
        fn now_or_never_poll(self) -> Option<Self::Out>;
    }
    impl<F: Future> NowOrNeverPoll for F {
        type Out = F::Output;
        fn now_or_never_poll(self) -> Option<Self::Out> {
            let mut pinned = std::pin::pin!(self);
            let pinned = &mut pinned;
            use std::task::Wake;
            struct ThreadWaker(std::thread::Thread);
            impl Wake for ThreadWaker {
                fn wake(self: Arc<Self>) {
                    self.0.unpark();
                }
            }
            let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
            let mut cx = TaskContext::from_waker(&waker);
            loop {
                match pinned.as_mut().poll(&mut cx) {
                    Poll::Ready(v) => return Some(v),
                    Poll::Pending => std::thread::park(),
                }
            }
        }
    }

    #[test]
    fn cancel_settles_within_bounded_grace_even_if_script_never_settles() {
        let eng = TestEngine::default();
        let mut run = eng.start(req("hang forever")).unwrap();
        run.dispose = run.dispose.with_grace(Duration::from_millis(50));
        let started = std::time::Instant::now();
        let res = run
            .cancel
            .clone()
            .with_grace(Duration::from_millis(50))
            .cancel_blocking();
        assert_eq!(res.stop_reason, StopReason::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(2), "must not hang");
    }

    #[test]
    fn dispose_waits_for_child_quiescence_then_returns() {
        let mut b = RunBuilder::default();
        let r = req("with children");
        let id = b.admit(&r).unwrap();
        let (completer, run) = b.build_run(id);
        let mut run = run;
        run.dispose = run.dispose.with_grace(Duration::from_millis(200));
        completer.child_started();
        completer.complete(WorkflowResult::completed("x".into()));
        std::thread::spawn({
            let c = completer.clone();
            move || {
                std::thread::sleep(Duration::from_millis(20));
                c.child_finished();
            }
        });
        let t0 = std::time::Instant::now();
        run.dispose.dispose();
        assert!(
            t0.elapsed() >= Duration::from_millis(15),
            "dispose waited for child quiescence"
        );
    }

    /// A throwing subscriber — the whole point of the containment test.
    #[allow(clippy::panic)]
    fn explode() -> crate::events::Verdict {
        panic!("listener explodes")
    }

    #[test]
    fn event_snapshots_are_data_and_survive_a_panicking_subscriber() {
        let hooks = Hooks::new();
        hooks
            .on(
                events::START,
                "test-panic",
                |_p: WorkflowStartedSnapshot| explode(),
            )
            .ok();
        let seen: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let seen2 = Arc::clone(&seen);
        hooks
            .on(
                events::START,
                "test-observe",
                move |p: WorkflowStartedSnapshot| {
                    if let Ok(mut g) = seen2.lock() {
                        g.push(p.name.clone());
                    }
                    crate::events::Verdict::Allow
                },
            )
            .ok();
        events::started(&hooks, &RunId("wf-1".into()), "demo", "parent");
        let report = hooks.emit(
            events::END,
            &WorkflowEndSnapshot {
                id: "wf-1".into(),
                stop_reason: "completed",
            },
        );
        assert_eq!(report.panicked(), 0, "panics are contained at dispatch");
        assert_eq!(
            seen.lock().map(|g| g.clone()).unwrap_or_default(),
            vec!["demo".to_string()],
            "a panicking subscriber did not starve the later listener"
        );
    }

    #[test]
    fn concurrency_bound_denies_not_queues() {
        let mut b = RunBuilder::default();
        let mut last_err = None;
        let mut started = 0;
        for _ in 0..MAX_TOTAL_AGENTS + 1 {
            match b.admit(&req("hang")) {
                Ok(_) => started += 1,
                Err(e) => last_err = Some(e),
            }
        }
        assert_eq!(started, MAX_TOTAL_AGENTS);
        assert!(matches!(last_err, Some(WorkflowError::Denied(_))));
    }

    // -- v1.28.17 Settle: settlement algebra as pure properties ---------------

    #[test]
    fn result_never_rejects_any_terminal_path() {
        const VOCAB: [&str; 3] = ["completed", "error", "cancelled"];
        // Exhaustive over the terminal constructor set: every path lands in
        // the dsh stop-reason union, and the future's Output carries no Err
        // channel (failure IS a value).
        let terminals = [
            WorkflowResult::completed("{\"done\":true}".into()),
            WorkflowResult::error(),
            WorkflowResult::cancelled(),
        ];
        for expected in &terminals {
            assert!(VOCAB.contains(&expected.stop_reason.as_str()));
            let mut b = RunBuilder::default();
            let id = b.admit(&req("any")).unwrap();
            let (completer, run) = b.build_run(id);
            completer.complete(expected.clone());
            let got = futures_block_on(run.result);
            assert_eq!(got, *expected);
        }
        // Cancel-after-terminal never overrides a settled result.
        let mut b = RunBuilder::default();
        let id = b.admit(&req("any")).unwrap();
        let (completer, run) = b.build_run(id);
        completer.complete(WorkflowResult::completed("x".into()));
        run.cancel.cancel();
        let got = futures_block_on(run.result);
        assert_eq!(got.stop_reason, StopReason::Completed);
    }

    /// The tick model: script progress measured in abstract ticks; a hanging
    /// script advances ticks forever without settling.
    struct TickModel {
        grace_ticks: u64,
        hanging: bool,
    }

    impl TickModel {
        /// Mirror of `CancelHandle`: cooperative signal first; an engine that
        /// observes it settles on its own, otherwise the abort path
        /// force-completes as cancelled AT the grace bound — never past it.
        fn settle_after_cancel(&self) -> (StopReason, u64) {
            let mut tick = 0u64;
            while tick < self.grace_ticks {
                tick += 1;
                if !self.hanging && tick >= 3 {
                    return (StopReason::Completed, tick);
                }
            }
            (StopReason::Cancelled, tick)
        }
    }

    #[test]
    fn cancel_settles_within_bounded_grace_under_tick_model() {
        // A hanging script: cancel must resolve BY the grace bound via the
        // abort path, with stop reason cancelled.
        let hang = TickModel {
            grace_ticks: 5,
            hanging: true,
        };
        let (reason, at) = hang.settle_after_cancel();
        assert_eq!(reason, StopReason::Cancelled);
        assert_eq!(at, hang.grace_ticks, "settled exactly at the bound");
        // A well-behaved script settles cooperatively BEFORE the bound.
        let coop = TickModel {
            grace_ticks: 5,
            hanging: false,
        };
        let (reason, at) = coop.settle_after_cancel();
        assert_eq!(reason, StopReason::Completed);
        assert!(at < coop.grace_ticks);

        // And the real seam agrees: wall-clock cancel_blocking on a
        // never-settling run returns cancelled within its grace.
        let mut b = RunBuilder::default();
        let id = b.admit(&req("hang forever")).unwrap();
        let (_completer, run) = b.build_run(id);
        let t0 = std::time::Instant::now();
        let res = run
            .cancel
            .clone()
            .with_grace(Duration::from_millis(40))
            .cancel_blocking();
        assert_eq!(res.stop_reason, StopReason::Cancelled);
        assert!(
            t0.elapsed() >= Duration::from_millis(40),
            "the abort path waited out the grace before forcing"
        );
        assert!(t0.elapsed() < Duration::from_secs(2), "never hangs");
    }

    #[test]
    fn dispose_waits_for_child_quiescence_within_bound() {
        // Child A settles at tick 3; child B never settles. Dispose =
        // cancel + bounded settle + child quiescence: resolves AT the bound,
        // records every child with its stop-reason, none left Running.
        #[derive(Debug, PartialEq)]
        enum ChildState {
            Running,
            Settled(&'static str),
        }
        let grace_ticks = 6u64;
        let mut tick = 0u64;
        let mut children = [
            ("child-a", ChildState::Running, Some(3u64)),
            ("child-b", ChildState::Running, None),
        ];
        let mut parent_settled = false;
        while tick <= grace_ticks {
            tick += 1;
            for (_, st, settles_at) in children.iter_mut() {
                if *st == ChildState::Running
                    && let Some(at) = settles_at
                    && tick >= *at
                {
                    *st = ChildState::Settled("cancelled");
                }
            }
            if tick >= grace_ticks {
                // The bound: force-complete whatever is left.
                for (_, st, _) in children.iter_mut() {
                    if *st == ChildState::Running {
                        *st = ChildState::Settled("cancelled");
                    }
                }
                parent_settled = true;
                break;
            }
        }
        assert!(parent_settled, "dispose resolved at the bound");
        assert!(
            children.iter().all(|(_, st, _)| *st != ChildState::Running),
            "no child left running: {children:?}"
        );
        assert_eq!(
            children[0].1,
            ChildState::Settled("cancelled"),
            "the settling child kept its own stop-reason"
        );
        assert_eq!(tick, grace_ticks, "dispose took exactly the bound");

        // Real-seam mirror: quiescence waits, the bound caps it, and the
        // result is forced to cancelled either way.
        let mut b = RunBuilder::default();
        let id = b.admit(&req("with children")).unwrap();
        let (completer, run) = b.build_run(id);
        let mut run = run;
        run.dispose = run.dispose.with_grace(Duration::from_millis(60));
        completer.child_started();
        completer.complete(WorkflowResult::completed("x".into()));
        let t0 = std::time::Instant::now();
        let res = run.dispose.dispose();
        assert_eq!(res.stop_reason, StopReason::Cancelled);
        assert!(t0.elapsed() < Duration::from_secs(2), "never hangs");
    }

    #[test]
    fn events_are_cloned_per_listener_and_throw_contained() {
        let hooks = Hooks::new();
        let seen_b: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let sb = Arc::clone(&seen_b);
        // Listener A: mutates ITS copy and throws (Deny verdict = the model's
        // Err-in-listener).
        hooks
            .on(events::LOG, "thrower", move |p: WorkflowLogSnapshot| {
                let mut mine = p.clone();
                mine.line.push_str(" TAMPERED");
                assert!(mine.line.ends_with("TAMPERED"));
                crate::events::Verdict::Deny("listener exploded".into())
            })
            .ok();
        // Listener B: registered AFTER the thrower — must still receive the
        // ORIGINAL payload unchanged.
        hooks
            .on(events::LOG, "observer", move |p: WorkflowLogSnapshot| {
                if let Ok(mut g) = sb.lock() {
                    g.push(p.line.clone());
                }
                crate::events::Verdict::Allow
            })
            .ok();
        let id = RunId("wf-9".into());
        events::log(&hooks, &id, "original payload");
        let report = hooks.emit(
            events::LOG,
            &WorkflowLogSnapshot {
                id: "wf-9".into(),
                line: "second dispatch".into(),
            },
        );
        assert_eq!(report.ran(), 1, "exactly the observer ran clean");
        assert_eq!(
            report.panicked(),
            0,
            "a Deny verdict is contained, not a starve"
        );
        assert_eq!(
            seen_b.lock().map(|g| g.clone()).unwrap_or_default(),
            vec![
                "original payload".to_string(),
                "second dispatch".to_string()
            ],
            "listener B saw pristine clones despite A's mutation + throw, every emit"
        );
        // The emit call itself succeeded — snapshots are data, dispatch is
        // broadcast-observe: later listeners are never starved.
    }

    #[test]
    fn admission_enforces_max_total_agents_16_and_released_slots_readmit() {
        let mut b = RunBuilder::default();
        let mut held_ids = Vec::new();
        // The 17th concurrent admission is refused regardless of arguments.
        for i in 0..MAX_TOTAL_AGENTS {
            let mut r = req("hang");
            r.script = format!("script-{i}");
            r.args = format!("{{\"n\":{i}}}");
            r.meta.name = format!("wf-{i}");
            held_ids.push(b.admit(&r).unwrap());
        }
        for variant in [
            ("totally different", "{\"x\":1}", "other-name"),
            ("", "{}", "z"),
        ] {
            let mut r = req(variant.0);
            r.args = variant.1.into();
            r.meta.name = variant.2.into();
            assert!(
                matches!(b.admit(&r), Err(WorkflowError::Denied(_))),
                "over-cap refused regardless of arguments"
            );
        }
        // Released slots readmit.
        b.released();
        let r = req("fresh");
        let id = b.admit(&r).expect("released slot readmits");
        assert_ne!(id, RunId(String::new()));
        assert_eq!(held_ids.len(), MAX_TOTAL_AGENTS);
    }
}
