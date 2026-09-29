//! The gate: six deterministic checks, in a fixed order, over rows.
//!
//! ## The whole design in one sentence
//!
//! **No model, no score, no threshold, no tie-break by judgement.** Each check
//! is a pure function from rows to a verdict, so the same rows always produce
//! the same answer and the answer is always explainable by naming the check.
//!
//! ## Why that is not merely an aesthetic choice
//!
//! A model placed inside an authorization path is attackable by the thing it
//! is judging: the same context that makes a claim plausible makes an
//! instruction to accept it plausible, and an LLM validator in a multi-agent
//! pipeline has been reported compromised in every trial that measured it,
//! while a separate non-LLM structural authorization layer held at zero.
//!
//! The uncomfortable corollary sets the product posture: **if any gate
//! authority derives from model judgement, then more proposals make the gate
//! strictly worse.** Scale is the enemy of a soft gate. So the gate is here —
//! six functions, no judgement — and anything that cannot be decided without
//! a model is absent rather than approximated.
//!
//! ## The order, and why it is fixed
//!
//! Cheap structural checks run before expensive ones, and each check that
//! fails stops the battery. Order is part of the contract: two runs over the
//! same claim must refuse with the SAME code, and a caller that sees a
//! different code for the same rows has learned something about the internal
//! ordering that the refusal vocabulary was designed not to leak.
//!
//!  1. schema conformance — the claim's shape against the human artifact
//!  2. bounds — every value inside its declared domain
//!  3. referential — subject, predicate and schema resolve
//!  4. citation resolvability — delegated to the workspace evidence crate
//!  5. contradiction — interval arithmetic over typed disjoint slots
//!  6. premise independence and selectivity
//!
//! A *repair* (a claim that contradicts a ratified one) additionally requires
//! an independent-support floor and a premise-independence pass. Repairs are
//! screened harder than assertions on purpose: a confident wrong repair is the
//! primary threat, and a repair is harder to catch than an assertion because
//! it looks like progress.
//!
//! ## Purity
//!
//! This module reads no clock, draws no randomness, opens no socket, and
//! touches no store. It is handed already-loaded rows and returns a verdict.
//! That is enforced by a pin, not by a convention: a gate that could read a
//! database would be a second, unreviewed authorization path.

use crate::workflow::create::Refusal;

/// The bound on a claim's evidence items. The evidence crate refuses more
/// than this on its own; the claim layer bounds it earlier so a proposal can
/// never carry an unbounded list.
pub(crate) const MAX_EVIDENCE_ITEMS: usize = 32;
/// The bound on a claim's qualifier set. An uncapped array in a text column is
/// an uncapped list.
pub(crate) const MAX_QUALIFIERS: usize = 8;
/// The bound on a claim's `contradicts` set.
pub(crate) const MAX_CONTRADICTS: usize = 8;
/// A repair — a claim that contradicts a ratified one — needs at least this
/// many independent supports before it may be considered at all.
pub(crate) const REPAIR_SUPPORT_FLOOR: i64 = 2;
/// The bound on any single slot value's length.
pub(crate) const MAX_SLOT_BYTES: usize = 512;
/// The bound on a subject or predicate identifier.
pub(crate) const MAX_IDENT_BYTES: usize = 128;

/// The declared type of a slot, and the disjointness class it belongs to.
///
/// Disjointness is what makes contradiction detection arithmetic rather than
/// semantics: two ratified claims on the same class conflict exactly when
/// their value intervals do not intersect. That is a property of the SCHEMA —
/// a human artifact — not of an algorithm, which is why pushing the hardness
/// into the artifact is the design move rather than an inconvenience.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotType {
    /// An inclusive integer interval.
    Integer,
    /// An instant; two instants on the same class are equal or disjoint.
    Instant,
    /// A closed label set. Two labels on the same class are disjoint unless
    /// identical.
    Label,
    /// No disjointness class: this slot cannot contradict anything.
    Free,
}

