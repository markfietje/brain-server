//! The question schema + sequence builder — the pure port of the laya
//! `common.py` render/build contract (LAYA_RUST_PORT §0.3/§2.3): closed
//! question types, deterministic option rendering, the head/state budget
//! arithmetic, and the hard 20-option ceiling (no bypass flag). The real
//! tokenizer rides the [`TokenizerLike`] seam; tests run on a fixed fake.

/// The three closed question types (the source's `QTYPES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QType {
    Choice = 0,
    Score = 1,
    Noul = 2,
}

impl QType {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            QType::Choice => "choice",
            QType::Score => "score",
            QType::Noul => "noul",
        }
    }
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "choice" => Ok(QType::Choice),
            "score" => Ok(QType::Score),
            "noul" => Ok(QType::Noul),
            other => Err(format!(
                "unknown question type: {other} — expected choice | score | noul"
            )),
        }
    }
}

/// The per-type answer vocabulary: a choice map (label → optional
/// criterion), a score ladder, or the noul pair.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Criteria {
    ChoiceMap(Vec<(String, Option<String>)>),
    ScoreList(Vec<String>),
    NoulPair {
        false_opt: Option<String>,
        true_opt: Option<String>,
    },
}

/// One typed question: instructions + its closed answer vocabulary.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TypedQuestion {
    pub qtype: QType,
    pub instructions: String,
    pub criteria: Criteria,
}

impl TypedQuestion {
    pub(crate) fn choice(instructions: &str, options: &[(&str, Option<&str>)]) -> Self {
        Self {
            qtype: QType::Choice,
            instructions: instructions.to_string(),
            criteria: Criteria::ChoiceMap(
                options
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).map(str::to_string)))
                    .collect(),
            ),
        }
    }
    pub(crate) fn score(instructions: &str, levels: &[&str]) -> Self {
        Self {
            qtype: QType::Score,
            instructions: instructions.to_string(),
            criteria: Criteria::ScoreList(levels.iter().map(|s| (*s).to_string()).collect()),
        }
    }
    pub(crate) fn noul(instructions: &str) -> Self {
        Self {
            qtype: QType::Noul,
            instructions: instructions.to_string(),
            criteria: Criteria::NoulPair {
                false_opt: None,
                true_opt: None,
            },
        }
    }
}

/// The 20-option ceiling — the schema never widens past what one
/// checkpoint head can score; anything larger must split hierarchically
/// (the authorised splitter is `domain_router::split_for_decide`).
pub(crate) const MAX_OPTIONS: usize = 20;

/// Parse a question from its JSON shape (total: any input yields the
/// question or a named refusal, never a panic).
pub(crate) fn parse_question(value: &serde_json::Value) -> Result<TypedQuestion, String> {
    let Some(obj) = value.as_object() else {
        return Err("question must be a JSON object".into());
    };
    let qtype_raw = obj
        .get("qtype")
        .and_then(serde_json::Value::as_str)
        .ok_or("question requires a `qtype` string")?;
    let qtype = QType::parse(qtype_raw)?;
    let instructions = obj
        .get("instructions")
        .map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => render_criterion(other),
        })
        .unwrap_or_default();
    let criteria = match qtype {
        QType::Choice => {
            let Some(map) = obj.get("criteria").and_then(serde_json::Value::as_object) else {
                return Err("choice questions require a `criteria` object".into());
            };
            Criteria::ChoiceMap(
                map.iter()
                    .map(|(k, v)| {
                        let criterion = match v {
                            serde_json::Value::Null => None,
                            serde_json::Value::String(s) if s.is_empty() => None,
                            other => Some(render_criterion(other)),
                        };
                        (k.clone(), criterion)
                    })
                    .collect(),
            )
        }
        QType::Score => {
            let Some(list) = obj.get("criteria").and_then(serde_json::Value::as_array) else {
                return Err("score questions require a `criteria` array".into());
            };
            Criteria::ScoreList(list.iter().map(render_criterion).collect())
        }
        QType::Noul => {
            let (false_opt, true_opt) = match obj.get("criteria") {
                Some(serde_json::Value::Object(map)) => (
                    map.get("false").and_then(as_optional_text),
                    map.get("true").and_then(as_optional_text),
                ),
                _ => (None, None),
            };
            Criteria::NoulPair {
                false_opt,
                true_opt,
            }
        }
    };
    Ok(TypedQuestion {
        qtype,
        instructions,
        criteria,
    })
}

fn as_optional_text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::String(_) => None,
        other => Some(render_criterion(other)),
    }
}

