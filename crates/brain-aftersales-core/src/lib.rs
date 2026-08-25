//! The aftersales engine core: fulfillment gates over the governed-workflow
//! substrate. Financial execution never happens here — dispositions are
//! HITL proposals, the gates only decide whether a proposal may exist.

#![deny(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Tests read as sibling cores' tests do (assert + unwrap on known-good
// fixtures); the production-code denies stay absolute.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod disposition;
pub mod evidence;
pub mod gates;

use gates::{GateFn, GateId, GateResult};

/// The standard fulfillment waterfall for a return/warranty/repair run:
/// entitlement → window → disposition, in that order.
pub fn fulfillment_waterfall(
    has_entitlement_row: bool,
    within_window: bool,
    disposition_is_proposal: bool,
) -> GateResult {
    let gates: Vec<GateFn> = vec![
        Box::new(move || {
            if has_entitlement_row {
                GateResult::Pass
            } else {
                GateResult::Reject(format!(
                    "{} no governed entitlement row grants coverage",
                    GateId::Entitlement.as_str()
                ))
            }
        }),
        Box::new(move || {
            if within_window {
                GateResult::Pass
            } else {
                GateResult::Reject(format!(
                    "{} claim is outside its legal window",
                    GateId::Window.as_str()
                ))
            }
        }),
        Box::new(move || {
            if disposition_is_proposal {
                GateResult::Pass
            } else {
                GateResult::Reject(format!(
                    "{} a disposition must be a HITL proposal, never an auto-execution",
                    GateId::Disposition.as_str()
                ))
            }
        }),
    ];
    gates::run_waterfall(gates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{EvidenceRef, EvidenceType};
    use brain_troubleshoot_core::evidence::EvidenceType as TsEvidence;

    #[test]
    fn aftersales_gates_are_a_waterfall() {
        // All legs hold → Pass.
        assert_eq!(fulfillment_waterfall(true, true, true), GateResult::Pass);
        // Entitlement failure is THE answer even when later gates would
        // fail too — order is the law.
        match fulfillment_waterfall(false, false, false) {
            GateResult::Reject(reason) => {
                assert!(
                    reason.starts_with("G_ENTITLEMENT"),
                    "first rejection wins: {reason}"
                );
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
        // Entitlement granted but outside the window → the window gate.
        match fulfillment_waterfall(true, false, false) {
            GateResult::Reject(reason) => {
                assert!(reason.starts_with("G_WINDOW"));
            }
            other => panic!("expected window rejection, got {other:?}"),
        }
        // A non-proposal disposition can never ride through.
        match fulfillment_waterfall(true, true, false) {
            GateResult::Reject(reason) => {
                assert!(reason.starts_with("G_DISPOSITION"));
            }
            other => panic!("expected disposition rejection, got {other:?}"),
        }
        // The evidence vocabulary is the fulfillment domain's own; the
        // diagnostic bundle name stays shared with troubleshoot-core (one
        // artifact shape across worktypes).
        let e = EvidenceRef {
            evidence_type: EvidenceType::DiagnosticBundle,
            locator: "s3://bundle/x".to_string(),
            digest: "abc".to_string(),
            captured_at: 0,
        };
        assert_eq!(e.evidence_type.as_str(), "diagnostic_bundle");
        assert_eq!(
            EvidenceType::DiagnosticBundle.as_str(),
            TsEvidence::DiagnosticBundle.as_str()
        );
        assert_eq!(EvidenceType::all().len(), 5);
    }
}