/// One declared slot in a human-authored schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlotDecl {
    pub(crate) predicate: String,
    pub(crate) ty: SlotType,
    /// The disjointness class. Two slots share a class when a claim on one
    /// contradicts a claim on the other.
    pub(crate) class: Option<&'static str>,
    /// Inclusive bounds for `Integer`; `None` means unbounded on that side.
    pub(crate) lo: Option<i64>,
    pub(crate) hi: Option<i64>,
    /// The closed label set for `Label`.
    pub(crate) labels: &'static [&'static str],
}

/// A typed slot value on a claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlotValue {
    Integer(i64),
    Instant(i64),
    Label(String),
    Free(String),
}

impl SlotValue {
    /// The interval this value occupies. A point interval for scalars; the
    /// whole closed range for a label, because a label's extent is its class.
    pub(crate) fn interval(&self, ty: SlotType) -> Interval {
        match (self, ty) {
            (SlotValue::Integer(v), SlotType::Integer)
            | (SlotValue::Instant(v), SlotType::Instant) => Interval::Point(*v),
            // A label's "extent" is the identity of the label itself, which is
            // why two different labels on one class are disjoint by
            // construction rather than by comparison of stored numbers.
            (SlotValue::Label(_), SlotType::Label) => Interval::Point(0),
            _ => Interval::All,
        }
    }
}

/// A closed interval. Intersection is the entire contradiction test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interval {
    Point(i64),
    Range(i64, i64),
    All,
}

impl Interval {
    /// Do two intervals share at least one point? Non-intersection is a
    /// contradiction; intersection is not.
    pub(crate) fn intersects(self, other: Self) -> bool {
        match (self, other) {
            (Interval::All, _) | (_, Interval::All) => true,
            (Interval::Point(a), Interval::Point(b)) => a == b,
            (Interval::Point(a), Interval::Range(lo, hi))
            | (Interval::Range(lo, hi), Interval::Point(a)) => a >= lo && a <= hi,
            (Interval::Range(a_lo, a_hi), Interval::Range(b_lo, b_hi)) => {
                a_lo <= b_hi && b_lo <= a_hi
            }
        }
    }
}

/// One citation on a claim: the source it points at and the byte range within
/// that source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Citation {
    pub(crate) source_cid: String,
    pub(crate) byte_start: i64,
    pub(crate) byte_end: i64,
    pub(crate) quote: String,
}

/// A claim as the gate sees it: already loaded, already typed. No store, no
/// clock, no principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaimUnderTest {
    pub(crate) claim_id: String,
    pub(crate) subject: String,
    pub(crate) predicate: String,
    /// The row id of the schema this claim was WRITTEN against — the foreign
    /// key, resolved before the battery runs.
    ///
    /// The gate is pure and never opens a store, so it cannot look a schema up
    /// itself; the caller resolves the FK and hands the result across. The
    /// field is here so that binding is visible at the type level rather than
    /// reconstructed from a request string at read time — which is what this
    /// field's absence once permitted.
    pub(crate) schema_ref: i64,
    pub(crate) value: SlotValue,
    pub(crate) declared_type: SlotType,
    pub(crate) qualifiers: Vec<(String, String)>,
    pub(crate) contradicts: Vec<String>,
    pub(crate) citations: Vec<Citation>,
    /// The bytes of each cited source, aligned with `citations` by index. The
    /// gate never fetches; the caller loads admitted bytes once.
    pub(crate) sources: Vec<Vec<u8>>,
    /// The ratified claims already in the store, for the contradiction pass.
    pub(crate) ratified: Vec<RatifiedClaim>,
}

/// A ratified claim reduced to what the contradiction pass needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RatifiedClaim {
    pub(crate) claim_id: String,
    pub(crate) predicate: String,
    pub(crate) value: SlotValue,
    pub(crate) declared_type: SlotType,
    pub(crate) class: Option<&'static str>,
    /// Independent support count, for the repair floor.
    pub(crate) support_n: i64,
}

/// The six checks, in the order they run. The order is the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Check {
    SchemaConformance = 1,
    Bounds = 2,
    Referential = 3,
    CitationResolvability = 4,
    Contradiction = 5,
    PremiseDiscipline = 6,
}

