//! The crank: `load_state -> decide -> act`, one governed step at a time.
//!
//! Every durable effect goes through the host seam (CAS persist + outbox
//! event), so a crash between any two effects replays exactly once by
//! idempotency key (`run-{id}-evt-{n}`). A gate rejection becomes a finding
//! row in state — never a silent skip. Budgets bound every turn; the 80%
//! iteration threshold surfaces as `warn_threshold_fired`.

use std::collections::BTreeMap;
use std::sync::Arc;

use brain_engine_sdk::host::{HostError, WorkflowHost};
use brain_engine_sdk::hostcall::CancellationToken;
use brain_engine_sdk::workflow_state::Decision;
use brain_troubleshoot_core::evidence::{EvidenceRef, EvidenceType};
use brain_troubleshoot_core::gates::{self, ConflictCode, GateResult};
use brain_troubleshoot_core::kernel::{
    RunState, Step, Verdict, clamp_max_steps, resolve_max_steps, should_warn_at_iteration_threshold,
};
use serde_json::Value;

/// Why the crank stopped. `Stale` carries the host's actual revision — a
/// contention REPORT, never a panic.
#[derive(Debug, Clone, PartialEq)]
pub enum StoppedAt {
    AskHuman {
        question: String,
    },
    Done,
    Budget,
    /// A cooperative cancel observed at a step boundary: settled, never a
    /// half-step (the cancel applies between steps, after the CAS twin).
    Cancelled,
    Stale {
        actual_revision: i64,
    },
}

impl StoppedAt {
    pub fn as_str(&self) -> &'static str {
        match self {
            StoppedAt::AskHuman { .. } => "ask_human",
            StoppedAt::Done => "done",
            StoppedAt::Budget => "budget",
            StoppedAt::Cancelled => "cancelled",
            StoppedAt::Stale { .. } => "stale",
        }
    }
}

/// The outcome of one crank invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct CrankReport {
    pub steps_executed: u32,
    pub stopped_at: StoppedAt,
    /// The 80% iteration threshold fired at least once this turn.
    pub warn_threshold_fired: bool,
    /// The in-run hostcall tally, `"<label>/<kind>" -> count` (additive JSON;
    /// consumers ignore unknown keys). The audit chain is the durable count —
    /// this is the cheap aggregate the report carries.
    pub hostcalls: BTreeMap<String, u64>,
}

#[derive(Debug)]
pub enum HarnessError {
    Host(HostError),
    /// The stored state_json is not parseable JSON — integrity-visible; the
    /// crank refuses rather than treating a poisoned run as terminal.
    CorruptState(String),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessError::Host(e) => write!(f, "host: {e}"),
            HarnessError::CorruptState(m) => write!(f, "corrupt state: {m}"),
        }
    }
}

impl std::error::Error for HarnessError {}

impl From<HostError> for HarnessError {
    fn from(e: HostError) -> Self {
        HarnessError::Host(e)
    }
}

/// Advisory steering reads at the step boundary. A separate opt-in trait —
/// the storage ABI stays untouched (the SDK is never modified for transport
/// or inbox concerns).
pub trait SteeringReader: Send + Sync {
    fn read_steering(&self, run_id: i64) -> Result<Vec<String>, HostError>;
}

/// Resolve the turn budget: `BRAIN_MAX_STEPS` env -> default 24, ceiling 1000
/// (the troubleshoot-core kernel owns the numbers).
pub fn resolve_budget(env_val: Option<u32>) -> u32 {
    clamp_max_steps(resolve_max_steps(env_val))
}

/// Run the governed loop until a stop condition. `max_steps` bounds ONE
/// crank invocation; the run itself may need many cranks (human-cranked).
pub async fn crank(
    host: Arc<dyn WorkflowHost>,
    run_id: i64,
    max_steps: u32,
) -> Result<CrankReport, HarnessError> {
    crank_with_steering(host, None, run_id, max_steps).await
}

/// `crank` with an optional advisory steering source drained at each step
/// boundary. Steering never redirects the loop autonomously — drained
/// messages land in `state.steering[]` as advisories for the next decision.
pub async fn crank_with_steering(
    host: Arc<dyn WorkflowHost>,
    reader: Option<Arc<dyn SteeringReader>>,
    run_id: i64,
    max_steps: u32,
) -> Result<CrankReport, HarnessError> {
    crank_full(host, reader, None, None, run_id, max_steps).await
}

