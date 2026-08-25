//! Aftersales evidence — the same law troubleshoot-core obeys, with the
//! fulfillment domain's own artifact vocabulary. Every disposition decision
//! cites the evidence it consumed by locator + digest.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    ProofOfPurchase,
    DiagnosticBundle,
    SerialBatch,
    Photos,
    InspectionReport,
}

impl EvidenceType {
    pub fn all() -> &'static [EvidenceType] {
        &[
            EvidenceType::ProofOfPurchase,
            EvidenceType::DiagnosticBundle,
            EvidenceType::SerialBatch,
            EvidenceType::Photos,
            EvidenceType::InspectionReport,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceType::ProofOfPurchase => "proof_of_purchase",
            EvidenceType::DiagnosticBundle => "diagnostic_bundle",
            EvidenceType::SerialBatch => "serial_batch",
            EvidenceType::Photos => "photos",
            EvidenceType::InspectionReport => "inspection_report",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub evidence_type: EvidenceType,
    pub locator: String,
    pub digest: String,
    pub captured_at: i64,
}
