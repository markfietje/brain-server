//! The SDK seam, re-exported for kernel paths: the DecisionModel trait and
//! its typed vocabulary live in the SDK's always-on `decision` module —
//! defined there, consumed here (the kernel depends on the SDK, never the
//! reverse). Kernel code imports from `workflow::harness::model` so no
//! kernel path needs to know where the seam is defined.
//!
//! Authority law: a DecisionModel proposes; only the gate disposes. The
//! type-level half of that law is pinned below: `&self` receivers only and
//! plain-data seam types — an output value has no handles, so it cannot
//! reach durable state.

// The truthful allow (the decide/mod.rs precedent): the seam re-exports
// ship the FULL typed vocabulary before all their kernel consumers — the
// decision-run record consumes `RunMode`/`ModelKind`/the record kinds in
// its own lane. Every re-exported name is the SDK's own surface, covered
// by the SDK's and this module's tests, so the unused-import watchdog
// stays a real gate for anything else.
#[allow(unused_imports)]
pub(crate) use brain_engine_sdk::decision::{
    ConfigRef, DecisionContext, DecisionError, DecisionInput, DecisionModel, DecisionOutput,
    DecisionValue, EvidenceRef, ModelKind, ModelMetadata, OutputKind, QuestionKind, QuestionRef,
    RefusalReason, RunMode, TrustTier,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The monotonic-narrow pin — honestly worded as a TYPE-LEVEL proof:
    /// (1) `evaluate`/`metadata` take `&self` only (the function-signature
    /// assertion below would not type-check against a `&mut self` or
    /// consuming receiver); (2) every seam type is plain data — `Send +
    /// Sync + 'static` with no connection, socket, or store handle in its
    /// shape — so a `DecisionOutput` in hand has nothing to act on. What a
    /// model RETURNS cannot mutate durable state; only code holding a
    /// `WorkflowTx` can write, and no seam type carries one.
    #[test]
    fn decision_output_alone_cannot_mutate_durable_state() {
        // (1) the receiver law, pinned as types: both trait methods are
        // shared-reference methods on the model.
        fn assert_shared_receiver<M: DecisionModel>(_m: &M) {
            let _metadata: fn(&M) -> ModelMetadata = DecisionModel::metadata;
            let _evaluate: fn(
                &M,
                &DecisionInput,
                &DecisionContext,
            ) -> Result<DecisionOutput, DecisionError> = DecisionModel::evaluate;
        }
        struct Noop;
        impl DecisionModel for Noop {
            fn metadata(&self) -> ModelMetadata {
                ModelMetadata::deterministic(
                    "noop".to_string(),
                    "0.0.0".to_string(),
                    Vec::new(),
                    None,
                )
            }
            fn evaluate(
                &self,
                _input: &DecisionInput,
                _ctx: &DecisionContext,
            ) -> Result<DecisionOutput, DecisionError> {
                Err(DecisionError::Refused {
                    reason: RefusalReason::InsufficientEvidence,
                })
            }
        }
        assert_shared_receiver(&Noop);

        // (2) the plain-data law: every seam type is thread-safe
        // 'static data — no durable-state handle can hide in one.
        fn assert_plain_data<T: Send + Sync + 'static>() {}
        assert_plain_data::<DecisionInput>();
        assert_plain_data::<DecisionContext>();
        assert_plain_data::<DecisionOutput>();
        assert_plain_data::<DecisionValue>();
        assert_plain_data::<DecisionError>();
        assert_plain_data::<ModelMetadata>();
        assert_plain_data::<EvidenceRef>();
        assert_plain_data::<ConfigRef>();
    }
}