/// The full crank: an optional [`Effects`] door makes every event emission
/// ride the mediated hostcall dispatch (counted, audited); without one the
/// emissions fall back to the host trait's own audited enqueue seam. An
/// optional cancel token (`brain_engine_sdk::hostcall::CancellationToken`,
/// clones share the signal) is honored at every step boundary and settles
/// the run as [`StoppedAt::Cancelled`] exactly between steps.
#[allow(clippy::too_many_arguments)]
pub async fn crank_full(
    host: Arc<dyn WorkflowHost>,
    reader: Option<Arc<dyn SteeringReader>>,
    effects: Option<Arc<crate::effects::Effects>>,
    cancel: Option<CancellationToken>,
    run_id: i64,
    max_steps: u32,
) -> Result<CrankReport, HarnessError> {
    let max_steps = clamp_max_steps(max_steps);
    let mut steps_executed: u32 = 0;
    let mut warned = false;
    let mut kernel = RunState::new(max_steps);
    let mut hostcalls: BTreeMap<String, u64> = BTreeMap::new();

    loop {
        let Some((js, rev)) = host.load_state(run_id)? else {
            return Err(HarnessError::Host(HostError::NotFound));
        };
        let mut st: Value =
            serde_json::from_str(&js).map_err(|e| HarnessError::CorruptState(e.to_string()))?;

        // Cancel check BEFORE executing another step: settle at the boundary
        // the previous CAS+event twin completed — never mid-step.
        if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Ok(report(
                steps_executed,
                StoppedAt::Cancelled,
                warned,
                hostcalls,
            ));
        }

        // Budget check BEFORE executing another step.
        if steps_executed >= max_steps {
            warned |= should_warn_at_iteration_threshold(steps_executed, max_steps);
            return Ok(report(steps_executed, StoppedAt::Budget, warned, hostcalls));
        }

        match brain_engine_sdk::decide(&st) {
            Decision::Done => {
                let already = st
                    .get("status")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s == "completed");
                if !already {
                    finalize(&*host, run_id, rev, &mut st, &effects, &mut hostcalls)?;
                }
                return Ok(report(steps_executed, StoppedAt::Done, warned, hostcalls));
            }
            Decision::AskHuman { question } => {
                return Ok(report(
                    steps_executed,
                    StoppedAt::AskHuman { question },
                    warned,
                    hostcalls,
                ));
            }
            Decision::Advance { next_state } => {
                let ns: Value = serde_json::from_str(&next_state)
                    .map_err(|e| HarnessError::CorruptState(e.to_string()))?;
                match cas_persist(&*host, run_id, rev, &ns) {
                    Ok(()) => continue,
                    Err(actual) => {
                        return Ok(report(
                            steps_executed,
                            StoppedAt::Stale {
                                actual_revision: actual,
                            },
                            warned,
                            hostcalls,
                        ));
                    }
                }
            }
            Decision::RunStep { step } => {
                // Advisory steering drains at the boundary; recorded as data.
                if let Some(r) = &reader
                    && let Ok(msgs) = r.read_steering(run_id)
                    && !msgs.is_empty()
                    && let Some(obj) = st.as_object_mut()
                {
                    let entry = obj
                        .entry("steering".to_string())
                        .or_insert_with(|| Value::Array(vec![]));
                    if let Some(arr) = entry.as_array_mut() {
                        arr.extend(msgs.iter().map(|m| Value::String(m.clone())));
                    }
                }

                let item = take_queue_head(&mut st);
                let has_answerer = st
                    .get("answers")
                    .and_then(|a| a.as_array())
                    .is_some_and(|a| !a.is_empty());

                // Gate waterfall over DECLARED constraints. Undeclared
                // constraints pass vacuously — replaying recorded steps is
                // evidence replay under gates those steps declared.
                let rejection = run_step_gates(item.as_ref(), has_answerer);

                // Kernel bookkeeping honors the same budget law.
                kernel
                    .record_step(Step {
                        id: u64::from(steps_executed) + 1,
                        action: step.clone(),
                        evidence_refs: vec![],
                        verdict: Some(if rejection.is_some() {
                            Verdict::Aborted
                        } else {
                            Verdict::Completed
                        }),
                    })
                    .map_err(|_| {
                        HarnessError::Host(HostError::Internal("step budget exhausted".to_string()))
                    })?;

                record_step_in_state(&mut st, item.as_ref(), rejection.as_deref());
                // Queue empty -> clear the routing keys IN THE PERSISTED
                // state so the next decide reaches Done naturally.
                if queue_remaining(&st) == 0
                    && let Some(obj) = st.as_object_mut()
                {
                    obj.remove("next_step");
                    obj.remove("queue");
                }
                warned |= should_warn_at_iteration_threshold(kernel.step_count, max_steps);

                match cas_persist(&*host, run_id, rev, &st) {
                    Ok(()) => {}
                    Err(actual) => {
                        return Ok(report(
                            steps_executed,
                            StoppedAt::Stale {
                                actual_revision: actual,
                            },
                            warned,
                            hostcalls,
                        ));
                    }
                }
                steps_executed += 1;
                // The event ordinal comes from the PERSISTED step count, not
                // the per-crank counter: a cancelled-then-resumed run must
                // not re-key its events (the idempotency gate would swallow
                // every resumed step's event twin).
                let ordinal = st
                    .get("steps")
                    .and_then(|s| s.as_array())
                    .map_or(steps_executed as usize, Vec::len);
                emit(
                    &*host,
                    &effects,
                    &mut hostcalls,
                    run_id,
                    "workflow/log",
                    &serde_json::json!({ "line": format!("step {} executed", step) }).to_string(),
                    &format!("run-{run_id}-evt-{ordinal}"),
                )?;
            }
        }
    }
}