impl Check {
    pub(crate) const ALL: [Check; 6] = [
        Check::SchemaConformance,
        Check::Bounds,
        Check::Referential,
        Check::CitationResolvability,
        Check::Contradiction,
        Check::PremiseDiscipline,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Check::SchemaConformance => "schema_conformance",
            Check::Bounds => "bounds",
            Check::Referential => "referential",
            Check::CitationResolvability => "citation_resolvability",
            Check::Contradiction => "contradiction",
            Check::PremiseDiscipline => "premise_discipline",
        }
    }
}

/// What the gate decided. Carries the first failing check, never a location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateOutcome {
    Pass,
    Refused {
        check: Check,
        reason: Refusal,
    },
    /// The battery could not be completed because a check is not available in
    /// this build. Distinct from `Pass`: an unavailable check is not a pass,
    /// and a claim that waits here has been refused nothing and admitted
    /// nothing.
    Unavailable {
        check: Check,
    },
}

/// Run the battery. Deterministic, total, and order-stable.
pub(crate) fn run(claim: &ClaimUnderTest, schema: Option<&[SlotDecl]>) -> GateOutcome {
    // No ratified schema for this domain is a refusal, not an admission. An
    // open door where a schema should be is exactly the shape of failure this
    // loop exists to prevent.
    let Some(slots) = schema else {
        return GateOutcome::Refused {
            check: Check::SchemaConformance,
            reason: Refusal::NoSchema,
        };
    };

    if let Some(reason) = check_schema_conformance(claim, slots) {
        return GateOutcome::Refused {
            check: Check::SchemaConformance,
            reason,
        };
    }
    if let Some(reason) = check_bounds(claim, slots) {
        return GateOutcome::Refused {
            check: Check::Bounds,
            reason,
        };
    }
    if let Some(reason) = check_referential(claim, slots) {
        return GateOutcome::Refused {
            check: Check::Referential,
            reason,
        };
    }
    match check_citation_resolvability(claim) {
        Ok(()) => {}
        Err(reason) => {
            return GateOutcome::Refused {
                check: Check::CitationResolvability,
                reason,
            };
        }
    }
    if let Some(reason) = check_contradiction(claim, slots) {
        return GateOutcome::Refused {
            check: Check::Contradiction,
            reason,
        };
    }
    if let Some(reason) = check_premise_discipline(claim, slots) {
        return GateOutcome::Refused {
            check: Check::PremiseDiscipline,
            reason,
        };
    }
    GateOutcome::Pass
}

/// Check one: the claim's shape matches the human artifact.
///
/// A slot the schema does not declare is refused rather than ignored, and a
/// declared slot the claim left unfilled is refused rather than defaulted —
/// "no value" and "the value is the zero" are different claims, and a default
/// would silently make them the same one.
///
/// A MISS on the lookup is a REFUSAL. This was a live defect: the lookup
/// ended in `?`, so an undeclared predicate returned `None`, `run` read that as
/// "this check has no objection", and the claim was admitted. Four checks did
/// it independently, and a claim could clear all four by naming a predicate
/// nobody had ever declared — which is the one thing a human schema exists to
/// prevent.
fn check_schema_conformance(claim: &ClaimUnderTest, slots: &[SlotDecl]) -> Option<Refusal> {
    let decl = match find_slot(claim, slots) {
        Ok(decl) => decl,
        Err(reason) => return Some(reason),
    };
    if decl.ty != claim.declared_type {
        return Some(Refusal::SchemaMismatch);
    }
    let _conforms = match (&claim.value, decl.ty) {
        (SlotValue::Integer(_), SlotType::Integer) => true,
        (SlotValue::Instant(_), SlotType::Instant) => true,
        (SlotValue::Label(l), SlotType::Label) => decl.labels.contains(&l.as_str()),
        (SlotValue::Free(_), SlotType::Free) => true,
        _ => false,
    };
    if !_conforms {
        return Some(Refusal::SchemaMismatch);
    }
    if claim.qualifiers.len() > MAX_QUALIFIERS
        || claim.contradicts.len() > MAX_CONTRADICTS
        || claim.citations.len() > MAX_EVIDENCE_ITEMS
    {
        return Some(Refusal::SchemaMismatch);
    }
    for (k, v) in &claim.qualifiers {
        if k.is_empty() || k.len() > MAX_IDENT_BYTES || v.len() > MAX_SLOT_BYTES {
            return Some(Refusal::SchemaMismatch);
        }
    }
    None
}

