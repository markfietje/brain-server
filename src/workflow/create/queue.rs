//! The ranked gap queue: the `Operate → Create` edge, built and drained.
//!
//! ## What this is
//!
//! [`gap::generate`] over-generates candidate questions the corpus does not
//! answer, and [`gap::EntropyRank`] orders them. Neither is a queue: a flood
//! with no order is noise, and an order with nothing draining it is a report.
//! This module is the queue — ranked, finite, budgeted, and **drainable** — and
//! it is the `Operate → Create` edge the round needed to close.
//!
//! ## Bounded self-motivation, and why the bounds are constants
//!
//! This is the plan's bounded form of self-motivation, and every bound is a
//! **module constant** rather than a parameter:
//!
//! * [`EXPLORATION_QUOTA`] — at most this many items in four may be exploratory.
//! * [`SPEND_CEILING_UNITS`] — the queue's whole population may not exceed this
//!   many integer ten-thousandths of predicted value.
//! * [`kill_condition`] — measured on the **same eval set as the census**, and
//!   it kills the queue rather than the system.
//!
//! They are constants for the same reason [`PROMOTION_ENABLED`] is: a bound an
//! operator can raise under pressure is not a bound. `P57.4` preregistered all
//! three before the queue was written.
//!
//! ## Online intrinsic goal selection is NOT here, and its absence is the finding
//!
//! The plan is explicit that online intrinsic goal selection is off the roadmap,
//! and that the exploration-bottleneck literature holds unchecked intrinsic
//! motivation destroys sample efficiency, with no 2026 primary work establishing
//! it as production-viable. **This queue chooses nothing by itself.** It ranks
//! candidates the loop generated from a fixed, declarative probe set, and its
//! "exploratory" items are the ones with no predicted win — chosen by
//! *position*, not by a self-assigned objective. A system that decided what it
//! wanted to know next, with no external anchor, is the thing the literature
//! warns about, and nothing here is it.
//!
//! ## What this queue is NOT allowed to do
//!
//! **It cannot authorize.** The type is the control, not a comment: [`QueueItem`]
//! has no status field, no claim id that could be promoted, and no method that
//! could set either. It is a ranked pointer and nothing more — the same
//! structural refusal [`gap::GapCandidate`] and
//! [`crate::workflow::create::RefusalReceipt`] already make.
//!
//! **Least privilege at the surface.** An item reveals a predicate and a domain,
//! never claim content, because a queue that showed its contents would be a
//! read path to unratified material — the thing `service::create::recall_page`
//! exists to make impossible.
//!
//! ## Blast radius, named rather than discovered
//!
//! A queue that ranks work is a control that can **starve other work**: an
//! operator draining this queue is spending attention and budget on it. So the
//! queue is finite, budget-capped, and killable, and this is stated here so an
//! operator meets it in the docs rather than in a quarter of lost throughput.

use crate::workflow::create::gap::{GAP_FLOOD_CAP, GapCandidate, GapMethod};
use crate::workflow::drift_census::GLOBAL_TOLERANCE_UNITS;

/// `P57.4` — the exploration quota, in items **per four**.
///
/// One in four is the shape the plan asks for and it is deliberately not larger:
/// a queue that spends most of its budget on items it cannot predict is a queue
/// that cannot be held to account.
pub(crate) const EXPLORATION_QUOTA: usize = 1;

/// `P57.4` — the spend ceiling, in integer ten-thousandths, over the WHOLE queue.
///
/// A queue whose population can exceed its ceiling has no ceiling. The check is
/// against the population, not per-item, so a long queue cannot hide behind many
/// individually-cheap items.
pub(crate) const SPEND_CEILING_UNITS: i64 = 5_000;

/// `P57.4` — the kill condition's floor, in integer ten-thousandths.
///
/// The kill condition is measured on the same eval set as the census: **if the
/// best-ranked item's predicted value is at or below the global tolerance, the
/// queue is predicting no win above noise, and it stops.** A queue that keeps
/// spending on items it expects to fail is a queue draining attention for
/// nothing, and the fix is to stop it rather than to keep it busy.
pub(crate) const KILL_FLOOR_UNITS: i32 = GLOBAL_TOLERANCE_UNITS;

