//! Dissemination: the set-level check, and the two-column visibility lock.
//!
//! ## The problem this exists for
//!
//! Individually benign memories are jointly harmful, and they survive both
//! dilution and memory-level defenses doing nothing about them. **Per-item
//! review is structurally blind to a collusive set**: each member looks fine on
//! its own, which is the definition of the attack. So the unit of judgement
//! here is the BATCH, not the claim.
//!
//! Claims promote individually. Their recall visibility does not. A ratified
//! claim lands with a batch assigned and `recall_visible = 0`, and the whole
//! batch becomes visible in one transaction with its audit row, only after a
//! deterministic set-level check passes.
//!
//! ## The ceiling, stated rather than implied
//!
//! **No published set-level control exists.** This is hand-built, and it is
//! the weakest-evidenced thing in the round. The check below is a declared
//! approximation: it catches declared predicate interactions — two claims that
//! interact through slots the schema says interact — and not arbitrary
//! semantic composition. A batch that composes harmfully through slots nobody
//! declared related is NOT caught, and no honest reading of this module
//! suggests otherwise.
//!
//! So the module's own verdict is three-valued. `Fail` is a definite no. `Pass`
//! is a definite no-harm *within the declared interactions*, which is much
//! weaker than "no harm", and the name says so.

use crate::workflow::create::verify::SlotType;

/// The set-level verdict. Three values, because two would overstate the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetVerdict {
    /// No declared interaction among the members produces a conflict. This is
    /// a statement about DECLARED interactions and nothing wider.
    Pass,
    /// The members conflict through a declared interaction, or a member is
    /// not itself ratified.
    Fail(&'static str),
    /// The check could not run. Distinct from `Pass`: an unrunnable check is
    /// not a pass, and a batch that waits here has been admitted nothing.
    Unavailable,
}

/// A batch member reduced to what the set check needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BatchMember {
    pub(crate) claim_id: String,
    pub(crate) predicate: String,
    pub(crate) declared_type: SlotType,
    pub(crate) class: Option<&'static str>,
    pub(crate) value: crate::workflow::create::verify::SlotValue,
    pub(crate) is_ratified: bool,
    /// The pairs of predicates this batch's schema declares as interacting.
    /// Declared by the human artifact — this is the entire scope of the check.
    pub(crate) interacts_with: &'static [(&'static str, &'static str)],
    /// The target this member acts on, when it names one.
    pub(crate) scope_value: Option<&'static str>,
}

/// The bound on a batch's membership. A set-level check over an unbounded set
/// is an unbounded check.
pub(crate) const MAX_BATCH_MEMBERS: usize = 64;

