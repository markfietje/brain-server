//! Deterministic disposition ranking for RMA/return runs. The rule engine
//! only RANKS candidates — the human disposes (every disposition is a HITL
//! proposal); fraud signals are deterministic heuristics that inform, never
//! autonomously deny. Every candidate cites its legal basis (regulation
//! article id or contract clause) — the decision trail regulators want.

/// Fraud-review threshold in ten-thousandths: a returnless refund whose
/// composite fraud signal reaches this level MUST carry fraud review.
pub const FRAUD_REVIEW_THRESHOLD_UNITS: i32 = 5_000;

/// The hard cap: at this signal level every disposition escalates to the
/// human team regardless of kind — signals never auto-deny, but they can
/// force escalation.
pub const HARD_ESCALATION_UNITS: i32 = 9_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispositionKind {
    ReturnForInspection,
    ReplaceFirst,
    ReturnlessRefund,
    Deny,
}

impl DispositionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DispositionKind::ReturnForInspection => "return_for_inspection",
            DispositionKind::ReplaceFirst => "replace_first",
            DispositionKind::ReturnlessRefund => "returnless_refund",
            DispositionKind::Deny => "deny",
        }
    }
}

/// Legal anchors. Withdrawal (14-day), warranty (2-year), and goodwill are
/// distinct paths with distinct citations; deny cites the fraud schedule.
pub const BASIS_WARRANTY_REPLACE: &str = "2019/771-art.13(2)";
pub const BASIS_WITHDRAWAL_REFUND: &str = "2011/83-art.16";
pub const BASIS_GOODWILL_REFUND: &str = "goodwill-policy";
pub const BASIS_INSPECTION_CLAUSE: &str = "contract-inspection-clause";
pub const BASIS_FRAUD_SCHEDULE: &str = "fraud-policy-schedule";

/// Deterministic fraud signals over the subject hash's history. Inputs are
/// computed from lineage only (repeat-return rate per subject hash, serial
/// mismatch against the entitlement registry's spine, window abuse).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FraudSignals {
    /// Repeat-return rate for the subject hash, ten-thousandths.
    pub repeat_return_rate_units: i32,
    /// Serial on the claim does not match the registry row.
    pub serial_mismatch: bool,
    /// Claim pattern abuses a legal window (repeated edge-of-window claims).
    pub window_abuse: bool,
}

