//! D-B's separation criterion: the smallest `N` at which the two arms can be told
//! apart, computed rather than asserted.
//!
//! ## What this is for
//!
//! The routing question is whether routing moves the operator's verdict. The
//! **primitive path** (D-A) is the same machine with the routing mechanism off;
//! the **skill path** has it on. The comparison that answers the question is not
//! "which arm scored higher" — it is *how many cases are needed before the two
//! arms' success rates are distinguishable at all*.
//!
//! ## Why the operator's judgment, and nothing else
//!
//! The success column is the operator's own recorded verdict, the one figure in
//! the corpus that is not the system's self-report. It is therefore the only one a
//! routing claim can be measured against: a comparison against a self-reported
//! column measures agreement with itself.
//! ## The criterion admits no threshold
//!
//! [`separation_n`] takes the two arms' paired outcomes and returns a verdict. It
//! takes **no tolerance, no confidence level, and no band** — the confidence is
//! [`SEPARATION_CONFIDENCE_UNITS`], a module constant.
//!
//! That is the same structural control [`crate::workflow::drift_census::Cell`]
//! uses for its global tolerance: a per-cell band is *unrepresentable*, not merely
//! discouraged. A threshold the caller supplies is a threshold the caller chose,
//! and a threshold chosen by the thing being tested is not a threshold.
//!
//! ## Why the refusal is the likely answer
//!
//! On a corpus of this size the honest result is very likely `NotSeparating`, and
//! that is a **published negative result**, not a reason to move the constant. D-B
//! non-claim (i) is explicit: `N` is *computed*, and a corpus that cannot reach
//! separation says so. This module has no path that returns a number the input did
//! not imply.

//! ## Why the success column is not named in this module
//!
//! The frozen verdict is read at exactly one seam in this tree, and a whole-tree
//! pin holds that. This module does not read it and does not name it: a second
//! reader name would be a second reader, and the guard
//! `no_production_site_outside_the_census_reads_the_verdict` is right to fail on
//! it. The criterion takes each arm's already-recorded outcome as an
//! [`ArmOutcome`], so **who decided the outcome stays at the one seam that owns
//! it** — this module compares, it does not re-read.
//!
//! The column's name is a fact about the corpus, asserted by the pin in
//! `tests/separation_criterion_pins.rs` rather than duplicated here. A constant
//! that had to agree with a reader elsewhere is a second place to be wrong.
//!
//! ## The confidence D-B preregistered
//!
//! [`SEPARATION_CONFIDENCE_UNITS`] is **95% as `9500`**, and it is a module
//! constant rather than a parameter of [`separation_n`] because D-B non-claim
//! (ii) forbids a threshold that can be chosen after labels exist — and a
//! parameter is exactly that. The whole vocabulary of thresholds in this module
//! is that one constant; see [`required_n_for_separation`] for why nothing else
//! stands beside it.

use crate::workflow::routing::RoutingClass;

/// The confidence D-B preregistered, in ten-thousandths. **95% is `9500`.**
///
/// A module constant, and the only one. It is not a parameter of
/// [`separation_n`] because D-B non-claim (ii) forbids a threshold that can be
/// chosen after labels exist — and a parameter is exactly that.
pub const SEPARATION_CONFIDENCE_UNITS: i64 = 9500;

/// Identifies the criterion that was preregistered, for receipts and pins.
///
/// A sibling criterion's refusal stands (the census and the model bindings are
/// disjoint populations), so this names **one** criterion rather than
/// a shared family.
pub const SEPARATION_CRITERION_ID: &str = "D-B/preregistered-2026-10-03";

/// One case's outcome in one arm: did the operator record a pass?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmOutcome {
    /// The case's identity. The two arms are compared **over the same cases**, so
    /// the pairing is by id, never by position — two orderings of the same cases
    /// are the same corpus.
    pub case_id: &'static str,
    /// The arm's recorded operator verdict for this case.
    pub success: bool,
}

/// The two arms over the same cases, ready to compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparationInput {
    /// The skill path — routing on.
    pub skill: Vec<ArmOutcome>,
    /// The primitive path — routing off (D-A).
    pub primitive: Vec<ArmOutcome>,
}

/// What the criterion decided, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Separation {
    /// The arms separate at this `N`. The skill path's rate is distinguishable
    /// from the primitive path's at the preregistered confidence.
    Separating { n: usize },
    /// They do not, and this is the reason. Every variant is a refusal with a
    /// cause; there is no variant meaning "try again with a looser band".
    NotSeparating { reason: NotSeparatingReason },
}