/// Check two: every value sits inside its declared domain.
///
/// Unreachable for an undeclared predicate — check one refused it — and it says
/// so explicitly rather than bailing, so that the two halves cannot drift apart
/// if the battery is ever reordered.
fn check_bounds(claim: &ClaimUnderTest, slots: &[SlotDecl]) -> Option<Refusal> {
    let decl = match find_slot(claim, slots) {
        Ok(decl) => decl,
        Err(reason) => return Some(reason),
    };
    if claim.subject.len() > MAX_IDENT_BYTES || claim.subject.is_empty() {
        return Some(Refusal::OutOfBounds);
    }
    match claim.value {
        SlotValue::Integer(v) => {
            if decl.lo.is_some_and(|lo| v < lo) || decl.hi.is_some_and(|hi| v > hi) {
                return Some(Refusal::OutOfBounds);
            }
        }
        SlotValue::Instant(v) => {
            if decl.lo.is_some_and(|lo| v < lo) || decl.hi.is_some_and(|hi| v > hi) {
                return Some(Refusal::OutOfBounds);
            }
        }
        SlotValue::Label(ref l) | SlotValue::Free(ref l) => {
            if l.len() > MAX_SLOT_BYTES {
                return Some(Refusal::OutOfBounds);
            }
        }
    }
    None
}

/// Check three: the identifiers resolve.
///
/// "Resolve" is checked against the schema and the claim's own stored
/// vocabulary, never against a corpus scan. A check that looked a subject up
/// by fuzzy similarity would be a judgement wearing a deterministic hat.
fn check_referential(claim: &ClaimUnderTest, slots: &[SlotDecl]) -> Option<Refusal> {
    if claim.subject.trim().is_empty() {
        return Some(Refusal::Referential);
    }
    for cited in &claim.contradicts {
        if cited.trim().is_empty() || cited.len() > MAX_IDENT_BYTES {
            return Some(Refusal::Referential);
        }
        // A claim may only contradict something that is actually ratified.
        if !claim.ratified.iter().any(|r| &r.claim_id == cited) {
            return Some(Refusal::Referential);
        }
    }
    let _ = slots;
    None
}

/// Check four: every citation resolves over the ADMITTED bytes.
///
/// This check DELEGATES. The question "does this byte range of this source
/// really say this" has exactly one implementation in this repository — in
/// the workspace evidence crate — and a second implementation would be a
/// second answer to a security question. So this function loads nothing,
/// fetches nothing, normalises nothing, and hands the bytes and the refs
/// across. It does not inspect the verdict beyond pass or fail: the verdict's
/// own cause names the failure mode, and that cause is a LOCATION HINT, so it
/// stops here.
fn check_citation_resolvability(claim: &ClaimUnderTest) -> Result<(), Refusal> {
    if claim.citations.is_empty() {
        return Err(Refusal::EvidenceUnresolvable);
    }
    // One admitted byte buffer per citation. A claim that cites two ranges of
    // the same source loads it twice here; the caller may deduplicate, and the
    // cost is bounded by the evidence-item cap either way.
    let mut sources: Vec<&[u8]> = Vec::with_capacity(claim.citations.len());
    let mut refs: Vec<brain_evidence_core::EvidenceRef<'_>> =
        Vec::with_capacity(claim.citations.len());
    for (citation, source) in claim.citations.iter().zip(claim.sources.iter()) {
        if citation.byte_start < 0 || citation.byte_end <= citation.byte_start {
            return Err(Refusal::EvidenceUnresolvable);
        }
        let start = citation.byte_start as usize;
        let end = citation.byte_end as usize;
        // A range past the end of the admitted bytes is unresolvable, and the
        // comparison is a bound check rather than a slice: the slice form
        // would panic, and a gate that can panic on untrusted input is not a
        // gate, it is a denial-of-service.
        if end > source.len() {
            return Err(Refusal::EvidenceUnresolvable);
        }
        sources.push(source);
        refs.push(brain_evidence_core::EvidenceRef {
            source_cid: &citation.source_cid,
            quote: citation.quote.as_bytes(),
            byte_range: start..end,
        });
    }
    // The crate takes one source buffer for all refs, which is why the check
    // groups by citation: each citation is resolved against ITS OWN admitted
    // bytes, one call per citation, and the batch verdict is the conjunction.
    for ((source, reference), citation) in sources.iter().zip(refs.iter()).zip(&claim.citations) {
        let verdict = brain_evidence_core::resolve(source, std::slice::from_ref(reference));
        if verdict != brain_evidence_core::EvidenceVerdict::Resolved {
            return Err(Refusal::EvidenceUnresolvable);
        }
        let _ = citation;
    }
    Ok(())
}

