//! The report pins: what the crank TELLS you about itself.
//!
//! R52 "harness truthfulness". Three claims, each of which was false or
//! absent before this round:
//!
//!   1. The 80% iteration threshold was a flag that nothing acted on.
//!   2. The report said nothing about how many constraints the gate
//!      waterfall actually saw.
//!   3. `lib.rs` claimed steering messages were "advisory inputs to the
//!      next decision" — `decide` never read them.
//!
//! `gold.rs`, `lineage.rs`, and `settle.rs` are untouched by this round
//! except for the one preregistered amendment in `gold.rs` (P52.4).

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use brain_engine_sdk::host::WorkflowHost;
use serde_json::{Value, json};
use steward_harness::engine::{self, StoppedAt};
use steward_harness::inmem::InMemHost;

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn repo_root() -> PathBuf {
    crate_root().join("..").join("..")
}

fn long_queue(n: usize) -> String {
    let queue: Vec<Value> = (0..n)
        // `mutations: 1` is what `record_step_in_state` does to a queued item,
        // so declaring it is TRUE by construction rather than an invented
        // provenance. The census counts presence, and `gate_one_variable`
        // already passed the absent case on its `.unwrap_or(1)` default — so
        // every gate verdict here is unchanged and only the report's honesty
        // moves. These are `RunKind::Live` turns.
        .map(|_| json!({"expected": "e", "actual": "a", "mutations": 1}))
        .collect();
    json!({"next_step": "step-0", "queue": queue}).to_string()
}

// ── R52.1 · warn-too-late ──────────────────────────────────────────────────

/// The threshold is a STOP, not a flag. A queue that would still admit
/// steps must not be run past the threshold.
#[tokio::test]
async fn warn_threshold_is_a_real_stop_not_a_flag() {
    let host = Arc::new(InMemHost::new());
    // 50 queued steps against max_steps=5: the 80% threshold fires at
    // step_count 4 (4 >= 5*4/5), and 46 steps would still be admissible.
    host.seed(1, &long_queue(50));
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank(h, 1, 5).await.unwrap();

    assert_eq!(
        report.stopped_at,
        StoppedAt::BudgetWarn,
        "crossing 80% must STOP the turn, not annotate it"
    );
    assert!(
        report.warn_threshold_fired,
        "the flag the report already carried stays true"
    );
    assert_eq!(
        report.steps_executed, 4,
        "stopped at the threshold (4), not at budget exhaustion (5)"
    );
    let (js, _) = host.state(1).unwrap();
    let st: Value = serde_json::from_str(&js).unwrap();
    assert!(
        st["queue"].as_array().is_some_and(|q| !q.is_empty()),
        "the queue still holds admissible work — the stop is what ended the turn"
    );
}

/// The stop lands at a step BOUNDARY: every recorded step has its event
/// twin, so a stop never orphans a recorded step.
#[tokio::test]
async fn budget_warn_stop_leaves_no_half_step() {
    let host = Arc::new(InMemHost::new());
    host.seed(2, &long_queue(50));
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank(h, 2, 5).await.unwrap();
    assert_eq!(report.stopped_at, StoppedAt::BudgetWarn);

    let (js, rev) = host.state(2).unwrap();
    let st: Value = serde_json::from_str(&js).unwrap();
    let steps = st["steps"].as_array().expect("steps array").len() as i64;
    assert_eq!(
        steps,
        i64::from(report.steps_executed),
        "recorded steps equal executed steps"
    );
    assert_eq!(rev, steps, "one CAS per recorded step");

    let mut evt_keys: Vec<String> = host
        .outbox_of(2)
        .into_iter()
        .filter(|(t, _, _)| t == "workflow/log")
        .map(|(_, _, k)| k)
        .collect();
    evt_keys.sort();
    let expected: Vec<String> = (1..=steps).map(|n| format!("run-2-evt-{n}")).collect();
    assert_eq!(evt_keys, expected, "every step has exactly its event twin");
}

/// Resumable: the stop emits a checkpoint, and re-arming with a larger
/// budget continues from the persisted state rather than restarting.
#[tokio::test]
async fn budget_warn_stop_is_resumable_with_a_larger_budget() {
    let host = Arc::new(InMemHost::new());
    host.seed(3, &long_queue(50));
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let first = engine::crank(h, 3, 5).await.unwrap();
    assert_eq!(first.stopped_at, StoppedAt::BudgetWarn);

    assert!(
        host.outbox_of(3)
            .iter()
            .any(|(t, p, _)| t == "workflow/checkpoint" && p.contains("\"queue\"")),
        "the stop emitted a checkpoint carrying the queue — a resume anchor"
    );

    // Re-arm: the caller grants a bigger budget, the crank continues.
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let second = engine::crank(h, 3, 100).await.unwrap();
    assert_eq!(
        second.stopped_at,
        StoppedAt::Done,
        "a re-armed crank finishes the work the stop deferred"
    );
    assert!(
        second.steps_executed > first.steps_executed,
        "the resumed turn picked up where the stop left off ({} then {})",
        first.steps_executed,
        second.steps_executed
    );
}

