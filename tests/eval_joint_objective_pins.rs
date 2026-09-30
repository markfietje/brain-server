// R53 — the joint evaluation objective: safety as a FEASIBILITY CONSTRAINT.
//
// # What this file exists to prove
//
// Decision D2 (`plans/DECISION_RECORD_AND_MODEL_ROUTING_2026-09-28.md` §D2)
// fixes the shape of the eval objective:
//
// ```text
// maximize   ( accuracy , local_inference_cost )
// subject to  safety_violations == 0
// ```
//
// The failure this shape exists to prevent is a *joint reward* (MoMHa,
// `arXiv:2609.30967`) blending accuracy, safety, and tokens with weights. A
// weight on safety is a licence to trade it, and that trade must not be
// available. So the load-bearing claims here are not "the objective is joint"
// — they are **which objectives accept a given case**, asserted as a
// discriminating set of arms against the same numbers.
//
// # Why these pins are behavioural, not structural assertions about a diff
//
// Each red-proof below was demonstrated failing against a planted pathology
// before it was accepted (transcripts in the round's report; method: copy to
// `target/` scratch, plant, run, restore with `cp`, prove with `sha256` +
// `cmp`). The plants were, in order: deleting the cost ceiling from `admit`
// (R53.2), deleting the accuracy floor (R53.3), making `SafetyEvidence::cleared`
// accept a non-zero count (R53.4), taking the element-wise minimum in
// `JointFloor::raised_to` (R53.5), and adding a `safety_violations` field to
// `EvalReport` (R53.1). The compile-time plants are real too: widening
// `FeasibleCase::new` to take a raw `u32` instead of a `SafetyEvidence` is an
// `E0308` at the `fn`-pointer binding below.
//
// # The honesty this file does not paper over
//
// The cost term is a CALLER-SUPPLIED PROXY. The harness has no per-case token
// accounting and no wall-clock instrumentation; `Usage` is `pub(crate)` and never
// reaches an eval. These pins therefore pin the objective's LOGIC, and no pin
// here claims the cost number means anything. The provenance of that number is
// the caller's responsibility, and nothing in this file can check it.

#![forbid(unsafe_code)]

use brain_server::eval::{
    FeasibleCase, FloorBreach, JointFloor, JointObjective, JointVerdict, LocalCost, SafetyEvidence,
    SafetyViolation, accuracy_only_held, admit,
};

// ---------------------------------------------------------------------------
// Fixtures — the preregistered floor, and a baseline that sits just inside it
// on every term, so any single term can be moved independently.
// ---------------------------------------------------------------------------

/// The preregistered joint floor used by every pin below. Fixed constants, not
/// computed, so a change to the objective cannot quietly move the bar.
const FLOOR_ACCURACY: f32 = 0.85;
const FLOOR_COST_TOKENS: u32 = 5_000;

fn floor() -> JointFloor {
    JointFloor::new(FLOOR_ACCURACY, LocalCost::new(FLOOR_COST_TOKENS))
}

/// Clears the floor on every term, with a little headroom on each.
fn baseline() -> JointObjective {
    JointObjective::new(0.86, 0, LocalCost::new(4_000))
}

/// A third arm for the discriminating pairs: an objective that guards ONLY
/// cost. Not shipped, and not proposed — it exists so each pin can show what
/// the other two objectives would have done with the same numbers, which is
/// what makes "the joint objective rejected it" a claim with content.
fn cost_only_held(cost: LocalCost, ceiling: LocalCost) -> bool {
    cost <= ceiling
}

// ---------------------------------------------------------------------------
// R53.1 — by absence, against the pre-existing report
// ---------------------------------------------------------------------------

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {rel}: {e}"))
}

