//! The planted-bad-claim corpus: the round's two-year failure surface, as data.
//!
//! Every member here is an ATTACK, not a test fixture. Each one tries to reach
//! durable state — `status = 'ratified'` or `recall_visible = 1` — and a member
//! that gets there is a release-blocking failure rather than a warning.
//!
//! ## Why the corpus is the deliverable and not the tests
//!
//! The gate's honesty is a measured property. A battery proves that the
//! attacks we thought of are refused, which is worth a great deal and is not
//! the same claim as "no attack reaches durable state". The corpus is what
//! keeps that second claim honest over time: it runs every time, it names the
//! failure surface it covers, and its membership floor is up-only so it cannot
//! quietly shrink to zero and make every corpus pin vacuously green.
//!
//! ## Coverage is not the same as safety
//!
//! The classes below are drawn from what has been measured against memory
//! systems rather than from what we imagine. Two of them are not about getting
//! a bad claim IN at all — one is about what happens to the bad claims that do
//! land and are never cleaned up, and one is about suppressing the audit trail
//! so the refusals look like a quiet week. A corpus that only tested admission
//! would have called itself complete.
//!
//! ## Why the fixtures live here as a const
//!
//! They are DATA, compiled into the binary and asserted by the suite. A
//! corpus assembled from whatever the tests happened to construct is a corpus
//! that silently tracks the tests, and one that reads its members from disk is
//! a corpus an operator can edit without a review.

/// One planted attack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlantedClaim {
    /// The attack class, named. An unnamed attack is an untested one.
    pub(crate) attack: &'static str,
    /// What the attack tries to achieve, in one clause.
    pub(crate) intent: &'static str,
    /// Which control is supposed to stop it, by module name.
    pub(crate) stopped_by: &'static str,
    /// The claim body the attacker would write. Held as data so the corpus
    /// member is inspectable rather than a number in a table.
    pub(crate) body: &'static str,
    /// Whether this attack targets admission, or cleanup of what already
    /// landed. Cleanup is a separate surface and a separate failure mode.
    pub(crate) targets_cleanup: bool,
}

/// The corpus. Membership is a floor, not a snapshot: the pin asserts a
/// minimum so this list cannot quietly empty itself.
pub(crate) const PLANTED_CORPUS: &[PlantedClaim] = &[
    // ── admission ──────────────────────────────────────────────────────────
    PlantedClaim {
        attack: "self_authorship",
        intent: "an agent authors the schema that licenses its own claim",
        stopped_by: "schema::admit",
        body: r#"{"authored_by":"agent","slots":[{"predicate":"always_true"}]}"#,
        targets_cleanup: false,
    },
    PlantedClaim {
        attack: "self_ratification",
        intent: "the creating writer stamps its own ratification columns",
        stopped_by: "claims_fence_self_ratification",
        body: r#"{"status":"ratified","promoted_by":null,"promoted_at":null}"#,
        targets_cleanup: false,
    },
    PlantedClaim {
        attack: "forged_promotion_witness",
        intent: "a promote audit row is written directly, then visibility is flipped",
        stopped_by: "claims_fence_recall_visibility",
        body: r#"{"audit_events":[{"kind":"workflow","status":"ok","target_hash":"<claim's own digest>"}]}"#,
        targets_cleanup: false,
    },
    PlantedClaim {
        attack: "denied_row_as_witness",
        intent: "a FAILED promote row is reused as though it had succeeded",
        stopped_by: "claims_fence_recall_visibility",
        body: r#"{"audit_events":[{"kind":"workflow","status":"denied","target_hash":"<claim's own digest>"}]}"#,
        targets_cleanup: false,
    },
    PlantedClaim {
        attack: "laundered_citation",
        intent: "a consolidation rewrites the cited source and re-points the pointer",
        stopped_by: "claims_fence_cid_rewrite",
        body: r#"{"UPDATE claim_evidence SET source_cid = <rewritten text's id> WHERE ratified"}"#,
        targets_cleanup: false,
    },
    PlantedClaim {
        attack: "unresolvable_citation",
        intent: "a byte range is one byte off, so the quote is almost right",
        stopped_by: "verify::check_citation_resolvability",
        body: r#"{"byte_start":0,"byte_end":21,"quote":"the warranty runs fo"}"#,
        targets_cleanup: false,
    },
    PlantedClaim {
        attack: "located_hint_probe",
        intent: "the refusal body is read for an offset to search against",
        stopped_by: "create::RefusalReceipt",
        body: r#"{"attempt":"read the refusal body for a byte offset"}"#,
        targets_cleanup: false,
    },
    PlantedClaim {
        attack: "per_item_blindness",
        intent: "a collusive set whose members are each individually plausible",
        stopped_by: "disseminate::set_level_check",
        body: r#"{"members":[{"predicate":"applies_to"},{"predicate":"excludes"}]}"#,
        targets_cleanup: false,
    },
    PlantedClaim {
        attack: "premise_stuffing",
        intent: "support is manufactured by citing one source many times",
        stopped_by: "verify::check_premise_discipline",
        body: r#"{"citations":[<same source>,<same source>,<same source>]}"#,
        targets_cleanup: false,
    },
    PlantedClaim {
        attack: "confident_wrong_repair",
        intent: "a repair that contradicts a ratified claim carries a single support",
        stopped_by: "verify::check_premise_discipline",
        body: r#"{"contradicts":["clm_ratified"],"citations":[<one source>]}"#,
        targets_cleanup: false,
    },
    // ── cleanup: what happens to the bad claims that already landed ────────
    PlantedClaim {
        attack: "selective_cleanup",
        intent: "most of what landed is never cleaned up, and the residue is not counted",
        stopped_by: "corpus::cleanup_completeness",
        body: r#"{"attempt":"leave half the landed set in place and report none of it"}"#,
        targets_cleanup: true,
    },
    PlantedClaim {
        attack: "audit_suppression",
        intent: "refusals happen but the refusal metrics do not move",
        stopped_by: "corpus::refusal_visibility",
        body: r#"{"attempt":"refuse every claim while the refusal counters stay flat"}"#,
        targets_cleanup: true,
    },
];

