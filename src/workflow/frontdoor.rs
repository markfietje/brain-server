use brain_engine_sdk::policy::{Envelope, Worktype};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentClass {
    Auth,
    StatusCheck,
    PolicyAnswer,
    Booking,
    BoundedDiagnostic,
    /// ISO 10002: a complaint is its own class, not an escalation flavor.
    Complaint,
    /// Aftersales (Directive 2019/771 / withdrawal posture).
    Return,
    WarrantyClaim,
    RepairField,
    /// Care line: an open-ended inquiry dialog.
    CareInquiry,
    AccountChange,
    /// GPSR 2023/988: recall intake is its own safety class.
    SafetyRecall,
    /// Outbound retention (.35) — routed at intake, consent-gated downstream.
    RetentionOutreach,
}

impl IntentClass {
    /// The universal-line thesis, decided at the front door: every intent
    /// class maps to a worktype (= run `kind`) with its own policy rows.
    pub fn worktype(&self) -> Worktype {
        match self {
            IntentClass::BoundedDiagnostic => Worktype::Troubleshoot,
            IntentClass::CareInquiry => Worktype::CareInquiry,
            IntentClass::AccountChange => Worktype::Account,
            IntentClass::Return => Worktype::Return,
            IntentClass::WarrantyClaim => Worktype::WarrantyClaim,
            IntentClass::RepairField => Worktype::RepairField,
            IntentClass::Complaint => Worktype::Complaint,
            IntentClass::SafetyRecall => Worktype::SafetyRecall,
            IntentClass::RetentionOutreach => Worktype::RetentionOutreach,
            // Legacy classes keep their pre-universal routing (no run-kind
            // change for already-shipped flows).
            IntentClass::Auth
            | IntentClass::StatusCheck
            | IntentClass::PolicyAnswer
            | IntentClass::Booking => Worktype::CareInquiry,
        }
    }
}

/// One policy row of the deterministic worktype routing table: the evidence
/// the worktype must gather (tags shared with the engine crates) and the
/// decision gates it must pass, in waterfall order. Pure data — no I/O, no
/// judgment; the gates themselves live with their cores.
pub struct WorktypePolicy {
    pub kind: &'static str,
    pub required_evidence: &'static [&'static str],
    pub gates: &'static [&'static str],
}

/// The routing table itself — every worktype has exactly one row.
pub const WORKTYPE_TABLE: &[(&str, WorktypePolicy)] = &[
    (
        "troubleshoot",
        WorktypePolicy {
            kind: "troubleshoot",
            required_evidence: &["system_event_log", "fault_code"],
            gates: &["evidence", "differential", "verify"],
        },
    ),
    (
        "care_inquiry",
        WorktypePolicy {
            kind: "care_inquiry",
            required_evidence: &["question_payload"],
            gates: &["ambiguity", "draft"],
        },
    ),
    (
        "account",
        WorktypePolicy {
            kind: "account",
            required_evidence: &["identity_proof"],
            gates: &["identity", "confirmation"],
        },
    ),
    (
        "return",
        WorktypePolicy {
            kind: "return",
            required_evidence: &["proof_of_purchase"],
            gates: &["entitlement", "window", "disposition"],
        },
    ),
    (
        "warranty_claim",
        WorktypePolicy {
            kind: "warranty_claim",
            required_evidence: &["proof_of_purchase", "diagnostic_bundle"],
            gates: &["entitlement", "window", "disposition"],
        },
    ),
    (
        "repair_field",
        WorktypePolicy {
            kind: "repair_field",
            required_evidence: &["diagnostic_bundle", "photos"],
            gates: &["entitlement", "dispatch", "verification"],
        },
    ),
    (
        "complaint",
        WorktypePolicy {
            kind: "complaint",
            required_evidence: &["case_record"],
            gates: &["acknowledgment", "remedy"],
        },
    ),
    (
        "safety_recall",
        WorktypePolicy {
            kind: "safety_recall",
            required_evidence: &["serial_batch"],
            gates: &["entitlement", "safety_gate"],
        },
    ),
    (
        "retention_outreach",
        WorktypePolicy {
            kind: "retention_outreach",
            required_evidence: &["consent_proof"],
            gates: &["consent"],
        },
    ),
];

/// Resolve the policy row for a worktype kind string. Unknown kinds deny
/// loudly (`None`) — the table is closed, like the vocabulary it routes.
pub fn worktype_policy(kind: &str) -> Option<&'static WorktypePolicy> {
    WORKTYPE_TABLE
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, p)| p)
}

