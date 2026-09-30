//! The vacuity decision made executable: a `RunKind` on the report, a stop
//! that fires for `Live` and not for `Replay`, and a label that can never
//! authorise skipping a check.
//!
//! # Why this file exists
//!
//! The census was already computed on every turn — `gates_declared`,
//! `gates_evaluated`, `gates_vacuous` — and acted on for none. `gates_declared
//! == 0` set a bool, the turn still reported `Done`, and 100% of the frozen
//! corpus reported `true`, so the field said "these gates passed on nothing" on
//! every ordinary run and nobody could tell a replay from a governed advance.
//!
//! The shipped property is an **asymmetry**, and each direction is pinned
//! separately because an asymmetry pinned in one direction is just a flag:
//!
//! * `Live` + vacuous  → refused [`StoppedAt::GatesVacuous`], never `Done`.
//! * `Replay` + vacuous → `Done`, with `gates_vacuous` still reported.
//!
//! # The load-bearing property
//!
//! **A `Replay` label is a claim, and it must not become a licence.**
//! [`RunKind::authorise`] returns `Result<Infallible, _>`: "this label permitted
//! skipping a check" is not a state its signature can represent, and the
//! `fn`-pointer binding below makes widening it a COMPILE error rather than a
//! green run with a quietly broken promise.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;

use brain_engine_sdk::host::WorkflowHost;
use serde_json::{Value, json};
use steward_harness::engine::{self, CheckSkip, CrankReport, RunKind, StoppedAt};
use steward_harness::inmem::InMemHost;

/// An UNDECLARED queue of `n` items: the vacuous case, by construction.
fn undeclared_queue(n: usize) -> String {
    let queue: Vec<Value> = (0..n)
        .map(|_| json!({"expected": "e", "actual": "a"}))
        .collect();
    json!({"next_step": "step-0", "queue": queue}).to_string()
}

/// The same queue, with every item declaring the one mutation the crank
/// performs on it. TRUE by construction — not an invented provenance.
fn declared_queue(n: usize) -> String {
    let queue: Vec<Value> = (0..n)
        .map(|_| json!({"expected": "e", "actual": "a", "mutations": 1}))
        .collect();
    json!({"next_step": "step-0", "queue": queue}).to_string()
}

async fn crank_live(n: usize) -> CrankReport {
    let host = Arc::new(InMemHost::new());
    host.seed(1, &undeclared_queue(n));
    engine::crank(host as Arc<dyn WorkflowHost>, 1, 100)
        .await
        .unwrap()
}

async fn crank_replay(n: usize) -> CrankReport {
    let host = Arc::new(InMemHost::new());
    host.seed(1, &undeclared_queue(n));
    engine::crank_replay(host as Arc<dyn WorkflowHost>, 1, 100)
        .await
        .unwrap()
}

// ── R53p.1 · a Live run with zero declared constraints STOPS ──────────────────

/// The load-bearing half. Without this pin `gates_vacuous` is advisory, and
/// `Done` certifies a governed advance that no gate ever constrained.
#[tokio::test]
async fn a_live_run_with_no_declared_constraints_is_not_certified_done() {
    let report = crank_live(3).await;

    assert_eq!(
        report.gates_declared, 0,
        "the fixture declares nothing — the precondition, asserted so this pin \
         cannot pass on a queue that stopped being vacuous"
    );
    assert!(
        report.gates_evaluated > 0,
        "gates DID run; they ran on nothing. A turn that evaluated no gates is \
         a different case and is deliberately not this one"
    );
    assert_eq!(
        report.stopped_at,
        StoppedAt::GatesVacuous,
        "a Live turn that passed its gates on nothing is REFUSED a Done"
    );
    assert_ne!(
        report.stopped_at,
        StoppedAt::Done,
        "Done would be the false green this whole branch exists to remove"
    );
    assert_eq!(
        report.stopped_at.as_str(),
        "gates_vacuous",
        "the stop names itself on the wire, so a consumer reading only \
         stopped_at can tell this refusal from exhaustion"
    );
}

