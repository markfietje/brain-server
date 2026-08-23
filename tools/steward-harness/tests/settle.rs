//! v1.28.17 "Settle" — engine conformance: cancel settles at a step
//! boundary, sigterm-then-resume replays exactly to the control artifacts,
//! bounded grace beats a stuck step, and event listeners never starve.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use brain_engine_sdk::host::tx::HostTx;
use brain_engine_sdk::host::{AuditKind, AuditStatus, CasError, HostError, WorkflowHost};
use brain_engine_sdk::hostcall::CancellationToken;
use serde_json::{Value, json};
use steward_harness::engine::{self, CrankReport, StoppedAt};
use steward_harness::inmem::InMemHost;

fn long_queue(n: usize) -> String {
    let queue: Vec<Value> = (0..n)
        .map(|_| json!({"expected": "e", "actual": "a"}))
        .collect();
    json!({"next_step": "step-0", "queue": queue}).to_string()
}

/// A host double that blocks its Nth-and-later CAS calls until the token is
/// cancelled: the deterministic mid-run cancel point. The crank is parked
/// mid-step; the cancel applies at the NEXT boundary, never splitting a
/// CAS/event twin.
struct BlockyHost {
    inner: InMemHost,
    block_from_cas_call: usize,
    cas_calls: AtomicUsize,
    token: Arc<CancellationToken>,
}

impl BlockyHost {
    fn new(
        run_id: i64,
        state: &str,
        block_from_cas_call: usize,
        token: Arc<CancellationToken>,
    ) -> Self {
        let inner = InMemHost::new();
        inner.seed(run_id, state);
        BlockyHost {
            inner,
            block_from_cas_call,
            cas_calls: AtomicUsize::new(0),
            token,
        }
    }
}

impl WorkflowHost for BlockyHost {
    fn tx(&self) -> Result<HostTx, HostError> {
        self.inner.tx()
    }
    fn enqueue(
        &self,
        run_id: i64,
        topic: &str,
        payload_json: &str,
        idempotency_key: &str,
    ) -> Result<bool, HostError> {
        self.inner
            .enqueue(run_id, topic, payload_json, idempotency_key)
    }
    fn cas(&self, run_id: i64, expected_rev: i64, state_json: &str) -> Result<(), CasError> {
        let n = self.cas_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if n >= self.block_from_cas_call {
            while !self.token.is_cancelled() {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        self.inner.cas(run_id, expected_rev, state_json)
    }
    fn load_state(&self, run_id: i64) -> Result<Option<(String, i64)>, HostError> {
        self.inner.load_state(run_id)
    }
    fn audit(&self, kind: AuditKind, actor: &str, target: &str, status: AuditStatus, detail: &str) {
        self.inner.audit(kind, actor, target, status, detail);
    }
}

fn block_on_crank(
    h: Arc<dyn WorkflowHost>,
    tk: CancellationToken,
    run_id: i64,
) -> tokio::task::JoinHandle<Result<CrankReport, steward_harness::engine::HarnessError>> {
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(engine::crank_full(h, None, None, Some(tk), run_id, 100))
    })
}

/// The artifacts the scorer derives from, for control-vs-cancelled equality.
fn artifacts_of(state_json: &str) -> Value {
    let v: Value = serde_json::from_str(state_json).unwrap();
    json!({
        "steps": v["steps"],
        "findings": v["findings"],
        "contradictions": v["contradictions"],
        "status": v["status"],
        "audit_ok": v["audit_ok"],
        "handoff_complete": v["handoff_complete"],
    })
}