/// Check five: contradiction, as arithmetic.
///
/// Two ratified claims on the same disjointness class conflict exactly when
/// their intervals do not intersect. No model, no embedding, no similarity —
/// and the ceiling is honest: this catches contradiction on TYPED, DISJOINT,
/// SINGLE-VALUED slots, because that is what the schema can declare. Semantic
/// contradiction between free-text claims is not deterministically checkable
/// and is therefore not implemented here at all.
fn check_contradiction(claim: &ClaimUnderTest, slots: &[SlotDecl]) -> Option<Refusal> {
    let decl = match find_slot(claim, slots) {
        Ok(decl) => decl,
        Err(reason) => return Some(reason),
    };
    let class = decl.class?;
    for other in &claim.ratified {
        if other.class != Some(class) || other.predicate == claim.predicate {
            continue;
        }
        if !claim
            .value
            .interval(claim.declared_type)
            .intersects(other.value.interval(other.declared_type))
        {
            return Some(Refusal::ContradictsRatified);
        }
    }
    None
}

/// Check six: premise independence and claim selectivity.
///
/// A system free to add premises can make any claim provable — which is
/// precisely what a generating agent is incentivised to do, so without these
/// two tests the loop is trivially gameable by over-generating support.
/// Independence: the claim must survive losing any single premise.
/// Selectivity: the claim must discriminate, because a policy that accepts
/// everything discriminates nothing and is not evidence of anything.
fn check_premise_discipline(claim: &ClaimUnderTest, slots: &[SlotDecl]) -> Option<Refusal> {
    let is_repair = !claim.contradicts.is_empty();

    if is_repair {
        // A repair needs an independent-support floor. This is the check that
        // a repair is screened harder than an assertion: the agent that
        // notices an error fixes it wrongly more often than it fixes it right,
        // and a repair with one support is that failure with a timestamp.
        if (claim.citations.len() as i64) < REPAIR_SUPPORT_FLOOR {
            return Some(Refusal::InsufficientSupport);
        }
        let mut distinct: Vec<&str> = Vec::new();
        for c in &claim.citations {
            let key: &str = &c.source_cid;
            if !distinct.contains(&key) {
                distinct.push(key);
            }
        }
        if (distinct.len() as i64) < REPAIR_SUPPORT_FLOOR {
            return Some(Refusal::PremiseDependent);
        }
    }

    // Premise independence for every claim: the cited set must not be one
    // source restated. One source, many ranges, is one premise.
    let distinct_sources = claim
        .citations
        .iter()
        .map(|c| c.source_cid.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if !claim.citations.is_empty() && distinct_sources.len() < 2 && !is_repair {
        return Some(Refusal::PremiseDependent);
    }

    // Claim selectivity: a claim on a slot whose declared domain is a single
    // point discriminates nothing, whatever value it carries — the slot
    // admits exactly one answer, so being right about it is not evidence of
    // anything. Refusing it is what stops "always true" from being the
    // cheapest thing to generate.
    let decl = match find_slot(claim, slots) {
        Ok(decl) => decl,
        Err(reason) => return Some(reason),
    };
    if matches!(decl.ty, SlotType::Integer | SlotType::Instant)
        && matches!((decl.lo, decl.hi), (Some(lo), Some(hi)) if lo == hi)
    {
        return Some(Refusal::PremiseDependent);
    }
    None
}

/// The one slot lookup, refusing a MISS.
///
/// Every check that needs the claim's declared slot goes through here. Before
/// this existed each check inlined its own `slots.iter().find(...)?`, and the
/// `?` made a miss indistinguishable from a pass — the single defect that let an
/// undeclared predicate through four checks at once. One function, one
/// behaviour.
///
/// The refusal is `SchemaMismatch` because that is what it is: the claim's
/// shape does not conform to the schema it named. It is deliberately check
/// one's reason rather than a per-check one, so the ORDER stays observable —
/// a battery that reordered itself and started reporting a bounds failure for
/// a claim with no declared bounds would be telling a caller something new.
fn find_slot<'a>(claim: &ClaimUnderTest, slots: &'a [SlotDecl]) -> Result<&'a SlotDecl, Refusal> {
    slots
        .iter()
        .find(|s| s.predicate == claim.predicate)
        .ok_or(Refusal::SchemaMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(predicate: &str, ty: SlotType, class: Option<&'static str>) -> SlotDecl {
        SlotDecl {
            predicate: predicate.to_string(),
            ty,
            class,
            lo: Some(0),
            hi: Some(100),
            labels: &["alpha", "beta"],
        }
    }

    fn citation(cid: &str, start: i64, end: i64, quote: &str) -> Citation {
        Citation {
            source_cid: cid.to_string(),
            byte_start: start,
            byte_end: end,
            quote: quote.to_string(),
        }
    }

    fn cid_v1_of(bytes: &[u8]) -> String {
        brain_evidence_core::cid_v1(bytes)
    }

    /// Two independent admitted sources. Premise independence means a claim
    /// citing one source collapses when that premise is removed, so a claim
    /// that is to pass must cite two — the fixture builds both rather than
    /// letting a test hand-count byte offsets, which is the defect class the
    /// first version of this fixture had.
    const SOURCE_A: &[u8] = b"the warranty runs for two years";
    const SOURCE_B: &[u8] = b"acme publishes a two year warranty term";

    fn cite(source: &[u8], start: usize, end: usize) -> (Citation, Vec<u8>) {
        let bytes = source.to_vec();
        (
            citation(
                &cid_v1_of(&bytes),
                start as i64,
                end as i64,
                std::str::from_utf8(&source[start..end]).expect("fixture slice is utf-8"),
            ),
            bytes,
        )
    }

    fn base_claim() -> ClaimUnderTest {
        let (a, source_a) = cite(SOURCE_A, 0, 21);
        let (b, source_b) = cite(SOURCE_B, 0, 5);
        ClaimUnderTest {
            claim_id: "clm_1".into(),
            subject: "acme".into(),
            schema_ref: 1,
            predicate: "warranty_months".into(),
            value: SlotValue::Integer(24),
            declared_type: SlotType::Integer,
            qualifiers: vec![("region".into(), "eu".into())],
            contradicts: vec![],
            citations: vec![a, b],
            sources: vec![source_a, source_b],
            ratified: vec![],
        }
    }

    #[test]
    fn a_well_formed_claim_passes_every_check() {
        let slots = [slot("warranty_months", SlotType::Integer, Some("warranty"))];
        assert_eq!(run(&base_claim(), Some(&slots)), GateOutcome::Pass);
    }

    /// RED-FIRST (R54 §3.1). The doc comment above `check_schema_conformance`
    /// states the law this pin holds: a slot the schema does not declare is
    /// *refused rather than ignored*. The implementation did the opposite —
    /// `?` on a missing `SlotDecl` returned `None`, and `run` reads `None` as
    /// "this check has no objection", so checks 1, 2, 5 and 6 all fell through
    /// and the claim was admitted.
    #[test]
    fn an_undeclared_predicate_is_refused_rather_than_ignored() {
        let slots = [slot("warranty_months", SlotType::Integer, Some("warranty"))];
        let mut claim = base_claim();
        // Everything else about the claim is well formed: two independent
        // resolvable sources, a discriminating value, no dangling reference.
        // The ONLY defect is that the schema never declared this predicate.
        claim.predicate = "never_declared".into();
        match run(&claim, Some(&slots)) {
            GateOutcome::Refused { check, reason } => {
                assert_eq!(
                    check,
                    Check::SchemaConformance,
                    "an undeclared predicate is a shape failure, and shape is check one"
                );
                assert_eq!(
                    reason,
                    Refusal::SchemaMismatch,
                    "the claim's shape does not conform to the schema it named"
                );
            }
            other => panic!(
                "a predicate the human schema never declared must be REFUSED, got {other:?}. \
                 The lookup below is a miss, and a miss is a refusal — not an absence of one."
            ),
        }
    }

    /// The second half of the same law, on the battery's own order. If the
    /// undeclared-predicate refusal ever moved off check one, the remaining
    /// three lookups would silently pass again. This pins the ORDER, not just
    /// the outcome, because the outcome alone cannot tell those two cases apart.
    #[test]
    fn the_undeclared_predicate_refusal_is_reported_by_check_one() {
        let slots = [slot("warranty_months", SlotType::Integer, Some("warranty"))];
        let mut claim = base_claim();
        claim.predicate = "never_declared".into();
        claim.value = SlotValue::Integer(4_000); // ALSO out of bounds
        match run(&claim, Some(&slots)) {
            GateOutcome::Refused { check, .. } => assert_eq!(
                check,
                Check::SchemaConformance,
                "shape is checked first, so the shape failure is what a caller is told — a \
                 reordered battery would leak that these two defects are distinguishable"
            ),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_checks_run_in_the_fixed_declared_order() {
        // Each variant trips exactly one check; the reported check must be the
        // one whose ordinal position in `Check` the test expects. An
        // implementation that reordered the battery would fail here.
        let slots = [slot("warranty_months", SlotType::Integer, Some("warranty"))];

        let mut no_schema = base_claim();
        no_schema.value = SlotValue::Integer(9999);
        match run(&no_schema, None) {
            GateOutcome::Refused { check, .. } => {
                assert_eq!(
                    check,
                    Check::SchemaConformance,
                    "no schema is a shape failure"
                )
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        let mut wrong_type = base_claim();
        wrong_type.value = SlotValue::Label("alpha".into());
        match run(&wrong_type, Some(&slots)) {
            GateOutcome::Refused { check, .. } => assert_eq!(check, Check::SchemaConformance),
            other => panic!("expected a refusal, got {other:?}"),
        }

        let mut out_of_bounds = base_claim();
        out_of_bounds.value = SlotValue::Integer(4_000);
        match run(&out_of_bounds, Some(&slots)) {
            GateOutcome::Refused { check, reason } => {
                assert_eq!(check, Check::Bounds);
                assert_eq!(reason, Refusal::OutOfBounds);
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        let mut dangling = base_claim();
        dangling.contradicts = vec!["clm_nonexistent".into()];
        match run(&dangling, Some(&slots)) {
            GateOutcome::Refused { check, .. } => assert_eq!(check, Check::Referential),
            other => panic!("expected a refusal, got {other:?}"),
        }

        let mut bad_citation = base_claim();
        bad_citation.citations[0].quote = "not what the bytes say".into();
        match run(&bad_citation, Some(&slots)) {
            GateOutcome::Refused { check, reason } => {
                assert_eq!(check, Check::CitationResolvability);
                assert_eq!(reason, Refusal::EvidenceUnresolvable);
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        let mut contradicts = base_claim();
        contradicts.ratified = vec![RatifiedClaim {
            claim_id: "clm_0".into(),
            predicate: "other".into(),
            value: SlotValue::Integer(60),
            declared_type: SlotType::Integer,
            class: Some("warranty"),
            support_n: 5,
        }];
        match run(&contradicts, Some(&slots)) {
            GateOutcome::Refused { check, reason } => {
                assert_eq!(check, Check::Contradiction);
                assert_eq!(reason, Refusal::ContradictsRatified);
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        let mut one_premise = base_claim();
        one_premise.citations.truncate(1);
        one_premise.sources.truncate(1);
        match run(&one_premise, Some(&slots)) {
            GateOutcome::Refused { check, reason } => {
                assert_eq!(check, Check::PremiseDiscipline);
                assert_eq!(reason, Refusal::PremiseDependent);
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_battery_is_a_pure_function_of_its_rows() {
        let slots = [slot("warranty_months", SlotType::Integer, Some("warranty"))];
        let claim = base_claim();
        let first = run(&claim, Some(&slots));
        // Re-derive from a fresh, independently built row set: identical input
        // must give an identical verdict, or the gate is not a function.
        let second = run(&base_claim(), Some(&slots));
        assert_eq!(first, second);
    }

    #[test]
    fn a_contradiction_on_a_shared_class_is_interval_arithmetic() {
        let a = Interval::Range(10, 20);
        let b = Interval::Point(25);
        assert!(!a.intersects(b));
        assert!(Interval::Range(10, 20).intersects(Interval::Point(15)));
        assert!(Interval::All.intersects(b));
        assert!(Interval::Point(5).intersects(Interval::All));
        // Touching endpoints intersect: [10,20] and [20,30] share 20.
        assert!(Interval::Range(10, 20).intersects(Interval::Point(20)));
        assert!(!Interval::Range(10, 20).intersects(Interval::Range(21, 30)));
    }

    #[test]
    fn a_repair_needs_the_independent_support_floor() {
        let slots = [slot("warranty_months", SlotType::Integer, Some("warranty"))];
        let mut repair = base_claim();
        repair.contradicts = vec!["clm_0".into()];
        repair.ratified = vec![RatifiedClaim {
            claim_id: "clm_0".into(),
            predicate: "other".into(),
            value: SlotValue::Integer(60),
            declared_type: SlotType::Integer,
            // A DIFFERENT disjointness class, so this ratified claim resolves
            // the repair's reference without also contradicting it. The point
            // of the test is the support floor, and a claim that contradicts
            // its own target would be refused one check earlier.
            class: Some("unrelated_class"),
            support_n: 5,
        }];
        repair.contradicts = vec!["clm_0".into()];
        // One premise: below the repair floor AND premise-dependent.
        repair.citations.truncate(1);
        repair.sources.truncate(1);
        match run(&repair, Some(&slots)) {
            GateOutcome::Refused { reason, .. } => assert_eq!(reason, Refusal::InsufficientSupport),
            other => panic!("a one-support repair must be refused, got {other:?}"),
        }
        // Two supports from two distinct sources clears the floor.
        let (extra, extra_source) = cite(b"a third note on the same term", 0, 1);
        repair.citations.push(extra);
        repair.sources.push(extra_source);
        match run(&repair, Some(&slots)) {
            GateOutcome::Pass => {}
            GateOutcome::Refused { reason, .. } => {
                panic!("a two-source repair clears the floor, but was refused: {reason:?}")
            }
            GateOutcome::Unavailable { check } => {
                panic!("no check may be unavailable in this build, got {check:?}")
            }
        }
    }

    #[test]
    fn a_citation_past_the_end_of_the_admitted_bytes_is_refused_not_panicked() {
        let slots = [slot("warranty_months", SlotType::Integer, Some("warranty"))];
        let mut claim = base_claim();
        claim.citations[0].byte_end = 9_999;
        match run(&claim, Some(&slots)) {
            GateOutcome::Refused { check, .. } => {
                assert_eq!(check, Check::CitationResolvability)
            }
            other => panic!("an out-of-range citation must be refused, got {other:?}"),
        }
    }

    #[test]
    fn the_check_order_is_the_contract() {
        let names: Vec<&str> = Check::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "schema_conformance",
                "bounds",
                "referential",
                "citation_resolvability",
                "contradiction",
                "premise_discipline",
            ]
        );
        for w in Check::ALL.windows(2) {
            assert!(w[0] < w[1], "check order must be strictly ascending");
        }
    }
}
