//! Retrieval-quality metrics.
//!
//! Pure metric functions for the regression bench harness: precision@k, recall@k,
//! MRR, NDCG, plus the paper's `answer_in_context` diagnostic. No I/O, no
//! deps — the smallest checks that fail if a metric is computed wrong.
//!
//! The harness loads a judgments file (`BRAIN_EVAL_JUDGMENTS`, JSON: a list of
//! `{query, relevant_ids: [int], gold_answer?: string}`) and runs each query
//! through `/recall`, then computes the metrics over the ranked results. The
//! 100-query hand-judged corpus against the live DB is an operator step — these
//! functions are the reproducible engine any corpus plugs into.
//!
//! # The joint objective: safety as a feasibility constraint
//!
//! [`EvalReport`] above optimises **accuracy only**, and the floor enforcement
//! in the CLI consumes six accuracy scalars with no way to express a trade-off.
//! The joint objective below implements the preregistered shape:
//!
//! ```text
//! maximize   ( accuracy , local_inference_cost )
//! subject to  safety_violations == 0
//! ```
//!
//! Safety is the **feasible region**, not a term in the objective, and there
//! are **no weights** — a weighted blend would permit trading safety for
//! accuracy, which is the trade this shape exists to make unavailable. The
//! non-tradeability is structural rather than documented: [`SafetyEvidence`]
//! has a private field and one constructor that *refuses* a non-zero violation
//! count, and the accept-variant [`JointVerdict::Accepted`] carries a
//! [`FeasibleCase`] that cannot be built without one. There is no path from a
//! violated observation to an acceptance.
//!
//! ## What this module does NOT measure, stated plainly
//!
//! **The cost term is a proxy, and it is a caller-supplied number.** Measured
//! at HEAD `e13e830e`: the eval harness has **no per-case token accounting and
//! no wall-clock instrumentation at all** — `grep -n "Instant::now\|elapsed()"`
//! over `src/bin/brain.rs` returns nothing, and `Usage` (`pub(crate)` at
//! `src/agentloop/provider.rs:162`) never reaches an eval. This module
//! therefore **does not source a token count**; [`LocalCost`] is an explicit
//! input parameter and **the provenance of that number is the caller's
//! responsibility**. A proxy labelled as a proxy is the deliverable; a proxy
//! labelled as a measurement is a failure.
//!
//! The plan's own ceiling concedes the proxy is unsound: it counts tokens, it
//! does not price a 2M local model against a frontier model. On a local-first
//! posture those are not the same thing, and the objective will under-value the
//! local tier. **Flagged, not solved.**
//!
//! **The safety term measures gate agreement, not truth.** A system that agrees
//! with its own gates and is systematically wrong scores perfectly. Nothing here
//! can detect a wrong gate.
//!
//! **Nothing in this section is wired to a gate yet.** The CLI floor check at
//! `src/bin/brain.rs` still enforces the six accuracy scalars and has no cost
//! or safety term; the CI lane still passes three accuracy floors only. Wiring
//! both is a follow-up this round's write scope excluded, and until it lands the
//! joint objective is a library with no production caller.

#![deny(unsafe_code)]

/// Precision@k: fraction of the top-k retrieved ids that are relevant.
/// Empty relevant set ⇒ 0 (no judgment means we can't credit a hit).
pub fn precision_at_k(retrieved: &[i64], relevant: &[i64], k: usize) -> f32 {
    if relevant.is_empty() || k == 0 {
        return 0.0;
    }
    let topk = retrieved.iter().take(k);
    let hits = topk.filter(|r| relevant.contains(r)).count();
    hits as f32 / k as f32
}

/// Recall@k: fraction of relevant ids appearing in the top-k retrieved.
pub fn recall_at_k(retrieved: &[i64], relevant: &[i64], k: usize) -> f32 {
    if relevant.is_empty() {
        return 0.0;
    }
    let topk: Vec<&i64> = retrieved.iter().take(k).collect();
    let hits = relevant.iter().filter(|r| topk.contains(r)).count();
    hits as f32 / relevant.len() as f32
}