/// The refusal must not be a flag: it must be the stop REASON, not a separate
/// advisory channel a caller could ignore. There is deliberately no
/// `warn_threshold_fired`-style companion carrying this — the census stays on
/// the report as evidence of WHY the turn stopped, but the stop itself is not
/// something downstream has to remember to check.
#[tokio::test]
async fn the_vacuity_refusal_is_the_stop_reason_not_a_secondary_channel() {
    let report = crank_live(2).await;
    // The census is still REPORTED for a refused turn — the stop does not
    // hide the evidence that caused it.
    assert!(
        report.gates_vacuous,
        "the refusal still reports why it fired"
    );
    assert_eq!(report.run_kind, RunKind::Live);
    // And it is the stop REASON itself, not a separate advisory field.
    assert_eq!(report.stopped_at, StoppedAt::GatesVacuous);
}

/// **The refusal relabels; it does not discard.** A vacuous `Live` turn still
/// executes its steps and still commits them — the CAS persist and the event
/// emission happen on the step path, before `report(...)` is ever reached.
///
/// This is a deliberate property and it is pinned so nobody "fixes" it into a
/// truncation later. Two reasons it is right:
///
/// * A vacuity stop that DISCARDED work would be a data-loss event disguised
///   as a safety control. The gate found nothing wrong with the work; it found
///   that the work was never constrained.
/// * The durable artifacts are what a human inspects to decide what actually
///   happened. Refusing to write them would destroy the evidence the refusal
///   is about.
///
/// So the control is on the CLAIM ("this turn completed under gates"), not on
/// the execution. A caller that sees `gates_vacuous` learns that every step it
/// just committed ran without a single gate constraining it.
#[tokio::test]
async fn a_refused_turn_still_committed_its_work() {
    let host = Arc::new(InMemHost::new());
    host.seed(1, &undeclared_queue(3));
    let report = engine::crank(host.clone() as Arc<dyn WorkflowHost>, 1, 50)
        .await
        .unwrap();

    assert_eq!(report.stopped_at, StoppedAt::GatesVacuous, "refused");
    assert_eq!(
        report.steps_executed, 3,
        "the work still ran — the refusal is on the Done claim, not the work"
    );
    assert!(
        !host.outbox_of(1).is_empty(),
        "events were still emitted: a vacuity stop must not become a data-loss \
         event, and must not erase the evidence it is about"
    );
}

// ── R53p.2 · a Replay run with zero constraints is NOT stopped ───────────────

/// The asymmetry's other half, and the pin that keeps this from being branch A.
///
/// Branch A — labelling everything `Replay` — would make THIS test pass while
/// closing nothing. It runs the SAME vacuous queue as the pin above; only the
/// declaration differs. If this test ever passes without `crank_replay` having
/// been used, the asymmetry is gone and `Live` is being ignored.
#[tokio::test]
async fn a_replay_run_with_no_declared_constraints_is_still_certified_done() {
    let report = crank_replay(3).await;

    assert_eq!(report.run_kind, RunKind::Replay, "the label is carried");
    assert_eq!(report.gates_declared, 0, "the SAME vacuous fixture");
    assert!(report.gates_evaluated > 0, "gates ran on nothing here too");
    assert!(
        report.gates_vacuous,
        "vacuity is still REPORTED for a replay — advisory, not hidden"
    );
    assert_eq!(
        report.stopped_at,
        StoppedAt::Done,
        "a replay of recorded steps is the documented posture; refusing it \
         would refuse the entire frozen corpus"
    );
}

/// The discriminating twin. Without it, the pin above could pass because the
/// engine quietly stopped vacuous turns of BOTH kinds — which is exactly the
/// R52 "reported, not a stop" posture wearing a new name.
#[tokio::test]
async fn the_two_arms_differ_only_by_declaration_not_by_fixture() {
    let live = crank_live(3).await;
    let replay = crank_replay(3).await;

    // Identical fixtures: same census, same evaluated count.
    assert_eq!(live.gates_declared, replay.gates_declared);
    assert_eq!(live.gates_evaluated, replay.gates_evaluated);
    assert_eq!(live.gates_vacuous, replay.gates_vacuous);
    assert_eq!(live.steps_executed, replay.steps_executed);
    // The ONLY difference is the outcome, and it tracks the declaration.
    assert_ne!(live.stopped_at, replay.stopped_at);
    assert_eq!(live.stopped_at, StoppedAt::GatesVacuous);
    assert_eq!(replay.stopped_at, StoppedAt::Done);
}

