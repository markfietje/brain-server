//! The create loop: the first phase in this system that *authors* knowledge.
//!
//! Every other loop consumes and reorganises. This one writes the system's
//! beliefs, and that single fact drives every decision below.
//!
//! ## What this loop is
//!
//! Five phases, in the only order that is safe:
//!
//! 1. **Discover** — generate candidate *gaps*: places where the corpus does
//!    not yet answer a question. A generator, not a detector: it over-generates
//!    under a hard cap and applies no precision filter, because a wrong gap
//!    costs one deterministic check and a missing gap costs the loop.
//! 2. **Hypothesize** — an agent *fills* a human-authored slot schema and never
//!    designs one. A proposal naming a schema it authored itself is refused at
//!    admission.
//! 3. **Verify** — the gate. Six deterministic checks in a fixed order, each a
//!    pure function over rows. No model, no score, no threshold, no tie-break
//!    by judgement. A check that cannot be decided without a model is NOT
//!    IMPLEMENTED, and the claim waits.
//! 4. **Promote** — a human, digest-bound, out-of-band, single-use token.
//! 5. **Disseminate** — the database fence. Recall visibility is a column the
//!    promoting path alone can move, and a trigger refuses every other move.
//!
//! ## The property the whole design rests on
//!
//! **The gate's authority is non-model, and more traffic makes it worse if that
//! ever stops being true.** A deterministic gate's honesty is a MEASURED
//! property, not an architectural one: a soft gate does not merely permit more
//! errors as volume rises, it loses the errors it was catching. So the gate is
//! six pure functions over rows, its refusals carry no diagnostic that points
//! at where a claim failed, and a check it cannot decide is absent rather than
//! approximated.
//!
//! ## Why no error-location feedback, ever
//!
//! A refusal returns a closed vocabulary and the claim's public id. It never
//! returns which byte range failed, what the corpus said at that offset, or
//! which evidence item was at fault — per-item identification is a location
//! hint wearing a different hat, so the vocabulary carries no index either.
//! The full diagnostic goes to the audit chain and to the promotion screen.
//! This is the most counter-intuitive rule in the loop and the one most likely
//! to be helpfully removed later.
//!
//! ## What this loop does NOT ship
//!
//! Any claim reaching durable state in a customer's hands. The promotion path
//! is implemented, authorized, audited, and returns a typed
//! `promotion_disabled` refusal in every configuration. It stays that way
//! until a published out-of-sample false-promotion figure exists and has a
//! named owner. The measurement harness accumulates from the moment the loop
//! is deployed: the measurement is the deliverable, the promotion is not.
//!
//! The corpus of planted bad claims is the other half of the deliverable. It
//! runs every time, tries to reach durable state, and a member that reaches
//! ratified or recall-visible state is a release-blocking failure rather than
//! a warning.
//!
//! ## Layout
//!
//! `gap` (generate) · `schema` (admit the human artifact) · `verify` (the
//! gate) · `promote` (the human act) · `disseminate` (the fence and the
//! set-level check) · `corpus` (the planted attack data).

pub(crate) mod corpus;
pub(crate) mod disproof;
pub(crate) mod disseminate;
pub(crate) mod gap;
pub(crate) mod promote;
pub(crate) mod queue;
pub(crate) mod schema;
pub(crate) mod verify;

use crate::auth::policy::PrincipalKind;

/// The closed refusal vocabulary. It is CLOSED on purpose and it is the whole
/// of what a refused proposer learns: a code and the claim's public id.
///
/// Nothing here names a byte offset, an evidence index, a predicate that
/// failed, or a value that was out of range. Every one of those is a location
/// hint, and a location hint handed back to a generator turns the gate into an
/// oracle that can be searched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Refusal {
    /// The claim's shape does not conform to the schema it named.
    SchemaMismatch,
    /// A value fell outside the declared bounds of its slot.
    OutOfBounds,
    /// A subject, predicate or schema reference does not resolve.
    Referential,
    /// A citation does not resolve over the admitted bytes.
    EvidenceUnresolvable,
    /// The claim contradicts a ratified claim on a disjoint slot.
    ContradictsRatified,
    /// The claim's premises are not independent, or it does not discriminate.
    PremiseDependent,
    /// Fewer independent supports than a repair requires.
    InsufficientSupport,
    /// No ratified schema exists for this domain.
    NoSchema,
}