/// Mean Reciprocal Rank: 1/rank of the first relevant result (0 if none in list).
pub fn mrr(retrieved: &[i64], relevant: &[i64]) -> f32 {
    if relevant.is_empty() {
        return 0.0;
    }
    for (i, r) in retrieved.iter().enumerate() {
        if relevant.contains(r) {
            return 1.0 / (i as f32 + 1.0);
        }
    }
    0.0
}

/// Normalized Discounted Cumulative Gain. Binary relevance (relevant=1 else 0),
/// ideal DCG = sort by relevance desc (all relevant first). NDCG ∈ [0,1].
pub fn ndcg(retrieved: &[i64], relevant: &[i64], k: usize) -> f32 {
    if relevant.is_empty() || k == 0 {
        return 0.0;
    }
    let dcg: f32 = retrieved
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, r)| {
            let rel = if relevant.contains(r) { 1.0 } else { 0.0 };
            rel / (i as f32 + 2.0).log2()
        })
        .sum();
    // Ideal: all relevant items ranked first.
    let ideal_hits = relevant.len().min(k);
    let idcg: f32 = (0..ideal_hits).map(|i| 1.0 / (i as f32 + 2.0).log2()).sum();
    if idcg == 0.0 { 0.0 } else { dcg / idcg }
}

/// Aggregate metrics over a set of queries.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EvalReport {
    pub queries: usize,
    pub precision_at_5: f32,
    pub recall_at_5: f32,
    pub mrr: f32,
    pub ndcg_at_5: f32,
    /// Fraction of queries whose gold answer survived into the packed context.
    pub answer_in_context_rate: f32,
}

/// One query's judgment + (optional) gold answer.
#[derive(Debug, Clone, Deserialize)]
pub struct Judgment {
    pub query: String,
    pub relevant_ids: Vec<i64>,
    #[serde(default)]
    pub gold_answer: Option<String>,
}

use serde::{Deserialize, Serialize};

/// Compute the aggregate report from per-query (retrieved_ids, gold_survived)
/// pairs. `k` defaults to 5 for P/R/NDCG (the recall default limit).
pub fn evaluate(judged: &[(Judgment, Vec<i64>, Option<bool>)], k: usize) -> EvalReport {
    if judged.is_empty() {
        return EvalReport::default();
    }
    let n = judged.len() as f32;
    let mut p = 0.0;
    let mut r = 0.0;
    let mut m = 0.0;
    let mut nd = 0.0;
    let mut aic_sum = 0.0;
    let mut aic_n = 0.0;
    for (j, retrieved, aic) in judged {
        p += precision_at_k(retrieved, &j.relevant_ids, k);
        r += recall_at_k(retrieved, &j.relevant_ids, k);
        m += mrr(retrieved, &j.relevant_ids);
        nd += ndcg(retrieved, &j.relevant_ids, k);
        if let Some(b) = aic {
            aic_sum += if *b { 1.0 } else { 0.0 };
            aic_n += 1.0;
        }
    }
    EvalReport {
        queries: judged.len(),
        precision_at_5: p / n,
        recall_at_5: r / n,
        mrr: m / n,
        ndcg_at_5: nd / n,
        answer_in_context_rate: if aic_n > 0.0 { aic_sum / aic_n } else { 0.0 },
    }
}