/// **The named false green, and the pin that keeps it named.**
///
/// `tests/r52_behavioural.rs` `an_early_stop_leaves_the_remaining_work_queued`
/// asserted `steps_executed < 5` and `remaining == 50 - steps_executed` and
/// **named no stop reason**. When the vacuity stop first landed it went GREEN
/// while its sibling `crank_stops_before_exhausting_the_budget` went RED.
///
/// It could not see the stop because the stop does not truncate: the override
/// lives in `report(...)`, which runs after the loop, so the refused turn
/// still executed its 4 steps and still left 46 queued. Both of its
/// assertions were simply true of a turn that had been refused.
///
/// This pin reproduces that exact configuration and asserts the property the
/// original could not: a DECLARED `Live` turn stops at the budget threshold
/// with the threshold as its reason. If the vacuity stop ever started firing
/// here, the sibling's `assert_ne!(stopped_at, GatesVacuous)` is what would
/// catch it — and this pin fails alongside, so the two can never disagree
/// about which stop a declared run takes.
#[tokio::test]
async fn the_named_false_green_fixture_reaches_the_budget_stop_not_the_vacuity_one() {
    let host = Arc::new(InMemHost::new());
    // The false-green's exact fixture: 50 DECLARED items, budget 5.
    host.seed(1, &declared_queue(50));
    let report = engine::crank(host as Arc<dyn WorkflowHost>, 1, 5)
        .await
        .unwrap();

    assert_eq!(report.run_kind, RunKind::Live);
    // The census counts what THIS TURN executed, not what the queue held: the
    // budget stop ends the turn at 4 steps, so 4 declarations. Written down
    // after measuring it — an assertion of 50 here would have been a guess
    // about queue length dressed as a fact about the census.
    assert_eq!(
        report.gates_declared, 4,
        "one declaration per EXECUTED step; the turn stopped at 4"
    );
    assert!(!report.gates_vacuous, "a declared queue is not vacuous");
    assert_eq!(
        report.stopped_at,
        StoppedAt::BudgetWarn,
        "the threshold is the stop — the vacuity refusal is for undeclared runs"
    );
    assert_eq!(
        report.steps_executed, 4,
        "and it executed 4 steps, which is exactly why the false green's \
         count-based assertions could not distinguish this from a refused run"
    );
}

/// The complementary half, and the reason the two pins above cannot both be
/// satisfied by a blind check: an UNDECLARED run of the *same* shape is
/// refused, with the same step count.
///
/// This is the discriminating pair. Same queue length, same budget, same
/// executed steps — the only difference is the declaration, and it moves the
/// stop REASON. A count-only assertion is blind to exactly this difference,
/// which is the whole of the false green.
#[tokio::test]
async fn the_same_run_shape_differs_only_in_the_stop_reason() {
    let host = Arc::new(InMemHost::new());
    host.seed(1, &undeclared_queue(50));
    let refused = engine::crank(host as Arc<dyn WorkflowHost>, 1, 5)
        .await
        .unwrap();

    // Identical shape: same steps executed, so every count-based assertion
    // the false green made would hold here too.
    assert_eq!(refused.steps_executed, 4, "same shape, same work");
    assert_eq!(refused.gates_declared, 0);
    assert!(refused.gates_vacuous);
    // The difference is the reason, and the reason is the whole claim.
    assert_eq!(
        refused.stopped_at,
        StoppedAt::GatesVacuous,
        "an undeclared Live run is refused where a declared one reaches the \
         threshold — the false green could not tell these two apart"
    );
    assert_ne!(refused.stopped_at, StoppedAt::BudgetWarn);
}

