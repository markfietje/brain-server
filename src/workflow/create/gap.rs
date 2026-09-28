//! Gap generation: a GENERATOR, not a detector.
//!
//! ## Why over-generate
//!
//! No published technique reliably answers "what does this knowledge base not
//! know". The techniques that exist are unreliable as DETECTORS and safe as
//! GENERATORS, because the gate downstream is deterministic: a wrong gap costs
//! one refusal, and a missing gap costs the loop. So the usual generator
//! precision metric is inverted here — recall of gaps is the thing that
//! matters and precision is nearly free.
//!
//! ## Why this module cannot set a status
//!
//! The type signature is the control, not a comment. [`GapRanking`] and
//! [`GapCandidate`] have no field that could hold a claim status, so no
//! ranking method can promote, reject or ratify anything even by accident. An
//! ordering method that could authorize is not a ranking method; the type
//! makes that unrepresentable instead of merely documented.
//!
//! ## Why these adapters are ungated
//!
//! A generator that requires a ratified schema before it may run produces
//! nothing at all until a human has authored one per domain, which means the
//! loop is dead on arrival in every new domain. The one method that genuinely
//! needs formalised predicates therefore ships ungated and says so: it emits
//! nothing when no schema exists, rather than refusing to load.

use crate::workflow::create::schema::SchemaDecl;

/// The hard cap on generated gaps per run. Over-generation is the design, but
/// it is still bounded: an unbounded flood is a denial of service against the
/// gate, and a gate that is being flooded is a gate that is being worn down.
pub(crate) const GAP_FLOOD_CAP: usize = 256;

/// A candidate gap: a question the corpus does not yet answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GapCandidate {
    /// A stable identity for the gap, derived from its shape so a rerun
    /// produces the same id and the loop is replayable.
    pub(crate) gap_id: String,
    pub(crate) domain: String,
    pub(crate) subject: String,
    pub(crate) predicate: String,
    /// The method that produced it. Recorded so an operator can tell a
    /// symbolic gap from a drill-induced one, which are not equally trusted.
    pub(crate) method: GapMethod,
    /// A deterministic ordering score. RANKING ONLY — see the module header.
    pub(crate) score: i64,
}

/// The four generation methods, ranked by how much of the loop they can carry.
///
/// They are adapters behind one trait, never a switchboard: a caller cannot
/// name a method in a request, because the method is chosen from the shape of
/// the question and the state of the domain, not by the proposer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum GapMethod {
    /// Cheap, adversarially safe — a failed drill is information rather than
    /// poison — and the only method that produces the byte-range evidence the
    /// claim phase needs. This is the default.
    DrillInduced,
    /// Implied-fact inference. The most dangerous class: the strongest single
    /// agent gets a fifth of the hardest template right, and the best result
    /// across all agents is one in eighty-four. Per-item human review is blind
    /// to this class BY CONSTRUCTION, because each item is individually
    /// plausible. Which is why these are generated and never auto-promoted.
    ImpliedFact,
    /// Symbolic predicate gaps. Highest confidence, and it requires the corpus
    /// already formalised as predicates — so it emits nothing without a
    /// ratified schema.
    SymbolicPredicate,
    /// An ordering method that can rank a queue and can never authorize.
    EntropyRank,
}

impl GapMethod {
    pub(crate) const ALL: [GapMethod; 4] = [
        GapMethod::DrillInduced,
        GapMethod::ImpliedFact,
        GapMethod::SymbolicPredicate,
        GapMethod::EntropyRank,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            GapMethod::DrillInduced => "drill_induced",
            GapMethod::ImpliedFact => "implied_fact",
            GapMethod::SymbolicPredicate => "symbolic_predicate",
            GapMethod::EntropyRank => "entropy_rank",
        }
    }

    /// May this method's candidates ever be promoted without a human reading
    /// them? Only the drill-induced class, and even that is refused in this
    /// round because nothing may be promoted at all. The predicate exists so
    /// that a future round changing the default has to change THIS.
    pub(crate) const fn requires_human_before_promotion(self) -> bool {
        true
    }
}

/// One generation method behind one trait.
pub(crate) trait GapGenerator {
    /// A stable name for the adapter.
    fn name(&self) -> GapMethod;
    /// Produce candidates for a domain. Pure: same input, same output.
    fn generate(
        &self,
        domain: &str,
        known: &[String],
        schema: Option<&SchemaDecl>,
    ) -> Vec<GapCandidate>;
}

/// Everything the generators are allowed to know about a domain's corpus.
///
/// Deliberately a plain list of covered predicates rather than a handle to the
/// store: a generator that can query the corpus is a generator whose output
/// depends on load, and the loop's replay property is worth more here than the
/// marginal coverage.
#[derive(Debug, Clone, Default)]
pub(crate) struct DomainState {
    pub(crate) covered: Vec<String>,
}

