//! The slot schema: a HUMAN artifact, forever.
//!
//! ## The rule
//!
//! Only a human principal writes a schema. No agent path writes one, and a
//! proposal that names a schema it authored itself is refused at ADMISSION —
//! not warned about, not scored, refused, because a warning is a default that
//! has not happened yet.
//!
//! The reason is not caution, it is measurement: an agent writing its own
//! specification gains nothing over an unaided baseline, and the failure mode
//! reported for it is FAITHFULNESS — it constrains part of the problem and
//! frees the rest. An author who does not know which part is which cannot
//! review that. The cost is real and stated up front: one human artifact per
//! domain, recurring forever. Price it; do not discover it.
//!
//! ## The honest limit of the table's CHECK
//!
//! A SQL CHECK reads a string in a column. It cannot interrogate the live
//! session principal. So `CHECK (authored_by = 'human')` is a TRIPWIRE on the
//! write path — any writer that names a non-human author is refused by the
//! database, and a planted violation must abort — while the BINDING check,
//! that the acting principal is a human, lives in the authorization layer and
//! in the promotion transaction.
//!
//! Calling the CHECK an identity proof would be exactly the kind of half-true
//! security law this repository exists to not write. The genuinely unforgeable
//! part is recall visibility: it moves only in the set-check transaction, the
//! trigger refuses every other move, and reaching recall requires three
//! deterministic gates regardless of who wrote the row.

use crate::workflow::create::verify::{SlotDecl, SlotType};

/// The bound on a schema body. Bounded because an unbounded document in a
/// text column is an unbounded list, and this body is parsed into slots.
pub(crate) const MAX_SCHEMA_BODY_BYTES: usize = 8 * 1024;
/// The bound on declared slots per schema.
pub(crate) const MAX_SCHEMA_SLOTS: usize = 64;
/// The bound on a predicate identifier.
pub(crate) const MAX_PREDICATE_BYTES: usize = 128;
/// The bound on a label inside a closed label set.
pub(crate) const MAX_LABELS: usize = 32;
/// The bound on a domain name.
pub(crate) const MAX_DOMAIN_BYTES: usize = 128;

/// A human-authored schema as the admission layer holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaDecl {
    pub(crate) domain: String,
    pub(crate) version: i64,
    pub(crate) slots: Vec<SlotDecl>,
}

/// Why a schema was refused. Distinct from the gate's refusal vocabulary: this
/// is admission, and it names the defect in the ARTIFACT rather than in a
/// claim about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaFault {
    /// The author is not a human principal. This is the rule, not a detail.
    NotHumanAuthored,
    BodyTooLarge,
    TooManySlots,
    EmptySchema,
    MalformedPredicate,
    MalformedBounds,
    MalformedLabels,
    MalformedVersion,
}

impl SchemaFault {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            SchemaFault::NotHumanAuthored => "not_human_authored",
            SchemaFault::BodyTooLarge => "body_too_large",
            SchemaFault::TooManySlots => "too_many_slots",
            SchemaFault::EmptySchema => "empty_schema",
            SchemaFault::MalformedPredicate => "malformed_predicate",
            SchemaFault::MalformedBounds => "malformed_bounds",
            SchemaFault::MalformedLabels => "malformed_labels",
            SchemaFault::MalformedVersion => "malformed_version",
        }
    }
}

/// Validate a schema at admission. Total, bounded, and it fails closed.
///
/// `authored_by` is the STORED string, which the caller must have produced
/// through the single mapping function in this module's parent — a string that
/// arrived from a request body would make this whole check theatre.
pub(crate) fn admit(
    domain: &str,
    version: i64,
    authored_by: &str,
    body: &str,
    slots: Vec<SlotDecl>,
) -> Result<SchemaDecl, SchemaFault> {
    if authored_by != "human" {
        return Err(SchemaFault::NotHumanAuthored);
    }
    if body.len() > MAX_SCHEMA_BODY_BYTES {
        return Err(SchemaFault::BodyTooLarge);
    }
    if version < 1 {
        return Err(SchemaFault::MalformedVersion);
    }
    if domain.is_empty() || domain.len() > MAX_DOMAIN_BYTES {
        return Err(SchemaFault::MalformedPredicate);
    }
    if slots.is_empty() {
        return Err(SchemaFault::EmptySchema);
    }
    if slots.len() > MAX_SCHEMA_SLOTS {
        return Err(SchemaFault::TooManySlots);
    }
    for slot in &slots {
        if slot.predicate.is_empty()
            || slot.predicate.len() > MAX_PREDICATE_BYTES
            || !slot
                .predicate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == ':')
        {
            return Err(SchemaFault::MalformedPredicate);
        }
        // Bounds must be ordered. An inverted interval is a schema defect and
        // admitting it would make the contradiction arithmetic vacuous.
        if let (Some(lo), Some(hi)) = (slot.lo, slot.hi)
            && lo > hi
        {
            return Err(SchemaFault::MalformedBounds);
        }
        if slot.ty == SlotType::Label {
            if slot.labels.is_empty() || slot.labels.len() > MAX_LABELS {
                return Err(SchemaFault::MalformedLabels);
            }
            for label in slot.labels {
                if label.is_empty() || label.len() > MAX_PREDICATE_BYTES {
                    return Err(SchemaFault::MalformedLabels);
                }
            }
        }
        // A disjointness class is what turns contradiction into arithmetic, so a
        // slot that claims to be part of a class without declaring one is a
        // defect rather than a default.
        if slot.ty == SlotType::Integer && slot.class.is_none() {
            return Err(SchemaFault::MalformedBounds);
        }
    }
    // Duplicate predicates would make the schema ambiguous: two slots with the
    // same name and different bounds is a schema that means two things.
    let mut seen: Vec<&str> = Vec::with_capacity(slots.len());
    for slot in &slots {
        if seen.contains(&slot.predicate.as_str()) {
            return Err(SchemaFault::MalformedPredicate);
        }
        seen.push(&slot.predicate);
    }
    Ok(SchemaDecl {
        domain: domain.to_string(),
        version,
        slots,
    })
}

