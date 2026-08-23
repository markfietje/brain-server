//! The I-PASS handoff packet, pure and dependency-light: the input is
//! pre-serialized facts (rows already read by the host), the output a plain
//! struct. Five sections mapped onto what a governed run actually records:
//!
//! - **I**llness   — the frontdoor seed (intent / is_seed / is_not_seed) plus
//!                   the run's opening event.
//! - **P**atient   — the run's domain + subject rows.
//! - **A**ction    — the step plan and step events on the current branch.
//! - **S**ituation — the latest checkpoint digest + any open pending_question.
//! - **S**afety    — the SLA envelope, legal-hold status, escalation flag.
//!
//! The packet is evidence assembled from records — never judgment: nothing
//! here summarizes or interprets; every line traces to an input fact.

/// One rendered I-PASS section: a fixed title plus its fact lines.
pub struct IpassSection {
    title: &'static str,
    lines: Vec<String>,
}

impl IpassSection {
    pub fn title(&self) -> &'static str {
        self.title
    }
    pub fn lines(&self) -> &[String] {
        &self.lines
    }
}

/// The five sections plus the completeness bit, exactly as the scoreboard
/// derives it (`handoff_complete = status == "completed"`).
pub struct HandoffPacket {
    pub illness: IpassSection,
    pub patient: IpassSection,
    pub action: IpassSection,
    pub situation: IpassSection,
    pub safety: IpassSection,
    pub complete: bool,
}

/// Pre-resolved facts. Strings arrive ready (the host sanitizes); the builder
/// only arranges them.
pub struct HandoffFacts {
    pub intent: String,
    pub is_seed: String,
    pub is_not_seed: String,
    pub opening_event: Option<String>,
    pub domain: String,
    pub patient_rows: Vec<String>,
    pub action_steps: Vec<String>,
    pub action_events: Vec<String>,
    /// SHA-256 of the latest checkpoint payload, when one exists.
    pub checkpoint_digest: Option<String>,
    pub pending_question: Option<String>,
    pub sla_deadline: Option<i64>,
    pub now: i64,
    pub legal_hold_active: bool,
    pub escalation_honored: bool,
    pub run_status: String,
}

fn push_line(lines: &mut Vec<String>, label: &str, value: String) {
    let v = if value.trim().is_empty() {
        "(none recorded)".to_string()
    } else {
        value
    };
    lines.push(format!("{label}: {v}"));
}