/// Why the criterion refused. Each variant names a cause the caller can act on;
/// none of them is a suggestion to relax the criterion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotSeparatingReason {
    /// Fewer than two paired cases: there is nothing to compare.
    TooFewCases { paired: usize },
    /// The arms are over different cases, so no comparison is defined. D-B says
    /// *the same cases*; this is that refusal.
    UnpairedCases { unpaired: usize },
    /// The two arms recorded identical outcomes on every case. Identical arms
    /// cannot separate at any `N` — this is the vacuity guard, and it is why a
    /// criterion that could separate identical arms would be measuring noise.
    Indistinguishable { paired: usize },
    /// Separation is possible in principle but the corpus does not reach the `N`
    /// that would show it. **A published negative result**, not a defect.
    CorpusTooSmall { paired: usize, required: usize },
}

/// Compute D-B's `N`: the smallest case count at which the arms' success rates are
/// distinguishable at [`SEPARATION_CONFIDENCE_UNITS`].
///
/// `n` is the **largest paired case count in the input**, because a criterion that
/// reported a smaller `n` would be reporting a count the corpus does not contain.
/// The arms must be over the same cases; a case present in one arm and not the
/// other refuses rather than being dropped, because silently discarding the
/// unpaired case is how an ablation ends up comparing different corpora.
///
/// **There is no threshold parameter, and that is the control.** See the module
/// doc.
pub fn separation_n(input: &SeparationInput) -> Separation {
    let primitive_by_id: std::collections::BTreeMap<&str, bool> = input
        .primitive
        .iter()
        .map(|o| (o.case_id, o.success))
        .collect();

    // Pair by id. An id in one arm and not the other is a refusal, never a drop.
    let mut unpaired = 0usize;
    let mut pairs: Vec<(&'static str, bool, bool)> = Vec::new();
    for outcome in &input.skill {
        // `if let` rather than a two-arm match: the miss arm is a counter bump,
        // not a value, so there is nothing for a match to return.
        if let Some(&primitive_success) = primitive_by_id.get(outcome.case_id) {
            pairs.push((outcome.case_id, outcome.success, primitive_success));
        } else {
            unpaired += 1;
        }
    }
    // An id only in the primitive arm is equally unpaired.
    let skill_ids: std::collections::BTreeSet<&str> =
        input.skill.iter().map(|o| o.case_id).collect();
    unpaired += primitive_by_id
        .keys()
        .filter(|id| !skill_ids.contains(*id))
        .count();

    if unpaired > 0 {
        return Separation::NotSeparating {
            reason: NotSeparatingReason::UnpairedCases { unpaired },
        };
    }

    let n = pairs.len();
    if n < 2 {
        return Separation::NotSeparating {
            reason: NotSeparatingReason::TooFewCases { paired: n },
        };
    }

    let skill_rate = success_rate(pairs.iter().map(|(_, s, _)| *s));
    let primitive_rate = success_rate(pairs.iter().map(|(_, _, p)| *p));

    // The vacuity guard: identical arms have no difference to find. A criterion
    // that separated them would be measuring its own arithmetic, not the corpus.
    if skill_rate == primitive_rate {
        return Separation::NotSeparating {
            reason: NotSeparatingReason::Indistinguishable { paired: n },
        };
    }

    // The separation test, at the preregistered confidence.
    let required = required_n_for_separation(skill_rate, primitive_rate);
    if n >= required {
        Separation::Separating { n }
    } else {
        Separation::NotSeparating {
            reason: NotSeparatingReason::CorpusTooSmall {
                paired: n,
                required,
            },
        }
    }
}

/// The arms' success rate, as ten-thousandths. Integer arithmetic: a rate the
/// corpus cannot express exactly is not rounded into a value it can.
fn success_rate(outcomes: impl Iterator<Item = bool>) -> i64 {
    let (passes, total) = outcomes.fold((0i64, 0i64), |(p, t), s| (p + i64::from(s), t + 1));
    if total == 0 {
        return 0;
    }
    (passes * 10_000) / total
}

/// The smallest `n` that could separate these two rates at the preregistered
/// confidence.
///
/// A **normal-approximation** requirement, stated rather than tuned: the gap
/// between the arms must be at least `z · sqrt(p(1-p)/n + q(1-q)/n)` where `z` is
/// the two-sided normal quantile for [`SEPARATION_CONFIDENCE_UNITS`].
///
/// `z` is **derived from the constant**, not carried beside it. A second constant
/// that has to agree with the first is a threshold a later round can move
/// independently, which is the failure D-B's preregistration exists to prevent.
///
/// No clamp to the corpus size. An earlier draft capped the requirement at the
/// `at_n` it was given, which made [`NotSeparatingReason::CorpusTooSmall`]
/// unreachable: the required `n` could never exceed what the corpus already had,
/// so every differing pair of arms separated. The in-module pin
/// `a_narrow_gap_reports_the_n_it_would_need` caught it by failing on a corpus
/// that demonstrably cannot separate (1.00 vs 0.75 needs 12 cases, and the
/// corpus had 4). Clamping the requirement to the evidence is the same move as
/// lowering a threshold to fit a measurement, in the direction that makes the
/// criterion agree with whatever it is handed.
///
/// **The boundary case is stated, not extrapolated.** At `p = 1.0, q = 0.0` both
/// variances are zero, so the normal approximation is `0/0` and undefined. That
/// is not a small `n` and it is not an infinite one: the arms do not overlap at
/// all, so the minimum pairable corpus separates them. Returning the floor of two
/// is the statement the data supports; returning `usize::MAX` would report a
/// corpus requirement no corpus can ever meet, and returning zero would let a
/// single case through.
fn required_n_for_separation(skill_rate: i64, primitive_rate: i64) -> usize {
    let z_units = z_units_for(SEPARATION_CONFIDENCE_UNITS);
    let p = skill_rate as f64 / 10_000.0;
    let q = primitive_rate as f64 / 10_000.0;
    let gap = (p - q).abs();
    if gap <= 0.0 {
        return usize::MAX;
    }
    let variance = p * (1.0 - p) + q * (1.0 - q);
    if variance <= 0.0 {
        // Zero variance on both arms with a non-zero gap: the arms cannot
        // overlap, which is the strongest separation the type admits.
        return 2;
    }
    let needed = (z_units * z_units * variance / (gap * gap)).ceil();
    if needed.is_finite() && needed > 0.0 {
        needed as usize
    } else {
        usize::MAX
    }
}

/// The two-sided normal quantile for a confidence level in ten-thousandths.
///
/// **Bounded by construction to the interval this programme can defend.** Outside
/// `[8000, 9999]` there is no published-table reasoning worth reproducing, so the
/// parse refuses rather than extrapolating a quantile it cannot justify. A
/// confidence outside the defensible range is a refusal, never a quiet default.
fn z_units_for(confidence_units: i64) -> f64 {
    assert!(
        (8000..=9999).contains(&confidence_units),
        "confidence {confidence_units} is outside the defensible range"
    );
    // The inverse normal CDF, by the Acklam rational approximation. Deterministic,
    // dependency-free, and exact enough for a preregistered threshold.
    let p = 1.0 - (1.0 - confidence_units as f64 / 10_000.0) / 2.0;
    inverse_normal_cdf(p)
}

/// The inverse standard-normal CDF (Acklam's rational approximation, |ε| < 1.15e-9).
fn inverse_normal_cdf(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    const P_LOW: f64 = 0.02425;

    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    if p < P_LOW {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - P_LOW {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

/// The classifier class a case was routed under, if the criterion is asked to
/// report one.
///
/// Present so a receipt can say which axis a case moved on. **It reads no number
/// and decides nothing** — D-A's arm turns the mechanism off, and a class on the
/// receipt is the only trace of the arm that a reader may compare.
pub fn axis_of(class: RoutingClass) -> &'static str {
    class.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::routing::RoutingClass;

    fn arms(skill: &[(&'static str, bool)], primitive: &[(&'static str, bool)]) -> SeparationInput {
        SeparationInput {
            skill: skill
                .iter()
                .map(|(case_id, success)| ArmOutcome {
                    case_id,
                    success: *success,
                })
                .collect(),
            primitive: primitive
                .iter()
                .map(|(case_id, success)| ArmOutcome {
                    case_id,
                    success: *success,
                })
                .collect(),
        }
    }

    #[test]
    fn the_criterion_declares_its_confidence_and_its_identity() {
        assert_eq!(SEPARATION_CONFIDENCE_UNITS, 9500, "D-B preregistered 95%");
        assert!(SEPARATION_CRITERION_ID.starts_with("D-B/"));
    }

    #[test]
    fn identical_arms_never_separate() {
        // The vacuity guard: two identical arms have no difference to find.
        let input = arms(
            &[("a", true), ("b", false), ("c", true)],
            &[("a", true), ("b", false), ("c", true)],
        );
        assert_eq!(
            separation_n(&input),
            Separation::NotSeparating {
                reason: NotSeparatingReason::Indistinguishable { paired: 3 }
            }
        );
    }

    #[test]
    fn a_single_case_cannot_separate() {
        let input = arms(&[("a", true)], &[("a", false)]);
        assert_eq!(
            separation_n(&input),
            Separation::NotSeparating {
                reason: NotSeparatingReason::TooFewCases { paired: 1 }
            }
        );
    }

    #[test]
    fn unpaired_cases_refuse_rather_than_being_dropped() {
        // The skill arm has a case the primitive arm does not. Dropping it would
        // compare two different corpora, which is the ablation's whole failure mode.
        let input = arms(&[("a", true), ("b", false)], &[("a", false)]);
        assert_eq!(
            separation_n(&input),
            Separation::NotSeparating {
                reason: NotSeparatingReason::UnpairedCases { unpaired: 1 }
            }
        );
    }

    #[test]
    fn a_wide_gap_separates_at_the_whole_corpus() {
        // Skill passes everything, primitive passes nothing: the widest gap the
        // type admits, so separation must be reachable within the corpus.
        let skill = [("a", true), ("b", true), ("c", true), ("d", true)];
        let primitive = [("a", false), ("b", false), ("c", false), ("d", false)];
        assert_eq!(
            separation_n(&arms(&skill, &primitive)),
            Separation::Separating { n: 4 }
        );
    }

    #[test]
    fn a_narrow_gap_reports_the_n_it_would_need() {
        // One case differs out of four: the criterion must report the count it
        // would need rather than separating on what it has.
        let skill = [("a", true), ("b", true), ("c", true), ("d", true)];
        let primitive = [("a", false), ("b", true), ("c", true), ("d", true)];
        match separation_n(&arms(&skill, &primitive)) {
            Separation::NotSeparating {
                reason: NotSeparatingReason::CorpusTooSmall { paired, required },
            } => {
                assert_eq!(paired, 4);
                assert!(
                    required > paired,
                    "required {required} must exceed paired {paired}"
                );
            }
            other => panic!("expected a corpus-too-small refusal, got {other:?}"),
        }
    }

    #[test]
    fn separation_is_computed_from_the_input_not_a_constant() {
        // Two different corpora must not yield the same verdict-and-N: a
        // hardcoded N would pass the tests above and answer nothing.
        let wide = arms(
            &[("a", true), ("b", true), ("c", true), ("d", true)],
            &[("a", false), ("b", false), ("c", false), ("d", false)],
        );
        let wider = arms(
            &[
                ("a", true),
                ("b", true),
                ("c", true),
                ("d", true),
                ("e", true),
                ("f", true),
            ],
            &[
                ("a", false),
                ("b", false),
                ("c", false),
                ("d", false),
                ("e", false),
                ("f", false),
            ],
        );
        assert_eq!(separation_n(&wide), Separation::Separating { n: 4 });
        assert_eq!(separation_n(&wider), Separation::Separating { n: 6 });
    }

    #[test]
    fn pairing_is_by_case_id_not_by_position() {
        // The same corpus in two orderings is the same corpus.
        let skill_a = [("a", true), ("b", false)];
        let primitive_a = [("a", false), ("b", true)];
        let skill_b = [("b", false), ("a", true)];
        let primitive_b = [("b", true), ("a", false)];
        assert_eq!(
            separation_n(&arms(&skill_a, &primitive_a)),
            separation_n(&arms(&skill_b, &primitive_b))
        );
    }

    #[test]
    fn the_axis_receipt_carries_the_classifiers_own_name() {
        assert_eq!(axis_of(RoutingClass::BusinessProcess), "business_process");
        assert_eq!(axis_of(RoutingClass::Compliance), "compliance");
    }

    #[test]
    fn the_inverse_normal_is_accurate_enough_for_a_preregistered_threshold() {
        // Acklam's approximation claims |error| < 1.15e-9. These four are the
        // standard points a threshold would be computed at.
        for (p, expected) in [
            (0.975, 1.959_964),
            (0.95, 1.644_854),
            (0.99, 2.326_348),
            (0.995, 2.575_829),
        ] {
            let got = inverse_normal_cdf(p);
            assert!(
                (got - expected).abs() < 1e-4,
                "inverse_normal_cdf({p}) = {got}, want ~{expected}"
            );
        }
        // And the derived z for the preregistered 95% is the textbook value.
        let z = z_units_for(SEPARATION_CONFIDENCE_UNITS);
        assert!((z - 1.959_964).abs() < 1e-4, "z = {z}, want ~1.959964");
    }
}