// ---------------------------------------------------------------------------
// The joint evaluation objective: safety as a feasibility constraint, not a term.
//
// Safety is a CONSTRAINT, not a term. The whole shape exists to make one trade
// unavailable: safety-for-accuracy. Three mechanisms carry that, and each is
// pinned from `tests/eval_joint_objective_pins.rs`:
//
//   1. `SafetyEvidence` is unforgeable — private field, and the sole
//      constructor returns `Result<_, SafetyViolation>`, refusing any non-zero
//      count. There is no way to name "zero violations" for a case that has one.
//   2. `JointVerdict::Accepted` carries a `FeasibleCase`, and a `FeasibleCase`
//      cannot be built without a `SafetyEvidence`. Acceptance is therefore
//      UNREPRESENTABLE for a violated case, rather than rejected at runtime.
//   3. `admit` checks safety FIRST and returns immediately. No amount of
//      accuracy or cost is examined on a violated case, so there is no
//      arithmetic anywhere that could net a violation against a gain.
// ---------------------------------------------------------------------------

/// Local inference cost as a **countable** integer — not a float, because
/// tokens are countable and a float cost would make two equal budgets compare
/// unequal.
///
/// **This is a proxy and it is not measured here.** The eval harness has no
/// per-case token accounting and no wall-clock instrumentation; `Usage` is
/// `pub(crate)` and never reaches an eval. `LocalCost::new` therefore takes a
/// caller-supplied count, and the caller owns its provenance. See the module
/// doc: the proxy does not price a local model against a frontier model, and
/// **will under-value the local tier**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct LocalCost(u32);

impl LocalCost {
    /// Zero cost — the value for "not measured", which is deliberately not the
    /// same claim as "measured and free". A caller with no measurement should
    /// say so rather than pass this.
    pub const ZERO: Self = Self(0);
    /// The saturation point of the count (4_294_967_295).
    pub const MAX: Self = Self(u32::MAX);

    /// Wrap a caller-supplied token count.
    pub const fn new(tokens: u32) -> Self {
        Self(tokens)
    }

    /// The raw count.
    pub const fn tokens(self) -> u32 {
        self.0
    }

    /// Accumulate a per-case count, saturating rather than wrapping.
    ///
    /// Saturation is verdict-safe, which is the only property that matters: a
    /// saturated cost is [`Self::MAX`], and no representable ceiling exceeds
    /// [`Self::MAX`], so an overflow can never convert a cost breach into an
    /// acceptance. (The one value it could mis-report is a ceiling of exactly
    /// `u32::MAX`, which is not a token budget anyone holds.)
    pub fn plus(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

/// A non-zero count of behavioural-safety violations: the fraction of eval
/// cases where the system took an action its own gates forbid.
///
/// This is a REJECTION, so its count is public — reading why a case was
/// refused is the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SafetyViolation {
    /// How many violations were observed. Always `> 0`.
    pub violations: u32,
}

/// Proof that a case's safety count was zero.
///
/// **The field is private and the only constructor refuses a non-zero count.**
/// That combination is what makes the constraint structural: a caller holding
/// an observation with violations cannot obtain one of these, so it cannot
/// build a [`FeasibleCase`], so it cannot build [`JointVerdict::Accepted`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SafetyEvidence {
    /// Retained only so an acceptance receipt can name the count it observed.
    /// Always `0`; not readable from outside this module.
    observed_violations: u32,
}

impl SafetyEvidence {
    /// The sole constructor. Zero violations yields evidence; anything else
    /// yields the violation, and no other path to an evidence value exists.
    pub fn cleared(safety_violations: u32) -> Result<Self, SafetyViolation> {
        if safety_violations == 0 {
            Ok(Self {
                observed_violations: 0,
            })
        } else {
            Err(SafetyViolation {
                violations: safety_violations,
            })
        }
    }

    /// The observed count. Always `0` — that is the invariant.
    pub const fn observed_violations(self) -> u32 {
        self.observed_violations
    }
}

/// A case proven to be **inside the feasible region**: safety has cleared, and
/// the two objective terms are present.
///
/// Fields are private so this cannot be built by struct literal or struct
/// update from outside the module; [`FeasibleCase::new`] is the accept-
/// constructor, and its signature is bound by a `fn`-pointer pin so widening it
/// to accept a raw count is a compile error.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeasibleCase {
    accuracy: f32,
    cost: LocalCost,
    safety: SafetyEvidence,
}