/// Every executed step carries BOTH its CAS twin (a recorded step row) and
/// its event twin (`run-{id}-evt-{n}`), contiguously from 1 — no half-step.
fn assert_no_half_step(host: &InMemHost, run_id: i64, state_json: &str) {
    let v: Value = serde_json::from_str(state_json).expect("state parses");
    let steps = v["steps"].as_array().expect("steps array").len() as u32;
    let mut evt_keys: Vec<String> = host
        .outbox_of(run_id)
        .into_iter()
        .filter(|(t, _, _)| t == "workflow/log")
        .map(|(_, _, k)| k)
        .collect();
    evt_keys.sort();
    let expected: Vec<String> = (1..=steps)
        .map(|n| format!("run-{run_id}-evt-{n}"))
        .collect();
    assert_eq!(evt_keys, expected, "every step has exactly its event twin");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crank_cancelled_mid_run_settles_at_step_boundary() {
    let token = Arc::new(CancellationToken::default());
    let host = Arc::new(BlockyHost::new(1, &long_queue(20), 2, Arc::clone(&token)));
    let h = Arc::clone(&host) as Arc<dyn WorkflowHost>;
    let task = block_on_crank(h, (*token).clone(), 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let t0 = Instant::now();
    token.cancel();
    let report: CrankReport = task.await.unwrap().unwrap();
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "cancel settled within bounded grace"
    );
    assert_eq!(report.stopped_at, StoppedAt::Cancelled);
    // State sits EXACTLY on a step boundary: parseable, revision consistent
    // with the executed steps, every CAS twin paired with its event twin.
    let (state_json, rev) = host.inner.state(1).unwrap();
    let v: Value = serde_json::from_str(&state_json).unwrap();
    let steps = v["steps"].as_array().unwrap().len() as i64;
    assert!(steps > 0, "cancelled mid-run, not before the first step");
    assert_eq!(rev, steps, "one CAS per recorded step");
    assert_eq!(u32::try_from(steps).unwrap(), report.steps_executed);
    assert_no_half_step(&host.inner, 1, &state_json);
    // The settled run is replayable from its persisted twins alone (the
    // audit chain itself lives at the server seam, not the in-mem double).
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigterm_settle_then_resume_exact() {
    // Control: the same seed, uncancelled, driven straight to Done.
    let control = Arc::new(InMemHost::new());
    control.seed(7, &long_queue(6));
    let hc = Arc::clone(&control) as Arc<dyn WorkflowHost>;
    engine::crank(hc, 7, 100).await.unwrap();

    let token = Arc::new(CancellationToken::default());
    let host = Arc::new(BlockyHost::new(7, &long_queue(6), 3, Arc::clone(&token)));
    let h = Arc::clone(&host) as Arc<dyn WorkflowHost>;
    let task = block_on_crank(h, (*token).clone(), 7);
    tokio::time::sleep(Duration::from_millis(50)).await;
    token.cancel(); // the sigterm between steps
    let cancelled = task.await.unwrap().unwrap();
    assert_eq!(cancelled.stopped_at, StoppedAt::Cancelled);
    assert!(cancelled.steps_executed > 0, "cancelled mid-run, not at t0");

    // Resume: re-crank the SAME run with no cancel — replays the queue and
    // lands Done with artifacts EQUAL to the uncancelled control.
    let h = Arc::clone(&host) as Arc<dyn WorkflowHost>;
    let resumed = engine::crank(h, 7, 100).await.unwrap();
    assert_eq!(resumed.stopped_at, StoppedAt::Done);
    let (final_state, _) = host.inner.state(7).unwrap();
    let (want_state, _) = control.state(7).unwrap();
    assert_no_half_step(&host.inner, 7, &final_state);
    assert_eq!(
        artifacts_of(&final_state),
        artifacts_of(&want_state),
        "cancelled-then-resumed artifacts equal the uncancelled control"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_grace_beats_a_stuck_step() {
    const GRACE: Duration = Duration::from_millis(300);
    let token = Arc::new(CancellationToken::default());
    // The stuck double: parked forever on its SECOND CAS (mid-step), and no
    // watcher will release it except the driver's cancel.
    let host = Arc::new(BlockyHost::new(9, &long_queue(20), 2, Arc::clone(&token)));
    let h = Arc::clone(&host) as Arc<dyn WorkflowHost>;
    let mut task = block_on_crank(h, (*token).clone(), 9);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Without cancel the crank hangs on the stuck step: the grace window
    // elapses with the task still wedged (this is the hazard being bounded).
    let still_stuck = tokio::time::timeout(GRACE, &mut task).await;
    assert!(still_stuck.is_err(), "the stuck step outlives the grace");

    // Cancel + grace ⇒ settles: the abort path releases the child, and the
    // report lands within the bound — never a hang, never a panic.
    let t0 = Instant::now();
    token.cancel();
    let report = tokio::time::timeout(GRACE, task)
        .await
        .expect("settled within the grace bound once cancelled")
        .unwrap()
        .unwrap();
    assert_eq!(report.stopped_at, StoppedAt::Cancelled);
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "settlement after cancel is prompt"
    );
}

#[test]
fn event_listeners_do_not_starve() {
    use brain_engine_sdk::events::{Hooks, Verdict};
    use brain_engine_sdk::workflow::events;
    use brain_engine_sdk::workflow::{RunId, WorkflowLogSnapshot};

    let hooks = Hooks::new();
    // Test-only throwing subscriber — the whole point of the containment
    // pin; dispatch catches it at the boundary.
    #[allow(clippy::panic)]
    fn explode() -> Verdict {
        panic!("subscriber explodes")
    }
    hooks
        .on(events::LOG, "panicker", |_p: WorkflowLogSnapshot| explode())
        .ok();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let s2 = Arc::clone(&seen);
    hooks
        .on(events::LOG, "observer", move |p: WorkflowLogSnapshot| {
            if let Ok(mut g) = s2.lock() {
                g.push(p.line.clone());
            }
            Verdict::Allow
        })
        .ok();
    events::log(&hooks, &RunId("wf-1".into()), "line-one");
    events::log(&hooks, &RunId("wf-1".into()), "line-two");
    assert_eq!(
        seen.lock().unwrap().clone(),
        vec!["line-one".to_string(), "line-two".to_string()],
        "later listeners receive every payload despite the panicking one"
    );
}