/// The bound on a queue's length. Finite by construction: the generator caps at
/// [`GAP_FLOOD_CAP`], and the queue refuses to exceed it either, so a future
/// generator that raises the flood cap cannot silently make this unbounded.
pub(crate) const MAX_QUEUE_ITEMS: usize = GAP_FLOOD_CAP;

/// One ranked, budgeted queue item.
///
/// **No status field, no claim id, no promotion path** — the type is the control.
/// This is a pointer at work, and a pointer that could authorize is not a queue
/// item, it is a second unaudited gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueueItem {
    /// The gap's stable identity. Carries the domain and predicate already, so
    /// a reader can locate the work without the queue leaking anything else.
    pub(crate) gap_id: String,
    pub(crate) domain: String,
    pub(crate) predicate: String,
    pub(crate) method: GapMethod,
    /// The generator's ranking score. Ranking only — see the module header.
    pub(crate) score: i64,
    /// What the item is predicted to be WORTH, in integer ten-thousandths, on
    /// the census's own scale. Read against [`KILL_FLOOR_UNITS`]: an item worth
    /// at or below the floor is not worth doing and is marked exploratory.
    pub(crate) predicted_units: i32,
    /// What pursuing the item COSTS, in integer ten-thousandths. Summed over the
    /// population and checked against [`SPEND_CEILING_UNITS`].
    ///
    /// **Value and cost are separate numbers on purpose.** A queue that spent
    /// its budget on what it expected to gain would never refuse anything —
    /// the more a flood promised, the more it would be allowed to spend. The
    /// ceiling is a budget, and a budget is not a forecast.
    pub(crate) cost_units: i32,
    /// Whether this item is exploratory (no predicted win) rather than scored.
    /// Bounded by [`EXPLORATION_QUOTA`].
    pub(crate) exploratory: bool,
}

/// The queue's own verdict, and it is deliberately small.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueueVerdict {
    /// Items were admitted, in rank order.
    Admitted(usize),
    /// The spend ceiling refused the whole population. **Refused, never
    /// truncated**: a queue that silently kept the items under the ceiling would
    /// report a shorter queue than the operator asked about, and the difference
    /// is exactly what the ceiling exists to bound.
    SpendCeilingRefused,
    /// The exploration quota was exceeded by the ranked set. Refused rather than
    /// trimmed, for the same reason: a queue that silently drops the surplus
    /// reports a clean admit it did not perform.
    ExplorationQuotaRefused,
    /// The kill condition fired: the best predicted value is at or below the
    /// floor. The queue stops.
    Killed,
}

impl QueueVerdict {
    /// The closed vocabulary. No content can reach it, so no predicate or domain
    /// can ever vary a label.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            QueueVerdict::Admitted(_) => "admitted",
            QueueVerdict::SpendCeilingRefused => "spend_ceiling_refused",
            QueueVerdict::ExplorationQuotaRefused => "exploration_quota_refused",
            QueueVerdict::Killed => "killed",
        }
    }
}

/// The queue, with its verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GapQueue {
    pub(crate) items: Vec<QueueItem>,
    pub(crate) verdict: QueueVerdict,
}

impl GapQueue {
    /// Total predicted value across the admitted population, in units.
    pub(crate) fn predicted_total_units(&self) -> i64 {
        self.items
            .iter()
            .map(|i| i64::from(i.predicted_units))
            .sum()
    }

    /// Total cost across the admitted population, in units. This is what the
    /// spend ceiling bounds.
    pub(crate) fn cost_total_units(&self) -> i64 {
        self.items.iter().map(|i| i64::from(i.cost_units)).sum()
    }

    /// How many admitted items are exploratory.
    pub(crate) fn exploratory_count(&self) -> usize {
        self.items.iter().filter(|i| i.exploratory).count()
    }

