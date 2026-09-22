//! The wizard pack core: schema-driven typed question sequences as DATA,
//! riding the shipped decide builders — never a chatbot, never free text.
//!
//! A pack is kernel-validatable JSON: every question parses through
//! [`crate::workflow::decide::sequence::parse_question`] (the closed
//! `choice | score | noul` types), the per-question option ceiling rides the
//! shipped [`crate::workflow::decide::sequence::validate_schema`] (so the
//! 20-option ceiling and its split-hierarchically demand hold verbatim —
//! the authorised splitter is `domain_router::split_for_decide`), and
//! branch-on-answer lives IN the pack (`next`: every answer maps to a
//! question id or the terminal `"end"` — total maps, no implicit
//! fallthrough, every path terminates). Answers assemble into ONE typed
//! case naming its origin channel; anything ambiguous or unanswered
//! ABSTAINS and is never invented. The case lands through the existing
//! `/webhooks/channel/{kind}` seam (the seam is mandatory, the channel
//! optional) — this module only mints the case value. The renderer and the
//! interaction telemetry are GUI-owned (the SvelteTauri shell plan).

use crate::workflow::decide::sequence::{
    Criteria, TypedQuestion, parse_question, render_options, validate_schema,
};

/// The closed origin-channel vocabulary the assembled case names.
pub(crate) const ORIGIN_CHANNELS: &[&str] = &[
    "email",
    "web_form",
    "chat",
    "crm_event",
    "wizard",
    "ingested",
];

/// The terminal branch target: the sequence ends here.
const END: &str = "end";

/// One pack question: its typed body (the decide machinery's own type) plus
/// the closed answer→target branch map.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WizardQuestion {
    pub id: String,
    pub question: TypedQuestion,
    /// Every answer key maps to a question id or [`END`]; the key set is
    /// exactly the question's answer vocabulary.
    pub next: Vec<(String, String)>,
}

impl WizardQuestion {
    fn answer_keys(&self) -> Vec<String> {
        match &self.question.criteria {
            Criteria::ChoiceMap(map) => map.iter().map(|(k, _)| k.clone()).collect(),
            Criteria::ScoreList(levels) => (0..levels.len()).map(|i| i.to_string()).collect(),
            Criteria::NoulPair { .. } => vec!["false".to_string(), "true".to_string()],
        }
    }

    fn target_of(&self, key: &str) -> Option<&str> {
        self.next
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, t)| t.as_str())
    }
}

/// A validated wizard pack.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WizardPack {
    pub pack: String,
    pub first: String,
    pub questions: Vec<WizardQuestion>,
}

impl WizardPack {
    fn question(&self, id: &str) -> Option<&WizardQuestion> {
        self.questions.iter().find(|q| q.id == id)
    }
}

fn refuse(reason: impl std::fmt::Display) -> String {
    format!("wizard_pack_invalid: {reason}")
}

