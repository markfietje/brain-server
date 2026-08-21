use serde::{Deserialize, Serialize};
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionPayload {
    pub question_id: String,
    pub text: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerPayload {
    pub question_id: String,
    pub answer: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultPayload {
    pub round_id: String,
    pub score: f64,
}
pub fn parse_question(s: &str) -> Result<QuestionPayload, String> {
    serde_json::from_str(s).map_err(|_| "DI_INVALID_QUESTION_JSON".to_string())
}
pub fn parse_answer(s: &str) -> Result<AnswerPayload, String> {
    serde_json::from_str(s).map_err(|_| "DI_INVALID_ANSWER_JSON".to_string())
}
pub fn parse_result(s: &str) -> Result<ResultPayload, String> {
    serde_json::from_str(s).map_err(|_| "DI_INVALID_RESULT_JSON".to_string())
}