impl FeasibleCase {
    /// The accept-constructor. Requires [`SafetyEvidence`], which requires a
    /// zero count.
    pub const fn new(accuracy: f32, cost: LocalCost, safety: SafetyEvidence) -> Self {
        Self {
            accuracy,
            cost,
            safety,
        }
    }

    pub const fn accuracy(self) -> f32 {
        self.accuracy
    }

    pub const fn cost(self) -> LocalCost {
        self.cost
    }

    pub const fn safety(self) -> SafetyEvidence {
        self.safety
    }
}

/// What a caller actually measured for one evaluation case: the two objective
/// terms plus the constraint that bounds the feasible region.
///
/// `accuracy` is expected in `[0,1]` like every other metric in this module and
/// is **not validated here**. Clamping or refusing an out-of-range accuracy
/// would be an objective term the decision does not preregister, so this is declared as a
/// non-defence instead. A `NaN` accuracy still cannot clear a floor (see
/// [`admit`]), so the failure direction is closed even though the range is not.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JointObjective {
    /// The accuracy term, maximised.
    pub accuracy: f32,
    /// The constraint. Zero is the boundary of the feasible set — not "low",
    /// zero.
    pub safety_violations: u32,
    /// The local-inference-cost term, minimised. A caller-supplied proxy.
    pub cost: LocalCost,
}

impl JointObjective {
    pub const fn new(accuracy: f32, safety_violations: u32, cost: LocalCost) -> Self {
        Self {
            accuracy,
            safety_violations,
            cost,
        }
    }
}

/// The preregistered floor: the accuracy that must be cleared and the cost that
/// must not be exceeded.
///
/// There are **no weights** anywhere in this module. A weighted blend is the
/// weighted shape the decision rejects, because a weight on safety is a licence to trade it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JointFloor {
    /// Accuracy must be at least this. Raising it tightens the region.
    pub accuracy: f32,
    /// Cost must be at most this. Lowering it tightens the region.
    pub cost: LocalCost,
}

impl JointFloor {
    pub const fn new(accuracy: f32, cost: LocalCost) -> Self {
        Self { accuracy, cost }
    }

    /// The only way to move a floor, and it can only **tighten** one.
    ///
    /// The two terms tighten in opposite directions, so "upward" is not a single
    /// arithmetic operation: the accuracy floor takes the element-wise
    /// **maximum** (a higher bar) and the cost ceiling takes the element-wise
    /// **minimum** (a smaller allowance). The invariant both halves serve is the
    /// one that matters — **the resulting feasible region is a subset of this
    /// floor's** — so loosening is not representable through this API
    /// (`I53.4`: floors move upward only).
    pub fn raised_to(self, other: &Self) -> Self {
        Self {
            accuracy: self.accuracy.max(other.accuracy),
            cost: LocalCost(self.cost.tokens().min(other.cost.tokens())),
        }
    }
}

/// Which preregistered term the case failed to clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloorBreach {
    /// Accuracy below the floor. `arXiv:2609.31430` shows compression costs
    /// accuracy when the budget is not binding — the accuracy floor is the
    /// control that stops cost pressure buying retrieval quality.
    AccuracyBelowFloor,
    /// Local inference cost above the ceiling. This is the rejection that
    /// makes the objective joint rather than accuracy-only.
    CostAboveCeiling,
}

/// The verdict. A closed enum, not a bool: the previous shape returned a bare
/// `held: bool`, which reports *that* a gate failed and never *why*, and
/// cannot name a reason that did not exist yet.
///
/// Exhaustive `match` without a wildcard arm is the intended consumption, so a
/// future variant becomes a compile error at the call site rather than a
/// silently-ignored case.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JointVerdict {
    /// Accepted — and it carries the [`FeasibleCase`] that proves why, so the
    /// variant cannot be written by hand.
    Accepted(FeasibleCase),
    /// Refused for a safety violation. **Not tradeable against anything**:
    /// `admit` returns this before examining accuracy or cost, so no figure
    /// elsewhere in the call can offset it.
    RejectedSafetyViolation(SafetyViolation),
    /// Refused for failing the preregistered joint floor.
    RejectedFloor {
        /// The observation as measured, so the receipt names the numbers.
        observed: JointObjective,
        /// Which term was breached.
        breach: FloorBreach,
    },
}

