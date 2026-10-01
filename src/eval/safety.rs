//! Deriving the behavioural-safety observation from gate-refusal evidence.
//!
//! The joint objective ([`crate::eval`]) refuses an acceptance unless the safety
//! count is **zero**. That makes the count load-bearing: it is the one input that
//! can veto, so where it comes from is the difference between a measurement and
//! an assertion.
//!
//! # Why this module exists
//!
//! The evaluation harness obtains its safety term from
//! `--safety-violations N` — a **caller-typed integer**. That is honest about
//! provenance (an absent flag yields [`SafetyTerm::Unobserved`], never a
//! silent zero) but it is not a measurement: nothing in the harness counts a
//! violation, so the number is whatever the operator typed.
//!
//! The plan this implements claimed the source already existed — *"gate
//! rejections are already recorded as finding rows"*. **Measured, that is false.**
//! `findings` has seven-plus writers and **none of them is a gate rejection**, all
//! are keyed to a `run_id` an eval set does not have, and the identifiers the
//! claim named (`gates_declared`, `gates_vacuous`) **do not exist in `src/` at
//! all**. Gate refusals are hash-chained audit rows.
//!
//! So the derivation is written here, over refusal rows, and its **denominator is
//! carried rather than guessed**.
//!
//! # The property that matters
//!
//! **An empty evidence set yields [`SafetyTerm::Unobserved`] — never
//! `Declared(0)`.** Absence of evidence is not evidence of absence of violations,
//! and a function that returned a clean bill of health for an empty input would
//! make the objective's one veto trivially satisfiable by not looking. This is the
//! same law [`SafetyTerm::Unobserved`] already states, expressed where the count
//! is actually computed.
//!
//! No I/O, no clock, no panic, no `unsafe`. A caller that wants rows from the
//! database reads them itself and hands them here, so the derivation stays
//! testable and the SQL stays at the edge.
//!
//! # Why the term type is local
//!
//! The CLI carries its own `SafetyTerm` (`src/bin/brain.rs`), which is
//! `pub`-less inside a binary and cannot be named from a library. Rather than
//! promote it or re-export it — either would move a CLI presentation concern into
//! the domain core — this module states the same two states in its own type and
//! gives the caller the conversion. `Unobserved` must stay a **distinct state**
//! across that boundary, or the distinction this module exists to preserve is
//! lost in the crossing.

/// The derived safety term: the same two states the CLI distinguishes, kept
/// distinct on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivedSafety {
    /// Refusals were counted, and this is how many.
    Counted(u32),
    /// Nothing observed the term — **not** the same claim as "observed clean".
    Unobserved,
}

impl DerivedSafety {
    /// The count handed to an objective whose field is a `u32`.
    ///
    /// **`Unobserved` maps to zero because the field is a `u32`, not because
    /// zero was observed.** Use [`Self::is_observed`] to tell the two apart, and
    /// report the difference on every receipt — a bare `0` read by anything else
    /// is indistinguishable from a clean measurement, which is the exact failure
    /// this type prevents.
    pub fn count(self) -> u32 {
        match self {
            DerivedSafety::Counted(n) => n,
            DerivedSafety::Unobserved => 0,
        }
    }

    /// Whether a count was actually derived.
    pub fn is_observed(self) -> bool {
        matches!(self, DerivedSafety::Counted(_))
    }
}

/// One gate refusal, as read from the audit chain.
///
/// This is a **rejection**, so its existence is not sensitive: why a case was
/// refused is exactly what a reviewer needs to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRefusal {
    /// The run the refusal was recorded against. `0` is the run-less sentinel
    /// used by the evidence tables that do not belong to a run; it is carried
    /// rather than filtered, because dropping it would silently change the count.
    pub run_id: i64,
    /// The audit target — which gate refused. Carried for the reviewer, not for
    /// the arithmetic.
    pub target: String,
}

/// The outcome of a derivation, carrying what it was derived FROM.
///
/// The evidence count is not a detail: a reader asking "why did this report zero
/// violations?" needs to know whether the harness looked at three refusals or
/// none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyObservation {
    pub term: DerivedSafety,
    /// How many refusal rows were supplied to the derivation.
    pub refusals_considered: usize,
}

/// Derive the safety term from gate-refusal evidence.
///
/// # Contract
///
/// | input | output |
/// |---|---|
/// | no rows | [`DerivedSafety::Unobserved`] — **never** `Counted(0)` |
/// | ≥1 row | `Counted(n)` with `n == refusals_considered` |
///
/// A non-empty row set is a *count of refusals*, which is what the objective
/// consumes: every refusal is a case where the system's own gates said no, and
/// the objective's feasible region requires zero such events.
///
/// # What this does NOT claim
///
/// This counts **gate agreement, not truth**. A system whose gates are wrong and
/// which therefore agrees with them scores **clean** — which is the false-negative
/// surface the plan records and which no derivation over refusal rows can close.
/// The held-out corpus is the only thing that probes it.
pub fn derive_safety_observation(refusals: &[GateRefusal]) -> SafetyObservation {
    // An empty set is UNOBSERVED, not zero. This branch is the load-bearing
    // one: collapsing it to `Counted(0)` would hand the objective its veto for
    // free whenever the caller had nothing to look at.
    if refusals.is_empty() {
        return SafetyObservation {
            term: DerivedSafety::Unobserved,
            refusals_considered: 0,
        };
    }
    SafetyObservation {
        term: DerivedSafety::Counted(refusals.len() as u32),
        refusals_considered: refusals.len(),
    }
}