/// The total pack validator: any input yields the pack or a
/// `wizard_pack_invalid: <reason>` refusal, never a panic. Rides the
/// shipped decide machinery for everything typed.
pub(crate) fn validate_wizard_pack(value: &serde_json::Value) -> Result<WizardPack, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| refuse("pack must be a JSON object"))?;
    let pack = obj
        .get("pack")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .ok_or_else(|| refuse("pack id must be a non-empty string of at most 64 chars"))?;
    let first = obj
        .get("first")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .ok_or_else(|| refuse("first must be a non-empty string of at most 64 chars"))?;
    let raw_questions = obj
        .get("questions")
        .and_then(serde_json::Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| refuse("questions must be a non-empty array"))?;

    let mut questions = Vec::with_capacity(raw_questions.len());
    let mut ids = std::collections::BTreeSet::new();
    for raw in raw_questions {
        let qobj = raw
            .as_object()
            .ok_or_else(|| refuse("question must be a JSON object"))?;
        let id = qobj
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 64)
            .ok_or_else(|| refuse("question id must be a non-empty string of at most 64 chars"))?;
        if !ids.insert(id.to_string()) {
            return Err(refuse(format!("duplicate question id {id}")));
        }
        // The typed body: the decide machinery's own parser — closed types
        // only, everything else names its refusal through here.
        let question = parse_question(raw).map_err(|e| refuse(format!("question {id}: {e}")))?;
        let next_map = qobj
            .get("next")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| refuse(format!("question {id}: next must be an object")))?;
        let probe = WizardQuestion {
            id: id.to_string(),
            question,
            next: Vec::new(),
        };
        // Total branch map: the key set is EXACTLY the answer vocabulary.
        let mut next = Vec::with_capacity(next_map.len());
        for key in probe.answer_keys() {
            let target = next_map
                .get(&key)
                .and_then(serde_json::Value::as_str)
                .filter(|t| !t.is_empty() && (*t == END || t.len() <= 64))
                .ok_or_else(|| {
                    refuse(format!(
                        "question {id}: next must map answer {key:?} to a question id or \"{END}\""
                    ))
                })?;
            next.push((key, target.to_string()));
        }
        if next.len() != next_map.len() {
            return Err(refuse(format!(
                "question {id}: next carries keys outside the answer vocabulary"
            )));
        }
        questions.push(WizardQuestion {
            id: id.to_string(),
            question: probe.question,
            next,
        });
    }

    // The per-question option ceiling rides the shipped validate_schema
    // (empty schema, zero-option and over-20-option refusals included).
    let schema: Vec<(String, TypedQuestion)> = questions
        .iter()
        .map(|q| (q.id.clone(), q.question.clone()))
        .collect();
    validate_schema(&schema).map_err(refuse)?;

    // Branch targets must exist.
    for q in &questions {
        for (_, target) in &q.next {
            if target != END && !ids.contains(target) {
                return Err(refuse(format!(
                    "question {}: branch target {target:?} does not exist",
                    q.id
                )));
            }
        }
    }

    // Every non-terminal target reachable from `first`, and no cycles:
    // every path terminates. DFS colors: absent = unvisited, 1 = on the
    // walk's stack (a revisit is a cycle), 2 = fully explored.
    let mut color: std::collections::BTreeMap<String, u8> = std::collections::BTreeMap::new();
    fn visit(
        id: &str,
        questions: &[WizardQuestion],
        color: &mut std::collections::BTreeMap<String, u8>,
    ) -> Result<(), String> {
        match color.get(id) {
            Some(&2) => return Ok(()),
            Some(&1) => return Err(refuse(format!("branch cycle through {id}"))),
            _ => {}
        }
        color.insert(id.to_string(), 1);
        let q = questions
            .iter()
            .find(|q| q.id == id)
            .ok_or_else(|| refuse(format!("first names absent question {id}")))?;
        for (_, target) in &q.next {
            if target != END {
                visit(target, questions, color)?;
            }
        }
        color.insert(id.to_string(), 2);
        Ok(())
    }
    visit(first, &questions, &mut color)?;
    for q in &questions {
        if !color.contains_key(&q.id) {
            return Err(refuse(format!(
                "question {} is unreachable from first",
                q.id
            )));
        }
    }

    Ok(WizardPack {
        pack: pack.to_string(),
        first: first.to_string(),
        questions,
    })
}

/// One typed answer key: the vocabulary key the branch map indexes by, plus
/// the rendered (typed, free-text-free) answer text.
fn classify_answer(
    q: &WizardQuestion,
    given: Option<&serde_json::Value>,
) -> Option<(String, String)> {
    let options = render_options(&q.question);
    match &q.question.criteria {
        Criteria::ChoiceMap(map) => {
            let label = given?.as_str()?;
            if !map.iter().any(|(k, _)| k == label) {
                return None;
            }
            // The typed choice value is the label itself.
            Some((label.to_string(), label.to_string()))
        }
        Criteria::ScoreList(_) => {
            let idx = given?.as_u64()? as usize;
            if idx >= options.len() {
                return None;
            }
            Some((idx.to_string(), options[idx].clone()))
        }
        Criteria::NoulPair { .. } => {
            let flag = given?.as_bool()?;
            let idx = usize::from(flag);
            Some((flag.to_string(), options[idx].clone()))
        }
    }
}

