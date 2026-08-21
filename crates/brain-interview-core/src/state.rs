use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq)]
pub enum InterviewError { Conflict(String), Invalid(String), LimitExceeded(String) }
impl std::fmt::Display for InterviewError { fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{match self{Self::Conflict(c)=>write!(f,"{}",c),Self::Invalid(c)=>write!(f,"{}",c),Self::LimitExceeded(c)=>write!(f,"{}",c)}}}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InterviewEnvelope { pub threshold: u32, pub threshold_units: Option<u32>, pub threshold_source: String, pub state: InterviewState }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InterviewState {
    #[serde(default)] pub rounds: Vec<Round>,
    #[serde(default)] pub established_facts: Vec<Fact>,
    #[serde(default)] pub current_ambiguity: u32,
    #[serde(default)] pub ambiguity_floor: u32,
    #[serde(default)] pub topology: Option<Topology>,
    #[serde(default)] pub state_revision: u64,
    #[serde(default)] pub auto_answered_rounds: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Round { pub id: String, pub question_id: String, pub lifecycle: String, #[serde(default)] pub answer: Option<String>, #[serde(default)] pub score: Option<u32> }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fact { pub id: String, pub disputed: bool, #[serde(default)] pub superseded_by: Option<String> }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Topology { pub components: Vec<String> }

pub fn validate_limits(env: &InterviewEnvelope) -> Result<(), InterviewError> {
    let s = serde_json::to_string(env).unwrap();
    if s.len() > 24*1024 { return Err(InterviewError::LimitExceeded("DI_OUTPUT_LIMIT_EXCEEDED".into())); }
    if env.state.rounds.len() > 64 { return Err(InterviewError::LimitExceeded("DI_OUTPUT_LIMIT_EXCEEDED".into())); }
    Ok(())
}
pub fn initialize_context(threshold: u32, source: &str) -> InterviewEnvelope {
    InterviewEnvelope{ threshold, threshold_units: Some(threshold), threshold_source: source.into(), state: InterviewState{ rounds: vec![], established_facts: vec![], current_ambiguity: 10000, ambiguity_floor: 0, topology: None, state_revision: 0, auto_answered_rounds: 0 } }
}
pub fn confirm_topology(env: &mut InterviewEnvelope, topo: Topology, expected_rev: u64) -> Result<(), InterviewError> {
    if env.state.state_revision != expected_rev { return Err(InterviewError::Conflict("DI_STATE_REVISION_CONFLICT".into())); }
    if env.state.topology.is_some() { return Err(InterviewError::Conflict("DI_TOPOLOGY_CONFLICT".into())); }
    env.state.topology = Some(topo); env.state.state_revision +=1; Ok(())
}
pub fn record_answer(env: &mut InterviewEnvelope, round: Round, expected_rev: u64) -> Result<(), InterviewError> {
    if env.state.state_revision != expected_rev { return Err(InterviewError::Conflict("DI_STATE_REVISION_CONFLICT".into())); }
    if env.state.rounds.iter().any(|r| r.id==round.id) { return Err(InterviewError::Conflict("DI_ANSWER_LIFECYCLE_CONFLICT".into())); }
    env.state.rounds.push(round); env.state.state_revision+=1; Ok(())
}
pub fn apply_round_result(env: &mut InterviewEnvelope, round_id: &str, score: u32, expected_rev: u64) -> Result<(), InterviewError> {
    if env.state.state_revision != expected_rev { return Err(InterviewError::Conflict("DI_STATE_REVISION_CONFLICT".into())); }
    let r = env.state.rounds.iter_mut().find(|r| r.id==round_id).ok_or(InterviewError::Conflict("DI_ROUND_NOT_FOUND".into()))?;
    if r.lifecycle != "answered" && r.lifecycle != "pending_scoring" { return Err(InterviewError::Conflict("DI_ROUND_RESULT_CONFLICT".into())); }
    r.score = Some(score); r.lifecycle="scored".into(); env.state.state_revision+=1; Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn rejects_stale_revision() {
        let mut e = initialize_context(3000,"native");
        confirm_topology(&mut e, Topology{components: vec!["a".into()]},0).unwrap();
        assert!(confirm_topology(&mut e, Topology{components: vec!["b".into()]},0).is_err());
    }
    #[test] fn rejects_oversized_state() {
        let mut e = initialize_context(1000,"native");
        e.state.rounds = (0..65).map(|i| Round{id:format!("r{i}"),question_id:"q".into(),lifecycle:"answered".into(),answer:None,score:None}).collect();
        assert!(validate_limits(&e).is_err());
    }
}
