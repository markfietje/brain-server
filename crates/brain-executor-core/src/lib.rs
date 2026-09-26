//! The delivery loop's executor core — pure, total, and I/O-free.
//!
//! One question: did this stage do the work it claims, with evidence a human
//! can check? The three laws here are [`validate_gate_json`] (the QA gate
//! refuses a checkpoint whose evidence is not a live surface), [`RunState`]
//! (the critic counts non-okay verdicts and pauses at the named ceiling), and
//! [`requires_delegation`] (a scope past the thresholds is delegated, never
//! absorbed). [`artifact_hash`] is the deterministic content digest the delivery
//! seam uses to address a typed artifact.
//!
//! What is deliberately absent: this crate does not persist, does not sign,
//! does not execute anything, and does not contact a host. It is a decision
//! core — a model proposes, only the gate disposes, and the authority that
//! disposes is not here.
//!
//! The honest ceiling: [`apply_steering`] is a DECLARED no-op. All six
//! [`SteeringKind`] variants are reserved vocabulary with no defined semantics
//! against a two-field [`Aggregate`], and no caller needs a mutation. It is
//! infallible by that decision — a `Result` it could never fail made "no
//! mutation needed" indistinguishable from "refused". A round that needs real
//! steering must change the signature deliberately, which is the point.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub id: String,
    pub title: String,
    pub body: String,
}