/// The brace-delimited body of the struct whose declaration line is `marker`.
///
/// A whole-file grep passes on the scanner's own literals, which is the vacuous
/// pass this repository treats as worse than no check. The scan is cut to the
/// struct body, and comment lines are stripped, so this pin can neither be
/// satisfied by prose nor broken by `cargo fmt` reflowing a doc line.
fn struct_body(marker: &str) -> String {
    let src = read("src/eval.rs");
    let start = src
        .find(marker)
        .unwrap_or_else(|| panic!("marker not found in src/eval.rs: {marker}"));
    let rest = &src[start..];
    let end = rest
        .find("\n}")
        .unwrap_or_else(|| panic!("no closing brace after {marker}"));
    rest[..=end].to_owned()
}

/// Non-comment, non-brace lines of a struct body — i.e. its field list.
fn field_lines(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.ends_with('{'))
        .filter(|l| *l != "}")
        .map(str::to_owned)
        .collect()
}

/// `R53.1` — the pre-existing `EvalReport` computes an accuracy-only objective:
/// it has **no safety term and no cost term at all**. The pin fails by absence,
/// and this is the absence the round closes.
///
/// The field list is read from the struct body with comment lines stripped, so
/// the surrounding prose (which discusses safety and cost at length) can neither
/// trip this pin nor satisfy it. Only field declarations are examined.
#[test]
fn eval_joint_r53_1_eval_report_has_no_safety_term_and_no_cost_term() {
    let body = struct_body("pub struct EvalReport {");
    let fields = field_lines(&body);

    assert!(
        !fields.is_empty(),
        "field extraction went vacuous — the struct body parsed to nothing, so \
         this pin would pass on a file it never read"
    );

    for banned in [
        "safety",
        "violation",
        "cost",
        "token",
        "usage",
        "wall_clock",
    ] {
        for field in &fields {
            assert!(
                !field.to_lowercase().contains(banned),
                "R53.1 is a by-absence pin: `EvalReport` gained a `{banned}` term \
                 (field `{field}`). The accuracy-only report is what the round \
                 closes — if this field is added deliberately, this pin and the \
                 claim it carries must both be revisited."
            );
        }
    }

    // The absence is of two specific terms, not of accuracy itself: the report
    // is still the six-scalar accuracy report the CLI consumes today.
    assert_eq!(
        fields.len(),
        6,
        "expected the six pre-existing accuracy scalars, found {fields:?}"
    );
}

/// The same absence, stated as a signature: the pre-R53 objective takes no
/// safety argument and no cost argument, so it cannot even express the trade.
///
/// This is the behavioural companion to the by-absence pin above — if a future
/// edit added a cost parameter to the old objective, the `fn`-pointer binding
/// below would stop compiling.
#[test]
fn eval_joint_r53_1_the_pre_r53_objective_has_no_safety_or_cost_input() {
    let held: fn(f32, f32) -> bool = accuracy_only_held;
    assert!(held(0.999, FLOOR_ACCURACY));
    assert!(!held(0.0, FLOOR_ACCURACY));
}

// ---------------------------------------------------------------------------
// R53.2 — THE LOAD-BEARING PIN
// ---------------------------------------------------------------------------

