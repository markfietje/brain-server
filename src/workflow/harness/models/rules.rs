//! The deterministic reference [`DecisionModel`]: evaluates typed inputs
//! against a DECLARED, serializable rule table (evidence thresholds +
//! closed-vocabulary checks) whose CANONICAL serialization hashes to the
//! config digest. The digest law: the model always hashes the compact
//! re-serialization of the LOADED table — never the incoming bytes — so
//! pretty and compact configs bind identically, and the context's
//! `config_digests` prove which configuration an evaluation ran under.
//!
//! Refusal law: out-of-vocabulary asks, empty evidence, and evidence
//! below the required trust tier are honest
//! [`DecisionError::Refused`]s — never a guess. Provenance-less evidence
//! is unrepresentable one level up: an [`EvidenceRef`] cannot exist
//! without an id and a tier, and a ref with an EMPTY id qualifies as
//! nothing.
//!
//! Determinism law: same input + same context ⇒ same output, including
//! `produced_at` (the context's own `created_at` — no clock is read).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workflow::harness::model::{
    ConfigRef, DecisionContext, DecisionError, DecisionInput, DecisionModel, DecisionOutput,
    DecisionValue, EvidenceRef, ModelMetadata, OutputKind, RefusalReason, TrustTier,
};

/// The kernel-side serde face of [`TrustTier`] (the SDK type carries no
/// serde by design; the lowercase string form is the declared-config
/// representation).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Tier {
    Governed,
    Vetted,
    Untrusted,
}

impl Tier {
    fn trust(self) -> TrustTier {
        match self {
            Tier::Governed => TrustTier::Governed,
            Tier::Vetted => TrustTier::Vetted,
            Tier::Untrusted => TrustTier::Untrusted,
        }
    }
}

/// One declared rule: the question it answers, the evidence it demands,
/// and the closed-vocabulary value it emits.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rule {
    pub(crate) question_id: String,
    pub(crate) min_evidence: u32,
    pub(crate) min_tier: Tier,
    pub(crate) output: RuleOutput,
}

/// The closed emission vocabulary — a choice label from a declared option
/// set, a score inside a declared range, or the binary pair. The loader
/// refuses a label outside its options and a value outside its range, so
/// a loaded table cannot emit outside its own declared vocabulary.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) enum RuleOutput {
    Choice { options: Vec<String>, label: String },
    Score { range: (i64, i64), value: i64 },
    Noul { value: bool },
}

/// The declared rule table — the reference model's whole configuration.
/// Declaration order is semantic and preserved; the canonical
/// serialization is this structure's compact JSON form.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleTable {
    pub(crate) model_id: String,
    pub(crate) model_version: String,
    pub(crate) rules: Vec<Rule>,
}

impl RuleTable {
    /// Total loader: any input yields the table or a named refusal —
    /// never a panic, never a widened vocabulary. Validation lives here so
    /// a loaded table is well-formed by construction.
    pub(crate) fn from_canonical_json(raw: &str) -> Result<Self, String> {
        let table: RuleTable =
            serde_json::from_str(raw).map_err(|e| format!("rules config refused: {e}"))?;
        table.validate()?;
        Ok(table)
    }

    fn validate(&self) -> Result<(), String> {
        if self.model_id.trim().is_empty() {
            return Err("rules config refused: model_id must be non-empty".into());
        }
        if self.model_version.trim().is_empty() {
            return Err("rules config refused: model_version must be non-empty".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for rule in &self.rules {
            if rule.question_id.trim().is_empty() {
                return Err("rules config refused: question_id must be non-empty".into());
            }
            if !seen.insert(rule.question_id.as_str()) {
                return Err(format!(
                    "rules config refused: duplicate question id {:?}",
                    rule.question_id
                ));
            }
            if rule.min_evidence < 1 {
                return Err(
                    "rules config refused: min_evidence must be at least 1 (a model never
                     decides on zero evidence)"
                        .into(),
                );
            }
            match &rule.output {
                RuleOutput::Choice { options, label } => {
                    if options.is_empty() {
                        return Err("rules config refused: a choice rule needs options".into());
                    }
                    if !options.contains(label) {
                        return Err(format!(
                            "rules config refused: label {label:?} is not one of the declared options"
                        ));
                    }
                }
                RuleOutput::Score { range, value } => {
                    if range.0 > range.1 {
                        return Err("rules config refused: an inverted score range".into());
                    }
                    if *value < range.0 || *value > range.1 {
                        return Err(format!(
                            "rules config refused: score value {value} outside the declared range"
                        ));
                    }
                }
                RuleOutput::Noul { .. } => {}
            }
        }
        Ok(())
    }
}

/// The reference model: a validated table + its canonical serialization +
/// the sha256 digest of that serialization, all fixed at load.
#[derive(Debug)]
pub(crate) struct RulesModel {
    table: RuleTable,
    canonical: String,
    digest: String,
    config_key: String,
}

impl RulesModel {
    pub(crate) fn from_canonical_json(raw: &str) -> Result<Self, String> {
        let table = RuleTable::from_canonical_json(raw)?;
        let canonical = serde_json::to_string(&table)
            .map_err(|e| format!("rules config refused: canonicalization failed: {e}"))?;
        let mut h = Sha256::new();
        h.update(canonical.as_bytes());
        let digest = hex::encode(h.finalize());
        let config_key = format!("rules:{}", table.model_id);
        Ok(Self {
            table,
            canonical,
            digest,
            config_key,
        })
    }

