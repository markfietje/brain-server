//! The decision pipeline's configuration document: a versioned, serde
//! record whose CANONICAL serialization hashes to the config hash every
//! [`StageRecord`](crate::workflow::harness::pipeline::StageRecord)
//! carries. The canonical-hash law is the rules-table law: the hash is
//! taken over the compact re-serialization of the LOADED document — never
//! the incoming bytes — so pretty and compact configs bind identically and
//! re-loading is stable.
//!
//! The loader is total: a config loads WHOLE or refuses with a named
//! reason (duplicate/unknown/out-of-order stages, bounds, threshold
//! sanity). A loaded config is well-formed by construction; the runner
//! never re-validates.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::search::query::LegFilter;

/// The stage vocabulary — the pipeline's closed eight-name list, in the
/// architecture's fixed execution order. Serde's closed enum makes an
/// unknown stage name a named load refusal, never a silent stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StageName {
    Normalize,
    RetrieveContext,
    CandidateGeneration,
    DecisionModel,
    RulesPolicy,
    Rerank,
    Threshold,
    ActionEscalation,
}

impl StageName {
    /// The closed string form (stage keys in idempotency keys and
    /// session-log payloads). Exhaustive match — a new variant breaks this
    /// here, not silently in a key.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            StageName::Normalize => "normalize",
            StageName::RetrieveContext => "retrieve_context",
            StageName::CandidateGeneration => "candidate_generation",
            StageName::DecisionModel => "decision_model",
            StageName::RulesPolicy => "rules_policy",
            StageName::Rerank => "rerank",
            StageName::Threshold => "threshold",
            StageName::ActionEscalation => "action_escalation",
        }
    }
}

impl core::fmt::Display for StageName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The fixed execution order (the architecture's pipeline, verbatim).
const CANONICAL_ORDER: [StageName; 8] = [
    StageName::Normalize,
    StageName::RetrieveContext,
    StageName::CandidateGeneration,
    StageName::DecisionModel,
    StageName::RulesPolicy,
    StageName::Rerank,
    StageName::Threshold,
    StageName::ActionEscalation,
];

/// The stage-list bound: the architecture's pipeline is fixed, so a
/// declared list can never exceed the canonical eight. A named constant,
/// test-pinned — the bound lives at the config, not at run time.
pub(crate) const MAX_DECLARED_STAGES: usize = CANONICAL_ORDER.len();

/// The retrieval over-fetch bound, re-stated at the config: a declared
/// `limit` above this refuses at load (the existing search-side law keeps
/// its own home upstream; this is the harness's independent floor).
pub(crate) const RETRIEVAL_LIMIT_CAP: u32 = 100;

/// The closed retrieval-leg vocabulary (the search-side leg filter's
/// declared-config face). `Both` fuses every leg; the other three restrict
/// the fused output to one leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RetrievalLeg {
    Both,
    Vector,
    Fts,
    Graph,
}

impl RetrievalLeg {
    pub(crate) fn leg_filter(self) -> Option<LegFilter> {
        match self {
            RetrievalLeg::Both => None,
            RetrievalLeg::Vector => Some(LegFilter::Vector),
            RetrievalLeg::Fts => Some(LegFilter::Fts),
            RetrievalLeg::Graph => Some(LegFilter::Graph),
        }
    }
}

/// The recorded retrieval parameters: reciprocal-rank-fusion constant,
/// fused-result limit, and the leg restriction. These are the params the
/// retriever runs under and the trace records — the same shape both places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrievalParams {
    pub(crate) rrf_k: u32,
    pub(crate) limit: u32,
    pub(crate) leg: RetrievalLeg,
}

/// The model binding the run's context must carry: the config-digest pair
/// the model itself validates at evaluation (an unbound evaluation is the
/// model's own misconfiguration refusal, not a silent guess).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelBinding {
    pub(crate) key: String,
    pub(crate) digest: String,
}

/// Where a threshold verdict lands when the decision value matches neither
/// the act nor the reject law. `approve` is the human path; `escalate`
/// treats ambiguity as a human decision too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThresholdFallback {
    Approve,
    Escalate,
}