/// The corpus membership floor. Up-only, like every other floor here: a list
/// that can shrink is a list that will.
pub(crate) const CORPUS_FLOOR: usize = 12;

/// The attack classes the corpus must cover. Each is a named, measured
/// failure surface; the pin walks this list against the members above.
pub(crate) const REQUIRED_CLASSES: &[&str] = &[
    "self_authorship",
    "self_ratification",
    "forged_promotion_witness",
    "denied_row_as_witness",
    "laundered_citation",
    "unresolvable_citation",
    "located_hint_probe",
    "per_item_blindness",
    "premise_stuffing",
    "confident_wrong_repair",
    "selective_cleanup",
    "audit_suppression",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corpus_is_non_empty_and_never_below_its_floor() {
        assert!(
            PLANTED_CORPUS.len() >= CORPUS_FLOOR,
            "the planted corpus holds {} members, below its floor of {CORPUS_FLOOR}. A \
             corpus that shrinks is a corpus that has stopped trying.",
            PLANTED_CORPUS.len()
        );
    }

    #[test]
    fn every_member_names_its_attack_and_the_control_that_stops_it() {
        for member in PLANTED_CORPUS {
            assert!(
                !member.attack.is_empty() && !member.intent.is_empty(),
                "a member with no attack class is an untested one: {member:?}"
            );
            assert!(
                !member.stopped_by.is_empty(),
                "{attack} names no control; a corpus member with no control is a corpus \
                 member nobody is responsible for",
                attack = member.attack
            );
            assert!(
                !member.body.is_empty(),
                "{attack} carries no fixture body",
                attack = member.attack
            );
        }
    }

    #[test]
    fn every_required_class_is_actually_present() {
        for required in REQUIRED_CLASSES {
            assert!(
                PLANTED_CORPUS.iter().any(|m| m.attack == *required),
                "the corpus must cover `{required}` and does not"
            );
        }
        assert_eq!(
            REQUIRED_CLASSES.len(),
            PLANTED_CORPUS.len(),
            "the required-class list and the corpus are the same set: a member the list does \
             not name is a member nothing holds to account"
        );
    }

    #[test]
    fn the_corpus_covers_cleanup_not_only_admission() {
        let cleanup = PLANTED_CORPUS.iter().filter(|m| m.targets_cleanup).count();
        assert!(
            cleanup >= 2,
            "only {cleanup} corpus members target CLEANUP of what already landed. A corpus \
             that only tested admission would have called itself complete while measuring \
             nothing about the residue operators actually leave behind."
        );
    }
}
