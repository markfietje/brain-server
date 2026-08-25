use brain_engine_sdk::policy::Envelope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentClass {
    Auth,
    StatusCheck,
    PolicyAnswer,
    Booking,
    BoundedDiagnostic,
    /// ISO 10002: a complaint is its own class, not an escalation flavor.
    Complaint,
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

fn classify_rules(input: &str) -> Option<(IntentClass, String)> {
    let l = input.to_ascii_lowercase();
    if l.contains("complain") {
        return Some((IntentClass::Complaint, "complaint keyword matched".into()));
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
