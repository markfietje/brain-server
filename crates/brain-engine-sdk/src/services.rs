//! Evidence + scoring as discoverable context services.
//!
//! No duplication: these are thin service wrappers over the pure cores in
//! [`crate::pure`] — the same deterministic functions, now mounted under
//! `ctx.evidence` / `ctx.scoring` so engines reach them via
//! `ctx.require::<EvidenceSvc>()` instead of importing server internals.

use crate::capability::{Capability, OpClass, allows};
use crate::plugin::{Context, KernelError, Service};
use crate::prompt::{CompactionPlan, compact_plan, should_compact, system_prompt};
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

/// `ctx.systemPrompt`: the bounded system-prompt builder and the
/// compaction-pressure policy — thin delegates to [`crate::prompt`], the
/// same deterministic functions engines already call directly.
pub struct SystemPromptSvc;

impl Service for SystemPromptSvc {
    fn key(&self) -> &'static str {
        "ctx.systemPrompt"
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl SystemPromptSvc {
    pub fn system_prompt(
        &self,
        lines: &[&str],
        skills: &[&str],
    ) -> Result<String, crate::prompt::PromptError> {
        system_prompt(lines, skills)
    }
    pub fn should_compact(&self, window_tokens: usize) -> bool {
        should_compact(window_tokens)
    }
    pub fn compact_plan(&self, token_counts: &[usize]) -> Option<CompactionPlan> {
        compact_plan(token_counts)
    }
}

/// The named, honest denial behind [`SandboxSvc::require_sandbox`]: no
/// sandbox backend exists in this tree. Callers receive this value instead
/// of any local-process stand-in — a denied capability is reported as
/// denied, never masqueraded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxUnavailable;

impl std::fmt::Display for SandboxUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "sandbox unavailable: no sandbox backend exists in this tree; capability denied"
        )
    }
}

impl std::error::Error for SandboxUnavailable {}

/// `ctx.sandbox`: a service whose isolation face is ALWAYS denied today.
/// The only allowed path is the capability ladder delegation — no sandbox
/// class exists there, so no posture ever reaches a sandbox through it.
pub struct SandboxSvc;

impl Service for SandboxSvc {
    fn key(&self) -> &'static str {
        "ctx.sandbox"
    }
    fn mount(&mut self, _ctx: &mut Context) {}
    fn unmount(&self) {}
}

impl SandboxSvc {
    /// Always `Err(SandboxUnavailable)`: there is no sandbox backend to
    /// admit anything. This is the contract, not a missing feature on a
    /// path to one.
    pub fn require_sandbox(&self) -> Result<(), SandboxUnavailable> {
        Err(SandboxUnavailable)
    }
    /// The only allowed path: the shared capability ladder. A (posture,
    /// class) pair the ladder denies is denied here identically.
    pub fn allows(&self, posture: Capability, class: OpClass) -> bool {
        allows(posture, class)
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
    use crate::prompt::MAX_SYSTEM_PROMPT_LINES;

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

    #[test]
    fn system_prompt_svc_is_discoverable_and_delegates_to_the_core() {
        let mut ctx = Context::new();
        ctx.provide(SystemPromptSvc).unwrap();
        let svc = ctx.require::<SystemPromptSvc>().unwrap();
        let lines: Vec<&str> = (0..10).map(|_| "line").collect();
        assert_eq!(
            svc.system_prompt(&lines, &["skill"]).unwrap(),
            crate::prompt::system_prompt(&lines, &["skill"]).unwrap()
        );
        let oversized: Vec<&str> = (0..MAX_SYSTEM_PROMPT_LINES + 1).map(|_| "x").collect();
        assert!(svc.system_prompt(&oversized, &[]).is_err());
        assert_eq!(svc.should_compact(0), crate::prompt::should_compact(0));
        assert_eq!(
            svc.should_compact(usize::MAX),
            crate::prompt::should_compact(usize::MAX)
        );
        let counts = vec![10, 20, 30];
        assert_eq!(
            svc.compact_plan(&counts),
            crate::prompt::compact_plan(&counts)
        );
        assert_eq!(svc.compact_plan(&[]), crate::prompt::compact_plan(&[]));
    }

    #[test]
    fn duplicate_system_prompt_provide_fails_loud() {
        let mut ctx = Context::new();
        assert!(ctx.provide(SystemPromptSvc).is_ok());
        assert!(ctx.provide(SystemPromptSvc).is_err());
    }

    #[test]
    fn sandbox_require_is_a_named_denial_never_a_local_stand_in() {
        let mut ctx = Context::new();
        ctx.provide(SandboxSvc).unwrap();
        let svc = ctx.require::<SandboxSvc>().unwrap();
        // The named denial, every time — not an error string, not a runner.
        assert_eq!(svc.require_sandbox(), Err(SandboxUnavailable));
        assert_eq!(svc.require_sandbox(), Err(SandboxUnavailable));
        let rendered = SandboxUnavailable.to_string();
        assert!(rendered.contains("denied"));
        assert!(!rendered.to_lowercase().contains("local runner"));
    }

    #[test]
    fn sandbox_allows_is_exact_capability_ladder_delegation() {
        let svc = SandboxSvc;
        let pairs = [
            (Capability::Safe, OpClass::ReadState),
            (Capability::Safe, OpClass::ProcessSpawn),
            (Capability::Standard, OpClass::ExecTool),
            (Capability::Standard, OpClass::FilesystemWrite),
            (Capability::Permissive, OpClass::ProcessSpawn),
        ];
        for (posture, class) in pairs {
            assert_eq!(svc.allows(posture, class), allows(posture, class));
        }
    }
}