    /// The config key this model binds under (what the context must carry).
    pub(crate) fn config_key(&self) -> &str {
        &self.config_key
    }

    /// The sha256 hex of the table's CANONICAL serialization.
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }

    /// The canonical bytes themselves (stable across re-serializations).
    pub(crate) fn canonical_json(&self) -> &str {
        &self.canonical
    }
}

impl DecisionModel for RulesModel {
    fn metadata(&self) -> ModelMetadata {
        let mut vocabulary: Vec<OutputKind> = Vec::new();
        for rule in &self.table.rules {
            let kind = match rule.output {
                RuleOutput::Choice { .. } => OutputKind::Choice,
                RuleOutput::Score { .. } => OutputKind::Score,
                RuleOutput::Noul { .. } => OutputKind::Noul,
            };
            if !vocabulary.contains(&kind) {
                vocabulary.push(kind);
            }
        }
        ModelMetadata::deterministic(
            self.table.model_id.clone(),
            self.table.model_version.clone(),
            vocabulary,
            None,
        )
    }

    fn evaluate(
        &self,
        input: &DecisionInput,
        ctx: &DecisionContext,
    ) -> Result<DecisionOutput, DecisionError> {
        // (1) The config binding: the context must carry THIS table's
        // digest under this model's key — an unbound evaluation is a
        // wiring error, not a decision.
        let bound = ctx
            .config_digests
            .iter()
            .any(|c: &ConfigRef| c.key == self.config_key && c.digest == self.digest);
        if !bound {
            return Err(DecisionError::Misconfigured);
        }
        // (2) The asked question must be in the table's vocabulary.
        let Some(question) = &input.question else {
            return Err(DecisionError::Refused {
                reason: RefusalReason::OutOfVocabulary,
            });
        };
        let Some(rule) = self
            .table
            .rules
            .iter()
            .find(|r| r.question_id == question.id)
        else {
            return Err(DecisionError::Refused {
                reason: RefusalReason::OutOfVocabulary,
            });
        };
        // (3) The evidence thresholds: no refs at all is a different
        // honest refusal from refs that do not qualify (empty ids are
        // provenance-less and qualify as nothing).
        if input.evidence.is_empty() {
            return Err(DecisionError::Refused {
                reason: RefusalReason::InsufficientEvidence,
            });
        }
        let floor = rule.min_tier.trust();
        let qualifying: Vec<EvidenceRef> = input
            .evidence
            .iter()
            .filter(|e| !e.evidence_id.is_empty() && e.tier.at_least(floor))
            .cloned()
            .collect();
        if qualifying.len() < rule.min_evidence as usize {
            return Err(DecisionError::Refused {
                reason: RefusalReason::InsufficientEvidenceTier,
            });
        }
        // (4) The emission: closed vocabulary in, typed output out; the
        // provenance travels with the output (ids + tiers only).
        let value = match &rule.output {
            RuleOutput::Choice { label, .. } => DecisionValue::Choice {
                label: label.clone(),
                probabilities: None,
                confidence: None,
            },
            RuleOutput::Score { range, value } => DecisionValue::Score {
                value: *value,
                range: *range,
            },
            RuleOutput::Noul { value } => DecisionValue::Noul { value: *value },
        };
        Ok(DecisionOutput {
            model_id: self.table.model_id.clone(),
            model_version: self.table.model_version.clone(),
            produced_at: ctx.created_at,
            value,
            evidence_refs: qualifying,
        })
    }
}

/// The committed hostile-config corpus (small, replayed by the named
/// test): every file loads to the table or a named refusal — never a
/// panic, never a widened vocabulary.
mod corpus {
    pub(crate) const EMPTY_RULES: &str = include_str!("corpus/empty_rules.json");
    pub(crate) const DUPLICATE_QUESTION_IDS: &str =
        include_str!("corpus/duplicate_question_ids.json");
    pub(crate) const UNKNOWN_FIELD: &str = include_str!("corpus/unknown_field.json");
    pub(crate) const BAD_OUTPUT_LABEL: &str = include_str!("corpus/bad_output_label.json");
    pub(crate) const SCORE_OUT_OF_RANGE: &str = include_str!("corpus/score_out_of_range.json");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::harness::model::QuestionKind;
    use crate::workflow::harness::model::QuestionRef;
    use brain_engine_sdk::decision::RunMode;

