//! The quality-intelligence scorer: integer ten-thousandths (0..=10000) so a
//! score is exact, orderable, and wire-stable — never a lossy float. Every
//! scored question carries its justification and evidence refs; the flywheel
//! yields PROPOSALS only (the type system offers no path to a direct write).

/// Score scale: 10000 units == 100%.
pub const SCALE: i32 = 10_000;

#[derive(Debug, Clone, PartialEq)]
pub struct RunArtifacts {
    pub steps: Vec<StepRow>,
    pub findings: Vec<String>,
    pub contradictions: usize,
    pub audit_ok: bool,
    pub repeat_contact: bool,
    pub handoff_complete: bool,
    pub verified: bool,
    pub escalation_honored: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepRow {
    pub expected: String,
    pub actual: String,
    pub skipped_verify: bool,
    pub abstained: bool,
    pub guidance_accepted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ScoredQuestion {
    pub id: String,
    pub score_units: i32,
    pub justification: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct QaScore {
    pub total_units: i32,
    pub questions: Vec<ScoredQuestion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Cause {
    Agent,
    System,
}

pub fn score_run(a: &RunArtifacts) -> QaScore {
    let qs = vec![
        score_resolution(a),
        score_correctness(a),
        score_verification(a),
        score_continuity(a),
        score_trust(a),
    ];
    let total = qs.iter().map(|q| q.score_units).sum::<i32>() / qs.len() as i32;
    QaScore {
        total_units: total,
        questions: qs,
    }
}

fn score_resolution(a: &RunArtifacts) -> ScoredQuestion {
    let (score, just) = if a.repeat_contact {
        (
            3000,
            "repeat contact indicates unresolved from customer perspective",
        )
    } else if a.contradictions > 0 {
        (6000, "open contradictions degrade resolution confidence")
    } else {
        (SCALE, "no repeat contact and no open contradictions")
    };
    ScoredQuestion {
        id: "resolution".into(),
        score_units: score,
        justification: just.into(),
        evidence_refs: vec!["steps".into(), "contradictions".into()],
    }
}

fn score_correctness(a: &RunArtifacts) -> ScoredQuestion {
    let (score, just) = if a.findings.iter().any(|f| f.contains("incorrect")) {
        (4000, "incorrect finding present")
    } else {
        (SCALE, "no incorrect findings")
    };
    ScoredQuestion {
        id: "correctness".into(),
        score_units: score,
        justification: just.into(),
        evidence_refs: vec!["findings".into()],
    }
}

fn score_verification(a: &RunArtifacts) -> ScoredQuestion {
    let skipped = a.steps.iter().any(|s| s.skipped_verify);
    let (score, just) = if skipped {
        (2000, "verify step skipped")
    } else {
        (SCALE, "all verify steps executed")
    };
    ScoredQuestion {
        id: "verification".into(),
        score_units: score,
        justification: just.into(),
        evidence_refs: vec!["steps".into()],
    }
}

fn score_continuity(a: &RunArtifacts) -> ScoredQuestion {
    let (score, just) = if a.handoff_complete {
        (SCALE, "handoff bundle complete")
    } else {
        (5000, "handoff bundle incomplete")
    };
    ScoredQuestion {
        id: "continuity".into(),
        score_units: score,
        justification: just.into(),
        evidence_refs: vec!["steps".into()],
    }
}

fn score_trust(a: &RunArtifacts) -> ScoredQuestion {
    let (score, just) = if !a.audit_ok {
        (0, "audit chain not green")
    } else if !a.escalation_honored {
        (3000, "escalation trigger not honored")
    } else {
        (SCALE, "audit green and escalation honored")
    };
    ScoredQuestion {
        id: "trust".into(),
        score_units: score,
        justification: just.into(),
        evidence_refs: vec!["audit".into()],
    }
}

pub fn classify_cause(a: &RunArtifacts) -> Cause {
    if a.steps.iter().any(|s| s.skipped_verify) {
        return Cause::Agent;
    }
    if !a.handoff_complete && a.steps.iter().all(|s| !s.skipped_verify) && a.audit_ok {
        return Cause::System;
    }
    Cause::Agent
}

pub fn override_rate(steps: &[StepRow]) -> i32 {
    if steps.is_empty() {
        return 0;
    }
    let diverged = steps.iter().filter(|s| s.expected != s.actual).count() as i32;
    diverged * SCALE / steps.len() as i32
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GapAction {
    ProposeNew,
    ProposeUpdate,
}

pub fn gap_decision(similarity_units: i32) -> Option<GapAction> {
    if similarity_units < 4000 {
        Some(GapAction::ProposeNew)
    } else if similarity_units < 8000 {
        Some(GapAction::ProposeUpdate)
    } else {
        None
    }
}

pub fn repeater_detected(count_same_30d: usize) -> bool {
    count_same_30d >= 3
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlywheelProposal {
    Gap(GapAction),
    Rca,
    /// v1.28.34: an RCA born from a complaint cluster — same HITL pipeline,
    /// ranked above incident repeaters (see [`super::complaint`]).
    ComplaintRca,
}

pub fn flywheel_proposals(similarity_units: i32, repeater_count: usize) -> Vec<FlywheelProposal> {
    let mut out = Vec::new();
    if let Some(g) = gap_decision(similarity_units) {
        out.push(FlywheelProposal::Gap(g));
    }
    if repeater_detected(repeater_count) {
        out.push(FlywheelProposal::Rca);
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Scoreboard {
    pub fcr_units: i32,
    pub repeat_contact_rate_units: i32,
    pub correctness_units: i32,
    pub override_rate_units: i32,
    pub gap_rate_units: i32,
    pub abstention_rate_units: i32,
    pub guidance_acceptance_units: i32,
    pub handoff_completeness_units: i32,
    pub audit_green: bool,
    pub escalation_honored_units: i32,
}

pub fn scoreboard(runs: &[RunArtifacts]) -> Scoreboard {
    if runs.is_empty() {
        return Scoreboard {
            fcr_units: 0,
            repeat_contact_rate_units: 0,
            correctness_units: 0,
            override_rate_units: 0,
            gap_rate_units: 0,
            abstention_rate_units: 0,
            guidance_acceptance_units: 0,
            handoff_completeness_units: 0,
            audit_green: true,
            escalation_honored_units: SCALE,
        };
    }
    let n = runs.len() as i32;
    let fcr = runs.iter().filter(|r| !r.repeat_contact).count() as i32 * SCALE / n;
    let repeat = runs.iter().filter(|r| r.repeat_contact).count() as i32 * SCALE / n;
    let correct = runs
        .iter()
        .filter(|r| !r.findings.iter().any(|f| f.contains("incorrect")))
        .count() as i32
        * SCALE
        / n;
    let all_steps: Vec<StepRow> = runs.iter().flat_map(|r| r.steps.clone()).collect();
    let ovr = override_rate(&all_steps);
    let total_steps = all_steps.len() as i32;
    let abst = if total_steps == 0 {
        0
    } else {
        all_steps.iter().filter(|s| s.abstained).count() as i32 * SCALE / total_steps
    };
    let guidance_total = all_steps
        .iter()
        .filter(|s| s.guidance_accepted.is_some())
        .count() as i32;
    let guidance_acc = if guidance_total == 0 {
        SCALE
    } else {
        all_steps
            .iter()
            .filter(|s| s.guidance_accepted == Some(true))
            .count() as i32
            * SCALE
            / guidance_total
    };
    let handoff = runs.iter().filter(|r| r.handoff_complete).count() as i32 * SCALE / n;
    let audit_green = runs.iter().all(|r| r.audit_ok);
    let esc = runs.iter().filter(|r| r.escalation_honored).count() as i32 * SCALE / n;
    let gap_rate = 0; // derived from proposals, not runs alone
    Scoreboard {
        fcr_units: fcr,
        repeat_contact_rate_units: repeat,
        correctness_units: correct,
        override_rate_units: ovr,
        gap_rate_units: gap_rate,
        abstention_rate_units: abst,
        guidance_acceptance_units: guidance_acc,
        handoff_completeness_units: handoff,
        audit_green,
        escalation_honored_units: esc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> RunArtifacts {
        RunArtifacts {
            steps: vec![StepRow {
                expected: "a".into(),
                actual: "a".into(),
                skipped_verify: false,
                abstained: false,
                guidance_accepted: Some(true),
            }],
            findings: vec![],
            contradictions: 0,
            audit_ok: true,
            repeat_contact: false,
            handoff_complete: true,
            verified: true,
            escalation_honored: true,
        }
    }

    #[test]
    fn scorer_oracle_fixture() {
        let a = base();
        let s = score_run(&a);
        assert!(
            s.questions.iter().all(|q| !q.justification.is_empty()),
            "every score needs justification"
        );
        assert!(s.questions.iter().all(|q| !q.evidence_refs.is_empty()));
        assert_eq!(s.questions.len(), 5);
        assert_eq!(s.total_units, SCALE);
        // repeat contact degrades resolution
        let mut b = base();
        b.repeat_contact = true;
        let sb = score_run(&b);
        assert!(
            sb.questions
                .iter()
                .find(|q| q.id == "resolution")
                .unwrap()
                .score_units
                < SCALE
        );
        // integer ten-thousandths
        for q in &s.questions {
            assert!(q.score_units >= 0 && q.score_units <= SCALE);
        }
    }

    #[test]
    fn cause_split_fixture_table() {
        let mut agent = base();
        agent.steps[0].skipped_verify = true;
        assert_eq!(classify_cause(&agent), Cause::Agent);
        let mut system = base();
        system.handoff_complete = false;
        assert_eq!(classify_cause(&system), Cause::System);
    }

    #[test]
    fn override_rate_computed() {
        let steps = vec![
            StepRow {
                expected: "a".into(),
                actual: "a".into(),
                skipped_verify: false,
                abstained: false,
                guidance_accepted: None,
            },
            StepRow {
                expected: "a".into(),
                actual: "b".into(),
                skipped_verify: false,
                abstained: false,
                guidance_accepted: None,
            },
        ];
        assert_eq!(override_rate(&steps), 5000);
    }

    #[test]
    fn gap_rule_thresholds() {
        assert_eq!(gap_decision(3000), Some(GapAction::ProposeNew));
        assert_eq!(gap_decision(6000), Some(GapAction::ProposeUpdate));
        assert_eq!(gap_decision(9000), None);
    }

    #[test]
    fn repeater_detection() {
        assert!(!repeater_detected(2));
        assert!(repeater_detected(3));
    }

    #[test]
    fn no_auto_publish_invariant() {
        // flywheel only yields proposals, never direct knowledge writes
        let proposals = flywheel_proposals(3000, 3);
        for p in proposals {
            match p {
                FlywheelProposal::Gap(_) | FlywheelProposal::Rca | FlywheelProposal::ComplaintRca => {}
            }
        }
        // no FlywheelProposal variant writes knowledge directly — enforced by type system
    }

    #[test]
    fn scoreboard_surface() {
        let runs = vec![base(), {
            let mut r = base();
            r.repeat_contact = true;
            r
        }];
        let sb = scoreboard(&runs);
        assert_eq!(sb.fcr_units, 5000);
        assert_eq!(sb.repeat_contact_rate_units, 5000);
        assert!(sb.audit_green);
    }

    // The gold-pack calibration pins: the same oracle contract, re-run
    // against versioned frozen truth instead of these hand fixtures.
    // Opt-in via the `gold-sets` feature; without it the hand fixtures above
    // are the contract (documented rollback posture).
    #[cfg(feature = "gold-sets")]
    mod gold {
        use super::*;
        use gold_sets::{CaseArtifacts, GoldCase};

        fn artifacts(c: &GoldCase) -> RunArtifacts {
            let a: &CaseArtifacts = &c.artifacts;
            RunArtifacts {
                steps: a
                    .steps
                    .iter()
                    .map(|s| StepRow {
                        expected: s.expected.clone(),
                        actual: s.actual.clone(),
                        skipped_verify: s.skipped_verify,
                        abstained: s.abstained,
                        guidance_accepted: s.guidance_accepted,
                    })
                    .collect(),
                findings: a.findings.clone(),
                contradictions: a.contradictions as usize,
                audit_ok: a.audit_ok,
                repeat_contact: a.repeat_contact,
                handoff_complete: a.handoff_complete,
                verified: a.verified,
                escalation_honored: a.escalation_honored,
            }
        }

        /// Machine verdict for a case: pass only at a perfect score.
        fn machine_pass(c: &GoldCase) -> bool {
            score_run(&artifacts(c)).total_units == SCALE
        }

        #[test]
        fn scorer_oracle_fixture_against_gold() {
            for c in gold_sets::all().unwrap() {
                let s = score_run(&artifacts(&c));
                assert_eq!(s.questions.len(), 5);
                assert!(s.questions.iter().all(|q| !q.justification.is_empty()));
                assert!(s.questions.iter().all(|q| !q.evidence_refs.is_empty()));
                for q in &s.questions {
                    assert!((0..=SCALE).contains(&q.score_units));
                }
                // Agreed human truth is the oracle, not the fixture table.
                assert_eq!(
                    machine_pass(&c),
                    c.human_pass,
                    "case {} disagrees with frozen human verdict",
                    c.id
                );
            }
        }

        #[test]
        fn cause_split_fixture_table_against_gold() {
            for c in gold_sets::all().unwrap() {
                let a = artifacts(&c);
                if !a.steps.iter().any(|s| s.skipped_verify) && !a.handoff_complete && a.audit_ok {
                    assert_eq!(classify_cause(&a), Cause::System);
                } else if a.steps.iter().any(|s| s.skipped_verify) {
                    assert_eq!(classify_cause(&a), Cause::Agent);
                }
            }
        }

        #[test]
        fn no_auto_publish_invariant_against_gold() {
            // Even on gold cases the flywheel yields proposals only — the type
            // system still offers no path to a direct knowledge write.
            for c in gold_sets::all().unwrap() {
                let a = artifacts(&c);
                let steps = a.steps.clone();
                for p in flywheel_proposals(3000, 3) {
                    match p {
                        FlywheelProposal::Gap(_) | FlywheelProposal::Rca | FlywheelProposal::ComplaintRca => {}
                    }
                }
                let _ = (steps, override_rate(&a.steps));
            }
        }

        #[test]
        fn kappa_gate_holds_on_gold_truth() {
            let cases = gold_sets::all().unwrap();
            let failures = gold_sets::kappa_gate_failures(&cases);
            assert!(failures.is_empty(), "κ gate failed: {failures:?}");
            // Independent recomputation from the paired labels.
            let human: Vec<bool> = cases.iter().map(|c| c.human_pass).collect();
            let machine: Vec<bool> = cases.iter().map(machine_pass).collect();
            let k = crate::pure::calibration::kappa_units(&human, &machine).expect("defined");
            assert!(k >= 7000, "recomputed κ {k} below 0.70");
        }
    }
}