fn report(
    steps: u32,
    stopped_at: StoppedAt,
    warned: bool,
    hostcalls: BTreeMap<String, u64>,
) -> CrankReport {
    CrankReport {
        steps_executed: steps,
        stopped_at,
        warn_threshold_fired: warned,
        hostcalls,
    }
}

/// Pop the head of `state.queue` — the engine's step source on replayed runs.
fn take_queue_head(st: &mut Value) -> Option<Value> {
    let arr = st.get_mut("queue")?.as_array_mut()?;
    if arr.is_empty() {
        None
    } else {
        Some(arr.remove(0))
    }
}

fn queue_remaining(st: &Value) -> usize {
    st.get("queue")
        .and_then(|q| q.as_array())
        .map_or(0, |a| a.len())
}

/// CAS persist with ONE reload-retry on `Stale`; a second stale result is
/// reported (`StoppedAt::Stale`), never panicked on.
fn cas_persist(host: &dyn WorkflowHost, run_id: i64, rev: i64, st: &Value) -> Result<(), i64> {
    let js = st.to_string();
    if host.cas(run_id, rev, &js).is_ok() {
        return Ok(());
    }
    // Reload and retry exactly once (the recovery half of the CAS
    // contract); if we are stale AGAIN, someone else is actively
    // driving — report, don't fight.
    match host.load_state(run_id) {
        Ok(Some((_, fresh_rev))) => match host.cas(run_id, fresh_rev, &js) {
            Ok(()) => Ok(()),
            Err(brain_engine_sdk::host::CasError::Stale { actual_revision }) => Err(actual_revision),
            Err(_) => Err(fresh_rev),
        },
        _ => Err(rev),
    }
}

/// Route an event emission through the Effects door when present, tallying
/// the dispatch; otherwise the host trait's audited enqueue carries it.
fn emit(
    host: &dyn WorkflowHost,
    effects: &Option<Arc<crate::effects::Effects>>,
    hostcalls: &mut BTreeMap<String, u64>,
    run_id: i64,
    topic: &str,
    payload: &str,
    key: &str,
) -> Result<(), HarnessError> {
    if let Some(fx) = effects {
        fx.event(run_id, topic, payload, key)
            .map_err(|e| HarnessError::Host(HostError::Internal(e.to_string())))?;
        *hostcalls.entry(format!("{key}/events")).or_insert(0) += 1;
    } else {
        host.enqueue(run_id, topic, payload, key)?;
    }
    Ok(())
}

/// Fold artifacts into the scoreboard keys the server's scorer derives from,
/// mark `status: completed`, persist, and emit the final event.
fn finalize(
    host: &dyn WorkflowHost,
    run_id: i64,
    rev: i64,
    st: &mut Value,
    effects: &Option<Arc<crate::effects::Effects>>,
    hostcalls: &mut BTreeMap<String, u64>,
) -> Result<(), HarnessError> {
    let Some(obj) = st.as_object_mut() else {
        return Err(HarnessError::CorruptState("state is not an object".into()));
    };
    obj.entry("steps".to_string())
        .or_insert_with(|| Value::Array(vec![]));
    obj.entry("findings".to_string())
        .or_insert_with(|| Value::Array(vec![]));
    obj.entry("contradictions".to_string())
        .or_insert_with(|| serde_json::json!(0));
    // Fail-closed audit posture mirrors the scoreboard: the engine's writes
    // all rode audited host calls, so a completed crank certifies audit_ok.
    obj.insert("audit_ok".into(), serde_json::json!(true));
    // `handoff_complete = status == "completed"` (the scoreboard derivation),
    // EXCEPT where the run already recorded an honest false (an incomplete
    // handoff stays incomplete — the engine never upgrades evidence).
    obj.entry("handoff_complete".to_string())
        .or_insert(serde_json::json!(true));
    obj.insert("status".into(), serde_json::json!("completed"));
    let js = st.to_string();
    host.cas(run_id, rev, &js)
        .map_err(|e| HarnessError::Host(HostError::Internal(e.to_string())))?;
    emit(
        host,
        effects,
        hostcalls,
        run_id,
        "workflow/end",
        &serde_json::json!({ "stop": "completed" }).to_string(),
        &format!("run-{run_id}-end"),
    )?;
    Ok(())
}