/// Assemble typed answers into the ONE typed case: origin-named, ordered
/// along the branch the answers drive, abstaining (never inventing) on
/// anything ambiguous or unanswered. The result lands through the existing
/// `/webhooks/channel/{kind}` seam — this fn only mints the value.
pub(crate) fn assemble_answers(
    pack: &WizardPack,
    answers: &serde_json::Value,
    origin: &str,
) -> Result<serde_json::Value, String> {
    if !ORIGIN_CHANNELS.contains(&origin) {
        return Err(refuse(
            "origin channel must be one of email | web_form | chat | crm_event | wizard | ingested",
        ));
    }
    let map = answers.as_object();
    let mut typed = Vec::new();
    let mut abstained = Vec::new();
    let mut current = Some(pack.first.clone());
    let mut steps = 0usize;
    while let Some(qid) = current {
        steps += 1;
        if steps > pack.questions.len() + 1 {
            // Defensive: the validator refuses cycles; the walk never
            // spins even on a pack that bypassed it.
            return Err(refuse("sequence walk exceeded the question count"));
        }
        let q = pack
            .question(&qid)
            .ok_or_else(|| refuse(format!("branch target {qid:?} does not exist")))?;
        match classify_answer(q, map.and_then(|m| m.get(&qid))) {
            Some((key, text)) => {
                typed.push(serde_json::json!({ "question": qid, "answer": text }));
                let target = q.target_of(&key).ok_or_else(|| {
                    refuse(format!("question {qid}: answer {key:?} has no branch"))
                })?;
                current = if target == END {
                    None
                } else {
                    Some(target.to_string())
                };
            }
            None => {
                // Ambiguous or unanswered: abstain, never invent — and the
                // walk ends rather than guessing a branch.
                abstained.push(qid);
                current = None;
            }
        }
    }
    Ok(serde_json::json!({
        "origin": origin,
        "pack": pack.pack,
        "abstain": !abstained.is_empty(),
        "answers": typed,
        "abstained": abstained,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::decide::sequence::QType;

    fn branched_pack_value() -> serde_json::Value {
        serde_json::json!({
            "pack": "triage-demo",
            "first": "q_class",
            "questions": [
                {"id": "q_class", "qtype": "choice",
                 "instructions": "Which class does this belong to?",
                 "criteria": {"billing": "payment, invoice, refund", "technical": "device, software, connectivity"},
                 "next": {"billing": "q_confirm", "technical": "q_repro"}},
                {"id": "q_repro", "qtype": "noul",
                 "instructions": "The issue reproduces on demand.",
                 "criteria": {"false": "no, intermittent", "true": "yes, on demand"},
                 "next": {"false": "end", "true": "end"}},
                {"id": "q_confirm", "qtype": "score",
                 "instructions": "How urgent is this?",
                 "criteria": ["low", "high"],
                 "next": {"0": "end", "1": "end"}}
            ]
        })
    }

    #[test]
    fn wizard_pack_schema_validates_against_closed_question_types() {
        let pack = validate_wizard_pack(&branched_pack_value()).unwrap();
        assert_eq!(pack.pack, "triage-demo");
        assert_eq!(pack.questions.len(), 3);
        // The typed bodies are the decide machinery's own values.
        assert_eq!(pack.questions[0].question.qtype, QType::Choice);
        assert_eq!(pack.questions[1].question.qtype, QType::Noul);
        assert_eq!(pack.questions[2].question.qtype, QType::Score);
    }

    #[test]
    fn wizard_pack_ratification_rejects_unknown_question_kinds() {
        let mut bad = branched_pack_value();
        bad["questions"][0]["qtype"] = serde_json::json!("essay");
        let err = validate_wizard_pack(&bad).unwrap_err();
        assert!(err.starts_with("wizard_pack_invalid: "), "{err}");
        assert!(err.contains("unknown question type"), "{err}");
    }

    #[test]
    fn wizard_pack_refuses_the_20_option_ceiling_and_structural_faults() {
        // Over the ceiling: the shipped split-hierarchically demand holds.
        let mut options = serde_json::Map::new();
        for i in 0..21 {
            options.insert(format!("opt{i}"), serde_json::Value::Null);
        }
        let mut bad = branched_pack_value();
        bad["questions"][0]["criteria"] = serde_json::Value::Object(options);
        let next: serde_json::Map<String, serde_json::Value> = (0..21)
            .map(|i| (format!("opt{i}"), serde_json::json!("end")))
            .collect();
        bad["questions"][0]["next"] = serde_json::Value::Object(next);
        let err = validate_wizard_pack(&bad).unwrap_err();
        assert!(err.contains("exceeds 20 options"), "{err}");
        assert!(err.contains("split hierarchically"), "{err}");

        // Structural faults: bad branch target, unreachable question,
        // cycle, incomplete next map, missing next.
        let mut bad = branched_pack_value();
        bad["questions"][0]["next"]["billing"] = serde_json::json!("q_absent");
        assert!(
            validate_wizard_pack(&bad)
                .unwrap_err()
                .contains("does not exist")
        );

        let mut bad = branched_pack_value();
        bad["questions"][2]["next"] = serde_json::json!({"0": "end", "1": "end", "2": "end"});
        assert!(
            validate_wizard_pack(&bad)
                .unwrap_err()
                .contains("outside the answer vocabulary")
        );

        let mut bad = branched_pack_value();
        bad["first"] = serde_json::json!("q_confirm");
        // q_class becomes unreachable from the new first.
        bad["questions"][2]["next"] = serde_json::json!({"0": "end", "1": "end"});
        assert!(
            validate_wizard_pack(&bad)
                .unwrap_err()
                .contains("unreachable from first")
        );

        let mut bad = branched_pack_value();
        bad["questions"][0]["next"]["billing"] = serde_json::json!("q_class");
        let err = validate_wizard_pack(&bad).unwrap_err();
        assert!(err.contains("cycle"), "{err}");

        let mut bad = branched_pack_value();
        bad["questions"][0].as_object_mut().unwrap().remove("next");
        assert!(
            validate_wizard_pack(&bad)
                .unwrap_err()
                .contains("next must be an object")
        );

        for structural in [
            serde_json::json!(null),
            serde_json::json!([]),
            serde_json::json!({"pack": "p"}),
            serde_json::json!({"pack": "p", "first": "q", "questions": []}),
        ] {
            let err = validate_wizard_pack(&structural).unwrap_err();
            assert!(err.starts_with("wizard_pack_invalid: "), "{err}");
        }
    }

    #[test]
    fn wizard_answer_assembly_names_origin_channel() {
        let pack = validate_wizard_pack(&branched_pack_value()).unwrap();
        let answers = serde_json::json!({"q_class": "technical", "q_repro": true});
        let case = assemble_answers(&pack, &answers, "wizard").unwrap();
        assert_eq!(case["origin"], "wizard");
        assert_eq!(case["pack"], "triage-demo");
        // The closed vocabulary: unknown origins refuse.
        let err = assemble_answers(&pack, &answers, "carrier-pigeon").unwrap_err();
        assert!(err.starts_with("wizard_pack_invalid: "), "{err}");
        assert!(err.contains("origin channel"), "{err}");
        for channel in ORIGIN_CHANNELS {
            let case = assemble_answers(&pack, &answers, channel).unwrap();
            assert_eq!(case["origin"], *channel);
        }
    }

    #[test]
    fn wizard_ambiguous_or_unanswered_abstains_never_invents() {
        let pack = validate_wizard_pack(&branched_pack_value()).unwrap();
        // Unanswered: the reached first question abstains, nothing is
        // invented, the case is marked.
        let case = assemble_answers(&pack, &serde_json::json!({}), "wizard").unwrap();
        assert_eq!(case["abstain"], true);
        assert_eq!(case["abstained"], serde_json::json!(["q_class"]));
        assert_eq!(case["answers"], serde_json::json!([]));
        // Out-of-vocabulary label: abstain, never a default.
        let case =
            assemble_answers(&pack, &serde_json::json!({"q_class": "legal"}), "wizard").unwrap();
        assert_eq!(case["abstained"], serde_json::json!(["q_class"]));
        // Wrong type on the score question: abstain.
        let case = assemble_answers(
            &pack,
            &serde_json::json!({"q_class": "billing", "q_confirm": "very"}),
            "wizard",
        )
        .unwrap();
        assert_eq!(case["abstained"], serde_json::json!(["q_confirm"]));
        // Out-of-range index: abstain.
        let case = assemble_answers(
            &pack,
            &serde_json::json!({"q_class": "billing", "q_confirm": 9}),
            "wizard",
        )
        .unwrap();
        assert_eq!(case["abstained"], serde_json::json!(["q_confirm"]));
        // Non-object answers: everything reachable abstains.
        let case = assemble_answers(&pack, &serde_json::json!("nonsense"), "wizard").unwrap();
        assert_eq!(case["abstain"], true);
        assert_eq!(case["answers"], serde_json::json!([]));
    }

    #[test]
    fn wizard_answers_land_as_one_typed_case() {
        let pack = validate_wizard_pack(&branched_pack_value()).unwrap();
        let case = assemble_answers(
            &pack,
            &serde_json::json!({"q_class": "billing", "q_confirm": 1}),
            "web_form",
        )
        .unwrap();
        // ONE object: the whole case lands as a value through the seam.
        assert!(case.is_object());
        assert_eq!(case["abstain"], false);
        assert_eq!(case["abstained"], serde_json::json!([]));
        assert_eq!(
            case["answers"],
            serde_json::json!([
                {"question": "q_class", "answer": "billing"},
                {"question": "q_confirm", "answer": "level 1: high"}
            ])
        );
        // Every answer text is typed — the noul pair renders its option.
        let case = assemble_answers(
            &pack,
            &serde_json::json!({"q_class": "technical", "q_repro": true}),
            "wizard",
        )
        .unwrap();
        assert_eq!(
            case["answers"],
            serde_json::json!([
                {"question": "q_class", "answer": "technical"},
                {"question": "q_repro", "answer": "yes, on demand"}
            ])
        );
    }

    #[test]
    fn wizard_sequence_branching_drives_next_question_not_free_text() {
        let pack = validate_wizard_pack(&branched_pack_value()).unwrap();
        // The billing branch drives the sequence to the score question.
        let billing = assemble_answers(
            &pack,
            &serde_json::json!({"q_class": "billing", "q_confirm": 0}),
            "wizard",
        )
        .unwrap();
        let billed: Vec<&str> = billing["answers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["question"].as_str().unwrap())
            .collect();
        assert_eq!(billed, vec!["q_class", "q_confirm"]);
        // The technical branch drives it to the noul question instead —
        // the ANSWER drives the next question, no text is involved.
        let technical = assemble_answers(
            &pack,
            &serde_json::json!({"q_class": "technical", "q_repro": true}),
            "wizard",
        )
        .unwrap();
        let teched: Vec<&str> = technical["answers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["question"].as_str().unwrap())
            .collect();
        assert_eq!(teched, vec!["q_class", "q_repro"]);
        // The assembled case carries no free-text field anywhere.
        let rendered = technical.to_string();
        assert!(
            !rendered.contains("free_text"),
            "the case shape is closed: {rendered}"
        );
    }

    #[test]
    fn wizard_pack_support_ticket_ratified_pack_validates() {
        let pack = ratified_pack("support-ticket");
        assert_eq!(pack.pack, "support-ticket");
        // A scripted pass assembles end to end.
        let case = assemble_answers(
            &pack,
            &serde_json::json!({"q_class": "billing", "q_priority": 1, "q_known": false}),
            "wizard",
        )
        .unwrap();
        assert_eq!(case["abstain"], false);
        assert_eq!(case["answers"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn wizard_pack_tele_health_ratified_pack_validates() {
        let pack = ratified_pack("tele-health");
        let case = assemble_answers(
            &pack,
            &serde_json::json!({"q_consent": true, "q_modality": "video", "q_urgent": 0}),
            "web_form",
        )
        .unwrap();
        assert_eq!(case["abstain"], false);
        // The consent refusal branch terminates the sequence honestly.
        let declined =
            assemble_answers(&pack, &serde_json::json!({"q_consent": false}), "chat").unwrap();
        assert_eq!(declined["answers"].as_array().unwrap().len(), 1);
        assert_eq!(declined["abstain"], false);
    }

    #[test]
    fn wizard_pack_capture_pre_screen_ratified_pack_validates() {
        let pack = ratified_pack("capture-pre-screen");
        let case = assemble_answers(
            &pack,
            &serde_json::json!({"q_scope": "hardware", "q_repro": true, "q_severity": 2}),
            "ingested",
        )
        .unwrap();
        assert_eq!(case["abstain"], false);
        assert_eq!(case["answers"].as_array().unwrap().len(), 3);
        // The process branch skips the repro/severity questions.
        let process =
            assemble_answers(&pack, &serde_json::json!({"q_scope": "process"}), "wizard").unwrap();
        assert_eq!(process["answers"].as_array().unwrap().len(), 1);
    }

    fn ratified_pack(name: &str) -> WizardPack {
        let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        dir.push(format!(
            "crates/brain-fuzz/corpus/accounts/packs/{name}.json"
        ));
        let bytes = std::fs::read(&dir).unwrap_or_else(|e| panic!("ratified pack {name}: {e}"));
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        validate_wizard_pack(&value).unwrap_or_else(|e| panic!("ratified pack {name}: {e}"))
    }

    #[test]
    fn wizard_pack_validator_is_total_on_fuzz_shapes() {
        // Totality spot-checks: garbage shapes refuse by name, never panic.
        for bad in [
            serde_json::json!(42),
            serde_json::json!("pack"),
            serde_json::json!({"pack": "", "first": "q", "questions": [{"id": "q", "qtype": "noul", "next": {"false": "end", "true": "end"}}]}),
            serde_json::json!({"pack": "p", "first": "", "questions": [{"id": "q", "qtype": "noul", "next": {"false": "end", "true": "end"}}]}),
            serde_json::json!({"pack": "p", "first": "q", "questions": [{"id": "q", "qtype": "choice", "criteria": {}, "next": {}}]}),
            serde_json::json!({"pack": "p", "first": "q", "questions": [{"id": "q", "qtype": "noul", "next": {"false": "end"}}]}),
        ] {
            let err = validate_wizard_pack(&bad).unwrap_err();
            assert!(err.starts_with("wizard_pack_invalid: "), "{err}");
        }
    }
}