/// `R53.2` — a change that improves accuracy while regressing cost.
///
/// The **accuracy-only** objective accepts it, because it can see only accuracy.
/// The **cost-only** objective rejects it. The **joint** objective rejects it.
/// That three-way split is the evidence: a joint objective on paper and an
/// accuracy objective in practice are otherwise indistinguishable, and this is
/// the one case that tells them apart.
///
/// This is also `D53.4` — accuracy was raised above the floor, and raising
/// accuracy alone bought nothing.
#[test]
fn eval_joint_r53_2_joint_rejects_the_accuracy_for_cost_trade_that_accuracy_only_accepts() {
    let floor = floor();
    let base = baseline();
    let traded = JointObjective::new(0.97, 0, LocalCost::new(9_000));

    // Fixture sanity: this case really is better on accuracy, worse on cost.
    assert!(
        traded.accuracy > base.accuracy,
        "fixture must improve accuracy"
    );
    assert!(traded.cost > base.cost, "fixture must regress cost");

    // Arm 1 — accuracy-only. Accepts, and its acceptance is the defect.
    assert!(
        accuracy_only_held(traded.accuracy, floor.accuracy),
        "fixture sanity: the accuracy-only objective must ACCEPT the traded case, \
         or this pin proves nothing"
    );

    // Arm 2 — cost-only. Rejects, so the joint verdict is not vacuous.
    assert!(
        !cost_only_held(traded.cost, floor.cost),
        "fixture sanity: a cost-only objective rejects the traded case"
    );

    // Arm 3 — the joint objective. Must reject, and must say which term failed.
    let verdict = admit(&traded, &floor);
    assert!(
        matches!(
            verdict,
            JointVerdict::RejectedFloor {
                breach: FloorBreach::CostAboveCeiling,
                ..
            }
        ),
        "R53.2 (load-bearing): the joint objective must REJECT an \
         accuracy-for-cost trade, and name the cost breach. Got {verdict:?} \
         (accuracy-only accepted it: true)."
    );

    // The baseline, for contrast: nothing was rejected because everything is
    // tight here — a rejection must come from the trade, not from the fixture.
    assert!(
        matches!(admit(&base, &floor), JointVerdict::Accepted(_)),
        "fixture sanity: the untraded baseline must be accepted"
    );

    // D53.4, stated directly: accuracy above the floor is not a licence.
    assert!(
        traded.accuracy > floor.accuracy,
        "D53.4: accuracy is well clear of the floor and acceptance still fails"
    );
}

// ---------------------------------------------------------------------------
// R53.3 — the mirror
// ---------------------------------------------------------------------------

/// `R53.3` — a change that regresses accuracy while improving cost and safety.
///
/// The **cost-only** objective accepts it: cost fell 4× and safety is clean. The
/// **accuracy-only** objective also rejects it. The **joint** objective rejects
/// it. A joint objective that only guarded cost would have accepted this, which
/// is why the cost-only arm is here.
///
/// Honest note, because it matters for reading the transcript: this case does
/// not separate the joint objective from the accuracy-only objective — R53.2 is
/// the discriminating case. What R53.3 adds is the negative direction: a cost
/// win and a clean safety count are not licence to lose accuracy.
#[test]
fn eval_joint_r53_3_joint_rejects_the_cost_and_safety_improvement_that_costs_accuracy() {
    let floor = floor();
    let base = baseline();
    let traded = JointObjective::new(0.62, 0, LocalCost::new(1_000));

    // Fixture sanity: worse accuracy, better cost, safety still clean.
    assert!(
        traded.accuracy < base.accuracy,
        "fixture must regress accuracy"
    );
    assert!(traded.cost < base.cost, "fixture must improve cost");
    assert_eq!(traded.safety_violations, 0, "fixture must be safety-clean");

    // Arm 1 — cost-only. ACCEPTS, which is why a cost guard is not enough.
    assert!(
        cost_only_held(traded.cost, floor.cost),
        "fixture sanity: a cost-only objective accepts the traded case, so this \
         pin would be vacuous without it"
    );

    // Arm 2 — accuracy-only. Also rejects; stated so the transcript is not read
    // as a two-way discrimination it is not.
    assert!(!accuracy_only_held(traded.accuracy, floor.accuracy));

    // Arm 3 — the joint objective. Rejects, on the accuracy floor.
    let verdict = admit(&traded, &floor);
    assert!(
        matches!(
            verdict,
            JointVerdict::RejectedFloor {
                breach: FloorBreach::AccuracyBelowFloor,
                ..
            }
        ),
        "R53.3: the joint objective must REJECT a cost+safety improvement that \
         costs accuracy, and name the accuracy breach. Got {verdict:?} \
         (cost-only accepted it: true)."
    );
}

// ---------------------------------------------------------------------------
// R53.4 — safety non-tradeability
// ---------------------------------------------------------------------------