    const TABLE_JSON: &str = r#"{
      "model_id": "rules-reference",
      "model_version": "1.0.0",
      "rules": [
        { "question_id": "needs_human", "min_evidence": 2, "min_tier": "vetted",
          "output": { "Choice": { "options": ["approve", "deny"], "label": "approve" } } },
        { "question_id": "risk", "min_evidence": 1, "min_tier": "governed",
          "output": { "Score": { "range": [0, 100], "value": 42 } } },
        { "question_id": "duplicate", "min_evidence": 1, "min_tier": "untrusted",
          "output": { "Noul": { "value": true } } }
      ]
    }"#;

    fn ctx(digest: Option<&str>) -> DecisionContext {
        let config_digests = match digest {
            Some(d) => vec![ConfigRef {
                key: "rules:rules-reference".into(),
                digest: d.into(),
            }],
            None => Vec::new(),
        };
        DecisionContext {
            run_id: 11,
            mode: RunMode::Deterministic,
            config_digests,
            role_scope: vec!["operator".into()],
            created_at: 1_800_000_123,
        }
    }

    fn input(question_id: &str, evidence: Vec<EvidenceRef>) -> DecisionInput {
        DecisionInput {
            request_id: "req-1".into(),
            question: Some(QuestionRef {
                id: question_id.into(),
                kind: QuestionKind::Choice,
            }),
            question_ids: vec![question_id.into()],
            evidence,
        }
    }

    fn governed(id: &str) -> EvidenceRef {
        EvidenceRef {
            evidence_id: id.into(),
            tier: TrustTier::Governed,
        }
    }

    /// Same input + same context ⇒ the same output (including
    /// produced_at), the digest is stable across loadings, and the
    /// canonical-serialization law holds: pretty and compact configs bind
    /// to the SAME digest, because the digest hashes the re-serialization
    /// of the loaded table, never the incoming bytes.
    #[test]
    fn rules_model_is_deterministic_same_input_context_same_output_and_digest() {
        let model = RulesModel::from_canonical_json(TABLE_JSON).unwrap();
        let ask = input("risk", vec![governed("e-1")]);

        let first = model.evaluate(&ask, &ctx(Some(model.digest()))).unwrap();
        let second = model.evaluate(&ask, &ctx(Some(model.digest()))).unwrap();
        assert_eq!(first, second, "same input + ctx ⇒ same output");
        assert_eq!(
            first.produced_at, 1_800_000_123,
            "produced_at is the context's created_at, never a clock"
        );
        match first.value {
            DecisionValue::Score { value, range } => {
                assert_eq!((value, range), (42, (0, 100)));
            }
            other => panic!("the risk rule scores: {other:?}"),
        }
        assert_eq!(first.evidence_refs.len(), 1);
        assert_eq!(first.evidence_refs[0].evidence_id, "e-1");

        // Metadata derives from the table.
        let meta = model.metadata();
        assert_eq!(meta.id(), "rules-reference");
        assert_eq!(meta.version(), "1.0.0");
        assert_eq!(meta.weights_digest(), None, "deterministic ⇒ no digest");
        assert_eq!(
            meta.output_vocabulary(),
            [OutputKind::Choice, OutputKind::Score, OutputKind::Noul]
        );

        // The digest law: pretty vs compact bytes, same digest — and the
        // same bytes on every re-load.
        let compact = r#"{"model_id":"rules-reference","model_version":"1.0.0","rules":[{"question_id":"needs_human","min_evidence":2,"min_tier":"vetted","output":{"Choice":{"options":["approve","deny"],"label":"approve"}}},{"question_id":"risk","min_evidence":1,"min_tier":"governed","output":{"Score":{"range":[0,100],"value":42}}},{"question_id":"duplicate","min_evidence":1,"min_tier":"untrusted","output":{"Noul":{"value":true}}}]}"#;
        let pretty_model = RulesModel::from_canonical_json(TABLE_JSON).unwrap();
        let compact_model = RulesModel::from_canonical_json(compact).unwrap();
        assert_eq!(pretty_model.digest(), compact_model.digest());
        assert_eq!(
            pretty_model.canonical_json(),
            compact_model.canonical_json()
        );
        let reloaded = RulesModel::from_canonical_json(TABLE_JSON).unwrap();
        assert_eq!(reloaded.digest(), model.digest());
    }

    /// The honest-refusal law: out-of-vocabulary asks refuse, zero
    /// evidence refuses, below-tier evidence refuses, empty-id
    /// (provenance-less) evidence qualifies as nothing, and an unbound
    /// context is a Misconfigured wiring error — never a guess.
    #[test]
    fn rules_model_refuses_out_of_vocabulary_and_provenance_less_evidence() {
        let model = RulesModel::from_canonical_json(TABLE_JSON).unwrap();

        // The context must bind the model's config first.
        match model.evaluate(&input("risk", vec![governed("e-1")]), &ctx(None)) {
            Err(DecisionError::Misconfigured) => {}
            other => panic!("unbound context misconfigures: {other:?}"),
        }

        let bound = ctx(Some(model.digest()));
        // Unknown question id → out of vocabulary.
        match model.evaluate(&input("no-such-question", vec![governed("e-1")]), &bound) {
            Err(DecisionError::Refused {
                reason: RefusalReason::OutOfVocabulary,
            }) => {}
            other => panic!("unknown question refuses: {other:?}"),
        }
        // No question at all → nothing is in vocabulary.
        let mut no_question = input("risk", vec![governed("e-1")]);
        no_question.question = None;
        match model.evaluate(&no_question, &bound) {
            Err(DecisionError::Refused {
                reason: RefusalReason::OutOfVocabulary,
            }) => {}
            other => panic!("question-less ask refuses: {other:?}"),
        }
        // Zero evidence → InsufficientEvidence.
        match model.evaluate(&input("risk", Vec::new()), &bound) {
            Err(DecisionError::Refused {
                reason: RefusalReason::InsufficientEvidence,
            }) => {}
            other => panic!("empty evidence refuses: {other:?}"),
        }
        // The risk rule needs GOVERNED evidence; vetted does not qualify.
        match model.evaluate(
            &input(
                "risk",
                vec![EvidenceRef {
                    evidence_id: "e-1".into(),
                    tier: TrustTier::Vetted,
                }],
            ),
            &bound,
        ) {
            Err(DecisionError::Refused {
                reason: RefusalReason::InsufficientEvidenceTier,
            }) => {}
            other => panic!("below-tier evidence refuses: {other:?}"),
        }
        // PROVENANCE-LESS evidence is unrepresentable at the type level —
        // an EvidenceRef cannot be constructed without an id AND a tier —
        // and an EMPTY id (the closest representable form) qualifies as
        // nothing even at the top tier.
        match model.evaluate(
            &input(
                "risk",
                vec![EvidenceRef {
                    evidence_id: String::new(),
                    tier: TrustTier::Governed,
                }],
            ),
            &bound,
        ) {
            Err(DecisionError::Refused {
                reason: RefusalReason::InsufficientEvidenceTier,
            }) => {}
            other => panic!("empty-id evidence qualifies as nothing: {other:?}"),
        }
        // The happy path still decides, and the refusal path never
        // invented an output on the way.
        assert!(
            model
                .evaluate(&input("risk", vec![governed("e-1")]), &bound)
                .is_ok()
        );
    }

    /// The committed hostile-config corpus replays clean: every file
    /// loads to the table or a NAMED refusal — never a panic, never a
    /// widened vocabulary; the loadable file is digest-stable.
    #[test]
    fn rules_model_config_hostile_corpus_replays_clean() {
        // Valid but empty: loads, digest-stable, everything refuses.
        let empty = RulesModel::from_canonical_json(corpus::EMPTY_RULES)
            .expect("empty rules are a valid table");
        let again = RulesModel::from_canonical_json(corpus::EMPTY_RULES).unwrap();
        assert_eq!(empty.digest(), again.digest(), "digest stability");
        assert_eq!(
            empty.metadata().output_vocabulary(),
            Vec::<OutputKind>::new()
        );
        let ctx = DecisionContext {
            config_digests: vec![ConfigRef {
                key: empty.config_key().into(),
                digest: empty.digest().into(),
            }],
            ..ctx(Some(""))
        };
        match empty.evaluate(&input("anything", vec![governed("e-1")]), &ctx) {
            Err(DecisionError::Refused {
                reason: RefusalReason::OutOfVocabulary,
            }) => {}
            other => panic!("an empty table decides nothing: {other:?}"),
        }

        let refusals: [(&str, &str); 4] = [
            (corpus::DUPLICATE_QUESTION_IDS, "duplicate question id"),
            (corpus::UNKNOWN_FIELD, "rules config refused"),
            (corpus::BAD_OUTPUT_LABEL, "not one of the declared options"),
            (corpus::SCORE_OUT_OF_RANGE, "outside the declared range"),
        ];
        for (raw, expected) in refusals {
            let msg = RulesModel::from_canonical_json(raw)
                .expect_err("hostile configs are refused, not loaded");
            assert!(
                msg.contains(expected),
                "named refusal must mention {expected:?}: {msg}"
            );
        }
    }
}