impl Refusal {
    /// The wire spelling. Stable, closed, and carrying no payload.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Refusal::SchemaMismatch => "schema_mismatch",
            Refusal::OutOfBounds => "out_of_bounds",
            Refusal::Referential => "referential",
            Refusal::EvidenceUnresolvable => "evidence_unresolvable",
            Refusal::ContradictsRatified => "contradicts_ratified",
            Refusal::PremiseDependent => "premise_dependent",
            Refusal::InsufficientSupport => "insufficient_support",
            Refusal::NoSchema => "no_schema",
        }
    }

    /// Every member, for the pins that walk the set.
    pub(crate) const ALL: [Refusal; 8] = [
        Refusal::SchemaMismatch,
        Refusal::OutOfBounds,
        Refusal::Referential,
        Refusal::EvidenceUnresolvable,
        Refusal::ContradictsRatified,
        Refusal::PremiseDependent,
        Refusal::InsufficientSupport,
        Refusal::NoSchema,
    ];
}

/// The complete refusal payload. Two fields, both bounded, neither diagnostic.
///
/// The claim id is a public identifier the proposer already chose, so echoing
/// it leaks nothing. `detail` exists only to be absent: a future contributor
/// adding a human-readable explanation here has defeated the control, and the
/// pin on this struct's field list is what stops them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefusalReceipt {
    pub(crate) claim_id: String,
    pub(crate) reason: &'static str,
}

impl RefusalReceipt {
    /// Build the receipt. This is the ONLY constructor, and it accepts no
    /// detail argument — the absence is structural rather than a convention.
    pub(crate) fn new(claim_id: &str, reason: Refusal) -> Self {
        RefusalReceipt {
            claim_id: claim_id.to_string(),
            reason: reason.as_str(),
        }
    }
}

/// The principal-kind string the record layer stores.
///
/// This is the ONLY place the mapping exists, and every write path routes
/// through it. That matters more than it looks: the database fences key on
/// this string, so a `created_by` taken from a request body would be a total
/// bypass of every fence in the loop — the caller would simply assert it is a
/// human. The mapping lives here, takes a typed principal kind, and cannot be
/// reached from untrusted input.
pub(crate) const fn principal_kind_string(kind: PrincipalKind) -> &'static str {
    match kind {
        PrincipalKind::Jwt => "human",
        PrincipalKind::AgentLoopback => "agent",
    }
}

/// The inverse mapping, for reading a stored row back. Unknown strings fail
/// closed rather than defaulting to the more privileged side.
pub(crate) fn principal_kind_from_stored(raw: &str) -> Option<PrincipalKind> {
    match raw {
        "human" => Some(PrincipalKind::Jwt),
        "agent" => Some(PrincipalKind::AgentLoopback),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_vocabulary_is_closed_and_carries_no_payload() {
        let seen: Vec<&str> = Refusal::ALL.iter().map(|r| r.as_str()).collect();
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            seen.len(),
            "the refusal vocabulary must be closed: {seen:?}"
        );
        for code in &seen {
            assert!(
                !code.chars().any(|c| c.is_ascii_digit()),
                "refusal code {code} carries a number, and a number in a refusal is a \
                 location hint: the proposer reads it as an index into its own evidence"
            );
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "refusal code {code} is not a plain closed-vocabulary token"
            );
        }
    }

    #[test]
    fn a_refusal_receipt_has_exactly_two_fields_and_no_detail() {
        let receipt = RefusalReceipt::new("clm_abc", Refusal::EvidenceUnresolvable);
        assert_eq!(receipt.claim_id, "clm_abc");
        assert_eq!(receipt.reason, "evidence_unresolvable");
        // The point of the control: there is no third field to fill in.
        let fields = std::mem::size_of::<RefusalReceipt>();
        let claim_id_bytes = receipt.claim_id.capacity();
        assert!(
            fields <= std::mem::size_of::<String>() * 2,
            "the receipt grew a field: {fields} bytes for a two-field struct"
        );
        assert!(claim_id_bytes > 0, "the claim id must be echoed back");
    }

    #[test]
    fn the_principal_kind_mapping_is_total_in_both_directions() {
        for kind in [PrincipalKind::Jwt, PrincipalKind::AgentLoopback] {
            let stored = principal_kind_string(kind);
            assert_eq!(
                principal_kind_from_stored(stored),
                Some(kind),
                "{stored} must map back to the kind it came from"
            );
        }
        assert_eq!(
            principal_kind_from_stored("operator"),
            None,
            "an unrecognised principal string must fail closed, never default to the \
             privileged side"
        );
        assert_eq!(principal_kind_from_stored("Agent"), None);
        assert_eq!(principal_kind_from_stored(""), None);
    }
}
