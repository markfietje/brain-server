//! The full ISO 10002/10003 complaint lifecycle, as deterministic policy:
//! the closed state chain (received → … → closed → ADR-referred), the
//! remedy matrix with role-capped approvals that escalate one level over
//! cap, and the KCS capture priority where complaint clusters outrank
//! incident repeaters. Pure functions only — no I/O, no clock.
//!
//! Mantra guardrails encoded here: financial execution never happens in the
//! engine — every remedy is a decision with an approval trail; caps are
//! deterministic per role level × support tier; an unknown role fails
//! CLOSED (no cap is ever guessed).

use crate::pure::qa_score::{FlywheelProposal, repeater_detected};

/// The remedy matrix (ISO 10002 §8.3 classes + goodwill). `ExplanationOnly`
/// carries no financial dimension and is always within any cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemedyKind {
    Repair,
    Replace,
    Refund,
    GoodwillPayment,
    ExplanationOnly,
}

impl RemedyKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "repair" => Some(Self::Repair),
            "replace" => Some(Self::Replace),
            "refund" => Some(Self::Refund),
            "goodwill_payment" | "goodwill" => Some(Self::GoodwillPayment),
            "explanation_only" => Some(Self::ExplanationOnly),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Repair => "repair",
            Self::Replace => "replace",
            Self::Refund => "refund",
            Self::GoodwillPayment => "goodwill_payment",
            Self::ExplanationOnly => "explanation_only",
        }
    }

    /// The legal anchor every proposal cites (the disposition-ranker law).
    /// Repair/replace sit on the conformity guarantee, refund on withdrawal/
    /// price reduction, goodwill explicitly cites POLICY not law, and an
    /// explanation-only response cites the ISO 10002 handling duty.
    pub fn legal_basis(&self) -> &'static str {
        match self {
            Self::Repair | Self::Replace => "2019/771-art.13(2)",
            Self::Refund => "2011/83-art.16",
            Self::GoodwillPayment => "goodwill-policy",
            Self::ExplanationOnly => "iso-10002-clause-9",
        }
    }
}

/// Approval levels, lowest first. Deployment role NAMES map onto this
/// ladder via [`approval_level_for_role`]; anything unrecognized denies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApprovalLevel {
    Agent,
    Supervisor,
    Manager,
    Executive,
}

impl ApprovalLevel {
    /// One rung up the ladder; the top rung escalates to itself only in the
    /// sense that its own cap is effectively unlimited (see [`CAP_TABLE`]).
    pub fn next(self) -> Self {
        match self {
            Self::Agent => Self::Supervisor,
            Self::Supervisor => Self::Manager,
            Self::Manager | Self::Executive => Self::Executive,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Supervisor => "supervisor",
            Self::Manager => "manager",
            Self::Executive => "executive",
        }
    }
}

/// Map a deployment role name to its approval rung. Closed vocabulary;
/// unknown names return `None` and every caller must fail closed.
pub fn approval_level_for_role(role: &str) -> Option<ApprovalLevel> {
    match role.trim().to_ascii_lowercase().as_str() {
        "agent" | "t1" => Some(ApprovalLevel::Agent),
        "supervisor" | "t2" => Some(ApprovalLevel::Supervisor),
        "manager" | "t3" => Some(ApprovalLevel::Manager),
        "executive" | "admin" | "dpo" | "t4" => Some(ApprovalLevel::Executive),
        _ => None,
    }
}

/// Support tier 1..=4 bounds the matrix row: the same role approves more at
/// a higher-tier site (global BPO vs small center), deterministically.
pub const MIN_TIER: u8 = 1;
pub const MAX_TIER: u8 = 4;

/// The deterministic cap table: cents an approval level may bind per
/// remedy at each support tier. Row = level, column = tier. The executive
/// row is unbounded (`i64::MAX`) — someone can always sign, or the packet
/// stalls forever.
pub const CAP_TABLE: [[i64; 4]; 4] = [
    //          T1       T2       T3        T4
    [2_500, 5_000, 7_500, 10_000],       // agent
    [10_000, 20_000, 30_000, 50_000],    // supervisor
    [50_000, 100_000, 150_000, 250_000], // manager
    [i64::MAX; 4],                       // executive
];

fn cap_for(level: ApprovalLevel, tier: u8) -> i64 {
    let col = (tier.clamp(MIN_TIER, MAX_TIER) - 1) as usize;
    CAP_TABLE[level as usize][col]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    WithinCap,
    /// Over cap: escalate exactly one level, packet attached.
    Escalated {
        to: ApprovalLevel,
    },
}