/// Assemble the packet. Deterministic over the facts — same input, same bytes.
pub fn assemble(f: &HandoffFacts) -> HandoffPacket {
    let mut illness = vec![];
    push_line(&mut illness, "intent", f.intent.clone());
    if !f.is_seed.is_empty() {
        illness.push(format!("is_seed: {}", f.is_seed));
    }
    if !f.is_not_seed.is_empty() {
        illness.push(format!("is_not_seed: {}", f.is_not_seed));
    }
    match &f.opening_event {
        Some(o) => illness.push(format!("opening event: {o}")),
        None => illness.push("opening event: (none recorded)".to_string()),
    }

    let mut patient = vec![format!("domain: {}", f.domain)];
    for row in &f.patient_rows {
        patient.push(row.clone());
    }

    let mut action = vec![];
    if f.action_steps.is_empty() && f.action_events.is_empty() {
        action.push("steps: (none recorded)".to_string());
    } else {
        for s in &f.action_steps {
            action.push(format!("step: {s}"));
        }
        for e in &f.action_events {
            action.push(format!("event: {e}"));
        }
    }

    let mut situation = vec![];
    match &f.checkpoint_digest {
        Some(d) => situation.push(format!("latest checkpoint digest: {d}")),
        None => situation.push("latest checkpoint digest: (none recorded)".to_string()),
    }
    push_line(
        &mut situation,
        "open question",
        f.pending_question.clone().unwrap_or_default(),
    );

    let mut safety = vec![];
    match f.sla_deadline {
        Some(deadline) => {
            if deadline < f.now {
                safety.push(format!(
                    "sla: BREACHED (deadline {deadline}, now {})",
                    f.now
                ));
            } else {
                safety.push(format!("sla: within envelope (deadline {deadline})"));
            }
        }
        None => safety.push("sla: no deadline stamped".to_string()),
    }
    safety.push(if f.legal_hold_active {
        "legal hold: ACTIVE on this domain".to_string()
    } else {
        "legal hold: none active".to_string()
    });
    safety.push(if f.escalation_honored {
        "escalation: honored".to_string()
    } else {
        "escalation: NOT honored".to_string()
    });

    HandoffPacket {
        illness: IpassSection {
            title: "Illness",
            lines: illness,
        },
        patient: IpassSection {
            title: "Patient",
            lines: patient,
        },
        action: IpassSection {
            title: "Action",
            lines: action,
        },
        situation: IpassSection {
            title: "Situation",
            lines: situation,
        },
        safety: IpassSection {
            title: "Safety",
            lines: safety,
        },
        // Exactly the scoreboard's derivation — not a private definition.
        complete: f.run_status == "completed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> HandoffFacts {
        HandoffFacts {
            intent: "order status check".into(),
            is_seed: "plan so far".into(),
            is_not_seed: String::new(),
            opening_event: Some(r#"{"line":"run opened"}"#.into()),
            domain: "acme".into(),
            patient_rows: vec!["domain:acme".into(), "run:7".into()],
            action_steps: vec!["inventory:execute".into()],
            action_events: vec!["workflow/log".into()],
            checkpoint_digest: Some("abc123".into()),
            pending_question: Some("which NL group?".into()),
            sla_deadline: Some(2000),
            now: 1000,
            legal_hold_active: false,
            escalation_honored: true,
            run_status: "completed".into(),
        }
    }

    /// handoff_packet_has_all_five_pass_sections
    #[test]
    fn handoff_packet_has_all_five_pass_sections() {
        let p = assemble(&facts());
        assert_eq!(p.illness.title(), "Illness");
        assert_eq!(p.patient.title(), "Patient");
        assert_eq!(p.action.title(), "Action");
        assert_eq!(p.situation.title(), "Situation");
        assert_eq!(p.safety.title(), "Safety");
        assert!(p.illness.lines().iter().any(|l| l.contains("intent")));
        assert!(p.action.lines().iter().any(|l| l.contains("inventory")));
    }

    /// handoff_includes_open_question_and_sla_state
    #[test]
    fn handoff_includes_open_question_and_sla_state() {
        let p = assemble(&facts());
        assert!(
            p.situation
                .lines()
                .iter()
                .any(|l| l.contains("which NL group?"))
        );
        assert!(
            p.safety
                .lines()
                .iter()
                .any(|l| l.contains("within envelope"))
        );
        // A breached deadline says so loudly instead of showing a bare number.
        let mut f = facts();
        f.now = 9999;
        let p2 = assemble(&f);
        assert!(p2.safety.lines().iter().any(|l| l.contains("BREACHED")));
        // An absent question renders as an explicit none, never silence.
        let mut f3 = facts();
        f3.pending_question = None;
        let p3 = assemble(&f3);
        assert!(
            p3.situation
                .lines()
                .iter()
                .any(|l| l.contains("(none recorded)"))
        );
    }

    /// handoff_complete_matches_scoreboard_derivation
    #[test]
    fn handoff_complete_matches_scoreboard_derivation() {
        let p = assemble(&facts());
        assert!(p.complete);
        let mut f = facts();
        f.run_status = "active".into();
        assert!(!assemble(&f).complete, "complete mirrors status==completed");
    }

    /// Determinism: same facts assemble byte-identical section lines.
    #[test]
    fn assembly_is_deterministic() {
        let f = facts();
        let a = assemble(&f);
        let b = assemble(&f);
        let flat = |p: &HandoffPacket| {
            [
                p.illness.lines(),
                p.patient.lines(),
                p.action.lines(),
                p.situation.lines(),
                p.safety.lines(),
            ]
            .concat()
            .join("\n")
        };
        assert_eq!(flat(&a), flat(&b));
    }
}