impl FraudSignals {
    /// Composite score in ten-thousandths: the repeat rate counts half,
    /// a serial mismatch is heavy (identity of the goods is the traceability
    /// spine), window abuse adds the remainder. Clamped to 0..=10000.
    pub fn score(&self) -> i32 {
        let raw = self.repeat_return_rate_units / 2
            + i32::from(self.serial_mismatch) * 3_000
            + i32::from(self.window_abuse) * 2_000;
        raw.clamp(0, 10_000)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DispositionInput {
    /// Item value in cents — drives the value threshold and escalation.
    pub item_value_cents: i64,
    pub value_threshold_cents: i64,
    /// The 14-day withdrawal window is still open on this order.
    pub withdrawal_open: bool,
    /// The 2-year conformity window covers this claim.
    pub warranty_open: bool,
    pub fraud: FraudSignals,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedDisposition {
    pub kind: DispositionKind,
    /// The cited basis: a regulation/article id or contract clause.
    pub basis: &'static str,
    /// Rank in ten-thousandths; higher = the rule engine ranks it first.
    pub rank_units: i32,
    /// Returnless refunds above the fraud threshold carry mandatory review.
    pub requires_fraud_review: bool,
    /// Over-threshold value or near-cap fraud always escalates to a human.
    pub escalated: bool,
}

fn withdrawal_basis(withdrawal_open: bool) -> &'static str {
    if withdrawal_open {
        BASIS_WITHDRAWAL_REFUND
    } else {
        BASIS_GOODWILL_REFUND
    }
}

/// Rank the disposition candidates deterministically: same input → same
/// output ordering (rank descending, ties broken by stable kind name).
pub fn rank_dispositions(input: &DispositionInput) -> Vec<RankedDisposition> {
    let fraud = input.fraud.score();
    let over_threshold = input.item_value_cents > input.value_threshold_cents;
    let escalated = over_threshold || fraud >= HARD_ESCALATION_UNITS;

    let mut out = vec![
        RankedDisposition {
            kind: DispositionKind::ReturnForInspection,
            basis: BASIS_INSPECTION_CLAUSE,
            rank_units: 5_000
                + if input.fraud.serial_mismatch {
                    2_500
                } else {
                    0
                },
            requires_fraud_review: false,
            escalated,
        },
        RankedDisposition {
            kind: DispositionKind::ReplaceFirst,
            basis: if input.warranty_open {
                BASIS_WARRANTY_REPLACE
            } else {
                BASIS_INSPECTION_CLAUSE
            },
            rank_units: if input.warranty_open { 6_000 } else { 3_000 },
            requires_fraud_review: false,
            escalated,
        },
        {
            // A serial mismatch kills the no-inspection path's rank entirely:
            // the goods' identity is unproven, so they must come back.
            let base = if input.withdrawal_open { 4_000 } else { 2_000 };
            RankedDisposition {
                kind: DispositionKind::ReturnlessRefund,
                basis: withdrawal_basis(input.withdrawal_open),
                rank_units: if input.fraud.serial_mismatch {
                    0
                } else if over_threshold {
                    base - 2_000
                } else {
                    base
                },
                requires_fraud_review: fraud >= FRAUD_REVIEW_THRESHOLD_UNITS,
                escalated,
            }
        },
        RankedDisposition {
            kind: DispositionKind::Deny,
            basis: BASIS_FRAUD_SCHEDULE,
            rank_units: if fraud >= FRAUD_REVIEW_THRESHOLD_UNITS {
                fraud
            } else {
                1_000
            },
            requires_fraud_review: false,
            escalated,
        },
    ];
    out.sort_by(|a, b| {
        b.rank_units
            .cmp(&a.rank_units)
            .then(a.kind.as_str().cmp(b.kind.as_str()))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(rate: i32, mismatch: bool, abuse: bool) -> FraudSignals {
        FraudSignals {
            repeat_return_rate_units: rate,
            serial_mismatch: mismatch,
            window_abuse: abuse,
        }
    }

    #[test]
    fn disposition_ranking_is_deterministic_and_cites_basis() {
        let input = DispositionInput {
            item_value_cents: 5_000,
            value_threshold_cents: 10_000,
            withdrawal_open: true,
            warranty_open: true,
            fraud: signals(1_000, false, false),
        };
        let first = rank_dispositions(&input);
        let second = rank_dispositions(&input);
        assert_eq!(first, second, "same input → identical ranking");
        assert_eq!(first.len(), 4, "all four candidates ranked");
        // Every candidate cites its basis; the citation comes from the
        // closed anchor table, never free text.
        for d in &first {
            assert!(
                !d.basis.is_empty(),
                "a disposition without a basis cannot ship"
            );
        }
        // Ordering is rank-descending.
        for pair in first.windows(2) {
            assert!(pair[0].rank_units >= pair[1].rank_units);
        }
        // Warranty open → replace-first leads on the 771 article; goodwill
        // (withdrawal closed) cites a DIFFERENT basis than withdrawal.
        let top = &first[0];
        assert_eq!(top.kind, DispositionKind::ReplaceFirst);
        assert_eq!(top.basis, BASIS_WARRANTY_REPLACE);
        let closed = DispositionInput {
            withdrawal_open: false,
            ..input
        };
        let ranked_closed = rank_dispositions(&closed);
        let r = ranked_closed
            .iter()
            .find(|d| d.kind == DispositionKind::ReturnlessRefund)
            .expect("returnless candidate always present");
        assert_eq!(
            r.basis, BASIS_GOODWILL_REFUND,
            "goodwill path cites policy, not law"
        );
        let ranked_open = rank_dispositions(&input);
        let open = ranked_open
            .iter()
            .find(|d| d.kind == DispositionKind::ReturnlessRefund)
            .expect("present");
        assert_eq!(
            open.basis, BASIS_WITHDRAWAL_REFUND,
            "withdrawal path cites 2011/83"
        );
        // A serial mismatch promotes inspection (identity unproven).
        let mismatched = DispositionInput {
            fraud: signals(1_000, true, false),
            ..input
        };
        let m = rank_dispositions(&mismatched);
        assert_eq!(m[0].kind, DispositionKind::ReturnForInspection);
    }

    #[test]
    fn returnless_refund_requires_fraud_review_over_threshold() {
        let base = DispositionInput {
            item_value_cents: 1_000,
            value_threshold_cents: 10_000,
            withdrawal_open: true,
            warranty_open: false,
            fraud: signals(FRAUD_REVIEW_THRESHOLD_UNITS - 1, false, false),
        };
        let ranked_below = rank_dispositions(&base);
        let below = ranked_below
            .iter()
            .find(|d| d.kind == DispositionKind::ReturnlessRefund)
            .expect("present");
        assert!(
            !below.requires_fraud_review,
            "below threshold: plain proposal"
        );

        // Composite score crosses the threshold via window abuse
        // (rate/2 + abuse×2000 ≥ FRAUD_REVIEW_THRESHOLD_UNITS).
        let at = DispositionInput {
            fraud: signals(7_000, false, true),
            ..base
        };
        let ranked_at = rank_dispositions(&at);
        let at = ranked_at
            .iter()
            .find(|d| d.kind == DispositionKind::ReturnlessRefund)
            .expect("present");
        assert!(
            at.requires_fraud_review,
            "at threshold: fraud review is mandatory"
        );

        // Over-threshold VALUE also forces escalation even with clean fraud.
        let pricey = DispositionInput {
            item_value_cents: 11_000,
            ..base
        };
        let ranked_pricey = rank_dispositions(&pricey);
        let p = ranked_pricey
            .iter()
            .find(|d| d.kind == DispositionKind::ReturnlessRefund)
            .expect("present");
        assert!(
            p.escalated,
            "over-threshold value always escalates to the human"
        );
        assert!(
            p.rank_units < 4_000,
            "expensive items rank toward inspection"
        );

        // At the hard cap EVERY candidate escalates; nothing auto-executes.
        let hot = DispositionInput {
            fraud: signals(10_000, true, true),
            ..base
        };
        for d in rank_dispositions(&hot) {
            assert!(d.escalated, "{:?} must escalate at the cap", d.kind);
        }
        // Signals clamp — a nonsense rate cannot exceed the scale.
        assert_eq!(signals(50_000, true, true).score(), 10_000);
    }
}