/// The threshold law, declared: which choice labels act, which reject, the
/// score cutoffs, the noul act-value, and the fallback. Sanity is enforced
/// at load — a label cannot mean both act and reject, and the score cutoffs
/// cannot be inverted (refuse, never guess).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Thresholds {
    pub(crate) act_labels: Vec<String>,
    pub(crate) reject_labels: Vec<String>,
    pub(crate) score_act_at_or_above: i64,
    pub(crate) score_reject_at_or_below: i64,
    pub(crate) noul_act_when: bool,
    pub(crate) fallback: ThresholdFallback,
}

/// The pipeline configuration document: stages, retrieval, model binding,
/// thresholds. Everything the deterministic runner needs, as data.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecisionPipelineConfig {
    /// The schema marker; the validator pins the exact accepted value so a
    /// document from a different schema generation refuses at load.
    pub(crate) config_schema: String,
    pub(crate) pipeline_id: String,
    /// The declared stage list: the canonical order with the optional
    /// re-rank leg absent or present — nothing else.
    pub(crate) stages: Vec<StageName>,
    pub(crate) retrieval: RetrievalParams,
    pub(crate) model: ModelBinding,
    pub(crate) thresholds: Thresholds,
}

/// The schema marker this loader accepts.
const CONFIG_SCHEMA: &str = "harness.pipeline/v1";

/// A config that has loaded, validated, and been hashed — the only form
/// the runner accepts. `config_hash` is the sha256 (lowercase hex) of
/// `canonical` (the compact re-serialization of the loaded document).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LoadedPipelineConfig {
    pub(crate) config: DecisionPipelineConfig,
    pub(crate) canonical: String,
    pub(crate) config_hash: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// The compact serialization of a loaded serde value. Plain data with
/// string keys only — serialization is structurally infallible here (the
/// house unwrap idiom for records of this shape).
pub(crate) fn canonical_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap()
}

pub(crate) fn sha256_of_json<T: Serialize>(value: &T) -> String {
    sha256_hex(canonical_json(value).as_bytes())
}

