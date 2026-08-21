use crate::state::{InterviewEnvelope, InterviewError};
pub fn verify_and_apply(env: &mut InterviewEnvelope, round_id: &str, score: u32, expected_rev: u64)->Result<(),InterviewError>{
    if env.state.auto_answered_rounds >3 { return Err(InterviewError::Conflict("DI_ANSWER_LIFECYCLE_CONFLICT".into())); }
    crate::state::apply_round_result(env, round_id, score, expected_rev)
}