/// Drill-induced gaps: ask the corpus a question it cannot answer.
///
/// A failed drill is information. That is the property that makes this the
/// default: every method that fails badly produces noise the gate must sift,
/// while a failed drill produces a specific, bounded, attributable gap.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DrillInduced;

impl GapGenerator for DrillInduced {
    fn name(&self) -> GapMethod {
        GapMethod::DrillInduced
    }

    fn generate(
        &self,
        domain: &str,
        known: &[String],
        _schema: Option<&SchemaDecl>,
    ) -> Vec<GapCandidate> {
        // Deterministic: the drill set is fixed, and a drill whose predicate is
        // already covered produces nothing. No clock, no randomness — a
        // generator that varies between runs cannot be replayed against the
        // corpus it was generated from.
        let mut out = Vec::new();
        for (index, probe) in DRILL_PROBES.iter().enumerate() {
            if known.iter().any(|k| k == probe) {
                continue;
            }
            out.push(GapCandidate {
                gap_id: format!("{domain}:{probe}:{index}"),
                domain: domain.to_string(),
                subject: domain.to_string(),
                predicate: (*probe).to_string(),
                method: GapMethod::DrillInduced,
                // A lower score sorts first; the score is the probe's index, so
                // the order is the declaration order and nothing else.
                score: index as i64,
            });
        }
        out
    }
}

/// The fixed drill set. One predicate per question the corpus must be able to
/// answer; a domain missing one of these has a real gap.
const DRILL_PROBES: &[&str] = &[
    "applies_to",
    "excludes",
    "supersedes",
    "evidence_for",
    "last_reviewed_by",
    "not_applicable_when",
];

/// Symbolic predicate gaps: the uncovered slots of a formalised domain.
///
/// Ungated by design, and it emits nothing when no schema is ratified — a
/// method that requires a human artifact to produce anything at all would make
/// every new domain a dead loop, so it degrades to producing nothing rather
/// than refusing to load.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SymbolicPredicateGap;

impl GapGenerator for SymbolicPredicateGap {
    fn name(&self) -> GapMethod {
        GapMethod::SymbolicPredicate
    }

    fn generate(
        &self,
        domain: &str,
        known: &[String],
        schema: Option<&SchemaDecl>,
    ) -> Vec<GapCandidate> {
        let Some(schema) = schema else {
            // Un-gated, not un-useful: with no ratified schema there are no
            // formalised predicates to be uncovered.
            return Vec::new();
        };
        schema
            .slots
            .iter()
            .filter(|s| !known.iter().any(|k| k == &s.predicate))
            .map(|s| GapCandidate {
                gap_id: format!("{domain}:{}:symbolic", s.predicate),
                domain: domain.to_string(),
                subject: domain.to_string(),
                predicate: s.predicate.clone(),
                method: GapMethod::SymbolicPredicate,
                score: 0,
            })
            .collect()
    }
}

/// Implied-fact gaps: the dangerous class, generated anyway.
///
/// Generated because a missing gap costs the loop, and never promoted without
/// a human because per-item review cannot see this failure mode — each
/// inference is individually plausible, so a reviewer has nothing to object to.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ImpliedFact;

impl GapGenerator for ImpliedFact {
    fn name(&self) -> GapMethod {
        GapMethod::ImpliedFact
    }

    fn generate(
        &self,
        domain: &str,
        known: &[String],
        _schema: Option<&SchemaDecl>,
    ) -> Vec<GapCandidate> {
        IMPLICATION_AXIOMS
            .iter()
            .enumerate()
            .filter(|(_, (base, _))| !known.iter().any(|k| k == *base))
            .map(|(index, (base, implied))| GapCandidate {
                gap_id: format!("{domain}:{base}:implied:{index}"),
                domain: domain.to_string(),
                subject: domain.to_string(),
                predicate: (*implied).to_string(),
                method: GapMethod::ImpliedFact,
                score: index as i64,
            })
            .collect()
    }
}

/// Implication rules: a known predicate implies another. Kept as data so the
/// generator's output is auditable by reading one table.
const IMPLICATION_AXIOMS: &[(&str, &str)] = &[
    ("applies_to", "excludes"),
    ("supersedes", "last_reviewed_by"),
    ("evidence_for", "not_applicable_when"),
];

/// An ordering over existing candidates. It returns an ORDER and nothing else.
///
/// There is no status field on this type and no method that could set one.
/// That is the control: an ordering method that could authorize would be a
/// second, unaudited gate, and it would be the one nobody reads.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EntropyRank;

impl EntropyRank {
    /// Sort by score, then by gap id, so the order is total and stable.
    /// The tie-break is the identity, never a comparison of scores that
    /// "feels" more informative.
    pub(crate) fn rank(&self, mut candidates: Vec<GapCandidate>) -> Vec<GapCandidate> {
        candidates.sort_by(|a, b| a.score.cmp(&b.score).then_with(|| a.gap_id.cmp(&b.gap_id)));
        candidates
    }
}

