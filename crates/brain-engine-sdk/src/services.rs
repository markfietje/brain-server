//! Evidence + scoring as discoverable context services.
//!
//! No duplication: these are thin service wrappers over the pure cores in
//! [`crate::pure`] — the same deterministic functions, now mounted under
//! `ctx.evidence` / `ctx.scoring` so engines reach them via
//! `ctx.require::<EvidenceSvc>()` instead of importing server internals.

use crate::plugin::{Context, KernelError, Service};
use crate::pure::evidence::{Finding, Reduction, reduce};
use crate::pure::qa_score::{
    Cause, GapAction, RunArtifacts, Scoreboard, StepRow, classify_cause, gap_decision,
    override_rate, score_run, scoreboard,
};

/// `ctx.evidence`: claim-grouping / dedup / contradiction surfacing.
pub struct EvidenceSvc;

impl Service for EvidenceSvc {
    fn key(&self) -> &'static str {
        "ctx.evidence"
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl EvidenceSvc {
    pub fn reduce(&self, raw: Vec<Finding>) -> Reduction {
        reduce(raw)
    }
}

/// `ctx.scoring`: the quality-intelligence scorer family.
pub struct ScoringSvc;

impl Service for ScoringSvc {
    fn key(&self) -> &'static str {
        "ctx.scoring"
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl ScoringSvc {
    pub fn score_run(&self, a: &RunArtifacts) -> crate::pure::qa_score::QaScore {
        score_run(a)
    }
    pub fn classify_cause(&self, a: &RunArtifacts) -> Cause {
        classify_cause(a)
    }
    pub fn override_rate(&self, steps: &[StepRow]) -> i32 {
        override_rate(steps)
    }
    pub fn gap_decision(&self, similarity_units: i32) -> Option<GapAction> {
        gap_decision(similarity_units)
    }
    pub fn scoreboard(&self, runs: &[RunArtifacts]) -> Scoreboard {
        scoreboard(runs)
    }
}

/// Install both services on a context. Fail-loud on a duplicate mount (the
/// kernel's normal posture).
pub fn install(ctx: &mut Context) -> Result<(), KernelError> {
    ctx.provide(EvidenceSvc)?;
    ctx.provide(ScoringSvc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_are_discoverable_and_delegate_to_the_pure_cores() {
        let mut ctx = Context::new();
        install(&mut ctx).unwrap();
        let evidence = ctx.require::<EvidenceSvc>().unwrap();
        let reduction = evidence.reduce(vec![Finding {
            claim: "  Disk   full ".into(),
            evidence: "df".into(),
            source: "agent".into(),
            confidence: 0.9,
            ts: 1,
        }]);
        assert_eq!(reduction.findings.len(), 1);
        assert_eq!(reduction.contradictions.len(), 0);

        let scoring = ctx.require::<ScoringSvc>().unwrap();
        let artifacts = RunArtifacts {
            steps: vec![],
            findings: vec![],
            contradictions: 0,
            audit_ok: true,
            repeat_contact: false,
            handoff_complete: true,
            verified: true,
            escalation_honored: true,
        };
        assert_eq!(
            scoring.score_run(&artifacts).total_units,
            crate::pure::qa_score::SCALE
        );
        assert_eq!(scoring.classify_cause(&artifacts), Cause::Agent);
        assert_eq!(scoring.override_rate(&[]), 0);
        assert_eq!(scoring.gap_decision(3000), Some(GapAction::ProposeNew));
        assert_eq!(
            scoring
                .scoreboard(std::slice::from_ref(&artifacts))
                .fcr_units,
            crate::pure::qa_score::SCALE
        );
    }

    #[test]
    fn duplicate_install_fails_loud() {
        let mut ctx = Context::new();
        install(&mut ctx).unwrap();
        assert!(install(&mut ctx).is_err());
    }

    #[test]
    fn missing_service_is_loud_not_default() {
        let ctx = Context::new();
        assert!(ctx.require::<EvidenceSvc>().is_err());
    }
}