/// The canonical digest of a schema body: the bytes a human signed.
///
/// This is the same primitive the claim layer uses for every digest in the
/// loop, and deliberately so: one hash function for one purpose. A body
/// digested with one algorithm and checked with another would verify nothing.
pub(crate) fn body_digest(body: &str) -> String {
    crate::audit::hash(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(predicate: &str) -> SlotDecl {
        SlotDecl {
            predicate: predicate.to_string(),
            ty: SlotType::Free,
            class: None,
            lo: None,
            hi: None,
            labels: &[],
        }
    }

    fn typed_slot() -> SlotDecl {
        SlotDecl {
            predicate: "warranty_months".into(),
            ty: SlotType::Integer,
            class: Some("warranty"),
            lo: Some(0),
            hi: Some(120),
            labels: &[],
        }
    }

    #[test]
    fn a_human_authored_schema_is_admitted() {
        let admitted = admit(
            "global",
            1,
            "human",
            "{}",
            vec![typed_slot(), slot("applies_to")],
        )
        .expect("a well-formed human schema must be admitted");
        assert_eq!(admitted.version, 1);
        assert_eq!(admitted.slots.len(), 2);
    }

    #[test]
    fn an_agent_authored_schema_is_refused_at_admission() {
        for author in ["agent", "", "HUMAN", "human ", "operator"] {
            assert_eq!(
                admit("global", 1, author, "{}", vec![typed_slot()]),
                Err(SchemaFault::NotHumanAuthored),
                "{author:?} must be refused at admission, not warned about"
            );
        }
    }

    #[test]
    fn the_schema_is_bounded_on_every_axis() {
        let big = "x".repeat(MAX_SCHEMA_BODY_BYTES + 1);
        assert_eq!(
            admit("global", 1, "human", &big, vec![typed_slot()]),
            Err(SchemaFault::BodyTooLarge)
        );
        assert_eq!(
            admit("global", 1, "human", "{}", vec![]),
            Err(SchemaFault::EmptySchema)
        );
        let many: Vec<SlotDecl> = (0..=MAX_SCHEMA_SLOTS)
            .map(|i| slot(&format!("p{i}")))
            .collect();
        assert_eq!(
            admit("global", 1, "human", "{}", many),
            Err(SchemaFault::TooManySlots)
        );
    }

    #[test]
    fn an_inverted_interval_is_refused_so_the_arithmetic_cannot_be_vacuous() {
        let mut bad = typed_slot();
        bad.lo = Some(100);
        bad.hi = Some(10);
        assert_eq!(
            admit("global", 1, "human", "{}", vec![bad]),
            Err(SchemaFault::MalformedBounds)
        );
    }

    #[test]
    fn a_duplicate_predicate_is_refused_because_the_schema_would_be_ambiguous() {
        assert_eq!(
            admit("global", 1, "human", "{}", vec![typed_slot(), typed_slot()]),
            Err(SchemaFault::MalformedPredicate)
        );
    }

    #[test]
    fn a_typed_slot_must_declare_the_class_that_makes_contradiction_arithmetic() {
        let mut orphan = typed_slot();
        orphan.class = None;
        assert_eq!(
            admit("global", 1, "human", "{}", vec![orphan]),
            Err(SchemaFault::MalformedBounds),
            "an integer slot with no disjointness class cannot participate in the \
             contradiction test, and admitting it would look like coverage"
        );
    }

    #[test]
    fn a_closed_label_set_must_actually_be_closed() {
        let mut empty_labels = typed_slot();
        empty_labels.ty = SlotType::Label;
        empty_labels.labels = &[];
        assert_eq!(
            admit("global", 1, "human", "{}", vec![empty_labels]),
            Err(SchemaFault::MalformedLabels)
        );
    }

    #[test]
    fn the_body_digest_is_the_shipped_hash_primitive() {
        // One hash function for one purpose: if the schema body were digested
        // with a different primitive than the claim layer verifies with, the
        // digest would bind nothing.
        assert_eq!(body_digest("{}"), crate::audit::hash("{}"));
        assert_eq!(body_digest("{}").len(), 64);
        assert_ne!(body_digest("{}"), body_digest("{ }"));
    }
}