/// Run every adapter and return the flood, capped.
///
/// The cap is the bound and it is applied HERE rather than by each adapter, so
/// a new adapter cannot quietly raise the loop's ceiling.
pub(crate) fn generate(
    domain: &str,
    state: &DomainState,
    schema: Option<&SchemaDecl>,
) -> Vec<GapCandidate> {
    let adapters: [&dyn GapGenerator; 3] = [&DrillInduced, &SymbolicPredicateGap, &ImpliedFact];
    let mut out: Vec<GapCandidate> = adapters
        .iter()
        .flat_map(|a| a.generate(domain, &state.covered, schema))
        .collect();
    // Deterministic order before the cap, so the cap drops the same candidates
    // on every run rather than whichever the allocator happened to yield.
    out.sort_by(|a, b| a.gap_id.cmp(&b.gap_id));
    out.truncate(GAP_FLOOD_CAP);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::create::schema::SchemaDecl;
    use crate::workflow::create::verify::SlotDecl;

    fn schema() -> SchemaDecl {
        SchemaDecl {
            domain: "global".into(),
            version: 1,
            slots: vec![
                SlotDecl {
                    predicate: "applies_to".into(),
                    ty: crate::workflow::create::verify::SlotType::Free,
                    class: None,
                    lo: None,
                    hi: None,
                    labels: &[],
                },
                SlotDecl {
                    predicate: "excludes".into(),
                    ty: crate::workflow::create::verify::SlotType::Free,
                    class: None,
                    lo: None,
                    hi: None,
                    labels: &[],
                },
            ],
        }
    }

    #[test]
    fn the_flood_cap_is_a_bound_and_it_is_enforced() {
        let state = DomainState::default();
        let out = generate("global", &state, Some(&schema()));
        assert!(
            out.len() <= GAP_FLOOD_CAP,
            "the flood is bounded: {} > {GAP_FLOOD_CAP}",
            out.len()
        );
    }

    #[test]
    fn generation_is_deterministic_and_replayable() {
        let state = DomainState::default();
        let first = generate("global", &state, Some(&schema()));
        let second = generate("global", &DomainState::default(), Some(&schema()));
        assert_eq!(first, second, "the same input must give the same flood");
        let ids: Vec<&str> = first.iter().map(|g| g.gap_id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "the flood is emitted in a stable order");
    }

    #[test]
    fn a_covered_predicate_produces_no_gap() {
        let state = DomainState {
            covered: DRILL_PROBES.iter().map(|s| (*s).to_string()).collect(),
        };
        let out = generate("global", &state, Some(&schema()));
        for gap in &out {
            assert!(
                !DRILL_PROBES.contains(&gap.predicate.as_str()),
                "a covered predicate must not generate a gap: {}",
                gap.predicate
            );
        }
    }

    #[test]
    fn the_symbolic_adapter_emits_nothing_without_a_ratified_schema() {
        let out = SymbolicPredicateGap.generate("global", &[], None);
        assert!(
            out.is_empty(),
            "with no ratified schema there are no formalised predicates to be uncovered; \
             the adapter degrades to silence rather than refusing to load"
        );
        let with_schema = SymbolicPredicateGap.generate("global", &[], Some(&schema()));
        assert!(
            !with_schema.is_empty(),
            "with a ratified schema the uncovered slots must appear"
        );
    }

    #[test]
    fn a_ranking_method_cannot_carry_a_status() {
        // Structural, not documentary: the ranking output type has no field
        // that could hold one, so no implementation can smuggle authorization
        // through a sort.
        let candidates = generate("global", &DomainState::default(), Some(&schema()));
        let ranked = EntropyRank.rank(candidates);
        assert_eq!(ranked.len(), GAP_FLOOD_CAP.min(ranked.len()));
        for gap in &ranked {
            assert!(
                gap.method.requires_human_before_promotion(),
                "{} must require a human before promotion",
                gap.method.as_str()
            );
        }
        // And the dangerous class is never exempt.
        for method in GapMethod::ALL {
            assert!(
                method.requires_human_before_promotion(),
                "{} is exempt from the human-read requirement; the implied-fact class is \
                 the one failure mode per-item review is blind to, so an exemption anywhere \
                 in this table is a hole",
                method.as_str()
            );
        }
    }

    #[test]
    fn the_generator_is_write_free_and_gate_free() {
        // Structural: the module's production region contains no store
        // reference and no claim-table write. The generator may not create the
        // thing it generates.
        let source = include_str!("gap.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or_default();
        for token in ["claims", "claim_evidence", "INSERT", "UPDATE"] {
            assert!(
                !production.contains(token),
                "the generator names `{token}` — gap generation is gate-free AND \
                 write-free. It proposes questions; it never answers them."
            );
        }
    }
}
