//! R52 behavioural red-proofs — pins that COMPILE against pre-fix code.
//!
//! Why this file exists: the pins in `report.rs` reference
//! `StoppedAt::BudgetWarn` and the three gate counters, so against the
//! pre-fix tree they fail at the COMPILER. That is a valid red and the
//! verified plan sanctions it for `R52.2` — but it proves only that the API
//! surface was absent, not that the behaviour was wrong.
//!
//! Every pin here uses ONLY the pre-existing surface: `engine::crank`,
//! `CrankReport::steps_executed`, `StoppedAt::as_str`, `warn_threshold_fired`,
//! and `InMemHost`. None of it changed in R52. So these compile before and
//! after, and the red lands on an ASSERTION — which is the stronger claim:
//! "the old code does the wrong thing", not "the old code lacks a field".

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

fn long_queue(n: usize) -> String {
    let queue: Vec<Value> = (0..n)
        // Declared, because this is a `RunKind::Live` turn. See the same
        // helper in `report.rs` for why `mutations: 1` is a true statement
        // about a queued item rather than an invented provenance.
        .map(|_| json!({"expected": "e", "actual": "a", "mutations": 1}))
        .collect();
    json!({"next_step": "step-0", "queue": queue}).to_string()
}

/// R52.1 · Warn-too-late, as an ASSERTION.
///
/// The pre-fix crank OR-ed the 80% flag into `warn_threshold_fired` and fell
/// through to `cas_persist`, so it ran to budget exhaustion and reported the
/// threshold only afterwards. This pin compiles against both trees and asks
/// the behavioural question: did the turn stop when it crossed 80%?
///
/// Pre-fix: `steps_executed == 5 == max_steps` — it ran to the end.
/// Post-fix: `steps_executed == 4` — it stopped at the threshold.
#[tokio::test]
async fn crank_stops_before_exhausting_the_budget() {
    let host = Arc::new(InMemHost::new());
    host.seed(1, &long_queue(50));
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank(h, 1, 5).await.unwrap();
    assert_eq!(report.run_kind, engine::RunKind::Live);
    assert!(
        report.steps_executed < 5,
        "crossing 80% of a 5-step budget must end the turn before the 5th step; \
         the crank ran all {} steps and reported the warning only at the end",
        report.steps_executed
    );
    // And the stop reason must name the threshold, not ordinary exhaustion.
    assert_eq!(
        report.stopped_at.as_str(),
        "budget_warn",
        "the stop reason distinguishes the threshold from budget exhaustion"
    );
}

/// R52.1, the other half · the same question asked of the PERSISTED state.
///
/// A report that stops early but leaves the whole queue executed would be
/// lying in the other direction. The work must still be there.
///
/// # This assertion used to be blind to WHY the turn stopped
///
/// It read `assert!(steps_executed < 5)` and
/// `remaining == 50 - steps_executed`, and **neither mentions the stop
/// reason**. Measured against the vacuity stop, the crank still executes its
/// four steps and still leaves `50 - 4` queued — the vacuity stop RELABELS
/// `stopped_at` rather than truncating the turn (the override lives in
/// `report(...)`, which runs after the loop) — so both assertions hold while
/// the turn has been refused. It went GREEN while its sibling
/// `crank_stops_before_exhausting_the_budget`, which names the stop reason,
/// went RED. That is the failure mode the surface mutant was renamed for: a
/// check structurally incapable of noticing.
///
/// The fix is not one assertion but three, because they cover three distinct
/// failures, and the red-proofs showed each is load-bearing:
///
/// * `assert_ne!(stopped_at, GatesVacuous)` — the only guard that catches the
///   vacuity stop, which changes the reason while leaving the counts intact.
/// * `steps_executed > 0` — catches a crank that executed nothing.
/// * `remaining > 0` — catches a turn that finished rather than stopped early.
///
/// Measured, not assumed: removing the `assert_ne!` turns this test RED, and
/// removing the two bounds while KEEPING it also turns it RED. None of the
/// three is redundant with the others.
#[tokio::test]
async fn an_early_stop_leaves_the_remaining_work_queued() {
    let host = Arc::new(InMemHost::new());
    host.seed(1, &long_queue(50));
    let h = host.clone() as Arc<dyn WorkflowHost>;
    let report = engine::crank(h, 1, 5).await.unwrap();
    assert_eq!(
        report.run_kind,
        engine::RunKind::Live,
        "a declared live run"
    );
    // The turn must not have been vacuity-stopped — the gates ran, on
    // declared constraints, and the stop under test is the BUDGET one. This
    // is the guard the original lacked: a vacuity stop relabels the reason
    // without changing the counts, so only naming the reason can see it.
    assert_ne!(
        report.stopped_at,
        StoppedAt::GatesVacuous,
        "this test is about the budget stop, not the vacuity refusal"
    );
    assert_eq!(
        report.stopped_at,
        StoppedAt::BudgetWarn,
        "and the reason is the budget threshold specifically"
    );
    assert!(
        report.steps_executed > 0,
        "work was actually performed; a zero-step crank defers everything by \
         doing nothing, which is not an early stop"
    );
    assert!(report.steps_executed < 5);

    let (js, _) = host.state(1).unwrap();
    let st: Value = serde_json::from_str(&js).unwrap();
    let remaining = st["queue"].as_array().expect("a queue").len();
    assert_eq!(
        remaining,
        50 - report.steps_executed as usize,
        "the stop deferred work rather than consuming it"
    );
    assert!(
        remaining > 0,
        "an early stop leaves admissible work behind — an empty queue means \
         the turn finished, it did not stop early"
    );
}

/// R52.3 · Steering, as an ASSERTION on the source of truth.
///
/// The pre-fix `lib.rs:11` claimed steering drains were "advisory inputs to
/// the next decision". `decide` never read them. This pin compiles against
/// both trees; pre-fix it FAILS on the claim, post-fix on nothing.
///
/// Read on RAW text deliberately: the claim IS a doc comment, so stripping
/// comments here would delete the thing under test and pass vacuously. The
/// comment-stripped half of this claim lives in `report.rs`
/// (`decide_does_not_read_the_steering_channel`), where a comment naming
/// `steering` must not be able to fake a binding read.
#[test]
fn no_source_file_claims_steering_reaches_the_next_decision() {
    let src = crate_root().join("src");
    for entry in ["lib.rs", "engine.rs", "remote_host.rs", "main.rs"] {
        let path = src.join(entry);
        let text = std::fs::read_to_string(&path).expect("the harness source is readable");
        assert!(
            !text.contains("advisory inputs to the next decision"),
            "{entry} claims steering reaches the next decision; `decide` never reads it"
        );
        assert!(
            !text.contains("as advisories for the next decision"),
            "{entry} claims drained steering is an advisory for `decide`"
        );
    }
}