fn is_lower_hex_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl DecisionPipelineConfig {
    /// Total loader: the config or a named refusal — never a partially
    /// loaded document, never a panic. Validation lives here so the runner
    /// receives only well-formed, hash-bound configs.
    pub(crate) fn load(raw: &str) -> Result<LoadedPipelineConfig, String> {
        let config: DecisionPipelineConfig =
            serde_json::from_str(raw).map_err(|e| format!("pipeline config refused: {e}"))?;
        config.validate()?;
        let canonical = canonical_json(&config);
        let config_hash = sha256_hex(canonical.as_bytes());
        Ok(LoadedPipelineConfig {
            config,
            canonical,
            config_hash,
        })
    }

    fn validate(&self) -> Result<(), String> {
        if self.config_schema != CONFIG_SCHEMA {
            return Err(format!(
                "pipeline config refused: config_schema must be {CONFIG_SCHEMA:?}, got {:?}",
                self.config_schema
            ));
        }
        if self.pipeline_id.trim().is_empty() {
            return Err("pipeline config refused: pipeline_id must be non-empty".into());
        }
        Self::validate_stages(&self.stages)?;
        Self::validate_retrieval(&self.retrieval)?;
        Self::validate_model(&self.model)?;
        Self::validate_thresholds(&self.thresholds)?;
        Ok(())
    }

    /// The stage-list law: at the cap, duplicate-free, and exactly the
    /// canonical order with the optional re-rank leg absent or in place.
    /// Anything else — a missing required stage, an out-of-order stage, a
    /// mispositioned re-rank — refuses by name.
    fn validate_stages(stages: &[StageName]) -> Result<(), String> {
        if stages.len() > MAX_DECLARED_STAGES {
            return Err(format!(
                "pipeline config refused: {} stages exceed the bound of {MAX_DECLARED_STAGES}",
                stages.len()
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for stage in stages {
            if !seen.insert(stage.as_str()) {
                return Err(format!(
                    "pipeline config refused: duplicate stage {:?}",
                    stage.as_str()
                ));
            }
        }
        let has_rerank = stages.contains(&StageName::Rerank);
        let expected: Vec<StageName> = CANONICAL_ORDER
            .iter()
            .copied()
            .filter(|s| has_rerank || *s != StageName::Rerank)
            .collect();
        if stages != expected {
            return Err(
                "pipeline config refused: the declared stages must be the canonical order \
                 with the optional re-rank leg absent or in place"
                    .into(),
            );
        }
        Ok(())
    }

    fn validate_retrieval(params: &RetrievalParams) -> Result<(), String> {
        if params.rrf_k == 0 || params.rrf_k > 1000 {
            return Err(format!(
                "pipeline config refused: retrieval rrf_k {} outside 1..=1000",
                params.rrf_k
            ));
        }
        if params.limit == 0 || params.limit > RETRIEVAL_LIMIT_CAP {
            return Err(format!(
                "pipeline config refused: retrieval limit {} outside 1..={RETRIEVAL_LIMIT_CAP}",
                params.limit
            ));
        }
        Ok(())
    }

    fn validate_model(model: &ModelBinding) -> Result<(), String> {
        if model.key.trim().is_empty() {
            return Err("pipeline config refused: model key must be non-empty".into());
        }
        if !is_lower_hex_digest(&model.digest) {
            return Err(
                "pipeline config refused: model digest must be 64 lowercase hex characters".into(),
            );
        }
        Ok(())
    }

    fn validate_thresholds(t: &Thresholds) -> Result<(), String> {
        // An empty choice law is lawful — the fallback carries every choice;
        // the per-label checks below still run.
        let mut seen = std::collections::BTreeSet::new();
        for label in t.act_labels.iter().chain(&t.reject_labels) {
            if label.trim().is_empty() {
                return Err("pipeline config refused: threshold labels must be non-empty".into());
            }
            if !seen.insert(label.as_str()) {
                return Err(format!(
                    "pipeline config refused: threshold label {label:?} declared twice — a label \
                     cannot mean both act and reject"
                ));
            }
        }
        if t.score_reject_at_or_below >= t.score_act_at_or_above {
            return Err(
                "pipeline config refused: the score reject cutoff must be below the score act \
                 cutoff (inverted cutoffs refuse at load)"
                    .into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL_DIGEST: &str = "ab1111111111111111111111111111111111111111111111111111111111111b";

    fn config_body(stages: &str) -> String {
        format!(
            r#"{{
              "config_schema": "harness.pipeline/v1",
              "pipeline_id": "reference-line",
              "stages": {stages},
              "retrieval": {{"rrf_k": 60, "limit": 100, "leg": "both"}},
              "model": {{"key": "rules:rules-reference", "digest": "{MODEL_DIGEST}"}},
              "thresholds": {{
                "act_labels": ["act"],
                "reject_labels": ["reject"],
                "score_act_at_or_above": 50,
                "score_reject_at_or_below": 10,
                "noul_act_when": true,
                "fallback": "approve"
              }}
            }}"#
        )
    }

    fn without_rerank() -> String {
        r#"["normalize","retrieve_context","candidate_generation","decision_model","rules_policy","threshold","action_escalation"]"#
            .to_string()
    }

    fn with_rerank() -> String {
        r#"["normalize","retrieve_context","candidate_generation","decision_model","rules_policy","rerank","threshold","action_escalation"]"#
            .to_string()
    }

    /// The canonical-hash law: pretty and compact configs bind the SAME
    /// hash (the digest is taken over the compact re-serialization of the
    /// LOADED document, never the incoming bytes), and re-loading is
    /// byte-stable.
    #[test]
    fn pipeline_config_hash_is_canonical_and_stable() {
        let pretty = DecisionPipelineConfig::load(&config_body(&without_rerank()))
            .expect("the config loads");
        let compact_raw = config_body(&without_rerank())
            .split_whitespace()
            .collect::<String>();
        let compact = DecisionPipelineConfig::load(&compact_raw).expect("the config loads");
        assert_eq!(
            pretty.config_hash, compact.config_hash,
            "pretty and compact bind identically"
        );
        assert_eq!(pretty.canonical, compact.canonical);
        assert!(
            !pretty.canonical.contains('\n'),
            "the canonical form is the compact serialization"
        );
        let reloaded = DecisionPipelineConfig::load(&config_body(&without_rerank()))
            .expect("re-load stability");
        assert_eq!(reloaded.config_hash, pretty.config_hash);
        assert_eq!(reloaded.canonical, pretty.canonical);

        // The optional re-rank leg, declared in place, is a different
        // document and hashes differently.
        let rerank = DecisionPipelineConfig::load(&config_body(&with_rerank()))
            .expect("the rerank config loads");
        assert_ne!(rerank.config_hash, pretty.config_hash);

        // Unknown top-level fields refuse (deny_unknown_fields) — a config
        // from another schema generation never silently loads.
        let hostile = config_body(&without_rerank()).replace(
            r#""pipeline_id": "reference-line","#,
            r#""pipeline_id": "reference-line","sneaky": true,"#,
        );
        let msg = DecisionPipelineConfig::load(&hostile).expect_err("unknown fields refuse");
        assert!(msg.contains("pipeline config refused"), "{msg}");
    }

    /// The bounds live at the config: duplicate/unknown/out-of-order/
    /// missing stages refuse, the retrieval clamp holds, inverted score
    /// cutoffs refuse, and a label cannot mean both act and reject.
    #[test]
    fn pipeline_bounds_are_enforced_at_the_config() {
        let cases: [(String, &str); 10] = [
            // duplicate stage
            (
                config_body(
                    r#"["normalize","retrieve_context","retrieve_context","candidate_generation","decision_model","rules_policy","threshold","action_escalation"]"#,
                ),
                "duplicate stage",
            ),
            // unknown stage (serde's closed enum)
            (
                config_body(
                    r#"["normalize","self_invoke","candidate_generation","decision_model","rules_policy","threshold","action_escalation"]"#,
                ),
                "pipeline config refused",
            ),
            // out of order
            (
                config_body(
                    r#"["retrieve_context","normalize","candidate_generation","decision_model","rules_policy","threshold","action_escalation"]"#,
                ),
                "canonical order",
            ),
            // missing required stage
            (
                config_body(
                    r#"["normalize","retrieve_context","candidate_generation","decision_model","rules_policy","threshold"]"#,
                ),
                "canonical order",
            ),
            // re-rank mispositioned
            (
                config_body(
                    r#"["normalize","rerank","retrieve_context","candidate_generation","decision_model","rules_policy","threshold","action_escalation"]"#,
                ),
                "canonical order",
            ),
            // over the stage cap (nine stages)
            (
                config_body(
                    r#"["normalize","retrieve_context","candidate_generation","decision_model","rules_policy","rerank","threshold","action_escalation","threshold"]"#,
                ),
                "exceed the bound",
            ),
            // inverted score cutoffs
            (
                config_body(&without_rerank()).replace(
                    r#""score_act_at_or_above": 50"#,
                    r#""score_act_at_or_above": 5"#,
                ),
                "cutoff",
            ),
            // a label that means both act and reject
            (
                config_body(&without_rerank()).replace(
                    r#""reject_labels": ["reject"]"#,
                    r#""reject_labels": ["act"]"#,
                ),
                "both act and reject",
            ),
            // retrieval limit above the clamp
            (
                config_body(&without_rerank()).replace(r#""limit": 100"#, r#""limit": 101"#),
                "limit 101",
            ),
            // model digest not 64 lowercase hex
            (
                config_body(&without_rerank()).replace(MODEL_DIGEST, "ABCD"),
                "64 lowercase hex",
            ),
        ];
        for (raw, expected) in cases {
            let msg =
                DecisionPipelineConfig::load(&raw).expect_err("hostile configs refuse at load");
            assert!(
                msg.contains(expected),
                "the refusal must name {expected:?}: {msg}"
            );
        }

        // The clamp edge loads: limit == 100 is lawful; rrf_k 0 refuses.
        assert!(DecisionPipelineConfig::load(&config_body(&without_rerank())).is_ok());
        let zero_k = config_body(&without_rerank()).replace(r#""rrf_k": 60"#, r#""rrf_k": 0"#);
        assert!(DecisionPipelineConfig::load(&zero_k).is_err());
    }
}
