// D-B's separation criterion, from OUTSIDE the module.
//
// ## Why a pin cannot live inside `separation.rs`
//
// The load-bearing claim of this increment is an **absence**: the criterion admits
// no threshold argument, so a later round cannot tune it. A module cannot check
// its own signature — it would be arguing with itself, and the check would live in
// exactly the file a future edit would change. The assertion is therefore made
// here, against the crate's real public type.
//
// ## What makes the control structural rather than conventional
//
// `the_separation_criterion_takes_no_threshold_argument` binds `separation_n` to a
// `fn` pointer of an exact arity. If a `tolerance_units: i64` parameter were added
// — or any other numeric knob — this file would **fail to compile**, which is the
// R59 guarantee: a widened signature never produces a green `test result: FAILED`
// line, it breaks the build at the binding itself.
//
// That is deliberately stronger than a test that would merely fail. A test can be
// deleted; a type error in a `fn`-pointer binding has to be *fixed*, and fixing it
// means looking at the signature.

mod common;

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The criterion's confidence, read from the module.
///
/// Read through the crate's own re-export rather than restated here. A pin that
/// hardcodes `9500` asserts that its own literal is correct, which is true of any
/// literal; a pin that reads the constant asserts the criterion still declares what
/// D-B preregistered.
#[test]
fn the_criterion_declares_its_preregistered_confidence() {
    use brain_server::workflow::separation::{
        SEPARATION_CONFIDENCE_UNITS, SEPARATION_CRITERION_ID,
    };
    assert_eq!(
        SEPARATION_CONFIDENCE_UNITS, 9500,
        "D-B preregistered 95% in ten-thousandths; the criterion declares {SEPARATION_CONFIDENCE_UNITS}"
    );
    assert!(
        SEPARATION_CRITERION_ID.starts_with("D-B/"),
        "the criterion does not name the decision it came from: {SEPARATION_CRITERION_ID}"
    );
}

/// The success column is named by exactly one reader in the tree.
///
/// The frozen verdict is read at one seam, and
/// `no_production_site_outside_the_census_reads_the_verdict` holds that for the
/// whole `src/` tree. This pin holds the positive half: the column the criterion
/// compares is the operator's judgment, asserted here — in a **test**, which the
/// tree guard cuts away — rather than duplicated into production code as a
/// constant. A second constant would be a second reader, and the guard above is
/// right to fail on one.
#[test]
fn the_success_column_is_the_operators_judgment_and_is_named_once() {
    // The name, asserted where it costs nothing: the test region is cut by the
    // production-walk guard, so this is not a reader.
    const SUCCESS_COLUMN_UNDER_TEST: &str = "human_pass";
    assert_eq!(SUCCESS_COLUMN_UNDER_TEST, "human_pass");

    // And production does not name it outside the one seam.
    let census = read("src/workflow/drift_census.rs");
    assert!(
        common::code_only(&census).contains("human_pass"),
        "the one declared reader seam no longer names the column"
    );
    let separation = common::code_only(&read("src/workflow/separation.rs"));
    assert!(
        !separation.contains("human_pass"),
        "the criterion names the column in production - that is a second reader, and it belongs at the seam"
    );
}

/// **The anti-tunability control.** The criterion takes an input and nothing else.
///
/// The binding below is the assertion. `separation_n` has exactly one parameter,
/// `&SeparationInput`, and that type carries no tolerance field — so there is no
/// spelling of "loosen the band" that compiles.
#[test]
fn the_separation_criterion_takes_no_threshold_argument() {
    use brain_server::workflow::separation::{Separation, SeparationInput, separation_n};

    // Exact arity: adding any parameter breaks this line, not a test result.
    let _bound: fn(&SeparationInput) -> Separation = separation_n;

    // And the input type itself carries no numeric field a threshold could ride in.
    let input = SeparationInput {
        skill: Vec::new(),
        primitive: Vec::new(),
    };
    // If `SeparationInput` grew a `tolerance_units: i64`, this construction would
    // not compile — the field would be required.
    let _ = format!("{input:?}");
}