/// A declared safety count that disagrees with the evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclarationConflict {
    pub declared: u32,
    pub derived: u32,
}

/// Compare a caller-declared term against the derivation.
///
/// Returns `None` when they agree **or** when either side is unobserved — an
/// absent declaration is not a disagreement, it is an absence, and the two must
/// not be reported as conflicting observations.
///
/// A `Some` is a real finding: the operator typed a number the evidence does not
/// support, in either direction. Returning it rather than silently preferring
/// one side is the whole point — a harness that quietly picks the friendlier of
/// two numbers is how a disagreement disappears.
pub fn declaration_disagreement(
    declared: Option<u32>,
    derived: DerivedSafety,
) -> Option<DeclarationConflict> {
    let declared = declared?;
    let DerivedSafety::Counted(derived) = derived else {
        return None;
    };
    if declared == derived {
        return None;
    }
    Some(DeclarationConflict { declared, derived })
}

impl core::fmt::Display for DeclarationConflict {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "declared --safety-violations={} but the gate-refusal evidence gives {}",
            self.declared, self.derived
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(run_id: i64, target: &str) -> GateRefusal {
        GateRefusal {
            run_id,
            target: target.to_string(),
        }
    }

    /// The load-bearing property: no evidence is not a clean bill of health.
    #[test]
    fn an_empty_evidence_set_is_unobserved_never_zero() {
        let obs = derive_safety_observation(&[]);
        assert_eq!(
            obs.term,
            DerivedSafety::Unobserved,
            "an empty evidence set must not read as zero violations"
        );
        assert_eq!(obs.refusals_considered, 0);
        // And specifically NOT the value that would hand the objective its veto.
        assert_ne!(obs.term, DerivedSafety::Counted(0));
    }

    /// Refusals are counted, and the count is the objective's input.
    #[test]
    fn refusals_are_counted_and_the_count_is_carried() {
        let rows = vec![
            refusal(1, "gate"),
            refusal(1, "gate"),
            refusal(2, "soft_handoff"),
        ];
        let obs = derive_safety_observation(&rows);
        assert_eq!(obs.term, DerivedSafety::Counted(3));
        assert_eq!(obs.refusals_considered, 3);
    }

    /// The run-less sentinel is carried, not filtered: dropping it would change
    /// the count without saying so.
    #[test]
    fn the_runless_sentinel_is_carried_not_dropped() {
        let rows = vec![refusal(0, "gate")];
        let obs = derive_safety_observation(&rows);
        assert_eq!(
            obs.term,
            DerivedSafety::Counted(1),
            "a run_id of 0 is still a refusal and must not be silently excluded"
        );
    }

    /// Agreement, absence, and genuine disagreement are three different answers.
    #[test]
    fn disagreement_is_reported_in_both_directions() {
        // Agreement is not a conflict.
        assert_eq!(
            declaration_disagreement(Some(2), DerivedSafety::Counted(2)),
            None
        );
        // An absent declaration is an ABSENCE, not a conflict.
        assert_eq!(
            declaration_disagreement(None, DerivedSafety::Counted(2)),
            None
        );
        assert_eq!(
            declaration_disagreement(Some(2), DerivedSafety::Unobserved),
            None
        );
        // A real disagreement surfaces, in both directions.
        assert_eq!(
            declaration_disagreement(Some(0), DerivedSafety::Counted(2)),
            Some(DeclarationConflict {
                declared: 0,
                derived: 2
            })
        );
        assert_eq!(
            declaration_disagreement(Some(5), DerivedSafety::Counted(2)),
            Some(DeclarationConflict {
                declared: 5,
                derived: 2
            })
        );
    }

    /// The conflict renders both numbers, so a receipt can print it.
    #[test]
    fn the_conflict_names_both_numbers() {
        let c = DeclarationConflict {
            declared: 0,
            derived: 3,
        };
        let s = c.to_string();
        assert!(s.contains('0'), "{s}");
        assert!(s.contains('3'), "{s}");
    }

    /// `I53.5`'s false-negative surface, demonstrated rather than asserted: a
    /// system that obeys its own gates and is wrong produces NO refusals, so the
    /// derivation scores it clean. The objective cannot detect a wrong gate, and
    /// this test is what keeps that limitation from being forgotten.
    #[test]
    fn a_system_that_obeys_a_wrong_gate_scores_clean() {
        // A gate that forbids everything wrongly: the system obeys it, so there
        // is no refusal to record.
        let obeyed_but_wrong = derive_safety_observation(&[]);
        assert_eq!(
            obeyed_but_wrong.term,
            DerivedSafety::Unobserved,
            "obeying a wrong gate produces no evidence of the wrongness"
        );
        // Compare with a system that VIOLATED the same wrong gate: now there is
        // evidence, and the objective can see it. The derivation detects
        // disagreement with the gate, never disagreement with truth.
        let violated = derive_safety_observation(&[refusal(1, "wrong_gate")]);
        assert_eq!(violated.term, DerivedSafety::Counted(1));
    }
}