/// The deterministic approval gate. A remedy whose amount sits within the
/// approver's tier cap passes; one cent over escalates ONE level — never
/// two, never zero. Explanation-only remedies carry no money and always
/// pass. An amount below zero is nonsense and denies by escalation to the
/// top (loud, human-reviewed).
pub fn approval_decision(
    level: ApprovalLevel,
    tier: u8,
    kind: RemedyKind,
    amount_cents: i64,
) -> ApprovalDecision {
    if kind == RemedyKind::ExplanationOnly {
        return ApprovalDecision::WithinCap;
    }
    if amount_cents < 0 {
        return ApprovalDecision::Escalated {
            to: ApprovalLevel::Executive,
        };
    }
    if amount_cents > cap_for(level, tier) {
        ApprovalDecision::Escalated { to: level.next() }
    } else {
        ApprovalDecision::WithinCap
    }
}

/// The lifecycle states (ISO 10002 clause 9 flow). All transitions are
/// lineage events; the register IS the audit chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplaintState {
    Received,
    Acknowledged,
    Investigated,
    RemedyProposed,
    RemedyApproved,
    Closed,
    AdrReferred,
}

impl ComplaintState {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "received" => Some(Self::Received),
            "acknowledged" => Some(Self::Acknowledged),
            "investigated" => Some(Self::Investigated),
            "remedy_proposed" => Some(Self::RemedyProposed),
            "remedy_approved" => Some(Self::RemedyApproved),
            "closed" => Some(Self::Closed),
            "adr_referred" => Some(Self::AdrReferred),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Acknowledged => "acknowledged",
            Self::Investigated => "investigated",
            Self::RemedyProposed => "remedy_proposed",
            Self::RemedyApproved => "remedy_approved",
            Self::Closed => "closed",
            Self::AdrReferred => "adr_referred",
        }
    }
}

/// The closed transition table. Strictly forward along the ISO 10002 flow;
/// ADR referral happens from `closed` when unresolved (ISO 10003 handoff).
/// Anything else is denied loudly by the caller.
pub fn transition_allowed(from: ComplaintState, to: ComplaintState) -> bool {
    matches!(
        (from, to),
        (ComplaintState::Received, ComplaintState::Acknowledged)
            | (ComplaintState::Acknowledged, ComplaintState::Investigated)
            | (ComplaintState::Investigated, ComplaintState::RemedyProposed)
            | (
                ComplaintState::RemedyProposed,
                ComplaintState::RemedyApproved
            )
            | (ComplaintState::RemedyApproved, ComplaintState::Closed)
            | (ComplaintState::Closed, ComplaintState::AdrReferred)
    )
}

/// KCS capture priority (Evolve): complaint clusters are the costliest
/// repeats, so they outrank plain incident repeaters; both outrank plain
/// capture. Returns the proposal salience (0.0–1.0) the HITL board sorts
/// on — deterministic, no surveys.
pub fn capture_salience(is_complaint_run: bool, count_same_30d: usize) -> f64 {
    let cluster = is_complaint_run && repeater_detected(count_same_30d);
    if cluster {
        0.9
    } else if is_complaint_run || repeater_detected(count_same_30d) {
        0.7
    } else {
        0.5
    }
}

/// The flywheel proposal for a closing case, extended for complaints: a
/// complaint cluster yields [`FlywheelProposal::Rca`] of the complaint
/// flavor — same HITL pipeline, ranked above incident repeaters by
/// [`capture_salience`].
pub fn flywheel_for_case(
    similarity_units: i32,
    repeater_count: usize,
    is_complaint_run: bool,
) -> Vec<FlywheelProposal> {
    crate::pure::qa_score::flywheel_proposals(similarity_units, repeater_count)
        .into_iter()
        .map(|p| match p {
            FlywheelProposal::Rca if is_complaint_run => FlywheelProposal::ComplaintRca,
            other => other,
        })
        .collect()
}