/// Run the set-level check. Deterministic and replayable: the same members in
/// the same declared interactions give the same verdict, always.
pub(crate) fn set_level_check(members: &[BatchMember]) -> SetVerdict {
    if members.is_empty() {
        // An empty batch cannot harm anyone, but it is also not a passed
        // batch: it has nothing to have checked. Refusing here keeps "no
        // members" from being a way to reach visibility with no evidence.
        return SetVerdict::Fail("empty_batch");
    }
    if members.len() > MAX_BATCH_MEMBERS {
        return SetVerdict::Fail("batch_too_large");
    }
    // Every member must be individually ratified. A batch may not become
    // visible while any of its members is still pending — that would let a
    // claim enter recall by riding a batch it did not individually earn.
    if let Some(m) = members.iter().find(|m| !m.is_ratified) {
        let _ = m;
        return SetVerdict::Fail("member_not_ratified");
    }
    // The joint-entailment pass: for every DECLARED interaction between two
    // members' predicates, check that their disjointness classes do not
    // contradict. This is the whole of the collusive-set defence, and its
    // reach is exactly the declared interactions.
    for (i, left) in members.iter().enumerate() {
        for right in members.iter().skip(i + 1) {
            let interacts = left.interacts_with.iter().any(|(a, b)| {
                (*a == left.predicate && *b == right.predicate)
                    || (*b == left.predicate && *a == right.predicate)
            });
            if !interacts {
                continue;
            }
            let (Some(lc), Some(rc)) = (left.class, right.class) else {
                continue;
            };
            if lc == rc
                && !left
                    .value
                    .interval(left.declared_type)
                    .intersects(right.value.interval(right.declared_type))
            {
                return SetVerdict::Fail("declared_interaction_conflicts");
            }
        }
    }
    // The joint-effect scan: a batch whose members all carry the same scope is
    // a batch acting on one target from several directions. One claim scoped
    // narrowly is ordinary; a set of them is the shape of a single actor's
    // plan expressed as many individually-true statements.
    let scoped: Vec<&str> = members.iter().filter_map(|m| m.scope_value).collect();
    if scoped.len() > 1 && scoped.windows(2).all(|w| w[0] == w[1]) && scoped.len() >= 3 {
        return SetVerdict::Fail("convergent_scope");
    }
    SetVerdict::Pass
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::create::verify::SlotValue;

    fn member(id: &str, predicate: &str, value: SlotValue, class: &'static str) -> BatchMember {
        BatchMember {
            claim_id: id.into(),
            predicate: predicate.into(),
            declared_type: SlotType::Integer,
            class: Some(class),
            value,
            is_ratified: true,
            interacts_with: &[],
            scope_value: None,
        }
    }

    #[test]
    fn a_batch_with_one_member_passes_and_an_empty_batch_does_not() {
        assert_eq!(
            set_level_check(&[member("a", "p", SlotValue::Integer(1), "c")]),
            SetVerdict::Pass
        );
        assert_eq!(
            set_level_check(&[]),
            SetVerdict::Fail("empty_batch"),
            "an empty batch has nothing to have checked; treating it as a pass would be a \
             way to reach visibility with no evidence behind it"
        );
    }

    #[test]
    fn an_unratified_member_withholds_the_whole_batch() {
        let mut pending = member("a", "p", SlotValue::Integer(1), "c");
        pending.is_ratified = false;
        let members = vec![pending, member("b", "q", SlotValue::Integer(2), "d")];
        assert_eq!(
            set_level_check(&members),
            SetVerdict::Fail("member_not_ratified"),
            "a claim must not reach recall by riding a batch it did not individually earn"
        );
    }

    #[test]
    fn a_declared_interaction_that_conflicts_is_caught_and_one_that_does_not_is_not() {
        // Two members whose predicates the schema declares as interacting,
        // sharing a disjointness class, with non-intersecting intervals.
        let mut left = member("a", "applies_to", SlotValue::Integer(10), "eligibility");
        left.interacts_with = &[("applies_to", "excludes")];
        let mut right = member("b", "excludes", SlotValue::Integer(90), "eligibility");
        right.interacts_with = &[("applies_to", "excludes")];
        assert_eq!(
            set_level_check(&[left.clone(), right.clone()]),
            SetVerdict::Fail("declared_interaction_conflicts"),
            "individually true claims that jointly exclude the same class are exactly the \
             collusive set per-item review cannot see"
        );
        // Same declared interaction, intersecting values: no conflict.
        right.value = SlotValue::Integer(10);
        assert_eq!(set_level_check(&[left, right]), SetVerdict::Pass);
    }

    #[test]
    fn an_undeclared_interaction_is_outside_the_check_and_the_module_says_so() {
        // Deliberately NOT declared as interacting, same disjointness class,
        // non-intersecting. The check passes — and that is the declared scope,
        // recorded here so the module's ceiling is a test rather than a claim.
        let left = member("a", "applies_to", SlotValue::Integer(10), "eligibility");
        let right = member("b", "unrelated", SlotValue::Integer(90), "eligibility");
        assert_eq!(
            set_level_check(&[left, right]),
            SetVerdict::Pass,
            "an UNDECLARED interaction is outside this check by construction — the ceiling \
             the docs must state, pinned so a widening is a deliberate act"
        );
    }

    #[test]
    fn the_check_is_deterministic_and_replayable() {
        let mut left = member("a", "applies_to", SlotValue::Integer(10), "eligibility");
        left.interacts_with = &[("applies_to", "excludes")];
        let mut right = member("b", "excludes", SlotValue::Integer(90), "eligibility");
        right.interacts_with = &[("applies_to", "excludes")];
        let members = vec![left, right];
        let first = set_level_check(&members);
        let second = set_level_check(&members);
        assert_eq!(
            first, second,
            "the same batch must always give the same verdict"
        );
    }

    #[test]
    fn an_oversized_batch_is_refused_rather_than_slowed() {
        let members: Vec<BatchMember> = (0..=MAX_BATCH_MEMBERS)
            .map(|i| member(&format!("c{i}"), "p", SlotValue::Integer(i as i64), "cls"))
            .collect();
        assert_eq!(
            set_level_check(&members),
            SetVerdict::Fail("batch_too_large")
        );
    }

    #[test]
    fn a_set_of_claims_all_scoped_identically_is_refused_as_convergent() {
        let mut members: Vec<BatchMember> = (0..3)
            .map(|i| {
                let mut m = member(&format!("c{i}"), "p", SlotValue::Integer(i), "cls");
                m.scope_value = Some("customer:acme");
                m
            })
            .collect();
        members.sort_by(|a, b| a.claim_id.cmp(&b.claim_id));
        assert_eq!(
            set_level_check(&members),
            SetVerdict::Fail("convergent_scope"),
            "three claims scoped to one target is the shape of a single plan expressed as \
             many individually-true statements"
        );
    }
}
