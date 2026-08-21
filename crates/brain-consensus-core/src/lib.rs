use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_ITERATIONS: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    pub id: String,
    pub content: String,
    pub hash: String,
}

impl Artifact {
    pub fn new(id: impl Into<String>, content: impl Into<String>) -> Self {
        let id = id.into();
        let content = content.into();
        let hash = hex::encode(Sha256::digest(content.as_bytes()));
        Self { id, content, hash }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    pub reviewer: String,
    pub artifact_id: String,
    pub verdict: Verdict,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Approve,
    Revise,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConsensusStatus {
    InProgress { iteration: usize },
    Stuck,
    Approved,
}

#[derive(Debug, Clone)]
pub struct ConsensusState {
    pub artifact: Artifact,
    pub iteration: usize,
    pub status: ConsensusStatus,
    pub history: Vec<Artifact>,
    pub resume_lineage: Vec<String>,
}

impl ConsensusState {
    pub fn new(artifact: Artifact) -> Self {
        Self {
            artifact,
            iteration: 0,
            status: ConsensusStatus::InProgress { iteration: 0 },
            history: Vec::new(),
            resume_lineage: Vec::new(),
        }
    }
    pub fn with_lineage(mut self, lineage: Vec<String>) -> Self {
        self.resume_lineage = lineage;
        self
    }
}

pub fn review_join_gate(reviews: &[Review]) -> Result<(), String> {
    if reviews.len() < 2 {
        return Err("need_two_reviews".into());
    }
    let first = &reviews[0].artifact_id;
    if !reviews.iter().all(|r| &r.artifact_id == first) {
        return Err("review_join_gate_requires_same_artifact".into());
    }
    Ok(())
}

pub fn advance(
    mut state: ConsensusState,
    reviews: Vec<Review>,
    next_artifact: Option<Artifact>,
) -> Result<ConsensusState, String> {
    if state.status == ConsensusStatus::Stuck || state.status == ConsensusStatus::Approved {
        return Err("terminal".into());
    }
    review_join_gate(&reviews)?;
    let needs_revision = reviews.iter().any(|r| r.verdict == Verdict::Revise);
    if !needs_revision {
        state.status = ConsensusStatus::Approved;
        return Ok(state);
    }
    if state.iteration + 1 >= MAX_ITERATIONS {
        state.status = ConsensusStatus::Stuck;
        state.iteration += 1;
        return Ok(state);
    }
    let next = next_artifact.ok_or("revision_required")?;
    state.history.push(state.artifact);
    state.artifact = next;
    state.iteration += 1;
    state.status = ConsensusStatus::InProgress {
        iteration: state.iteration,
    };
    Ok(state)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageFile {
    pub name: String,
    pub content: String,
    pub sha256: String,
}

pub fn stage_writer(artifacts: &[Artifact], kinds: &[&str]) -> (Vec<StageFile>, String) {
    let mut files = Vec::new();
    let mut index_lines = Vec::new();
    for (i, (art, kind)) in artifacts.iter().zip(kinds.iter()).enumerate() {
        let name = format!("stage-{:02}-{}.md", i + 1, kind);
        let sha256 = hex::encode(Sha256::digest(art.content.as_bytes()));
        let content = format!("# {}\n\n{}\n", art.id, art.content);
        index_lines.push(format!(
            "{{\"name\":\"{}\",\"sha256\":\"{}\"}}",
            name, sha256
        ));
        files.push(StageFile {
            name,
            content,
            sha256,
        });
    }
    let pending = StageFile {
        name: "pending-approval.md".into(),
        content: artifacts
            .last()
            .map(|a| a.content.clone())
            .unwrap_or_default(),
        sha256: artifacts
            .last()
            .map(|a| hex::encode(Sha256::digest(a.content.as_bytes())))
            .unwrap_or_default(),
    };
    files.push(pending);
    (files, index_lines.join("\n"))
}

pub fn intent_reconciliation(
    spec_hash: &str,
    prior_hashes: &[String],
    confirmed: bool,
) -> Result<(), String> {
    if prior_hashes.contains(&spec_hash.to_string()) {
        return Ok(());
    }
    if !confirmed {
        return Err("intent_reconciliation_requires_explicit_confirm".into());
    }
    Ok(())
}

pub fn approval_gate(status: &ConsensusStatus) -> bool {
    *status == ConsensusStatus::Approved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(id: &str) -> Artifact {
        Artifact::new(id, format!("content-{}", id))
    }

    #[test]
    fn review_join_gate_requires_same_artifact() {
        let a1 = art("a1");
        let a2 = art("a2");
        let reviews = vec![
            Review {
                reviewer: "architect".into(),
                artifact_id: a1.id.clone(),
                verdict: Verdict::Approve,
                notes: "".into(),
            },
            Review {
                reviewer: "critic".into(),
                artifact_id: a2.id.clone(),
                verdict: Verdict::Approve,
                notes: "".into(),
            },
        ];
        assert!(review_join_gate(&reviews).is_err());
    }

    #[test]
    fn max5_cap_fails_closed_planning_stuck() {
        let mut state = ConsensusState::new(art("v0"));
        for i in 0..5 {
            let cur_id = state.artifact.id.clone();
            let reviews = vec![
                Review {
                    reviewer: "architect".into(),
                    artifact_id: cur_id.clone(),
                    verdict: Verdict::Revise,
                    notes: "".into(),
                },
                Review {
                    reviewer: "critic".into(),
                    artifact_id: cur_id,
                    verdict: Verdict::Revise,
                    notes: "".into(),
                },
            ];
            let next = Artifact::new(format!("v{}", i + 1), "next");
            state = advance(state, reviews, Some(next)).unwrap();
        }
        assert_eq!(state.status, ConsensusStatus::Stuck);
    }

    #[test]
    fn persisted_planner_resumes_with_consolidated_feedback() {
        let lineage = vec![
            "rev1: architect feeedback".into(),
            "rev1: critic feedback".into(),
        ];
        let state = ConsensusState::new(art("v1")).with_lineage(lineage.clone());
        assert_eq!(state.resume_lineage, lineage);
    }

    #[test]
    fn stage_writer_emits_receipt() {
        let arts = vec![art("a1"), art("a2")];
        let (files, index) = stage_writer(&arts, &["planner", "architect"]);
        assert!(files.iter().any(|f| f.sha256.len() == 64));
        assert!(index.contains("stage-01-planner.md"));
    }

    #[test]
    fn stage_writer_deterministic_index() {
        let arts = vec![art("a1")];
        let (_, idx1) = stage_writer(&arts, &["planner"]);
        let (_, idx2) = stage_writer(&arts, &["planner"]);
        assert_eq!(idx1, idx2);
    }

    #[test]
    fn intent_reconciliation_requires_explicit_confirm() {
        let err = intent_reconciliation("newhash", &["oldhash".into()], false).unwrap_err();
        assert!(err.contains("intent_reconciliation_requires_explicit_confirm"));
        assert!(intent_reconciliation("newhash", &["oldhash".into()], true).is_ok());
    }

    #[test]
    fn plan_approval_gate_blocks_execution() {
        assert!(!approval_gate(&ConsensusStatus::InProgress {
            iteration: 0
        }));
        assert!(approval_gate(&ConsensusStatus::Approved));
        assert!(!approval_gate(&ConsensusStatus::Stuck));
    }

    #[test]
    fn spec_to_approved_plan_end_to_end() {
        let mut state = ConsensusState::new(art("spec-1"));
        let cur = state.artifact.id.clone();
        let reviews = vec![
            Review {
                reviewer: "architect".into(),
                artifact_id: cur.clone(),
                verdict: Verdict::Approve,
                notes: "".into(),
            },
            Review {
                reviewer: "critic".into(),
                artifact_id: cur,
                verdict: Verdict::Approve,
                notes: "".into(),
            },
        ];
        state = advance(state, reviews, None).unwrap();
        assert!(approval_gate(&state.status));
        intent_reconciliation("spec-1-hash", &[], true).unwrap();
    }
}
