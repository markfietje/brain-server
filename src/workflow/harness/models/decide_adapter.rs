//! The decide adapter — the shipped pure decide line (the Phase-0 port)
//! CONSUMED, never rebuilt: the shipped `TypedQuestion`/`QType` closed
//! vocabulary maps onto the SDK seam's question/output vocabulary (0/1/2
//! on both sides), and the shipped `route()` — the closed precedence
//! chain over the router's own three checkpoints — becomes a `Choice`
//! output. The adapter adds no vocabulary of its own: if the router ever
//! returned a name outside its declared checkpoints, the adapter refuses
//! rather than widen.
//!
//! Seam laws hold here too: no raw text crosses the boundary (the
//! language detection the router needs is computed by the shipped
//! `lang::analyse` over an EMPTY state — the deterministic unknown-script
//! arm), and the adapter proposes only.

use std::collections::BTreeSet;

use crate::workflow::decide::lang;
use crate::workflow::decide::router::{CHECKPOINTS, route};
use crate::workflow::decide::sequence::{QType, TypedQuestion};

use crate::workflow::harness::model::{
    DecisionContext, DecisionError, DecisionInput, DecisionModel, DecisionOutput, DecisionValue,
    ModelMetadata, OutputKind, QuestionKind, QuestionRef, RefusalReason,
};

/// The shipped `QType` → the seam's question kind (the 0/1/2 mirror).
pub(crate) fn question_kind_of(qtype: QType) -> QuestionKind {
    match qtype {
        QType::Choice => QuestionKind::Choice,
        QType::Score => QuestionKind::Score,
        QType::Noul => QuestionKind::Noul,
    }
}

/// The seam's question kind → the shipped `QType` (the reverse mirror;
/// the round trip is lossless by construction).
pub(crate) fn qtype_of(kind: QuestionKind) -> QType {
    match kind {
        QuestionKind::Choice => QType::Choice,
        QuestionKind::Score => QType::Score,
        QuestionKind::Noul => QType::Noul,
    }
}

/// The shipped `QType` → the seam's output kind (a question's type is the
/// answer's type in this vocabulary — the same 0/1/2 codes).
pub(crate) fn output_kind_of(qtype: QType) -> OutputKind {
    match qtype {
        QType::Choice => OutputKind::Choice,
        QType::Score => OutputKind::Score,
        QType::Noul => OutputKind::Noul,
    }
}

/// A shipped question → the seam's question ref (id + mirrored kind).
pub(crate) fn question_ref_of(id: &str, q: &TypedQuestion) -> QuestionRef {
    QuestionRef {
        id: id.to_string(),
        kind: question_kind_of(q.qtype),
    }
}

/// The decide-line adapter: wraps the shipped router behind the seam.
pub(crate) struct DecideAdapter {
    model_id: String,
    model_version: String,
    default_checkpoint: String,
    standalone: bool,
    auto_task_detection: bool,
}

impl DecideAdapter {
    /// The checkpoint default must be one of the router's OWN declared
    /// checkpoints — the adapter inherits the router's vocabulary, it
    /// does not define one.
    pub(crate) fn new(
        model_id: String,
        model_version: String,
        default_checkpoint: &str,
        standalone: bool,
        auto_task_detection: bool,
    ) -> Result<Self, String> {
        if !CHECKPOINTS.contains(&default_checkpoint) {
            return Err(format!(
                "decide adapter refused: {default_checkpoint:?} is not one of the router's \
                 checkpoints ({})",
                CHECKPOINTS.join(" | ")
            ));
        }
        Ok(Self {
            model_id,
            model_version,
            default_checkpoint: default_checkpoint.to_string(),
            standalone,
            auto_task_detection,
        })
    }
}

impl DecisionModel for DecideAdapter {
    fn metadata(&self) -> ModelMetadata {
        ModelMetadata::deterministic(
            self.model_id.clone(),
            self.model_version.clone(),
            vec![OutputKind::Choice],
            None,
        )
    }

