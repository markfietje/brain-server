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
    "replayExempt",
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
            if self.non_okay_count > 5 {
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

pub fn apply_steering(agg: &Aggregate, kind: SteeringKind) -> Result<Aggregate, String> {
    // aggregate immutable — steering never mutates it
    let _ = kind;
    Ok(agg.clone())
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
        let out = apply_steering(&agg, SteeringKind::Revise).unwrap();
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
}