/// Admit or refuse one observation against the preregistered joint floor.
///
/// **Order is the design.** Safety is tested first and returns immediately, so
/// on a violated case no accuracy and no cost figure is ever compared against
/// anything. There is no sum, no product, and no weight that could net a
/// violation against a gain — the trade is absent from the arithmetic, not
/// merely discouraged by it.
///
/// Floor comparisons use `>=` / `>` to match the pre-existing CLI convention
/// (`src/bin/brain.rs`). `NaN` accuracy compares false against `>=`, so it
/// lands in [`FloorBreach::AccuracyBelowFloor`] rather than sneaking through.
pub fn admit(observed: &JointObjective, floor: &JointFloor) -> JointVerdict {
    // Constraint first. No other term is read on this path.
    let safety = match SafetyEvidence::cleared(observed.safety_violations) {
        Err(violation) => return JointVerdict::RejectedSafetyViolation(violation),
        Ok(evidence) => evidence,
    };

    // Inside the feasible region. From here acceptance is representable.
    let feasible = FeasibleCase::new(observed.accuracy, observed.cost, safety);

    // The negation of `>=` is deliberate and load-bearing, not a slip: a `NaN`
    // accuracy compares false against `<`, so the readable `accuracy < floor`
    // form would let a `NaN` measurement CLEAR the floor. `!(x >= y)` rejects
    // it. That is why this carries a local allow instead of the tidier operator
    // — see the `JointObjective::accuracy` doc, which records that the RANGE is
    // not validated even though the failure direction is closed.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(feasible.accuracy() >= floor.accuracy) {
        return JointVerdict::RejectedFloor {
            observed: *observed,
            breach: FloorBreach::AccuracyBelowFloor,
        };
    }
    if feasible.cost() > floor.cost {
        return JointVerdict::RejectedFloor {
            observed: *observed,
            breach: FloorBreach::CostAboveCeiling,
        };
    }
    JointVerdict::Accepted(feasible)
}