// ── R52.2 · gate-vacuity ───────────────────────────────────────────────────

/// The census is REPORTED for a replay, and the replay is exactly why that is
/// correct: recorded steps replayed under the gates those steps declared. This
/// is the `RunKind::Replay` arm of the asymmetry, pinned behaviourally — the
/// same synthetic queue is vacuous either way, and only the DECLARED kind
/// decides whether it may be certified.
#[tokio::test]
async fn vacuous_gates_are_reported_not_hidden() {
    let host = Arc::new(InMemHost::new());
    // Undeclared on purpose: this test's SUBJECT is the vacuous census.
    let queue: Vec<Value> = (0..3)
        .map(|_| json!({"expected": "e", "actual": "a"}))
        .collect();
    host.seed(
        1,
        &json!({"next_step": "step-0", "queue": queue}).to_string(),
    );
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank_replay(h, 1, 50).await.unwrap();

    assert_eq!(report.stopped_at, StoppedAt::Done);
    assert_eq!(
        report.run_kind,
        engine::RunKind::Replay,
        "a replay turn keeps the advisory posture — asserted, not assumed"
    );
    assert_eq!(
        report.gates_declared, 0,
        "these steps declare none of the five constraint keys"
    );
    assert_eq!(
        report.gates_evaluated, 9,
        "three gates per step x three steps — they ran, and passed on nothing"
    );
    assert!(
        report.gates_vacuous,
        "gates ran and every one of them passed on an undeclared constraint set"
    );
}

/// The discriminating twin: a step that DOES declare constraints is not
/// vacuous. Without this, the pin above would pass for a counter stuck at 0.
#[tokio::test]
async fn declared_constraints_are_counted_and_clear_vacuity() {
    let host = Arc::new(InMemHost::new());
    host.seed(
        2,
        &json!({
            "next_step": "step-0",
            "queue": [
                {"expected": "a", "actual": "a", "evidence_refs": ["run:1"], "mutations": 1},
                {"expected": "b", "actual": "b", "needs_approval": false}
            ]
        })
        .to_string(),
    );
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank(h, 2, 50).await.unwrap();
    assert_eq!(report.stopped_at, StoppedAt::Done);
    assert_eq!(
        report.run_kind,
        engine::RunKind::Live,
        "a declared live run"
    );
    assert_eq!(
        report.gates_declared, 3,
        "step 1 declares evidence_refs + mutations; step 2 declares needs_approval"
    );
    assert_eq!(report.gates_evaluated, 6, "three gates per step, two steps");
    assert!(
        !report.gates_vacuous,
        "a declared constraint set is not vacuous"
    );
}

/// The census counts PRESENCE, not the resolved value: `mutations` defaults
/// to 1 when absent, and a step that declares nothing must stay
/// distinguishable from one that declares `mutations: 1`.
#[tokio::test]
async fn mutations_default_does_not_count_as_a_declaration() {
    let host = Arc::new(InMemHost::new());
    host.seed(
        3,
        &json!({
            "next_step": "step-0",
            "queue": [
                {"expected": "a", "actual": "a"},
                {"expected": "b", "actual": "b", "mutations": 1}
            ]
        })
        .to_string(),
    );
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank(h, 3, 50).await.unwrap();
    assert_eq!(report.run_kind, engine::RunKind::Live);
    assert_eq!(
        report.gates_declared, 1,
        "only the second step declared one"
    );
    assert!(
        !report.gates_vacuous,
        "one declared constraint is enough to clear vacuity"
    );
}

// ── R52.3 / D52.5 · steering is a log ──────────────────────────────────────