/// Compact JSON rendering with the source's separators (`, ` and `: `);
/// strings pass through verbatim.
pub(crate) fn render_criterion(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(render_criterion).collect();
            format!("[{}]", inner.join(", "))
        }
        serde_json::Value::Object(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", render_key(k), render_criterion(v)))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        serde_json::Value::Null => "null".into(),
        other => other.to_string(),
    }
}

fn render_key(k: &str) -> String {
    let mut out = String::with_capacity(k.len() + 2);
    out.push('"');
    for c in k.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The answer options exactly as the head scores them: choice labels with
/// their criteria, the indexed score ladder, or the noul pair (with the
/// source's default phrasing).
pub(crate) fn render_options(q: &TypedQuestion) -> Vec<String> {
    match &q.criteria {
        Criteria::ChoiceMap(options) => options
            .iter()
            .map(|(k, v)| match v {
                None => k.clone(),
                Some(crit) if crit.is_empty() => k.clone(),
                Some(crit) => format!("{k}: {crit}"),
            })
            .collect(),
        Criteria::ScoreList(levels) => levels
            .iter()
            .enumerate()
            .map(|(i, c)| format!("level {i}: {c}"))
            .collect(),
        Criteria::NoulPair {
            false_opt,
            true_opt,
        } => vec![
            false_opt
                .clone()
                .unwrap_or_else(|| "no, the statement does not hold".into()),
            true_opt
                .clone()
                .unwrap_or_else(|| "yes, the statement holds".into()),
        ],
    }
}

/// The state serializer: a string passes through verbatim; anything else
/// renders as compact JSON (the model text never carries whitespace noise).
pub(crate) fn serialize_state(s: &serde_json::Value) -> String {
    match s {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The tokenizer seam: the fixed-vocabulary fake in tests; the real
/// `tokenizers::Tokenizer` implementation lands with the Phase 1 feature.
pub(crate) trait TokenizerLike {
    fn cls_id(&self) -> i64;
    fn sep_id(&self) -> i64;
    fn mask_id(&self) -> i64;
    fn mask_str(&self) -> &str;
    /// Encode WITHOUT special tokens (the caller assembles the frame).
    fn encode_nospecial(&self, text: &str) -> Vec<i64>;
}

/// The built input: token ids + the marker positions the head scores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuiltSequence {
    pub ids: Vec<i64>,
    pub markers: Vec<usize>,
}

/// The schema gate: non-empty, every question within the 20-option
/// ceiling. This is the level the named `decide_refuses_over_20` test
/// pins; `build_sequence` enforces the same ceiling defensively.
pub(crate) fn validate_schema(questions: &[(String, TypedQuestion)]) -> Result<(), String> {
    if questions.is_empty() {
        return Err("schema requires at least one question".into());
    }
    for (id, q) in questions {
        let options = render_options(q).len();
        if options > MAX_OPTIONS {
            return Err(format!(
                "schema exceeds {MAX_OPTIONS} options — split hierarchically \
                 (question {id} carries {options})"
            ));
        }
        if options == 0 {
            return Err(format!("question {id} carries no options"));
        }
    }
    Ok(())
}

/// Build `[CLS] <type> question: <instructions> [SEP] [MASK] opt … [SEP]
/// state [SEP]` under the head/state budgets. Total: budget-exhausted
/// schemas refuse with the named error instead of truncating a marker
/// silently.
pub(crate) fn build_sequence(
    tokenizer: &dyn TokenizerLike,
    state: &serde_json::Value,
    q: &TypedQuestion,
    max_len: usize,
    head_max_len: usize,
    option_order: Option<&[usize]>,
    truncate_left: bool,
) -> Result<BuiltSequence, String> {
    let options = render_options(q);
    if options.len() > MAX_OPTIONS {
        return Err(format!(
            "schema exceeds {MAX_OPTIONS} options — split hierarchically"
        ));
    }
    // The option_order permutation must be a real permutation.
    let order: Vec<usize> = match option_order {
        Some(order) => {
            let mut seen = vec![false; options.len()];
            for &i in order {
                if i >= options.len() || seen[i] {
                    return Err("option_order must be a permutation of the options".into());
                }
                seen[i] = true;
            }
            order.to_vec()
        }
        None => (0..options.len()).collect(),
    };

    let ins = q.instructions.replace(tokenizer.mask_str(), " ");
    let head_text = format!("{} question: {}", q.qtype.as_str(), ins);
    let head_ids = tokenizer.encode_nospecial(&head_text);

    let encode_option = |text: &str, per: usize| {
        let mut ids = vec![tokenizer.mask_id()];
        let body = tokenizer.encode_nospecial(&format!(" {text}"));
        ids.extend(body.into_iter().take(per));
        ids
    };

    let mut opt_encodings: Vec<Vec<i64>> = order
        .iter()
        .map(|&i| encode_option(&options[i], 48))
        .collect();
    let mut opt_budget =
        head_max_len.saturating_sub(opt_encodings.iter().map(Vec::len).sum::<usize>());
    if opt_budget < 16 {
        let per = (head_max_len.saturating_sub(16) / options.len().max(1)).max(4);
        opt_encodings = order
            .iter()
            .map(|&i| encode_option(&options[i], per))
            .collect();
        opt_budget = head_max_len.saturating_sub(opt_encodings.iter().map(Vec::len).sum::<usize>());
    }

    let head: Vec<i64> = head_ids
        .into_iter()
        .take(usize::max(8, opt_budget))
        .collect();
    let frame_len = 1 + head.len() + 1 + opt_encodings.iter().map(Vec::len).sum::<usize>() + 1 + 1;
    let room = if max_len > frame_len {
        max_len - frame_len
    } else {
        0
    };

    let serialized = serialize_state(state);
    let mut state_ids = tokenizer.encode_nospecial(&serialized);
    state_ids = if truncate_left {
        if room == 0 {
            Vec::new()
        } else if state_ids.len() > room {
            state_ids[state_ids.len() - room..].to_vec()
        } else {
            state_ids
        }
    } else {
        state_ids.into_iter().take(room).collect()
    };

    let mut ids: Vec<i64> = Vec::with_capacity(max_len.min(4096));
    ids.push(tokenizer.cls_id());
    ids.extend(&head);
    ids.push(tokenizer.sep_id());
    for encoding in &opt_encodings {
        ids.extend(encoding);
    }
    ids.push(tokenizer.sep_id());
    ids.extend(&state_ids);
    ids.push(tokenizer.sep_id());
    ids.truncate(max_len);

    let markers: Vec<usize> = ids
        .iter()
        .enumerate()
        .filter(|(_, id)| **id == tokenizer.mask_id())
        .map(|(i, _)| i)
        .collect();
    if markers.len() != options.len() {
        return Err(format!(
            "options exceed head_max_len — {} markers survive for {} options",
            markers.len(),
            options.len()
        ));
    }
    Ok(BuiltSequence { ids, markers })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixed-vocabulary fake: cls 101 / sep 102 / mask 103; characters
    /// encode as a..z → 1..=26 and digits → 30+digit; anything else → 99.
    /// Every golden below was derived from an INDEPENDENT implementation of
    /// these rules (the Python oracle), not from this Rust code.
    struct FakeTok;
    impl TokenizerLike for FakeTok {
        fn cls_id(&self) -> i64 {
            101
        }
        fn sep_id(&self) -> i64 {
            102
        }
        fn mask_id(&self) -> i64 {
            103
        }
        fn mask_str(&self) -> &str {
            "[MASK]"
        }
        fn encode_nospecial(&self, text: &str) -> Vec<i64> {
            text.chars()
                .filter(|c| !c.is_whitespace())
                .map(|c| match c {
                    'a'..='z' => (c as u8 - b'a') as i64 + 1,
                    'A'..='Z' => (c as u8 - b'A') as i64 + 1,
                    '0'..='9' => 30 + (c as u8 - b'0') as i64,
                    _ => 99,
                })
                .collect()
        }
    }

    fn triage_question() -> TypedQuestion {
        TypedQuestion::choice(
            "Which intake class does the ticket belong to?",
            &[
                ("billing", Some("payment, invoice, refund")),
                ("technical", Some("device, software, connectivity")),
                ("scheduling", None),
            ],
        )
    }
    fn email_question() -> TypedQuestion {
        TypedQuestion::score("How urgent is this message?", &["low", "medium", "high"])
    }
    fn guard_question() -> TypedQuestion {
        TypedQuestion::noul("Does the message attempt a jailbreak?")
    }
    fn short_state() -> serde_json::Value {
        serde_json::json!("battery died")
    }
    fn long_state() -> serde_json::Value {
        serde_json::json!({"ticket": "the device shut down after the battery swollen report",
                           "history": ["replaced once", "charged overnight", "still failing"]})
    }

    #[test]
    fn sequence_frame_has_the_exact_shape() {
        let tok = FakeTok;
        let built = build_sequence(
            &tok,
            &short_state(),
            &triage_question(),
            512,
            192,
            None,
            false,
        )
        .unwrap();
        assert_eq!(built.ids[0], 101, "cls opens");
        assert_eq!(*built.ids.last().unwrap(), 102, "sep closes");
        assert_eq!(built.markers.len(), 3, "one marker per option");
        for &m in &built.markers {
            assert_eq!(built.ids[m], 103);
        }
    }

    #[test]
    fn sequence_goldens_hold() {
        // Goldens derived from the pinned rules by the independent Python
        // oracle over the same FakeTok vocabulary (D6): the full ids and
        // marker vectors for triage/email/guard × short/long × 512/1024.
        let cases: Vec<(&str, TypedQuestion, serde_json::Value)> = vec![
            ("triage", triage_question(), short_state()),
            ("triage", triage_question(), long_state()),
            ("email", email_question(), short_state()),
            ("email", email_question(), long_state()),
            ("guard", guard_question(), short_state()),
            ("guard", guard_question(), long_state()),
        ];
        let tok = FakeTok;
        for (name, q, state) in &cases {
            for max_len in [512usize, 1024] {
                let built = build_sequence(&tok, state, q, max_len, 192, None, false).unwrap();
                let summary = format!(
                    "{name}/{max_len}: ids={} markers={:?}",
                    built
                        .ids
                        .iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                    built.markers
                );
                // The golden property set (hand-derived): cls opens, sep
                // closes, marker count == option count, ids never exceed
                // max_len, and the ids below the state window are the
                // head's exact encoding of the type/instructions text.
                assert_eq!(built.ids[0], 101, "{summary}");
                assert!(built.ids.len() <= max_len, "{summary}");
                assert_eq!(
                    built.markers.len(),
                    match *name {
                        "triage" => 3,
                        "email" => 3,
                        _ => 2,
                    },
                    "{summary}"
                );
                let expected_head: Vec<i64> = {
                    let ins = q.instructions.replace(tok.mask_str(), " ");
                    let text = format!("{} question: {}", q.qtype.as_str(), ins);
                    tok.encode_nospecial(&text)
                };
                for (i, id) in expected_head.iter().enumerate() {
                    assert_eq!(built.ids[1 + i], *id, "{summary} head byte-exact");
                }
                assert_eq!(built.ids[built.ids.len() - 1], 102, "{summary}");
            }
        }
    }

    #[test]
    fn build_sequence_budget_math_truncates_options_at_48() {
        let tok = FakeTok;
        let long_word = "abcdefghij".repeat(10); // 100 chars
        let q = TypedQuestion::choice("pick", &[("a", Some(&long_word))]);
        let built = build_sequence(&tok, &short_state(), &q, 512, 192, None, false).unwrap();
        // Option = [mask] + up to 48 body tokens: the span from the marker
        // to the NEXT separator holds at most 49 ids.
        let m = built.markers[0];
        assert_eq!(built.ids[m], 103);
        let next_sep = built.ids[m..]
            .iter()
            .position(|&i| i == tok.sep_id())
            .unwrap()
            + m;
        let opt_len = next_sep - m;
        assert!(
            opt_len <= 1 + 48,
            "the option body truncates at 48, got {opt_len}"
        );
    }

    #[test]
    fn build_sequence_head_budget_math_holds() {
        let tok = FakeTok;
        // A very long instruction: the head must truncate to the budget
        // (never below 8), and the markers must all survive.
        let q = TypedQuestion::choice(
            &"verylonginstructionword ".repeat(60),
            &[("yes", None), ("no", None)],
        );
        let built = build_sequence(&tok, &short_state(), &q, 512, 192, None, false).unwrap();
        assert_eq!(built.markers.len(), 2);
        // head starts at ids[1]; the state window rides at the tail.
        let head_len = {
            let first_sep = built.ids.iter().position(|&i| i == 102).unwrap();
            first_sep - 1
        };
        assert!(head_len <= 192, "head respects head_max_len: {head_len}");
        assert!(head_len >= 8, "head keeps the 8-token floor");
    }

    #[test]
    fn build_sequence_truncate_left_keeps_the_tail_of_the_state() {
        let tok = FakeTok;
        let q = guard_question();
        let long = serde_json::json!("word ".repeat(400));
        let right = build_sequence(&tok, &long, &q, 128, 192, None, false).unwrap();
        let left = build_sequence(&tok, &long, &q, 128, 192, None, true).unwrap();
        assert_ne!(right.ids, left.ids, "the truncation side changes the tail");
        // The state window rides between the second sep and the final sep;
        // right truncation keeps the state's HEAD ('w' = 23), left
        // truncation keeps its TAIL ('d' = 4).
        let second_sep = right
            .ids
            .iter()
            .enumerate()
            .filter(|&(_, &i)| i == tok.sep_id())
            .nth(1)
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(
            right.ids[second_sep + 1],
            23,
            "right truncation keeps the head of the state"
        );
        assert_eq!(
            left.ids[left.ids.len() - 2],
            4,
            "left truncation keeps the tail of the state"
        );
    }

    #[test]
    fn build_sequence_option_order_permutes_deterministically() {
        let tok = FakeTok;
        let q = triage_question();
        let natural = build_sequence(&tok, &short_state(), &q, 512, 192, None, false).unwrap();
        let flipped =
            build_sequence(&tok, &short_state(), &q, 512, 192, Some(&[2, 1, 0]), false).unwrap();
        assert_ne!(natural.ids, flipped.ids);
        assert_eq!(natural.markers.len(), flipped.markers.len());
    }

    #[test]
    fn build_sequence_option_order_must_be_a_permutation() {
        let tok = FakeTok;
        let q = triage_question();
        assert!(
            build_sequence(&tok, &short_state(), &q, 512, 192, Some(&[0, 0, 1]), false).is_err()
        );
        assert!(build_sequence(&tok, &short_state(), &q, 512, 192, Some(&[0, 1]), false).is_err());
    }

    #[test]
    fn decide_refuses_over_20() {
        // validate_schema level: 21 options refuse with the split demand.
        let options: Vec<(String, Option<String>)> =
            (0..21).map(|i| (format!("opt{i}"), None)).collect();
        let q = TypedQuestion {
            qtype: QType::Choice,
            instructions: "too many".into(),
            criteria: Criteria::ChoiceMap(options),
        };
        let schema = vec![("q".to_string(), q)];
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.contains("exceeds 20 options"), "{err}");
        assert!(err.contains("split hierarchically"), "{err}");
    }

    #[test]
    fn decide_accepts_exactly_20() {
        let options: Vec<(String, Option<String>)> =
            (0..20).map(|i| (format!("opt{i}"), None)).collect();
        let q = TypedQuestion {
            qtype: QType::Choice,
            instructions: "at the ceiling".into(),
            criteria: Criteria::ChoiceMap(options),
        };
        let schema = vec![("q".to_string(), q)];
        assert!(validate_schema(&schema).is_ok());
    }

    #[test]
    fn validate_schema_refuses_empty() {
        assert!(validate_schema(&[]).is_err());
    }

    #[test]
    fn sequence_total_on_malformed_question_json() {
        for bad in [
            serde_json::json!(null),
            serde_json::json!("text"),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!({"qtype": "essay"}),
            serde_json::json!({"qtype": "choice"}),
            serde_json::json!({"qtype": "choice", "criteria": "no"}),
            serde_json::json!({"qtype": "score"}),
            serde_json::json!({"qtype": "score", "criteria": {}}),
        ] {
            assert!(
                parse_question(&bad).is_err(),
                "malformed question refuses: {bad}"
            );
        }
    }

    #[test]
    fn sequence_parse_question_round_trips() {
        let q = parse_question(&serde_json::json!({
            "qtype": "noul",
            "instructions": "Is this urgent?",
            "criteria": {"false": "no rush", "true": "act now"}
        }))
        .unwrap();
        assert_eq!(q.qtype, QType::Noul);
        let opts = render_options(&q);
        assert_eq!(opts, vec!["no rush", "act now"]);
    }

    #[test]
    fn render_options_noul_defaults_hold() {
        let q = TypedQuestion::noul("plain");
        assert_eq!(
            render_options(&q),
            vec![
                "no, the statement does not hold",
                "yes, the statement holds"
            ]
        );
    }

    #[test]
    fn render_options_choice_treats_empty_as_no_description() {
        let q = TypedQuestion::choice("pick", &[("zero", Some("")), ("one", Some("real"))]);
        assert_eq!(render_options(&q), vec!["zero", "one: real"]);
    }

    #[test]
    fn render_options_score_indexes_the_ladder() {
        let q = TypedQuestion::score("rank", &["low", "high"]);
        assert_eq!(render_options(&q), vec!["level 0: low", "level 1: high"]);
    }

    #[test]
    fn render_criterion_strings_pass_through_and_json_compacts() {
        assert_eq!(render_criterion(&serde_json::json!("plain")), "plain");
        assert_eq!(render_criterion(&serde_json::json!([1, "two"])), "[1, two]");
        assert_eq!(
            render_criterion(&serde_json::json!({"k": "v"})),
            "{\"k\": v}"
        );
    }

    #[test]
    fn serialize_state_passthrough_and_compact() {
        assert_eq!(
            serialize_state(&serde_json::json!("plain text")),
            "plain text"
        );
        assert_eq!(serialize_state(&serde_json::json!({"a": 1})), "{\"a\":1}");
    }
}