pub fn parse_brief(brief: &str) -> Vec<Goal> {
    let has_delim = brief.lines().any(|l| l.starts_with("@goal:"));
    if !has_delim {
        let t = brief.trim();
        return vec![Goal {
            id: "G001".into(),
            title: "G001".into(),
            body: t.into(),
        }];
    }
    let mut goals = Vec::new();
    let mut cur_id: Option<String> = None;
    let mut cur_body = String::new();
    for line in brief.lines() {
        if let Some(rest) = line.strip_prefix("@goal:") {
            if let Some(id) = cur_id.take() {
                goals.push(Goal {
                    id: id.clone(),
                    title: id,
                    body: cur_body.trim().into(),
                });
                cur_body.clear();
            }
            cur_id = Some(rest.trim().to_string());
        } else if cur_id.is_some() {
            cur_body.push_str(line);
            cur_body.push('\n');
        }
    }
    if let Some(id) = cur_id {
        goals.push(Goal {
            id: id.clone(),
            title: id,
            body: cur_body.trim().into(),
        });
    }
    goals
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointGate {
    #[serde(default)]
    pub architect_review: Option<String>,
    #[serde(default)]
    pub executor_qa: Option<ExecutorQa>,
    #[serde(default)]
    pub critic_review: Option<String>,
    #[serde(default)]
    pub replay_exempt: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutorQa {
    pub contract_coverage: String,
    pub surface_evidence: Vec<SurfaceEvidence>,
    #[serde(default)]
    pub adversarial_cases: Vec<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    pub iteration: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SurfaceEvidence {
    pub kind: String,
    pub receipt: String,
}

const ALLOWED_QA_KEYS: &[&str] = &[
    "contractCoverage",
    "surfaceEvidence",
    "adversarialCases",
    "artifactRefs",
    "iteration",
    "inlineEvidence",
    // NOT `replayExempt`: that is a CheckpointGate field, not an ExecutorQa
    // one. It was listed here, and nothing used `deny_unknown_fields`, so a
    // nested `executorQa.replayExempt` validated and was then silently dropped
    // — a caller believed it was exempt while the gate still refused. Listed
    // keys must be fields that exist; the top-level gate key is the real one.
];
const ALLOWED_GATE_KEYS: &[&str] = &[
    "architectReview",
    "executorQa",
    "criticReview",
    "replayExempt",
];

pub fn validate_gate_json(raw: &str) -> Result<CheckpointGate, String> {
    let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let obj = v.as_object().ok_or("gate must be object")?;
    for k in obj.keys() {
        if !ALLOWED_GATE_KEYS.contains(&k.as_str()) {
            return Err(format!("quality_gate_rejects_unknown_keys: {k}"));
        }
    }
    if let Some(qa) = obj.get("executorQa").and_then(|x| x.as_object()) {
        for k in qa.keys() {
            if !ALLOWED_QA_KEYS.contains(&k.as_str()) {
                return Err(format!("quality_gate_rejects_unknown_keys: {k}"));
            }
        }
    }
    let gate: CheckpointGate = serde_json::from_value(v).map_err(|e| e.to_string())?;
    // Live-surface evidence check for complete
    if let Some(qa) = &gate.executor_qa {
        let has_live = qa.surface_evidence.iter().any(|e| {
            matches!(
                e.kind.as_str(),
                "gui" | "cli" | "native" | "api" | "algorithm"
            ) && !e.receipt.is_empty()
        });
        if !has_live && !gate.replay_exempt {
            return Err("quality_gate_requires_live_surface_evidence".into());
        }
    } else {
        return Err("quality_gate_requires_live_surface_evidence".into());
    }
    Ok(gate)
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockerKind {
    Resolvable,
    HumanBlocked,
}

#[derive(Debug, Clone)]
pub struct RunState {
    pub non_okay_count: usize,
    pub paused: bool,
}

/// The critic ceiling — the design owner's "5 → pause" (the governing text).
const CRITIC_CEILING: usize = 5;

impl Default for RunState {
    fn default() -> Self {
        Self::new()
    }
}

impl RunState {
    pub fn new() -> Self {
        Self {
            non_okay_count: 0,
            paused: false,
        }
    }
    pub fn record_verdict(&mut self, okay: bool) {
        if !okay {
            self.non_okay_count += 1;
            if self.non_okay_count >= CRITIC_CEILING {
                self.paused = true;
            }
        }
    }
    pub fn triage(&mut self, kind: BlockerKind) {
        match kind {
            BlockerKind::Resolvable => {}
            BlockerKind::HumanBlocked => self.paused = true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Aggregate {
    pub objective: String,
    pub brief_hash: String,
}
#[derive(Debug, Clone)]
pub enum SteeringKind {
    Add,
    Split,
    Reorder,
    Revise,
    Annotate,
    Supersede,
}

/// Apply steering to an aggregate. A DECLARED no-op, and infallible by that
/// declaration — the aggregate is immutable and no variant defines a mutation
/// against it. The six [`SteeringKind`] values are reserved vocabulary.
///
/// The signature is the contract: a caller cannot read an `Err` as "refused"
/// because there is no failure to read. A round that needs real steering gives
/// this function defined semantics and a signature that can report them.
pub fn apply_steering(agg: &Aggregate, kind: SteeringKind) -> Aggregate {
    // aggregate immutable — steering never mutates it
    let _ = kind;
    agg.clone()
}

pub fn requires_delegation(files: usize, lines: usize, parallel: bool) -> bool {
    files >= 3 || lines >= 200 || parallel
}

pub fn artifact_hash(content: &str) -> String {
    hex::encode(Sha256::digest(content.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brief_no_delimiter_single_goal() {
        let g = parse_brief("hello world");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].id, "G001");
    }
    #[test]
    fn quality_gate_requires_live_surface_evidence() {
        let raw = r#"{"executorQa":{"contractCoverage":"x","surfaceEvidence":[],"adversarialCases":[],"artifactRefs":[],"iteration":1}}"#;
        assert!(
            validate_gate_json(raw)
                .unwrap_err()
                .contains("quality_gate_requires_live_surface_evidence")
        );
        let raw2 = r#"{"executorQa":{"contractCoverage":"x","surfaceEvidence":[{"kind":"cli","receipt":"abc"}],"adversarialCases":[],"artifactRefs":[],"iteration":1}}"#;
        assert!(validate_gate_json(raw2).is_ok());
    }
    #[test]
    fn quality_gate_rejects_unknown_keys() {
        let raw = r#"{"executorQa":{"contractCoverage":"x","surfaceEvidence":[{"kind":"cli","receipt":"r"}],"adversarialCases":[],"artifactRefs":[],"iteration":1,"unknownKey":1}}"#;
        assert!(
            validate_gate_json(raw)
                .unwrap_err()
                .contains("quality_gate_rejects_unknown_keys")
        );
    }
    #[test]
    fn terminal_critic_ceiling_fails_closed() {
        let mut s = RunState::new();
        for _ in 0..6 {
            s.record_verdict(false);
        }
        assert!(s.paused);
    }
    #[test]
    fn blocker_triage_resolvable_never_pauses() {
        let mut s = RunState::new();
        s.triage(BlockerKind::Resolvable);
        assert!(!s.paused);
        s.triage(BlockerKind::HumanBlocked);
        assert!(s.paused);
    }
    #[test]
    fn steering_keeps_aggregate_immutable() {
        let agg = Aggregate {
            objective: "obj".into(),
            brief_hash: "h".into(),
        };
        let out: Aggregate = apply_steering(&agg, SteeringKind::Revise);
        assert_eq!(out.objective, "obj");
    }
    #[test]
    fn big_scope_mandates_delegation() {
        assert!(requires_delegation(3, 10, false));
        assert!(requires_delegation(1, 200, false));
        assert!(requires_delegation(1, 10, true));
        assert!(!requires_delegation(1, 10, false));
    }
    #[test]
    fn approved_plan_to_checkpointed_execution() {
        let goals = parse_brief("@goal: G001\nbody one\n@goal: G002\nbody two");
        assert_eq!(goals.len(), 2);
        let raw = r#"{"executorQa":{"contractCoverage":"x","surfaceEvidence":[{"kind":"api","receipt":"tok"}],"adversarialCases":[],"artifactRefs":[],"iteration":1}}"#;
        assert!(validate_gate_json(raw).is_ok());
        let mut run = RunState::new();
        run.triage(BlockerKind::HumanBlocked);
        assert!(run.paused);
    }

    // ── the R39 battery ───────────────────────────────────────────────────

    /// Anti-vacuous source scan: the scan is scoped to the PRODUCTION region
    /// only — everything before the `#[cfg(test)]` marker.
    ///
    /// Scoping is not decoration. A whole-file `contains("#![forbid(…)]")`
    /// passes on THIS TEST'S OWN LITERAL STRING: red-proofing it by rewriting
    /// the attribute to `allow` left the string in the assertion body and the
    /// pin went green on a crate with no `forbid` at all. That is the R38
    /// `blast_radius` failure mode (a guard that scanned its own fixture), and
    /// it is why the region is cut before the assertion is made.
    #[test]
    fn crate_forbids_unsafe_code() {
        let src = include_str!("../src/lib.rs");
        let production = src.split_once("#[cfg(test)]").map_or(src, |(head, _)| head);
        assert!(
            production.contains("#![forbid(unsafe_code)]"),
            "the executor core must carry the un-overridable lint at the crate root — \
             `forbid` cannot be silenced downstream"
        );
        assert!(
            production.trim_start().starts_with("//!"),
            "the crate must open with a //! header naming what the core is and is not"
        );
    }

    /// The critic ceiling is NAMED and lands where the design owner says it
    /// lands: "5 → pause". The prior code compared `> 5` against a bare inline
    /// literal, so the sixth non-okay verdict tripped it — one verdict later
    /// than the governing text, and with nothing pinning the boundary.
    #[test]
    fn critic_ceiling_is_named_and_pinned() {
        assert_eq!(
            CRITIC_CEILING, 5,
            "the design owner states the critic ceiling as 5; a change moves the \
             ceiling AND the pause rule together"
        );
        let mut at_four = RunState::new();
        for _ in 0..4 {
            at_four.record_verdict(false);
        }
        assert!(
            !at_four.paused,
            "four non-okay verdicts are under the ceiling"
        );

        let mut at_five = RunState::new();
        for _ in 0..5 {
            at_five.record_verdict(false);
        }
        assert!(
            at_five.paused,
            "the FIFTH non-okay verdict reaches the ceiling and pauses"
        );
    }

    /// A known answer, not self-consistency: the digest of a fixed byte string
    /// against the value computed out-of-band. A pin that only compared
    /// `artifact_hash(x)` to `artifact_hash(x)` would pass with any function.
    #[test]
    fn artifact_hash_is_a_known_answer() {
        assert_eq!(
            artifact_hash("delivery"),
            "b4af39d5b65a14849e885a9d65f0efe4f4e689989689c28c16cfcb3a6e78db5a",
            "sha256(\"delivery\") — the empty string's known answer below pins the \
             length-extension/empty-input edge"
        );
        assert_eq!(
            artifact_hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "sha256 of the empty string is the canonical edge case"
        );
    }

    /// The TOP-LEVEL unknown-key path. The nested `executorQa` path was
    /// covered; the outer allowlist check had no test at all, so a regression
    /// that dropped the top-level loop would have gone green.
    #[test]
    fn gate_rejects_unknown_keys_at_top_level() {
        let raw = r#"{"executorQa":{"contractCoverage":"x","surfaceEvidence":[{"kind":"cli","receipt":"r"}],"adversarialCases":[],"artifactRefs":[],"iteration":1},"unknownTopLevel":1}"#;
        let err = validate_gate_json(raw).unwrap_err();
        assert!(
            err.contains("quality_gate_rejects_unknown_keys"),
            "an unknown TOP-LEVEL gate key must be refused: {err}"
        );
        assert!(
            err.contains("unknownTopLevel"),
            "the refusal names the offending key: {err}"
        );
    }

    /// `replayExempt` is accepted as a nested QA key but `ExecutorQa` has no
    /// such field, and nothing uses `deny_unknown_fields` — so it was silently
    /// dropped while the top-level `replay_exempt` stayed false. That is a
    /// false exemption: the caller believes it is exempt and the gate still
    /// refuses. The nested key is now REFUSED rather than dropped.
    #[test]
    fn nested_replay_exempt_is_refused_not_silently_dropped() {
        let raw = r#"{"executorQa":{"contractCoverage":"x","surfaceEvidence":[],"adversarialCases":[],"artifactRefs":[],"iteration":1,"replayExempt":true}}"#;
        let err = validate_gate_json(raw).unwrap_err();
        assert!(
            err.contains("quality_gate_rejects_unknown_keys"),
            "a nested replayExempt is not a QA field — it must be refused outright \
             rather than validated and dropped: {err}"
        );
    }

    /// The steering seam is INFALLIBLE by decision, and every variant is a
    /// declared no-op. The prior signature returned `Result` it could never
    /// fail, so a caller could not tell "no mutation needed" from "refused".
    /// The signature is now the honest one, and this pin fails if a future
    /// change re-wraps it in a `Result` without making a deliberate decision.
    #[test]
    fn steering_is_declared_infallible_and_a_no_op() {
        let agg = Aggregate {
            objective: "obj".into(),
            brief_hash: "h".into(),
        };
        for kind in [
            SteeringKind::Add,
            SteeringKind::Split,
            SteeringKind::Reorder,
            SteeringKind::Revise,
            SteeringKind::Annotate,
            SteeringKind::Supersede,
        ] {
            let out: Aggregate = apply_steering(&agg, kind);
            assert_eq!(out.objective, "obj", "steering never mutates the objective");
            assert_eq!(out.brief_hash, "h", "steering never mutates the brief hash");
        }
    }
}