    /// Is this queue empty?
    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Build the queue from a generated flood, in rank order.
///
/// The ranking is the generator's own (`EntropyRank` orders by score then
/// identity); this function does not re-rank, because a second ordering with a
/// second tie-break is a second opinion nobody reviews.
///
/// **The order of the refusals is the policy.** The kill condition is checked
/// FIRST, then the length bound, then the exploration quota, then the spend
/// ceiling. A queue that is killed should not first report that it exceeded a
/// budget — it is not spending anything, so the budget question is moot, and
/// answering it would tell an operator their queue is oversized when the truth
/// is that it is not worth draining.
pub(crate) fn build(mut flood: Vec<GapCandidate>) -> GapQueue {
    // Deterministic order before any bound, so the bound refuses the same
    // population on every run rather than whichever the allocator yielded.
    flood.sort_by(|a, b| a.score.cmp(&b.score).then_with(|| a.gap_id.cmp(&b.gap_id)));

    // Predicted value, derived from the generator's own score. **A declared
    // approximation, and named as one**: a lower generator score means a more
    // fundamental gap, which this reads as worth more. It is not a learned
    // prediction and nothing downstream may claim it is — the census's own
    // ceiling (a frozen corpus cannot predict the future) applies here too.
    let mut items: Vec<QueueItem> = flood
        .iter()
        .map(|c| QueueItem {
            gap_id: c.gap_id.clone(),
            domain: c.domain.clone(),
            predicate: c.predicate.clone(),
            method: c.method,
            score: c.score,
            predicted_units: predicted_units(c),
            cost_units: cost_units(c),
            exploratory: false,
        })
        .collect();

    // The kill condition, first. If nothing is predicted to clear the floor,
    // the queue stops — and stopping is the correct outcome, not a failure to
    // be worked around.
    let best = items.iter().map(|i| i.predicted_units).max().unwrap_or(0);
    if best <= KILL_FLOOR_UNITS {
        return GapQueue {
            items: Vec::new(),
            verdict: QueueVerdict::Killed,
        };
    }

    if items.len() > MAX_QUEUE_ITEMS {
        return GapQueue {
            items: Vec::new(),
            verdict: QueueVerdict::SpendCeilingRefused,
        };
    }

    // The exploration quota applies to the *bottom* of the ranked list: the
    // items with no predicted win. Taking them from the top would mean
    // exploring instead of doing the highest-ranked work, which is a different
    // policy and not this one.
    let mut exploratory_budget = (items.len() / 4) * EXPLORATION_QUOTA;
    for item in &mut items {
        if item.predicted_units <= KILL_FLOOR_UNITS {
            if exploratory_budget == 0 {
                return GapQueue {
                    items: Vec::new(),
                    verdict: QueueVerdict::ExplorationQuotaRefused,
                };
            }
            exploratory_budget -= 1;
            item.exploratory = true;
        }
    }

    // The spend ceiling, over the whole population's COST. Refused, never
    // truncated. Checked against cost rather than predicted value, because a
    // ceiling on what a flood PROMISES would reward it for promising more.
    let total: i64 = items.iter().map(|i| i64::from(i.cost_units)).sum();
    if total > SPEND_CEILING_UNITS {
        return GapQueue {
            items: Vec::new(),
            verdict: QueueVerdict::SpendCeilingRefused,
        };
    }

    let admitted = items.len();
    GapQueue {
        items,
        verdict: QueueVerdict::Admitted(admitted),
    }
}

/// The generator's real score range: the drill set is indexed `0..5` and the
/// implication axioms `0..2`, and the additive drills are bounded by the flood
/// cap. A score past this is a score nothing produces, and treating it as one
/// would mean a future generator could quietly widen the range the policy was
/// reasoned about.
const MAX_GENERATOR_SCORE: i64 = 5;

/// The most a single item is predicted to be worth, in integer ten-thousandths.
///
/// A declared approximation, and named as one: a lower generator score means a
/// more fundamental gap, which this reads as worth more. It is not a learned
/// prediction and nothing downstream may claim it is — the census's own ceiling
/// (a frozen corpus cannot predict the future) applies here too.
const MAX_PREDICTED_UNITS: i32 = 1_000;

/// The most a single item costs to pursue, in integer ten-thousandths.
///
/// A more fundamental gap costs more to close, which is the same ordering as
/// the prediction and for a different reason: the first probe of a core
/// question is expensive, and the twelfth is not.
const MAX_COST_UNITS: i32 = 500;

/// The predicted value of a candidate, in integer ten-thousandths.
///
/// **A declared approximation.** A lower generator score means a more
/// fundamental gap, so the value decreases with the score. The far end lands
/// *below* the kill floor on purpose: an item nothing is predicted to gain from
/// is exactly what the exploration quota is for, and if the mapping never
/// produced one the quota would be a bound on nothing.
fn predicted_units(candidate: &GapCandidate) -> i32 {
    let score = candidate.score.clamp(0, MAX_GENERATOR_SCORE);
    let step = (MAX_PREDICTED_UNITS - KILL_FLOOR_UNITS + 100) / (MAX_GENERATOR_SCORE as i32 + 1);
    (MAX_PREDICTED_UNITS - score as i32 * step).max(KILL_FLOOR_UNITS / 2)
}

/// The cost of pursuing a candidate, in integer ten-thousandths.
///
/// Same ordering, same declared-approximation status, and a DIFFERENT number from
/// the prediction. Keeping them apart is what makes the spend ceiling a budget
/// rather than a forecast — see [`QueueItem::cost_units`].
fn cost_units(candidate: &GapCandidate) -> i32 {
    let score = candidate.score.clamp(0, MAX_GENERATOR_SCORE);
    let step = MAX_COST_UNITS / (MAX_GENERATOR_SCORE as i32 + 1);
    (MAX_COST_UNITS - score as i32 * step).max(1)
}

/// The queue's verdict, as a reader OUTSIDE the crate sees it.
///
/// Least privilege is the control: a consumer of the queue learns whether the
/// queue admitted anything and nothing else. It cannot reach an item, a
/// predicate, a domain, or a predicted value — so a queue exposed on a route
/// could not become a read path into claim content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueProbe {
    verdict: &'static str,
    admitted: usize,
}