/// The **previous accuracy-only objective**, reproduced so a test can put the two side by
/// side.
///
/// It returns a bare `bool` on purpose: that is the old shape, and it is why
/// the two objectives are distinguishable at all. It takes no safety argument
/// and no cost argument, so no trade is even expressible in it — which is the
/// same absence the by-absence pin asserts about [`EvalReport`], stated as a signature.
///
/// **This is a comparison instrument, not a gate.** No production path calls it.
pub fn accuracy_only_held(accuracy: f32, floor_accuracy: f32) -> bool {
    accuracy >= floor_accuracy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_at_k_hand_computed() {
        // retrieved [1,2,3,4,5], relevant {2,4}, k=5 → 2/5
        let p = precision_at_k(&[1, 2, 3, 4, 5], &[2, 4], 5);
        assert!((p - 0.4).abs() < 1e-6);
    }

    #[test]
    fn recall_at_k_hand_computed() {
        // retrieved [1,2,3], relevant {2,4,6}, k=3 → 1/3
        let r = recall_at_k(&[1, 2, 3], &[2, 4, 6], 3);
        assert!((r - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn mrr_first_position() {
        assert!((mrr(&[1, 2], &[1]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mrr_third_position() {
        assert!((mrr(&[4, 5, 1], &[1]) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn mrr_no_relevant() {
        assert_eq!(mrr(&[1, 2], &[99]), 0.0);
    }

    #[test]
    fn ndcg_perfect_ranking_is_one() {
        // Both relevant items ranked first → NDCG = 1.0
        let n = ndcg(&[1, 2, 3], &[1, 2], 5);
        assert!((n - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ndcg_worst_ranking_is_zero() {
        // No relevant in top-k → NDCG = 0
        let n = ndcg(&[7, 8, 9], &[1, 2], 3);
        assert!((n - 0.0).abs() < 1e-6);
    }

    #[test]
    fn empty_relevant_returns_zero() {
        assert_eq!(precision_at_k(&[1], &[], 1), 0.0);
        assert_eq!(recall_at_k(&[1], &[], 1), 0.0);
        assert_eq!(mrr(&[1], &[]), 0.0);
        assert_eq!(ndcg(&[1], &[], 1), 0.0);
    }

    #[test]
    fn evaluate_aggregates_across_queries() {
        let judged = vec![
            (
                Judgment {
                    query: "q1".into(),
                    relevant_ids: vec![1, 2],
                    gold_answer: Some("answer".into()),
                },
                vec![1, 3],
                Some(true),
            ),
            (
                Judgment {
                    query: "q2".into(),
                    relevant_ids: vec![5],
                    gold_answer: None,
                },
                vec![5, 6],
                None,
            ),
        ];
        let rep = evaluate(&judged, 5);
        assert_eq!(rep.queries, 2);
        assert!((rep.answer_in_context_rate - 1.0).abs() < 1e-6); // 1/1 judged
    }

    /// the regression-harness metric functions produce the
    /// hand-computed values (the smallest check that fails if a metric breaks).
    /// (Relocated verbatim from main.rs's tests block, Spire v1.28.54 —
    /// the pin travels with its subjects, the metric fns.)
    #[test]
    fn eval_metrics_compute_correctly() {
        assert!((precision_at_k(&[1, 2, 3, 4, 5], &[2, 4], 5) - 0.4).abs() < 1e-6);
        assert!((recall_at_k(&[1, 2, 3], &[2, 4, 6], 3) - 1.0 / 3.0).abs() < 1e-6);
        assert!((mrr(&[4, 5, 1], &[1]) - 1.0 / 3.0).abs() < 1e-6);
        assert!((ndcg(&[1, 2, 3], &[1, 2], 5) - 1.0).abs() < 1e-6);
    }

    // -----------------------------------------------------------------------
    // Is the objective WIRED? — the question the whole section above exists to
    // answer, asked from the module that shipped without a caller.
    //
    // These two pins live here, not in `tests/`, for one reason: they must
    // COMPILE against the tree that had no caller. A behavioural pin in the
    // CLI's own test module disappears with the code it pins, so against the
    // pre-wiring tree it would simply not run — a green suite proving nothing.
    // Reading the CLI's source from here means the pre-wiring tree gets a
    // runtime FAILED instead.
    // -----------------------------------------------------------------------

    fn cli_source() -> String {
        let path = format!("{}/src/bin/brain.rs", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading brain.rs: {e}"))
    }

    /// The body of `run_eval`, cut at the next top-level `fn` so a mention
    /// elsewhere in the file — a test, a doc comment — cannot satisfy it.
    fn run_eval_body(src: &str) -> String {
        let start = src
            .find("fn run_eval(")
            .unwrap_or_else(|| panic!("run_eval must exist in src/bin/brain.rs"));
        let rest = &src[start..];
        let end = rest[1..].find("\nfn ").map_or(rest.len(), |i| i + 1);
        rest[..end].to_owned()
    }

    /// The non-comment lines of [`run_eval_body`] that are not a print.
    ///
    /// Both filters are load-bearing and both are here because of a pin that
    /// could be fooled: an earlier version of the check below grepped for one
    /// hand-chosen literal (`mean[i] < *floor`), and planting the same bare
    /// comparison on a different index sailed straight through it. The rule is
    /// now a SHAPE — *no line may compare a floor without going through
    /// `admit_floor`* — so it holds for any spelling, index or variable name.
    /// Comment lines are dropped so the prose describing the comparison cannot
    /// trip it, print lines are dropped because `FLOOR BREACH: … >= …` is a
    /// receipt rather than a decision, and the signature line is dropped because
    /// `-> Result<…>` is an arrow, not a comparison.
    fn run_eval_decision_lines(body: &str) -> Vec<String> {
        body.lines()
            .map(|l| l.split("//").next().unwrap_or(l).trim().to_owned())
            .filter(|l| !l.is_empty() && !l.contains("println!") && !l.contains("->"))
            .collect()
    }

    fn compares_a_floor(line: &str) -> bool {
        line.contains("floor") && ["<", ">", "<=", ">="].iter().any(|op| line.contains(op))
    }

    /// `brain eval` must route its floor decision through `eval::admit`, and
    /// must render every verdict it is given — not compare accuracy to a number
    /// and call that a gate.
    ///
    /// The negative half is the load-bearing one: the pre-wiring tree printed
    /// `FLOOR BREACH` / `floor ok` off a bare `mean[i] < *floor` comparison,
    /// which is precisely the shape that cannot refuse a safety violation
    /// because it never looks at one.
    #[test]
    fn eval_joint_the_eval_cli_admits_through_the_joint_objective() {
        let src = cli_source();
        let body = run_eval_body(&src);
        assert!(
            body.contains("judge_admission(") && body.contains("admission.held"),
            "run_eval must take its answer from judge_admission. A gate that decides for itself \
             is a gate whose exit code is one edit away from its printing"
        );
        // The decision is a value, so the printer cannot move it. Asserted
        // structurally: `held` may be READ here and never assigned.
        assert!(
            !body.contains("let mut held") && !body.contains("held = false"),
            "run_eval must not assign the gate's answer — it reads `admission.held` and prints. \
             A printer that can flip the exit code is a printer that decides"
        );
        for line in run_eval_decision_lines(&body) {
            assert!(
                !compares_a_floor(&line),
                "run_eval compares a floor outside the joint objective, which is a second floor \
                 law and the one with no safety term: `{line}`"
            );
        }
        // Every outcome is rendered: an unrendered refusal is a refusal the
        // operator never sees.
        for variant in [
            "FloorOutcome::Held",
            "FloorOutcome::SafetyRefused",
            "FloorOutcome::AccuracyBreached",
            "FloorOutcome::CostBreached",
        ] {
            assert!(
                body.contains(variant),
                "run_eval does not render `{variant}`. A refusal the CLI does not report is a \
                 refusal the CLI does not report at all"
            );
        }
    }

    /// The cost floor must stay **visibly unset**, and the "visibly" is the part
    /// that is pinned: the ceiling is MAX (not zero, which would read as "cost
    /// is free"), the reason names the telemetry it awaits and the
    /// preregistration order, and the gate says so on every run.
    #[test]
    fn eval_joint_the_cost_ceiling_is_left_visibly_unset_and_does_not_default_to_free() {
        let src = cli_source();
        assert!(
            src.contains(
                "const EVAL_COST_CEILING_UNSET: brain_server::eval::LocalCost = \
                 brain_server::eval::LocalCost::MAX;"
            ),
            "the cost ceiling must be a named MAX, not a literal and not a zero. Zero makes the \
             comparison vacuous in a way that reads as 'measured and free'"
        );
        assert!(
            src.contains("R53a") && src.contains("P53.4"),
            "the reason the ceiling is unset must name the telemetry it awaits and the \
             preregistration order that puts that telemetry first. A cost gate attached to an \
             unmeasured term would be a number with no provenance wearing a gate's clothes"
        );
        assert!(
            src.contains("cost term    : UNSET"),
            "every eval run must print that the cost term was not enforced. A gate that refuses \
             loudly and stays silent about what it did NOT check is a gate whose silence reads \
             as coverage"
        );
        assert!(
            src.contains("safety term  :"),
            "every eval run must print the safety term's provenance, so an assumed-clean count is \
             never read as a measurement"
        );
    }
}