/// Strip `//` and `/* */` comments (block comments nest in Rust) while
/// leaving string, raw-string, and char literals intact.
///
/// This exists because a comment that NAMES a symbol otherwise satisfies a
/// predicate about CODE. The R51 lesson: a bare name-mention assertion
/// false-passed on the very file whose gap the pin was closing.
fn strip_comments(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let n = b.len();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < n {
        let c = b[i];
        if c == '/' && i + 1 < n && b[i + 1] == '/' {
            while i < n && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && b[i + 1] == '*' {
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if b[i] == '/' && i + 1 < n && b[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && i + 1 < n && b[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }
        // Raw string: r"..." / r#"..."# — a `//` inside one is not a
        // comment. Copied VERBATIM: a string literal is code, and a pin that
        // matches on a key has to see the key.
        if c == 'r' {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < n && b[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < n && b[j] == '"' {
                j += 1;
                while j < n {
                    if b[j] == '"' {
                        let mut k = j + 1;
                        let mut cnt = 0usize;
                        while k < n && b[k] == '#' && cnt < hashes {
                            cnt += 1;
                            k += 1;
                        }
                        if cnt == hashes {
                            break;
                        }
                    }
                    j += 1;
                }
                out.push('r');
                for ch in &b[i + 1..(j + 1 + hashes).min(n)] {
                    out.push(*ch);
                }
                i = (j + 1 + hashes).min(n);
                continue;
            }
        }
        if c == '"' {
            let mut j = i + 1;
            while j < n {
                if b[j] == '\\' {
                    j += 2;
                    continue;
                }
                if b[j] == '"' {
                    break;
                }
                j += 1;
            }
            let end = (j + 1).min(n);
            for ch in &b[i..end] {
                out.push(*ch);
            }
            i = end;
            continue;
        }
        if c == '\'' {
            let mut j = i + 1;
            if j < n && b[j] == '\\' {
                j += 2;
            } else {
                j += 1;
            }
            if j < n && b[j] == '\'' {
                j += 1;
            }
            out.push('\'');
            i = j;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The `decide` function body, comment-stripped.
fn decide_body() -> String {
    let path = repo_root().join("crates/brain-engine-sdk/src/workflow_state.rs");
    // This crate denies `clippy::panic` even in tests (only unwrap/expect are
    // released by the `cfg_attr(test, ...)` at the top of this file), so the
    // failure paths use `expect`.
    let raw = std::fs::read_to_string(&path).expect("workflow_state.rs is readable");
    let stripped = strip_comments(&raw);
    let start = stripped
        .find("pub fn decide(")
        .expect("workflow_state.rs declares `pub fn decide`");
    let rest = &stripped[start..];
    let end = rest
        .find("\n}\n")
        .expect("`decide` has a closing brace at column 0");
    rest[..end + 3].to_string()
}

/// The FALSE claim is gone. Asserted on RAW text, deliberately: the claim
/// was a doc comment, so comment-stripping here would delete the very thing
/// under test and pass vacuously.
#[test]
fn the_false_steering_advisory_claim_is_gone() {
    let src = crate_root().join("src");
    for entry in ["lib.rs", "engine.rs", "remote_host.rs", "main.rs"] {
        let path = src.join(entry);
        let text = std::fs::read_to_string(&path).expect("the harness source is readable");
        assert!(
            !text.contains("advisory inputs to the next decision"),
            "{entry} still claims steering reaches the next decision"
        );
        assert!(
            !text.contains("as advisories for the next decision"),
            "{entry} still claims drained steering is an advisory for decide"
        );
    }
}

/// The rename landed, and the drained channel is named as a LOG.
#[test]
fn steering_is_named_as_a_log_and_not_as_a_binding_channel() {
    let engine = std::fs::read_to_string(crate_root().join("src/engine.rs")).unwrap();
    let stripped = strip_comments(&engine);

    assert!(
        stripped.contains("fn read_steering_log("),
        "the trait method is read_steering_log"
    );
    assert!(
        !stripped.contains("fn read_steering("),
        "the old read_steering name is gone from CODE (not just from prose)"
    );
    assert!(
        stripped.contains("\"steering_log\""),
        "drained steering is recorded under the steering_log state key"
    );
    assert!(
        !stripped.contains("\"steering\""),
        "the old bare `steering` state key is gone from CODE"
    );
}

/// The structural half, COMMENT-STRIPPED: `decide` reads no steering key, so
/// the round's rename is an accurate description rather than a relabel.
///
/// Comment-stripping is required here and forbidden in the pin above, for
/// opposite reasons: a comment naming `steering` must not be able to fake a
/// binding read, and a doc comment under test must not be deleted before the
/// assertion runs.
#[test]
fn decide_does_not_read_the_steering_channel() {
    let body = decide_body();
    assert!(
        !body.contains("steering"),
        "decide still reads a steering key — the rename would be a lie"
    );
    for key in ["status", "pending_question", "next_step", "next_state"] {
        assert!(
            body.contains(&format!("\"{key}\"")),
            "decide reads the routing key `{key}`"
        );
    }
}