impl QueueProbe {
    /// The closed verdict name.
    pub fn verdict_str(&self) -> &'static str {
        self.verdict
    }

    /// How many items were admitted. A count, never the items.
    pub fn admitted(&self) -> usize {
        self.admitted
    }
}

/// Build a probe of the real queue over a real flood.
///
/// The one function in this module reachable from outside the crate, and it
/// returns a [`QueueProbe`] rather than the queue — the same posture
/// `service::create::recall_page` takes, where the reader cannot ask for
/// material it should not see.
pub fn probe(flood: &[GapCandidate]) -> QueueProbe {
    let q = build(flood.to_vec());
    QueueProbe {
        verdict: q.verdict.as_str(),
        admitted: q.items.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, score: i64) -> GapCandidate {
        GapCandidate {
            gap_id: id.into(),
            domain: "acme".into(),
            subject: "acme".into(),
            predicate: format!("p_{id}"),
            method: GapMethod::DrillInduced,
            score,
        }
    }

    fn flood(specs: &[(&str, i64)]) -> Vec<GapCandidate> {
        specs.iter().map(|(id, s)| candidate(id, *s)).collect()
    }

    fn distinct(n: usize, score: i64) -> Vec<GapCandidate> {
        (0..n)
            .map(|i| {
                let mut c = candidate(&format!("g{i}"), score);
                c.gap_id = format!("g{i}");
                c
            })
            .collect()
    }

    /// The bounds are a *policy*, and the policy is only meaningful if a real
    /// flood can actually satisfy it. The generator produces scores `0..5` from
    /// the drill set and `0..2` from the implication axioms, so this asserts
    /// the real distribution rather than a hand-picked one — a mapping tuned
    /// until a fixture passes is a mapping tuned to the fixture.
    #[test]
    fn the_real_flood_fits_inside_the_preregistered_bounds() {
        let flood = crate::workflow::create::gap::generate(
            "acme",
            &crate::workflow::create::gap::DomainState {
                covered: Vec::new(),
            },
            None,
        );
        let q = build(flood);
        assert_eq!(
            q.verdict,
            QueueVerdict::Admitted(q.items.len()),
            "a real generator flood must be admissible under the preregistered bounds. If the \
             mapping or the ceiling is wrong, the queue refuses every population and the bound \
             bounds nothing."
        );
        assert!(
            q.cost_total_units() <= SPEND_CEILING_UNITS,
            "the admitted population cost {} units, over the ceiling of {SPEND_CEILING_UNITS}",
            q.cost_total_units()
        );
        assert!(
            q.exploratory_count() * 4 <= q.items.len().max(1) * EXPLORATION_QUOTA,
            "the exploratory items must stay inside the one-in-four quota: {} of {} items are \
             exploratory, quota is {EXPLORATION_QUOTA} in four",
            q.exploratory_count(),
            q.items.len()
        );
        assert!(!q.is_empty(), "a real flood must produce a drainable queue");
    }

    #[test]
    fn the_bounds_are_the_preregistered_ones() {
        // P57.4 committed these before the queue existed. Changing one is an
        // amendment that must be disclosed, never an edit.
        assert_eq!(EXPLORATION_QUOTA, 1, "one in four may be exploratory");
        assert_eq!(SPEND_CEILING_UNITS, 5_000, "the population spend ceiling");
        assert_eq!(
            KILL_FLOOR_UNITS, GLOBAL_TOLERANCE_UNITS,
            "the kill condition is measured against the census's OWN tolerance. A second \
             instrument with its own number would be a second scale nobody can compare."
        );
    }

    #[test]
    fn a_queue_nobody_can_rank_is_still_ranked_deterministically() {
        let q = build(flood(&[("c", 2), ("a", 0), ("b", 1)]));
        assert_eq!(q.verdict, QueueVerdict::Admitted(3));
        let ids: Vec<&str> = q.items.iter().map(|i| i.gap_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"], "rank order, and total on ties");
        assert_eq!(build(flood(&[("c", 2), ("a", 0), ("b", 1)])), q);
    }

    #[test]
    fn a_queue_that_predicts_no_win_is_killed_and_drains_nothing() {
        // Every candidate past the generator's score range maps to the minimum
        // predicted value, which is at or below the floor.
        let q = build(flood(&[("a", 99), ("b", 99)]));
        assert_eq!(q.verdict, QueueVerdict::Killed);
        assert!(
            q.is_empty(),
            "a killed queue drains nothing. A queue that keeps spending on items it expects \
             to fail is a queue draining attention for nothing."
        );
    }

    #[test]
    fn the_kill_condition_is_checked_before_the_budget() {
        // A population that is BOTH over the spend ceiling AND predicts no win
        // must report the kill. Reporting "over budget" would tell an operator
        // their queue is oversized when the truth is that it is not worth
        // draining — a different problem with a different fix.
        let q = build(distinct(40, 99));
        assert_eq!(
            q.verdict,
            QueueVerdict::Killed,
            "the kill condition is the first check, and it is checked first on purpose"
        );
    }

    #[test]
    fn the_spend_ceiling_bounds_cost_and_not_the_forecast() {
        // The distinction is the whole reason the two numbers are separate. Two
        // populations with the SAME COST — one at the cheap end of the score
        // range, one at the expensive end, sized so the totals match — must be
        // admitted identically, and must predict different values. If the
        // ceiling read the forecast instead, the population that promised more
        // would be admitted on the strength of its promise, which is a budget
        // that inverts.
        //
        // The property that matters is not that one population predicts more —
        // count dominates, so three cheap items can out-predict two expensive
        // ones and that is fine. It is that a POPULATION is admitted or refused
        // by what it COSTS, so two populations priced alike are admitted alike
        // no matter how differently they forecast. A ceiling that read the
        // forecast would let a flood buy its way in by promising more.
        let expensive = build(distinct(2, 0));
        let cheap = build(distinct(3, 2));
        assert!(
            (expensive.cost_total_units() - cheap.cost_total_units()).abs() <= 2,
            "the two populations must be priced alike for this to test anything: {} vs {}",
            expensive.cost_total_units(),
            cheap.cost_total_units()
        );
        assert_ne!(
            expensive.predicted_total_units(),
            cheap.predicted_total_units(),
            "the two populations must forecast differently or the ceiling is not being read \
             against the cost"
        );
        assert!(
            matches!(expensive.verdict, QueueVerdict::Admitted(_))
                && matches!(cheap.verdict, QueueVerdict::Admitted(_)),
            "populations priced alike are both admitted. A ceiling that read the forecast would \
             admit the greedier population and refuse the thrifty one, which is a budget that \
             inverts into a reward. Got {:?} and {:?}.",
            expensive.verdict,
            cheap.verdict
        );
        assert_eq!(
            expensive.items.len() + cheap.items.len(),
            5,
            "both populations drain in full; neither was trimmed"
        );
    }

    #[test]
    fn a_population_over_the_spend_ceiling_is_refused_not_truncated() {
        // Distinct ids at the most-fundamental end, so each carries the maximum
        // per-item cost. Enough of them and the population cannot fit.
        let q = build(distinct(MAX_QUEUE_ITEMS + 1, 0));
        assert_eq!(q.verdict, QueueVerdict::SpendCeilingRefused);
        assert!(
            q.is_empty(),
            "a refused queue reports the refusal, not a shorter queue. Truncating would report \
             an admit the operator did not get."
        );
    }

    #[test]
    fn the_exploration_quota_is_enforced_and_refused_not_trimmed() {
        // Eight items: four at the most-fundamental end (max predicted value)
        // and four past the generator's range (mapped to the floor, i.e. no
        // predicted win). `(8/4)*1 = 1` exploratory is allowed, so a SECOND
        // floor-scored item must refuse the whole queue rather than being
        // silently dropped.
        let specs = [
            ("h0", 0i64),
            ("h1", 0),
            ("h2", 0),
            ("h3", 0),
            ("l0", 99),
            ("l1", 99),
            ("l2", 99),
            ("l3", 99),
        ];
        let q = build(flood(&specs));
        assert_eq!(
            q.verdict,
            QueueVerdict::ExplorationQuotaRefused,
            "four floor-scored items against a one-in-four quota must REFUSE. Trimming the \
             surplus would report an admit the operator did not get — the queue would look \
             drained when three items were silently dropped."
        );
        assert!(q.is_empty());
    }

    #[test]
    fn the_queue_cannot_carry_a_status_or_authorize_anything() {
        // The type IS the control. A queue item with a status field would be a
        // second, unreviewed gate — the exact failure `gap::GapCandidate` is
        // built to make unrepresentable.
        let item = QueueItem {
            gap_id: "g".into(),
            domain: "d".into(),
            predicate: "p".into(),
            method: GapMethod::DrillInduced,
            score: 0,
            predicted_units: 1,
            cost_units: 1,
            exploratory: false,
        };
        // Seven fields, and none of them could hold a claim status or a promotion.
        let rendered = format!("{item:?}");
        for forbidden in ["status", "claim_id", "promoted", "ratified", "authoriz"] {
            assert!(
                !rendered.contains(forbidden),
                "the queue item grew a `{forbidden}` field: {rendered}"
            );
        }
        let source = include_str!("queue.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or_default();
        for forbidden in ["fn promote", "fn ratify", "fn authorize", "fn set_status"] {
            assert!(
                !production.contains(forbidden),
                "the queue grew `{forbidden}`. A queue is a ranked pointer; anything that can \
                 authorize is a second gate nobody reads."
            );
        }
    }

    #[test]
    fn an_item_reveals_a_pointer_and_never_claim_content() {
        let q = build(flood(&[("a", 0)]));
        let item = &q.items[0];
        assert_eq!(item.domain, "acme");
        assert!(item.gap_id.contains('a'));
        // No field could carry a claim body, a value, or an evidence quote.
        let fields = std::mem::size_of::<QueueItem>();
        assert!(
            fields <= std::mem::size_of::<String>() * 3 + 80,
            "the queue item grew: {fields} bytes is more than seven identifiers and two small \
             integers need"
        );
    }

    #[test]
    fn the_verdict_vocabulary_is_closed() {
        for v in [
            QueueVerdict::Admitted(0),
            QueueVerdict::SpendCeilingRefused,
            QueueVerdict::ExplorationQuotaRefused,
            QueueVerdict::Killed,
        ] {
            let s = v.as_str();
            assert!(
                s.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "verdict `{s}` is not a plain closed token"
            );
        }
    }

    #[test]
    fn the_queue_reaches_the_real_generator_and_is_bounded() {
        // The real `gap::generate` flood, not a hand-built one: the queue is
        // only honest if it is fed what the loop actually produces.
        let real = crate::workflow::create::gap::generate(
            "acme",
            &crate::workflow::create::gap::DomainState {
                covered: Vec::new(),
            },
            None,
        );
        assert!(!real.is_empty(), "the generator produced no candidates");
        let q = build(real);
        assert!(
            q.items.len() <= MAX_QUEUE_ITEMS,
            "the queue exceeded its bound: {} items",
            q.items.len()
        );
    }

    #[test]
    fn a_covered_predicate_produces_no_item_so_the_queue_shrinks_with_the_corpus() {
        let probes = ["applies_to", "excludes", "supersedes", "evidence_for"];
        let before = build(crate::workflow::create::gap::generate(
            "acme",
            &crate::workflow::create::gap::DomainState {
                covered: Vec::new(),
            },
            None,
        ));
        let after = build(crate::workflow::create::gap::generate(
            "acme",
            &crate::workflow::create::gap::DomainState {
                covered: probes.iter().map(|s| s.to_string()).collect(),
            },
            None,
        ));
        assert!(
            after.items.len() < before.items.len(),
            "covering predicates must SHRINK the queue: {before:?} then {after:?}"
        );
    }

    /// The measured mapping, pinned against the generator's REAL output.
    ///
    /// The predicted-value and cost mappings were derived from a measurement of
    /// what `gap::generate` actually emits (9 candidates: 6 drill-induced at
    /// scores `0..5`, 3 implied-fact at scores `0..2`), not from a fixture
    /// chosen to make a test pass. This pin holds that derivation: a generator
    /// whose score range moves would silently re-price the whole queue, and a
    /// queue re-priced by a generator change is a budget nobody re-approved.
    #[test]
    fn the_mapping_is_anchored_to_the_generator_it_prices() {
        let flood = crate::workflow::create::gap::generate(
            "acme",
            &crate::workflow::create::gap::DomainState {
                covered: Vec::new(),
            },
            None,
        );
        assert!(
            !flood.is_empty(),
            "the generator produced nothing; the mapping is anchored to a real distribution and \
             must be re-derived if the generator changes"
        );
        for c in &flood {
            assert!(
                (0..=MAX_GENERATOR_SCORE).contains(&c.score),
                "the generator emitted score {} for {}, outside the range the value and cost \
                 mappings were derived over (0..={MAX_GENERATOR_SCORE}). Re-measure both mappings \
                 and re-state P57.4's bounds — do not widen the clamp silently.",
                c.score,
                c.gap_id
            );
            assert!(
                predicted_units(c) > 0 && cost_units(c) > 0,
                "a real candidate priced at zero would be indistinguishable from an item the \
                 mapping could not score"
            );
        }
    }
}