    fn evaluate(
        &self,
        input: &DecisionInput,
        ctx: &DecisionContext,
    ) -> Result<DecisionOutput, DecisionError> {
        // The shipped analyser over an EMPTY state: deterministic (the
        // unknown-script arm), and no raw text ever crosses the seam.
        let detection = lang::analyse(&serde_json::json!({}));
        let ids: BTreeSet<String> = input.question_ids.iter().cloned().collect();
        let routed = route(
            &ids,
            None,
            None,
            None,
            self.auto_task_detection,
            &self.default_checkpoint,
            self.standalone,
            &detection,
        )
        .map_err(|_| DecisionError::Refused {
            reason: RefusalReason::OutOfVocabulary,
        })?;
        // Closed-vocabulary defense: the router's own checkpoints are the
        // only labels this adapter may emit.
        if !CHECKPOINTS.contains(&routed.model.as_str()) {
            return Err(DecisionError::Refused {
                reason: RefusalReason::OutOfVocabulary,
            });
        }
        Ok(DecisionOutput {
            model_id: self.model_id.clone(),
            model_version: self.model_version.clone(),
            produced_at: ctx.created_at,
            value: DecisionValue::Choice {
                label: routed.model,
                probabilities: None,
                confidence: None,
            },
            evidence_refs: input.evidence.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::harness::model::RunMode;
    use brain_engine_sdk::decision::{ConfigRef, EvidenceRef, TrustTier};

    fn ctx() -> DecisionContext {
        DecisionContext {
            run_id: 3,
            mode: RunMode::Deterministic,
            config_digests: vec![ConfigRef {
                key: "k".into(),
                digest: "d".into(),
            }],
            role_scope: Vec::new(),
            created_at: 1_800_000_456,
        }
    }

    fn input(ids: &[&str], auto: bool) -> DecisionInput {
        let _ = auto;
        DecisionInput {
            request_id: "req-1".into(),
            question: None,
            question_ids: ids.iter().map(|s| s.to_string()).collect(),
            evidence: vec![EvidenceRef {
                evidence_id: "e-1".into(),
                tier: TrustTier::Governed,
            }],
        }
    }

    /// The shipped typed-question vocabulary maps 0/1/2-exact onto the
    /// seam's vocabulary (both directions, losslessly), a shipped
    /// question becomes a seam question ref, and the REAL router decides
    /// through the adapter: the empty state's default arm lands the
    /// default checkpoint, and an exact typed-decisions workflow id set
    /// lands `typed-decisions` — the R19 port consumed, not rebuilt.
    #[test]
    fn decide_adapter_maps_the_shipped_typed_question_vocabulary() {
        // The mirror over all three shipped types: codes stay 0/1/2 on
        // both sides, and the round trip is lossless.
        for (qtype, code) in [(QType::Choice, 0), (QType::Score, 1), (QType::Noul, 2)] {
            let kind = question_kind_of(qtype);
            assert_eq!(kind.as_u8(), code);
            assert_eq!(kind.as_u8(), qtype as i32 as u8);
            assert_eq!(output_kind_of(qtype).as_u8(), code);
            assert_eq!(qtype_of(kind), qtype, "the mirror round-trips");
        }
        let shipped = TypedQuestion::choice(
            "who acts?",
            &[("steward", Some("the operator")), ("none", None)],
        );
        let reference = question_ref_of("actor", &shipped);
        assert_eq!(reference.id, "actor");
        assert_eq!(reference.kind, QuestionKind::Choice);

        // The real router, through the adapter: empty state → the
        // unknown-script arm → the adapter's declared default.
        let adapter = DecideAdapter::new(
            "decide-router".into(),
            "1.0.0".into(),
            "multilingual",
            false,
            false,
        )
        .unwrap();
        assert_eq!(adapter.metadata().id(), "decide-router");
        assert_eq!(adapter.metadata().output_vocabulary(), [OutputKind::Choice]);
        let out = adapter.evaluate(&input(&[], false), &ctx()).unwrap();
        match out.value {
            DecisionValue::Choice { label, .. } => assert_eq!(label, "multilingual"),
            other => panic!("the router decides a checkpoint: {other:?}"),
        }
        assert_eq!(out.produced_at, 1_800_000_456);
        assert_eq!(out.evidence_refs.len(), 1, "provenance travels");

        // The real workflow matcher, through the adapter: the EXACT
        // invoice_processing id set auto-routes typed-decisions.
        let auto = DecideAdapter::new(
            "decide-router".into(),
            "1.0.0".into(),
            "english",
            false,
            true,
        )
        .unwrap();
        let out = auto
            .evaluate(
                &input(
                    &[
                        "discrepancy_severity",
                        "disposition",
                        "duplicate",
                        "matches_order",
                        "urgency",
                    ],
                    true,
                ),
                &ctx(),
            )
            .unwrap();
        match out.value {
            DecisionValue::Choice { label, .. } => assert_eq!(label, "typed-decisions"),
            other => panic!("the workflow match lands typed-decisions: {other:?}"),
        }

        // The adapter adds no vocabulary: a default outside the router's
        // checkpoints is refused at construction.
        assert!(DecideAdapter::new("x".into(), "1.0.0".into(), "laya-pro", false, false).is_err());
    }
}