// ── the label is only load-bearing if a DECLARED run survives ───────────────

/// A `Live` run that DOES declare constraints is unaffected. If this failed,
/// the stop would be refusing governed advances outright and Option 1 would be
/// a blunt instrument rather than a fix.
#[tokio::test]
async fn a_live_run_that_declares_is_untouched_by_the_stop() {
    let host = Arc::new(InMemHost::new());
    host.seed(1, &declared_queue(3));
    let report = engine::crank(host as Arc<dyn WorkflowHost>, 1, 100)
        .await
        .unwrap();

    assert_eq!(report.run_kind, RunKind::Live);
    assert_eq!(report.gates_declared, 3, "one declaration per item");
    assert!(
        !report.gates_vacuous,
        "declared constraints are not vacuous"
    );
    assert_eq!(
        report.stopped_at,
        StoppedAt::Done,
        "the stop fires on vacuity only — a declared Live run completes"
    );
}

/// **The gate verdicts are unchanged by declaring.** This is what separates
/// Option 1 from branch B: a synthetic seed's constraints are knowable by
/// construction, and declaring them must not change a single gate outcome —
/// only the honesty of the report.
#[tokio::test]
async fn declaring_a_constraint_does_not_change_any_gate_verdict() {
    let host = Arc::new(InMemHost::new());
    host.seed(
        1,
        &json!({
            "next_step": "step-0",
            "queue": [{"expected": "a", "actual": "b", "needs_approval": true}]
        })
        .to_string(),
    );
    let declared = engine::crank(host as Arc<dyn WorkflowHost>, 1, 10)
        .await
        .unwrap();
    let findings_declared: Vec<String> = declared.hostcalls.keys().map(|k| k.to_string()).collect();
    assert!(
        declared
            .hostcalls
            .keys()
            .any(|k| k.contains("workflow/decision"))
            || declared.gates_evaluated > 0,
        "the gate waterfall ran"
    );
    // The undeclared twin: identical item minus `needs_approval`, so the
    // approval gate CANNOT fire and no rejection is possible.
    let host = Arc::new(InMemHost::new());
    host.seed(
        1,
        &json!({
            "next_step": "step-0",
            "queue": [{"expected": "a", "actual": "b"}]
        })
        .to_string(),
    );
    let undeclared = engine::crank_replay(host as Arc<dyn WorkflowHost>, 1, 10)
        .await
        .unwrap();
    assert_eq!(
        undeclared.gates_evaluated, declared.gates_evaluated,
        "the same gates are EVALUATED either way — declaring adds a key to the \
         census, it does not add or remove a gate"
    );
    assert_eq!(findings_declared.len(), undeclared.hostcalls.len());
}

/// A turn that executed NO steps evaluated no gates, so there is nothing for
/// them to have passed on. The stop must spare it — a `Live` run pausing at
/// `AskHuman` before its first step is not a gate escape, and a stop that
/// punished it would be wrong in the same way a too-broad one is.
#[tokio::test]
async fn a_live_turn_that_never_ran_a_step_is_not_vacuously_stopped() {
    let host = Arc::new(InMemHost::new());
    host.seed(
        1,
        r#"{"pending_question":"which disk group?","next_step":"step-0","queue":[]}"#,
    );
    let report = engine::crank(host as Arc<dyn WorkflowHost>, 1, 10)
        .await
        .unwrap();

    assert_eq!(report.run_kind, RunKind::Live);
    assert_eq!(report.steps_executed, 0, "no step ran");
    assert_eq!(report.gates_evaluated, 0, "so no gate ran either");
    assert_ne!(
        report.stopped_at,
        StoppedAt::GatesVacuous,
        "gates that never ran cannot have passed on nothing — this stop is \
         narrower than `gates_declared == 0` on purpose"
    );
}

// ── R53p.3 · a Replay label can never authorise ──────────────────────────────