/// `R53.4` — with `safety_violations > 0`, acceptance is refused regardless of
/// how good the accuracy and cost figures are.
///
/// The sweep is exhaustive over the extremes: perfect accuracy with free
/// inference is the strongest case a safety violation could buy, and it is
/// refused. `u32::MAX` violations and `u32::MAX` tokens are included so a
/// saturation or ordering bug cannot hide at the boundary.
#[test]
fn eval_joint_r53_4_safety_violations_refuse_acceptance_however_good_the_figures() {
    let floor = floor();

    for accuracy in [0.0_f32, 0.5, 0.85, 0.99, 1.0] {
        for tokens in [0_u32, 1, FLOOR_COST_TOKENS, u32::MAX] {
            for violations in [1_u32, 2, 7, u32::MAX] {
                let observed = JointObjective::new(accuracy, violations, LocalCost::new(tokens));
                let verdict = admit(&observed, &floor);
                assert!(
                    matches!(
                        verdict,
                        JointVerdict::RejectedSafetyViolation(SafetyViolation { violations: v })
                            if v == violations
                    ),
                    "a safety violation must refuse acceptance whatever else is true \
                     (accuracy={accuracy}, tokens={tokens}, violations={violations}). \
                     Got {verdict:?}"
                );
            }
        }
    }
}

/// The same claim, at its root: `SafetyEvidence` is the only path to a feasible
/// case, and a zero count is the only count that produces one.
///
/// This is the mechanism pin. `R53.4` above pins the verdict; this pins that the
/// verdict cannot be reached by a different route, because there is no other
/// constructor to take.
#[test]
fn eval_joint_r53_4_safety_evidence_is_only_constructible_from_a_zero_count() {
    assert!(
        SafetyEvidence::cleared(0).is_ok(),
        "zero violations is the boundary of the feasible set and must be admitted"
    );
    for violations in 1..=1_000_u32 {
        let refused = SafetyEvidence::cleared(violations);
        assert!(
            matches!(
                refused,
                Err(SafetyViolation {
                    violations: v
                }) if v == violations
            ),
            "SafetyEvidence must be unconstructible from {violations} violations — \
             that is the whole mechanism. Got {refused:?}"
        );
    }
}

/// The accept-constructor's signature, bound exactly. Widening
/// `FeasibleCase::new` to take a raw `u32` violation count — which is how the
/// non-tradeability would quietly be deleted — is a **compile error** here, not a
/// green run with a broken promise.
///
/// Note the scope, inherited from the R53-Option1 precedent: integration tests
/// are not part of the library build, so this is enforced wherever the pins are
/// compiled and run (`cargo test --no-run` / `cargo build --all-targets`).
#[test]
fn eval_joint_r53_4_the_accept_constructor_signature_is_pinned() {
    let accept: fn(f32, LocalCost, SafetyEvidence) -> FeasibleCase = FeasibleCase::new;

    let evidence = SafetyEvidence::cleared(0).expect("zero is the only admitted count");
    let case = accept(0.90, LocalCost::new(10), evidence);

    assert!(
        matches!(admit(&JointObjective::new(0.90, 0, LocalCost::new(10)), &floor()),
            JointVerdict::Accepted(accepted) if accepted == case),
        "an accepted case must be the one the accept-constructor produced, carrying \
         its own feasibility proof"
    );
    assert_eq!(case.safety().observed_violations(), 0);
}

/// The verdict enum is matched WITHOUT a wildcard arm, so a future variant is a
/// compile error here rather than a silently-dropped case. This is the Rust
/// form of the exhaustiveness requirement; `assertNever` in a language without
/// checked matches.
#[test]
fn eval_joint_r53_4_the_verdict_is_exhaustively_matchable() {
    fn render(v: &JointVerdict) -> &'static str {
        match v {
            JointVerdict::Accepted(_) => "accepted",
            JointVerdict::RejectedSafetyViolation(_) => "rejected-safety",
            JointVerdict::RejectedFloor { .. } => "rejected-floor",
        }
    }

    let floor = floor();
    assert_eq!(
        render(&admit(&baseline(), &floor)),
        "accepted",
        "a clean case on every term is accepted"
    );
    assert_eq!(
        render(&admit(
            &JointObjective::new(0.86, 1, LocalCost::new(4_000)),
            &floor
        )),
        "rejected-safety"
    );
    assert_eq!(
        render(&admit(
            &JointObjective::new(0.10, 0, LocalCost::new(4_000)),
            &floor
        )),
        "rejected-floor"
    );
}