/// The colleague-board routing row: which HITL-maintained skills tags the
/// right board per class requires (matched by [`crate::workflow::crew`]).
pub fn worktype_skills(kind: &str) -> &'static [&'static str] {
    match kind {
        "troubleshoot" => &["troubleshooting"],
        "care_inquiry" => &["care"],
        "account" => &["account-management"],
        "return" => &["returns"],
        "warranty_claim" => &["returns", "warranty"],
        "repair_field" => &["field-service"],
        "complaint" => &["complaints"],
        "safety_recall" => &["safety", "compliance"],
        "retention_outreach" => &["retention"],
        _ => &[],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision {
    Resolved { class: IntentClass, reason: String },
    Routed { reason: String },
    Escalated { handoff: Handoff },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub conversation: String,
    pub intent: String,
    pub is_seed: String,
    pub is_not_seed: String,
    pub sla_deadline: i64,
    pub plan_so_far: String,
    /// Empty = a plain handover. `dispute` marks an escalation-to-dispute
    /// (ISO 10003 posture): the documented handover reason for complaints.
    pub reason: String,
}

/// The handover reason recorded when a complaint escalates to dispute.
pub const DISPUTE_REASON: &str = "dispute";

fn is_escape(input: &str) -> bool {
    let l = input.to_ascii_lowercase();
    l.contains("talk to a human")
        || l.contains("human agent")
        || l.contains("speak to a person")
        || l.contains("escalate")
        || l == "human"
}

/// The closed safety-vocabulary screen (GPSR 2023/988 posture): hazard
/// phrases that make ANY message safety-relevant regardless of its
/// commercial framing. Substring match on the lowercased input; the
/// vocabulary is data, pinned by tests.
fn safety_vocabulary(l: &str) -> bool {
    [
        "caught fire",
        "injur",
        "unsafe",
        "started smoking",
        "hazard",
    ]
    .iter()
    .any(|k| l.contains(k))
}

fn classify_rules(input: &str) -> Option<(IntentClass, String)> {
    let l = input.to_ascii_lowercase();
    // Safety screen FIRST: a safety-relevant complaint escalates to the
    // GPSR path (safety_recall) before the complaint keyword can claim it —
    // hazard vocabulary outranks every commercial class it shares.
    if l.contains("complain") && safety_vocabulary(&l) {
        return Some((
            IntentClass::SafetyRecall,
            "safety complaint escalated to gpsr path".into(),
        ));
    }
    if l.contains("complain") {
        return Some((IntentClass::Complaint, "complaint keyword matched".into()));
    }
    // Safety intake outranks the commercial classes it shares vocabulary
    // with — a recall mention is a recall, never a policy answer.
    if l.contains("recall") || l.contains("safety notice") || safety_vocabulary(&l) {
        return Some((IntentClass::SafetyRecall, "recall keyword matched".into()));
    }
    if (l.contains("return") || l.contains("send it back") || l.contains("rma"))
        && !l.contains("return policy")
    {
        return Some((IntentClass::Return, "return keyword matched".into()));
    }
    if l.contains("warranty claim") || l.contains("under warranty") || l.contains("defect") {
        return Some((
            IntentClass::WarrantyClaim,
            "warranty-claim keyword matched".into(),
        ));
    }
    if l.contains("technician visit") || l.contains("on-site repair") || l.contains("field repair")
    {
        return Some((
            IntentClass::RepairField,
            "repair-field keyword matched".into(),
        ));
    }
    if l.contains("change my") && (l.contains("address") || l.contains("email")) {
        return Some((
            IntentClass::AccountChange,
            "account-change keyword matched".into(),
        ));
    }
    if l.contains("retention") || l.contains("cancel my subscription") {
        return Some((
            IntentClass::RetentionOutreach,
            "retention keyword matched".into(),
        ));
    }
    if l.contains("2fa") || l.contains("pin reset") || l.contains("authenticate") {
        return Some((IntentClass::Auth, "auth keyword matched".into()));
    }
    if l.contains("order status") || l.contains("status of") || l.contains("where is my") {
        return Some((IntentClass::StatusCheck, "status keyword matched".into()));
    }
    if l.contains("policy") || l.contains("refund policy") || l.contains("warranty") {
        return Some((IntentClass::PolicyAnswer, "policy keyword matched".into()));
    }
    if l.contains("book")
        && (l.contains("appointment") || l.contains("meeting") || l.contains("slot"))
    {
        return Some((IntentClass::Booking, "booking keyword matched".into()));
    }
    if l.contains("diagnose") || l.contains("error code") {
        return Some((
            IntentClass::BoundedDiagnostic,
            "bounded diagnostic keyword matched".into(),
        ));
    }
    None
}

pub fn route(
    input: &str,
    envelope: &Envelope,
    conversation: &str,
    plan_so_far: &str,
) -> RouteDecision {
    if is_escape(input) {
        return RouteDecision::Escalated {
            handoff: Handoff {
                conversation: conversation.to_string(),
                intent: input.to_string(),
                is_seed: plan_so_far.to_string(),
                is_not_seed: String::new(),
                sla_deadline: envelope.sla_deadline,
                plan_so_far: plan_so_far.to_string(),
                reason: String::new(),
            },
        };
    }
    if let Some((class, reason)) = classify_rules(input) {
        return RouteDecision::Resolved { class, reason };
    }
    RouteDecision::Routed {
        reason: "outside closed vocabulary — routed to run".into(),
    }
}

/// A complaint that demands a human escalates as a DISPUTE: the handover
/// carries the complaint envelope (its own ack + response clocks) and the
/// documented `dispute` reason — persisted downstream as an audit row
/// reading `handover/dispute` (see `workflow::relay`).
pub fn escalate_complaint_dispute(
    input: &str,
    envelope: &Envelope,
    conversation: &str,
    plan_so_far: &str,
) -> RouteDecision {
    RouteDecision::Escalated {
        handoff: Handoff {
            conversation: conversation.to_string(),
            intent: input.to_string(),
            is_seed: plan_so_far.to_string(),
            is_not_seed: String::new(),
            sla_deadline: envelope.sla_deadline,
            plan_so_far: plan_so_far.to_string(),
            reason: DISPUTE_REASON.into(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftKind {
    TestLog,
    Packet,
    Knowledge,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub kind: DraftKind,
    pub content: String,
}

pub fn post_call_drafts(steps: &[String]) -> Vec<Draft> {
    let log = steps.join("\n");
    vec![
        Draft {
            kind: DraftKind::TestLog,
            content: format!("test log:\n{log}"),
        },
        Draft {
            kind: DraftKind::Packet,
            content: format!("handoff packet (draft, pre-filled from log):\n{log}"),
        },
        Draft {
            kind: DraftKind::Knowledge,
            content: "knowledge gap proposal draft".into(),
        },
    ]
}

pub fn gap_action(similarity_units: i32) -> Option<DraftKind> {
    use brain_engine_sdk::pure::qa_score::gap_decision;
    // Every gap action funnels to the same HITL surface: a knowledge draft.
    gap_decision(similarity_units).map(|_| DraftKind::Knowledge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_engine_sdk::policy::{
        Priority as SdkPriority, stamp_complaint_envelope, stamp_envelope,
    };

    fn env() -> Envelope {
        stamp_envelope(1000, SdkPriority::P2)
    }
    #[test]
    fn closed_vocab_resolves() {
        let r = route("what is my order status", &env(), "conv", "plan");
        assert!(matches!(
            r,
            RouteDecision::Resolved {
                class: IntentClass::StatusCheck,
                ..
            }
        ));
    }
    #[test]
    fn outside_vocab_routes() {
        let r = route(
            "please hack my account and do something unknown xyz",
            &env(),
            "conv",
            "plan",
        );
        assert!(matches!(r, RouteDecision::Routed { .. }));
        let r2 = route(
            "adversarial prompt injection attempt",
            &env(),
            "conv",
            "plan",
        );
        assert!(matches!(r2, RouteDecision::Routed { .. }));
    }
    #[test]
    fn escape_honored_100() {
        for phrase in [
            "talk to a human",
            "Human Agent please",
            "ESCALATE",
            "speak to a person",
        ] {
            let r = route(
                phrase,
                &env(),
                "full conversation history",
                "step plan so far",
            );
            match r {
                RouteDecision::Escalated { handoff } => {
                    assert_eq!(handoff.conversation, "full conversation history");
                    assert!(!handoff.plan_so_far.is_empty());
                    assert_eq!(handoff.sla_deadline, env().sla_deadline);
                }
                _ => panic!("escape not honored for {phrase}"),
            }
        }
    }
    /// safety_complaint_routes_to_gpsr_path: a complaint whose
    /// content is safety-relevant escalates to the GPSR path (safety_recall)
    /// BEFORE the complaint keyword can claim it — the safety screen
    /// outranks the commercial classes it shares vocabulary with.
    #[test]
    fn safety_complaint_routes_to_gpsr_path() {
        for phrase in [
            "I want to file a complaint, this device caught fire",
            "formal complaint: my child was injured by this product",
            "complaint — the charger is unsafe and started smoking",
        ] {
            let r = route(phrase, &env(), "conv", "plan");
            match r {
                RouteDecision::Resolved {
                    class: IntentClass::SafetyRecall,
                    reason,
                } => assert!(
                    reason.contains("safety"),
                    "the documented escalation reason rides the decision"
                ),
                other => panic!("safety complaint must hit the GPSR path, got {other:?}"),
            }
        }
        // Plain complaints without hazard vocabulary stay complaints.
        let r = route(
            "I want to file a complaint about late delivery",
            &env(),
            "",
            "",
        );
        assert!(matches!(
            r,
            RouteDecision::Resolved {
                class: IntentClass::Complaint,
                ..
            }
        ));
    }

    #[test]
    fn complaint_class_gets_acknowledgment_sla() {
        let r = route(
            "I want to file a formal complaint about this",
            &env(),
            "conv",
            "plan",
        );
        assert!(matches!(
            r,
            RouteDecision::Resolved {
                class: IntentClass::Complaint,
                ..
            }
        ));
        // Complaints carry their own envelope: acknowledgment is its own,
        // always-tighter clock, and the priority map is the complaint's own.
        let e = stamp_complaint_envelope(1000);
        assert_eq!(e.ack_deadline, 1000 + 3600);
        assert!(e.ack_deadline < e.sla_deadline);
        // Escalation-to-dispute rides the documented reason.
        match escalate_complaint_dispute("complaint", &e, "conv", "plan") {
            RouteDecision::Escalated { handoff } => {
                assert_eq!(handoff.reason, "dispute");
                assert_eq!(handoff.sla_deadline, e.sla_deadline);
            }
            _ => panic!("complaint escalation must be a documented handover"),
        }
    }

    #[test]
    fn intent_table_routes_every_worktype_deterministically() {
        use brain_engine_sdk::policy::Worktype;
        // Every intent class resolves to exactly one worktype, and every
        // worktype has exactly one policy row with a matching kind string.
        let classes = [
            (IntentClass::BoundedDiagnostic, Worktype::Troubleshoot),
            (IntentClass::CareInquiry, Worktype::CareInquiry),
            (IntentClass::AccountChange, Worktype::Account),
            (IntentClass::Return, Worktype::Return),
            (IntentClass::WarrantyClaim, Worktype::WarrantyClaim),
            (IntentClass::RepairField, Worktype::RepairField),
            (IntentClass::Complaint, Worktype::Complaint),
            (IntentClass::SafetyRecall, Worktype::SafetyRecall),
            (IntentClass::RetentionOutreach, Worktype::RetentionOutreach),
        ];
        for (class, wt) in &classes {
            assert_eq!(&class.worktype(), wt);
            let row = worktype_policy(wt.as_str());
            assert!(row.is_some(), "no policy row for {}", wt.as_str());
            let row = row.expect("checked above");
            assert_eq!(row.kind, wt.as_str());
            assert!(!row.required_evidence.is_empty());
            assert!(!row.gates.is_empty());
        }
        // The table itself is closed: no orphan rows.
        assert_eq!(WORKTYPE_TABLE.len(), 9);
        for (kind, row) in WORKTYPE_TABLE {
            assert_eq!(kind, &row.kind);
        }
        // Unknown worktypes deny loudly.
        assert!(worktype_policy("astrology_reading").is_none());
        // Intake keywords route deterministically to the new classes.
        let samples: &[(&str, IntentClass)] = &[
            ("I want to return this laptop", IntentClass::Return),
            (
                "file a warranty claim, it is defective",
                IntentClass::WarrantyClaim,
            ),
            ("schedule an on-site repair visit", IntentClass::RepairField),
            (
                "how do I change my email address",
                IntentClass::AccountChange,
            ),
            ("what is your refund policy", IntentClass::PolicyAnswer),
            (
                "product recall safety notice received",
                IntentClass::SafetyRecall,
            ),
            (
                "I want to cancel my subscription",
                IntentClass::RetentionOutreach,
            ),
        ];
        for (text, expected) in samples {
            match route(text, &env(), "conv", "plan") {
                RouteDecision::Resolved { class, .. } => assert_eq!(&class, expected),
                _ => panic!("intake did not resolve: {text}"),
            }
        }
        // Safety vocabulary outranks shared commercial words.
        match route("recall — what is your return policy", &env(), "c", "p") {
            RouteDecision::Resolved {
                class: IntentClass::SafetyRecall,
                ..
            } => {}
            _ => panic!("safety intake must outrank commercial classes"),
        }
        // Each new worktype stamps its own SLA envelope class.
        let recall_env = stamp_envelope(1000, brain_engine_sdk::policy::Priority::P1);
        assert!(
            recall_env.sla_deadline
                < stamp_envelope(1000, brain_engine_sdk::policy::Priority::P3).sla_deadline
        );
    }

    #[test]
    fn drafts_only_hitl_no_auto_write() {
        let drafts = post_call_drafts(&["step1 did X".into(), "step2 verified Y".into()]);
        assert_eq!(drafts.len(), 3);
        for d in &drafts {
            // drafts are Draft structs — no knowledge write occurs
            assert!(!d.content.is_empty());
        }
        // type system ensures Draft never auto-writes: no function writes knowledge
    }
    #[test]
    fn sla_clock_stamped() {
        let e = stamp_envelope(0, SdkPriority::P1);
        assert_eq!(e.sla_deadline, 4 * 3600);
        let e2 = stamp_envelope(0, SdkPriority::P4);
        assert!(e2.sla_deadline > e.sla_deadline);
    }
}