/// Reg. 2024/3228: the EU ODR platform was discontinued on 20 July 2025.
/// Every external-dispute packet states this basis and targets the
/// competent NATIONAL ADR body instead — the constant the docs cite.
pub const ODR_DISCONTINUATION_BASIS: &str = "reg-2024/3228-odr-discontinued";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_caps_escalate_deterministically() {
        // Within cap passes; one cent over escalates EXACTLY one rung.
        let d = approval_decision(ApprovalLevel::Agent, 1, RemedyKind::Refund, CAP_TABLE[0][0]);
        assert_eq!(d, ApprovalDecision::WithinCap);
        let d = approval_decision(
            ApprovalLevel::Agent,
            1,
            RemedyKind::Refund,
            CAP_TABLE[0][0] + 1,
        );
        assert_eq!(
            d,
            ApprovalDecision::Escalated {
                to: ApprovalLevel::Supervisor
            }
        );
        // The escalation target's cap covers what the lower rung could not.
        let escalated_amount = CAP_TABLE[0][1] + 1;
        assert_eq!(
            approval_decision(
                ApprovalLevel::Supervisor,
                2,
                RemedyKind::GoodwillPayment,
                escalated_amount
            ),
            ApprovalDecision::WithinCap
        );
        // Tier widens the same role's reach, deterministically.
        assert_eq!(
            approval_decision(ApprovalLevel::Agent, 4, RemedyKind::Refund, 10_000),
            ApprovalDecision::WithinCap
        );
        // Explanation-only carries no money — always within any cap.
        assert_eq!(
            approval_decision(
                ApprovalLevel::Agent,
                1,
                RemedyKind::ExplanationOnly,
                i64::MAX
            ),
            ApprovalDecision::WithinCap
        );
        // Negative amounts are nonsense → loud top-level review.
        assert_eq!(
            approval_decision(ApprovalLevel::Manager, 3, RemedyKind::Refund, -1),
            ApprovalDecision::Escalated {
                to: ApprovalLevel::Executive
            }
        );
        // Same input → same output, always.
        for tier in MIN_TIER..=MAX_TIER {
            for amount in [0i64, 2_499, 2_500, 2_501, 49_999, 250_001] {
                for kind in [
                    RemedyKind::Repair,
                    RemedyKind::Replace,
                    RemedyKind::Refund,
                    RemedyKind::GoodwillPayment,
                    RemedyKind::ExplanationOnly,
                ] {
                    let a = approval_decision(ApprovalLevel::Agent, tier, kind, amount);
                    let b = approval_decision(ApprovalLevel::Agent, tier, kind, amount);
                    assert_eq!(a, b);
                }
            }
        }
        // Unknown role names deny: no cap is ever guessed.
        assert_eq!(approval_level_for_role("intern"), None);
        assert_eq!(approval_level_for_role(""), None);
        assert_eq!(
            approval_level_for_role("Admin"),
            Some(ApprovalLevel::Executive)
        );
    }

    #[test]
    fn complaint_lifecycle_is_a_closed_chain() {
        use ComplaintState::*;
        // The happy path walks end to end, one legal step at a time.
        for step in [
            (Received, Acknowledged),
            (Acknowledged, Investigated),
            (Investigated, RemedyProposed),
            (RemedyProposed, RemedyApproved),
            (RemedyApproved, Closed),
            (Closed, AdrReferred),
        ] {
            assert!(transition_allowed(step.0, step.1), "{step:?} must be legal");
        }
        // Skipping, reversing, and self-transitions all deny loudly.
        assert!(!transition_allowed(Received, Closed));
        assert!(!transition_allowed(Received, AdrReferred));
        assert!(!transition_allowed(Closed, Received));
        assert!(!transition_allowed(Investigated, Investigated));
        assert!(!transition_allowed(Acknowledged, RemedyApproved));
        // Every named state round-trips its string.
        for s in [
            Received,
            Acknowledged,
            Investigated,
            RemedyProposed,
            RemedyApproved,
            Closed,
            AdrReferred,
        ] {
            assert_eq!(ComplaintState::parse(s.as_str()), Some(s));
        }
        assert_eq!(ComplaintState::parse("closed_solved"), None);
    }

    #[test]
    fn complaint_clusters_outrank_incident_repeaters_in_capture_priority() {
        // Plain case: base salience.
        assert_eq!(capture_salience(false, 0), 0.5);
        // Incident repeaters rise…
        assert_eq!(capture_salience(false, 3), 0.7);
        // …but a complaint CLUSTER (complaint run × repeat window) rises
        // strictly above them.
        assert_eq!(capture_salience(true, 3), 0.9);
        assert!(capture_salience(true, 3) > capture_salience(false, 3));
        // A single complaint is still above plain capture but below the
        // cluster — clustering, not the class alone, is the priority signal.
        assert_eq!(capture_salience(true, 1), 0.7);
        // The flywheel maps complaint runs' RCA proposals to the dedicated
        // variant feeding the SAME HITL pipeline.
        let p = flywheel_for_case(3_000, 3, true);
        assert!(p.contains(&FlywheelProposal::ComplaintRca));
        let p = flywheel_for_case(3_000, 3, false);
        assert!(p.contains(&FlywheelProposal::Rca));
        // Every remedy kind cites a non-empty legal basis, and the goodwill
        // path cites POLICY, never a regulation (same law as dispositions).
        for k in [
            RemedyKind::Repair,
            RemedyKind::Replace,
            RemedyKind::Refund,
            RemedyKind::GoodwillPayment,
            RemedyKind::ExplanationOnly,
        ] {
            assert!(!k.legal_basis().is_empty());
        }
        assert_eq!(RemedyKind::GoodwillPayment.legal_basis(), "goodwill-policy");
    }
}