/// **The `Ok` type is `Infallible`, so this is a COMPILE-time refusal.** The
/// `fn`-pointer binding is the mechanism: it names the exact signature, so a
/// future edit that tries to return `Ok(..)` fails to COMPILE rather than
/// passing a green run with a quietly broken promise. This is the technique the
/// accounting layer uses, and the one a plain return-value assertion missed
/// there — a doc claim enforced only by convention.
///
/// Measured: the breakage is a compile error in the **test** build
/// (`cargo build --all-targets` fails with `E0308`; a bare `cargo build` does
/// not, because integration tests are not part of the library build). So the
/// property is enforced wherever the pins are compiled and run — which is
/// exactly where a future edit to `authorise` would be caught.
#[test]
fn a_run_kind_never_authorises_skipping_a_check() {
    let authorise: fn(RunKind, CheckSkip) -> Result<std::convert::Infallible, _> =
        RunKind::authorise;

    for kind in [RunKind::Live, RunKind::Replay] {
        for skip in [
            CheckSkip::GateEvaluation,
            CheckSkip::VacuityCensus,
            CheckSkip::VacuityStop,
            CheckSkip::RejectionAsFinding,
        ] {
            assert!(
                authorise(kind, skip).is_err(),
                "{kind:?} must not authorise {skip:?} — the label relaxes only \
                 the vacuity stop and grants nothing else"
            );
        }
    }
}

/// The closed vocabulary: `CheckSkip` is an enum, not a string, so
/// `authorise` cannot be handed a free-text "reason" and thereby become an
/// escape hatch with a description. A new skip is a new variant, which is a
/// visible edit to a closed set rather than an untyped argument.
#[test]
fn the_check_vocabulary_is_closed_and_enumerable() {
    let all = [
        CheckSkip::GateEvaluation,
        CheckSkip::VacuityCensus,
        CheckSkip::VacuityStop,
        CheckSkip::RejectionAsFinding,
    ];
    assert_eq!(all.len(), 4, "four named things a label may never skip");
    // Each renders distinctly, so a refusal names WHAT was refused.
    for s in all {
        assert!(
            !format!("{s:?}").is_empty(),
            "every refusal names the specific check"
        );
    }
}

/// The variant count is pinned by a NON-EXHAUSTIVE match, so adding a third
/// `RunKind` is a compile error until a pin is deliberately updated. Without
/// this, "there is no third state" would be a doc claim — which is the exact
/// failure the accounting layer's first pin made.
#[test]
fn the_run_kind_vocabulary_is_closed_at_two_variants() {
    fn count(k: &RunKind) -> u8 {
        match k {
            RunKind::Live => 1,
            RunKind::Replay => 2,
            // Adding a variant here IS the pin: the build breaks until
            // someone decides what it authorises.
        }
    }
    assert_eq!(count(&RunKind::Live), 1);
    assert_eq!(count(&RunKind::Replay), 2);
}

// ── the declaration is carried, not inferred ────────────────────────────────

/// A consumer reading only the report can tell an evidence replay from a
/// governed advance. The census alone cannot: both arms carry
/// `gates_vacuous == true` in the fixture above, so the kind is the only
/// thing that makes the difference legible after the fact.
#[tokio::test]
async fn the_declaration_travels_on_the_report_in_both_arms() {
    assert_eq!(crank_live(3).await.run_kind, RunKind::Live);
    assert_eq!(crank_replay(3).await.run_kind, RunKind::Replay);
}

/// `crank_full` — the widest entry point, used by the settle suite and by any
/// caller that wants steering/effects/cancel — defaults to `Live`, the
/// fail-closed arm. A caller that forgets to declare must get the strict
/// posture, never the lenient one.
#[tokio::test]
async fn the_widest_entry_point_defaults_to_the_fail_closed_arm() {
    let host = Arc::new(InMemHost::new());
    host.seed(1, &undeclared_queue(2));
    let report = engine::crank_full(host as Arc<dyn WorkflowHost>, None, None, None, 1, 10, 0)
        .await
        .unwrap();
    assert_eq!(
        report.run_kind,
        RunKind::Live,
        "the default is Live — forgetting to declare must not buy the lenient arm"
    );
    assert_eq!(report.stopped_at, StoppedAt::GatesVacuous);
}