// ---------------------------------------------------------------------------
// R53.5 — floors move upward only (I53.4 / D53.4)
// ---------------------------------------------------------------------------

/// `R53.5` — a lower floor never tightens the verdict; a higher one can only
/// reject.
///
/// `JointFloor::raised_to` can only tighten: the accuracy floor takes the
/// element-wise MAXIMUM and the cost ceiling takes the element-wise MINIMUM, so
/// the resulting feasible region is a subset of the original. The "lower floor"
/// in this test is therefore an *attempt*, and it is a no-op. The verdict
/// assertions then pin the consequence independently of that arithmetic, so a
/// future rewrite of `raised_to` cannot pass by leaving the verdict loose.
#[test]
fn eval_joint_r53_5_floors_move_upward_only() {
    let prereg = floor();
    let looser = JointFloor::new(0.10, LocalCost::new(90_000));
    let tighter = JointFloor::new(0.95, LocalCost::new(1_000));

    // Loosening is not representable: the attempt returns the floor unchanged.
    assert_eq!(
        prereg.raised_to(&looser),
        prereg,
        "raised_to must tighten only — max on the accuracy floor, min on the cost \
         ceiling — so a lower floor cannot be applied through this API"
    );
    // Raising works, on both terms.
    assert_eq!(
        prereg.raised_to(&tighter),
        tighter,
        "raised_to must tighten both terms"
    );

    // A case that breaches cost under the preregistered floor.
    let observed = JointObjective::new(0.90, 0, LocalCost::new(50_000));

    assert!(matches!(
        admit(&observed, &prereg),
        JointVerdict::RejectedFloor {
            breach: FloorBreach::CostAboveCeiling,
            ..
        }
    ));

    // The verdict under a "lowered" floor is byte-identical to the verdict under
    // the preregistered floor — not merely "still rejected", but the same
    // decision, so no information is smuggled in through the update path.
    assert_eq!(
        admit(&observed, &prereg.raised_to(&looser)),
        admit(&observed, &prereg),
        "a looser proposed floor must not change the verdict at all"
    );

    // A raised floor can only reject: it never turns a rejection into acceptance.
    for observed in [
        baseline(),
        JointObjective::new(0.90, 0, LocalCost::new(50_000)),
        JointObjective::new(0.10, 0, LocalCost::new(4_000)),
        JointObjective::new(0.99, 0, LocalCost::new(9_000)),
    ] {
        let before = admit(&observed, &prereg);
        let after = admit(&observed, &prereg.raised_to(&tighter));
        if matches!(before, JointVerdict::Accepted(_)) {
            assert!(
                !matches!(after, JointVerdict::Accepted(_)),
                "a raised floor turned an acceptance into another acceptance at a \
                 worse point; raising must be able to reject, and the verdict must \
                 not be silently re-derived (before={before:?}, after={after:?})"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The `NaN` case behind the one local clippy allow in the module
// ---------------------------------------------------------------------------

/// The `!(accuracy >= floor)` negation in `admit` is load-bearing, and this is
/// the pin that makes it so.
///
/// A `NaN` accuracy compares false against `<`, so the tidier `accuracy <
/// floor` form would let a `NaN` measurement **clear** the floor. The negation
/// rejects it. Without this pin the `neg_cmp_op_on_partial_ord` allow would be
/// a lint workaround with nothing holding it up.
///
/// The infinite case is the other half of the story and is asserted honestly:
/// the accuracy RANGE is not validated (declaring a range check would be an
/// objective term D2 does not preregister), so `+inf` clears the floor. That is
/// a declared non-defence, stated here so it is a recorded gap rather than a
/// discovered one.
#[test]
fn eval_joint_r53_a_nan_accuracy_cannot_clear_the_floor() {
    let floor = floor();

    for bad in [f32::NAN, f32::NEG_INFINITY] {
        let observed = JointObjective::new(bad, 0, LocalCost::new(0));
        let verdict = admit(&observed, &floor);
        assert!(
            matches!(
                verdict,
                JointVerdict::RejectedFloor {
                    breach: FloorBreach::AccuracyBelowFloor,
                    ..
                }
            ),
            "a non-finite-below accuracy must not clear the floor \
             (accuracy={bad}). Got {verdict:?}"
        );
    }

    // The comparison facts the allow rests on, asserted rather than assumed.
    // Both lints below fire on exactly these two lines and on nothing else: the
    // NaN comparisons are the SUBJECT of the pin, not an oversight in it.
    // The lint name is the one this toolchain (clippy 0.1.98) actually
    // registers: `invalid_nan_comparisons`, unprefixed — `clippy::cmp_nan` is
    // reported as renamed and `clippy::invalid_nan_comparisons` as unknown.
    #[allow(clippy::neg_cmp_op_on_partial_ord, invalid_nan_comparisons)]
    {
        assert!(
            !(f32::NAN < floor.accuracy),
            "`NaN < x` is false, so the readable `<` form would ACCEPT a NaN"
        );
        assert!(
            !(f32::NAN >= floor.accuracy),
            "`NaN >= x` is false, so the negated form REJECTS a NaN"
        );
    }

    // And the declared non-defence: the range is not checked, so +inf clears.
    assert!(
        f32::INFINITY >= floor.accuracy,
        "+inf is above the floor and is therefore not caught by a negated \
         comparison — this is why the range is a declared non-defence"
    );
    assert!(matches!(
        admit(
            &JointObjective::new(f32::INFINITY, 0, LocalCost::new(0)),
            &floor
        ),
        JointVerdict::Accepted(_)
    ));
}

/// The cost term is a **bounded integer**, because tokens are countable. A float
/// cost would make two equal budgets compare unequal, and a weight would make
/// safety tradeable. Both are pinned as declarations.
#[test]
fn eval_joint_r53_the_cost_term_is_a_bounded_integer_and_the_objective_has_no_weights() {
    let src = read("src/eval.rs");
    assert!(
        src.contains("pub struct LocalCost(u32);"),
        "LocalCost must wrap a u32 — tokens are countable and the count is bounded"
    );

    // The pins, taken as a type, also cannot express a weight.
    let admit_pinned: fn(&JointObjective, &JointFloor) -> JointVerdict = admit;
    assert!(matches!(
        admit_pinned(&baseline(), &floor()),
        JointVerdict::Accepted(_)
    ));

    // And no `weight` identifier appears anywhere in the joint-objective
    // section's CODE. D2's shape has no weights because a weight on safety is
    // a licence to trade it; a weighted blend is the shape the decision
    // rejects. Comments are stripped first, so the documentation that explains
    // the absence of weights can say so without tripping its own pin — the
    // earlier draft of this pin failed for exactly that reason, which is the
    // r57b lesson (a scan that can be satisfied or broken by prose).
    let start = src
        .find("feasibility constraint, not a term")
        .expect("joint-objective section marker must exist in src/eval.rs");
    let end = src
        .find("mod tests {")
        .expect("test module must follow the joint-objective section");
    let code: String = src[start..end]
        .lines()
        .map(|l| l.split("//").next().unwrap_or(l))
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.to_lowercase().contains("weight"),
        "a `weight` appeared in the joint-objective section's code. D2 forbids \
         weights: a weighted blend permits trading safety for accuracy, which is \
         the trade this objective exists to make unavailable."
    );
    // The comment stripper is not a parser; prove it actually stripped, so this
    // pin cannot pass because the stripper zeroed the whole section.
    assert!(
        code.contains("pub fn admit") && code.contains("pub struct LocalCost"),
        "the comment stripper emptied or mangled the section — the weight scan \
         would then be vacuous. Rebuilt code was:\n{code}"
    );
}