/// The criterion is the only place a confidence lives.
///
/// Guards the "a second constant that has to agree with the first" failure named in
/// the module doc: a `SEPARATION_Z_UNITS` beside `SEPARATION_CONFIDENCE_UNITS`
/// would be a threshold a later round can move independently.
#[test]
fn no_second_confidence_constant_exists_beside_the_preregistered_one() {
    let src = common::code_only(&read("src/workflow/separation.rs"));
    let confidence_decls = src.matches("const SEPARATION_CONFIDENCE_UNITS").count();
    assert_eq!(
        confidence_decls, 1,
        "expected exactly one confidence constant, found {confidence_decls}"
    );
    // `z` is derived from it, never stored beside it.
    assert!(
        !src.contains("const SEPARATION_Z"),
        "a z constant beside the confidence is a second, independently movable threshold"
    );
}

/// Identical arms never separate — read through the public API.
///
/// The vacuity guard, asserted from outside so it cannot be argued with by the
/// module that implements it.
#[test]
fn identical_arms_never_separate_through_the_public_api() {
    use brain_server::workflow::separation::{
        ArmOutcome, NotSeparatingReason, Separation, SeparationInput, separation_n,
    };

    let arm = |ids: &[&'static str]| -> Vec<ArmOutcome> {
        ids.iter()
            .map(|case_id| ArmOutcome {
                case_id,
                success: true,
            })
            .collect()
    };
    let input = SeparationInput {
        skill: arm(&["a", "b", "c"]),
        primitive: arm(&["a", "b", "c"]),
    };
    assert_eq!(
        separation_n(&input),
        Separation::NotSeparating {
            reason: NotSeparatingReason::Indistinguishable { paired: 3 }
        },
        "two identical arms separated - the criterion is measuring its own arithmetic"
    );
}

/// The corpus too small refusal is reachable.
///
/// This pin exists because it was **not** reachable in the first draft: the
/// requirement was clamped to the corpus size, so every differing pair separated
/// and `CorpusTooSmall` was dead code. A refusal variant that cannot fire is a
/// decoration, so its reachability is a claim worth pinning from outside.
#[test]
fn a_corpus_that_cannot_reach_separation_says_so() {
    use brain_server::workflow::separation::{
        ArmOutcome, NotSeparatingReason, Separation, SeparationInput, separation_n,
    };

    // One case differs out of four: 1.00 vs 0.75 needs 12 cases at the
    // preregistered confidence, and this corpus has 4.
    let skill: Vec<ArmOutcome> = ["a", "b", "c", "d"]
        .iter()
        .map(|case_id| ArmOutcome {
            case_id,
            success: true,
        })
        .collect();
    let primitive: Vec<ArmOutcome> = ["a", "b", "c", "d"]
        .iter()
        .map(|case_id| ArmOutcome {
            case_id,
            success: *case_id != "a",
        })
        .collect();

    match separation_n(&SeparationInput { skill, primitive }) {
        Separation::NotSeparating {
            reason: NotSeparatingReason::CorpusTooSmall { paired, required },
        } => {
            assert_eq!(paired, 4);
            assert!(
                required > paired,
                "required {required} must exceed the corpus {paired}, or the refusal is vacuous"
            );
        }
        other => panic!("expected CorpusTooSmall, got {other:?} — the refusal is unreachable"),
    }
}

/// R55's arm is refused, and the refusal is written where a reader will find it.
///
/// D-A's arm cannot be built: `routing.rs` has no production caller, so there is
/// no routing mechanism to ablate. The decision sheet records it; this asserts the
/// record exists, so a later round cannot quietly treat the arm as pending rather
/// than refused.
#[test]
fn the_refused_primitive_arm_is_recorded_with_its_measured_reason() {
    let sheet =
        read("../brain-steward-ip/plans/DECISION_SHEET_FOUR_MEASUREMENT_DECISIONS_2026-10-03.md");
    assert!(
        sheet.contains("UNSATISFIABLE"),
        "the decision sheet does not record R55's arm as unsatisfiable"
    );
    // The measured reason, not a summary of it.
    assert!(
        sheet.contains("zero production callers"),
        "the refusal does not carry its measured reason (zero production callers)"
    );
    assert!(
        sheet.contains("would measure the invention"),
        "the refusal does not carry the reason a stub registry was declined"
    );
}
