//! Lineage tests (the "Lineage" release): checkpoints as events, rewind
//! cursor seeding from `state.branches[]`, and idempotent replay on a
//! branched chain.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::{Arc, Mutex};

use brain_engine_sdk::host::tx::HostTxHandle;
use brain_engine_sdk::host::{AuditKind, AuditStatus, CasError, HostError, HostTx, WorkflowHost};
use steward_harness::engine;

type EventRow = (i64, String, String, String, Option<i64>);

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A file-free host that stores events WITH ancestry: `(id, topic, payload,
/// key, parent)`. CAS is revision-checked like the real adapter.
#[derive(Default)]
struct TapeHost {
    inner: Mutex<TapeInner>,
}

#[derive(Default)]
struct TapeInner {
    state: Option<(String, i64)>,
    events: Vec<(i64, String, String, String, Option<i64>)>,
    next_event_id: i64,
}

struct NoopHandle;
impl HostTxHandle for NoopHandle {
    fn finish(self: Box<Self>, _commit: bool) -> Result<(), HostError> {
        Ok(())
    }
}

impl TapeHost {
    fn snapshot(&self) -> (Option<(String, i64)>, Vec<EventRow>) {
        let g = lock(&self.inner);
        (g.state.clone(), g.events.clone())
    }
    fn seed(&self, state_json: &str) {
        let mut g = lock(&self.inner);
        g.state = Some((state_json.to_string(), 0));
    }
}

impl WorkflowHost for TapeHost {
    fn tx(&self) -> Result<HostTx, HostError> {
        Ok(HostTx::new(Box::new(NoopHandle)))
    }
    fn enqueue(&self, _run: i64, topic: &str, payload: &str, key: &str) -> Result<bool, HostError> {
        self.enqueue_with_parent(_run, None, topic, payload, key)
            .map(|(c, _)| c)
    }
    fn enqueue_with_parent(
        &self,
        _run: i64,
        parent: Option<i64>,
        topic: &str,
        payload: &str,
        key: &str,
    ) -> Result<(bool, i64), HostError> {
        let mut g = lock(&self.inner);
        if g.events.iter().any(|(_, _, _, k, _)| k == key) {
            let Some(id) = g
                .events
                .iter()
                .find(|(.., k, _)| k == key)
                .map(|(id, ..)| *id)
            else {
                unreachable!("key membership checked above");
            };
            return Ok((false, id));
        }
        g.next_event_id += 1;
        let id = g.next_event_id;
        g.events.push((
            id,
            topic.to_string(),
            payload.to_string(),
            key.to_string(),
            parent,
        ));
        Ok((true, id))
    }
    fn cas(&self, _run: i64, expected_rev: i64, state_json: &str) -> Result<(), CasError> {
        let mut g = lock(&self.inner);
        match &g.state {
            Some((_, rev)) if *rev == expected_rev => {
                g.state = Some((state_json.to_string(), expected_rev + 1));
                Ok(())
            }
            Some((_, rev)) => Err(CasError::Stale {
                actual_revision: *rev,
            }),
            None => Err(CasError::Gone),
        }
    }
    fn load_state(&self, _run: i64) -> Result<Option<(String, i64)>, HostError> {
        Ok(lock(&self.inner).state.clone())
    }
    fn audit(&self, _: AuditKind, _: &str, _: &str, _: AuditStatus, _: &str) {}
}

#[tokio::test]
async fn checkpoint_payload_round_trips_state_exactly() {
    let host = Arc::new(TapeHost::default());
    host.seed(r#"{"next_step":"inventory","queue":[{"expected":"a","actual":"a"}]}"#);
    let report = engine::crank(host.clone(), 1, 8).await.unwrap();
    assert_eq!(report.stopped_at.as_str(), "done");
    let (final_state, _) = host.snapshot().0.expect("run state exists after crank");
    // The LAST checkpoint event carries the full state snapshot AT ITS STEP
    // BOUNDARY (finalize mutates status afterwards — the rewind contract
    // targets boundaries, never mid-settlement states).
    let (_, events) = host.snapshot();
    let ckpt = events
        .iter()
        .filter(|(_, topic, ..)| topic == "workflow/checkpoint")
        .max_by_key(|(id, ..)| *id)
        .expect("at least one checkpoint event");
    // Byte-identical round-trip: restoring from the checkpoint reproduces the
    // exact snapshot bytes (no truncation, no re-serialize drift), and the
    // executed step record matches what the final state carries.
    let restored: serde_json::Value = serde_json::from_str(&ckpt.2).unwrap();
    assert_eq!(ckpt.2, restored.to_string());
    let v: serde_json::Value = serde_json::from_str(&final_state).unwrap();
    assert_eq!(restored["steps"], v["steps"]);
}

#[tokio::test]
async fn rewind_creates_branch_and_replay_is_idempotent() {
    let host = Arc::new(TapeHost::default());
    host.seed(
        r#"{"next_step":"inventory","queue":[{"expected":"a","actual":"a"}],
            "branches":[{"from_event":42,"reason":"wrong turn","at":9}]}"#,
    );
    // First crank: the first emitted event must PARENT at the rewind target.
    let r1 = engine::crank(host.clone(), 1, 8).await.unwrap();
    assert_eq!(r1.stopped_at.as_str(), "done");
    let (_, events_after_first) = host.snapshot();
    assert!(
        events_after_first.iter().any(|(.., p)| *p == Some(42)),
        "some event parents at the branch target"
    );
    // Every non-root emission threads its predecessor's id.
    for w in events_after_first.windows(2) {
        assert_eq!(w[1].4, Some(w[0].0), "events chain by id within the branch");
    }

    // Re-crank (crash recovery / human re-drive): deterministic keys mean
    // every event replays EXACTLY once — no duplicates, no re-parenting.
    let before = host.snapshot().1.len();
    let r2 = engine::crank(host.clone(), 1, 8).await.unwrap();
    assert_eq!(r2.stopped_at.as_str(), "done");
    let (_, after) = host.snapshot();
    assert_eq!(after.len(), before, "replayed keys added no rows");
    let keys: std::collections::HashSet<&str> =
        after.iter().map(|(_, _, _, k, _)| k.as_str()).collect();
    assert_eq!(keys.len(), after.len(), "keys stay unique");
}