/// The waterfall over one queue item's DECLARED constraints:
/// `required_evidence[]`, `mutations`, `supporting_lines`, `needs_approval`.
/// Absent constraints pass vacuously. A rejection returns the conflict-code
/// string that becomes a finding row — never a silent skip.
fn run_step_gates(item: Option<&Value>, has_answerer: bool) -> Option<String> {
    let item = item?;
    let refs: Vec<EvidenceRef> = item
        .get("evidence_refs")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str())
                .map(|s| EvidenceRef {
                    evidence_type: EvidenceType::SystemEventLog,
                    locator: s.to_string(),
                    digest: String::new(),
                    captured_at: 0,
                    first_symptom_ts: None,
                })
                .collect()
        })
        .unwrap_or_default();
    let required: Vec<EvidenceType> = item
        .get("required_evidence")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .filter_map(parse_evidence_type)
                .collect()
        })
        .unwrap_or_default();
    let needs_approval = item
        .get("needs_approval")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let mutations = item.get("mutations").and_then(|m| m.as_u64()).unwrap_or(1) as usize;
    let supporting = item.get("supporting_lines").and_then(|v| v.as_u64());

    let mut gates_to_run: Vec<Box<dyn Fn() -> GateResult + Send + Sync>> = vec![
        Box::new({
            let refs = refs.clone();
            let required = required.clone();
            move || gates::gate_evidence(&refs, &required)
        }),
        Box::new(move || gates::gate_one_variable(mutations)),
    ];
    if supporting.is_some() {
        let lines = supporting.unwrap_or(0) as usize;
        gates_to_run.push(Box::new(move || gates::gate_corroborate(lines)));
    }
    gates_to_run.push(Box::new(move || {
        gates::gate_approval(has_answerer, needs_approval)
    }));

    match gates::run_waterfall(gates_to_run) {
        GateResult::Pass => None,
        GateResult::Reject(rej) => {
            let code = ConflictCode::GateOpen {
                gate: rej.gate.as_str().to_string(),
                missing: rej.missing.clone(),
            };
            Some(format!(
                "{}:{}:{}",
                code.as_str(),
                rej.gate.as_str(),
                rej.reason
            ))
        }
    }
}

fn parse_evidence_type(s: &str) -> Option<EvidenceType> {
    EvidenceType::all()
        .iter()
        .find(|t| t.as_str() == s)
        .cloned()
}

/// Append the executed step to `state.steps` with exactly the artifact keys
/// the scorer derives from (`expected/actual/skipped_verify/abstained/
/// guidance_accepted`), so a replayed run's artifacts equal its frozen case
/// field-for-field. Declared findings and gate rejections append finding rows.
fn record_step_in_state(st: &mut Value, item: Option<&Value>, rejection: Option<&str>) {
    let Some(obj) = st.as_object_mut() else {
        return;
    };
    let empty = serde_json::Map::new();
    let it = item.and_then(|v| v.as_object()).unwrap_or(&empty);
    let mut rec = serde_json::json!({
        "expected": it.get("expected").and_then(|v| v.as_str()).unwrap_or(""),
        "actual": it.get("actual").and_then(|v| v.as_str()).unwrap_or(""),
        "skipped_verify": it.get("skipped_verify").and_then(|v| v.as_bool()).unwrap_or(false),
        "abstained": it.get("abstained").and_then(|v| v.as_bool()).unwrap_or(false),
        "guidance_accepted": it.get("guidance_accepted").cloned().unwrap_or(Value::Null),
    });
    if let Some(r) = rejection {
        rec["gate_rejection"] = Value::String(r.to_string());
    }
    if let Some(a) = obj
        .entry("steps".to_string())
        .or_insert_with(|| Value::Array(vec![]))
        .as_array_mut()
    {
        a.push(rec);
    }
    let mut new_findings: Vec<Value> = Vec::new();
    if let Some(finding) = it.get("finding").and_then(|f| f.as_str()) {
        new_findings.push(Value::String(finding.to_string()));
    }
    if let Some(r) = rejection {
        new_findings.push(Value::String(r.to_string()));
    }
    if !new_findings.is_empty()
        && let Some(a) = obj
            .entry("findings".to_string())
            .or_insert_with(|| Value::Array(vec![]))
            .as_array_mut()
    {
        a.extend(new_findings);
    }
}

/// A stable single-token tag from arbitrary detail text (idempotency keys).
pub fn stable_tag(detail: &str) -> String {
    detail
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(48)
        .collect()
}
