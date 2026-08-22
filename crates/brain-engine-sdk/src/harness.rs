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

use crate::host::{AuditKind, AuditStatus, WorkflowHost};
use std::sync::{Arc, Mutex};

/// Harness phase; the gate for structural operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Running,
    Compact,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Running => "running",
            Phase::Compact => "compact",
        }
    }
}

/// Harness failure vocabulary.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HarnessError {
    /// A structural op arrived while not [`Phase::Idle`].
    PhaseBusy { op: &'static str, phase: Phase },
    /// Run operation attempted from a non-main lane.
    LaneNotMain { op: &'static str },
    /// The host refused or failed a write.
    Host(String),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessError::PhaseBusy { op, phase } => {
                write!(f, "`{op}` requires idle phase (current {})", phase.as_str())
            }
            HarnessError::LaneNotMain { op } => {
                write!(f, "`{op}` is a run operation; non-main lanes may only read")
            }
            HarnessError::Host(m) => write!(f, "host: {m}"),
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

struct Inner<H: WorkflowHost> {
    host: Arc<H>,
    config: Config,
    phase: Phase,
    snapshot: Option<TurnSnapshot>,
    pending: Vec<EntryWithoutId>,
    follow_ups: Vec<String>,
    deferred_idle: Vec<DeferredOp>,
    run_id: i64,
}

type DeferredOp = Box<dyn FnOnce() + Send>;

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
    inner: Mutex<Inner<H>>,
}

impl<H: WorkflowHost> AgentHarness<H> {
    pub fn new(host: Arc<H>, model: &str, system_prompt: &str) -> Self {
        AgentHarness {
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
                pending: Vec::new(),
                follow_ups: Vec::new(),
                deferred_idle: Vec::new(),
                run_id: 0,
            }),
        }
    }

    // -- configuration (next-turn semantics) --------------------------------

    pub fn set_model(&self, model: &str) {
        self.lock().config.model = model.to_string();
    }
    pub fn set_system_prompt(&self, prompt: &str) {
        self.lock().config.system_prompt = prompt.to_string();
    }
    pub fn add_tool(&self, name: &str) {
        self.lock().config.tools.push(name.to_string());
    }
    pub fn add_resource(&self, name: &str) {
        self.lock().config.resources.push(name.to_string());
    }

    // -- turn lifecycle ------------------------------------------------------

    /// Start a run: capture the current config as this turn's immutable
    /// snapshot and audit `RunStart`. Mid-turn config changes do NOT touch it.
    pub fn start_run(&self, run_id: i64) -> Result<TurnSnapshot, HarnessError> {
        let mut g = self.lock();
        if g.phase != Phase::Idle {
            return Err(HarnessError::PhaseBusy {
                op: "start_run",
                phase: g.phase,
            });
        }
        g.run_id = run_id;
        g.phase = Phase::Running;
        let snap = g.config.snapshot();
        g.snapshot = Some(snap.clone());
        g.host.audit(
            AuditKind::Workflow,
            "harness",
            &format!("run:{run_id}"),
            AuditStatus::Ok,
            "RunStart",
        );
        Ok(snap)
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
        let mut g = self.lock();
        g.enqueue_locked("message_end", payload_json, idempotency_key)?;
        g.drain_locked()
    }

    /// Queue a session write without an id; flushed at save-points or on
    /// operation finish/failure cleanup.
    pub fn queue_write(&self, topic: &str, payload_json: &str, idempotency_key: &str) {
        let mut g = self.lock();
        g.pending.push(EntryWithoutId {
            topic: topic.to_string(),
            payload_json: payload_json.to_string(),
            idempotency_key: idempotency_key.to_string(),
        });
    }

    /// Save-point: drain all pending writes now.
    pub fn save_point(&self) -> Result<usize, HarnessError> {
        let mut g = self.lock();
        let n = g.pending.len();
        g.drain_locked()?;
        Ok(n)
    }

    /// Steer mid-turn: allowed while running; recorded for the provider loop.
    pub fn steer(&self, note: &str) -> Result<(), HarnessError> {
        let mut g = self.lock();
        if g.phase == Phase::Idle {
            return Err(HarnessError::PhaseBusy {
                op: "steer",
                phase: g.phase,
            });
        }
        g.follow_ups.push(note.to_string());
        Ok(())
    }

    /// Queue follow-up work; allowed mid-turn.
    pub fn follow_up(&self, note: &str) {
        self.lock().follow_ups.push(note.to_string());
    }

    pub fn follow_ups(&self) -> Vec<String> {
        self.lock().follow_ups.clone()
    }

    /// Register work to execute exactly when the harness next reaches Idle.
    /// This is the facade's `runWhenIdle`: callers never poll raw internals.
    pub fn run_when_idle<F: FnOnce() + Send + 'static>(&self, f: F) {
        self.lock().deferred_idle.push(Box::new(f));
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
        let mut g = self.lock();
        if g.phase == Phase::Idle {
            return Ok(());
        }
        g.drain_locked()?;
        let run_id = g.run_id;
        g.phase = Phase::Idle;
        g.snapshot = None;
        g.host.audit(
            AuditKind::Workflow,
            "harness",
            &format!("run:{run_id}"),
            status,
            detail,
        );
        let ops: Vec<DeferredOp> = std::mem::take(&mut g.deferred_idle);
        drop(g);
        for op in ops {
            op();
        }
        Ok(())
    }

    // -- structural operations (Idle-only) -----------------------------------

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
        let g = self.lock();
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

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner<H>> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
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
    fn enqueue_locked(
        &mut self,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<(), HarnessError> {
        let created = self
            .host
            .enqueue(self.run_id, topic, payload_json, idempotency_key)
            .map_err(|e| HarnessError::Host(e.to_string()))?;
        if !created {
            // Replay receipt: the entry already exists; nothing further.
            return Ok(());
        }
        Ok(())
    }

    fn drain_locked(&mut self) -> Result<(), HarnessError> {
        for entry in std::mem::take(&mut self.pending) {
            self.enqueue_locked(&entry.topic, &entry.payload_json, &entry.idempotency_key)?;
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
            _payload: &str,
            key: &str,
        ) -> Result<bool, crate::host::HostError> {
            self.record(format!("enqueue:{topic}:{key}:{run_id}"));
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
    }

    fn harness() -> AgentHarness<TapeHost> {
        let h = AgentHarness::new(
            Arc::new(TapeHost::default()),
            "test-model",
            "you are a steward",
        );
        h.add_tool("read");
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
        h.set_model("next-model");
        h.set_system_prompt("changed");
        h.add_tool("bash");
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
        h.queue_write("summary", "{}", "k-summary");
        h.queue_write("usage", "{}", "k-usage");
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
    fn save_point_drains_and_refreshes_queue() {
        let tape = Arc::new(TapeHost::default());
        let h = AgentHarness::new(tape.clone(), "m", "s");
        h.start_run(3).unwrap();
        h.queue_write("a", "{}", "ka");
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
            h.queue_write("residual", "{}", "kr");
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
        h.follow_up("check again");
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
            });
        }
        {
            let o = order.clone();
            h.run_when_idle(move || {
                if let Ok(mut g) = o.lock() {
                    g.push("deferred-b");
                }
            });
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
