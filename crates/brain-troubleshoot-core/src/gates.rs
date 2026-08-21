use crate::evidence::{EvidenceRef, EvidenceType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateId {
    Evidence,
    Differential,
    SearchFirst,
    OneVariable,
    Corroborate,
    Verify,
    Bundle,
    Handoff,
    JustifiedRevisit,
}

impl GateId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Evidence => "G_EVIDENCE",
            Self::Differential => "G_DIFFERENTIAL",
            Self::SearchFirst => "G_SEARCH_FIRST",
            Self::OneVariable => "G_ONE_VARIABLE",
            Self::Corroborate => "G_CORROBORATE",
            Self::Verify => "G_VERIFY",
            Self::Bundle => "G_BUNDLE",
            Self::Handoff => "G_HANDOFF",
            Self::JustifiedRevisit => "G_JUSTIFIED_REVISIT",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GateRejection {
    pub gate: GateId,
    pub reason: String,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum GateResult {
    Pass,
    Reject(GateRejection),
}

pub fn gate_evidence(evidence: &[EvidenceRef], required: &[EvidenceType]) -> GateResult {
    let mut missing = Vec::new();
    for r in required {
        if !evidence.iter().any(|e| &e.evidence_type == r) {
            missing.push(r.as_str().to_string());
        }
    }
    if missing.is_empty() {
        GateResult::Pass
    } else {
        GateResult::Reject(GateRejection {
            gate: GateId::Evidence,
            reason: format!("missing evidence types: {}", missing.join(", ")),
            missing,
        })
    }
}

pub fn gate_one_variable(mutations_in_step: usize) -> GateResult {
    if mutations_in_step <= 1 {
        GateResult::Pass
    } else {
        GateResult::Reject(GateRejection {
            gate: GateId::OneVariable,
            reason: format!("{mutations_in_step} mutations in one step, only 1 allowed"),
            missing: vec![],
        })
    }
}

pub fn gate_corroborate(supporting_lines: usize) -> GateResult {
    if supporting_lines >= 2 {
        GateResult::Pass
    } else {
        GateResult::Reject(GateRejection {
            gate: GateId::Corroborate,
            reason: "need >=2 supporting lines".into(),
            missing: vec![],
        })
    }
}

pub fn gate_bundle(fields: &[(&str, bool)]) -> GateResult {
    let missing: Vec<String> = fields
        .iter()
        .filter(|(_, present)| !present)
        .map(|(k, _)| k.to_string())
        .collect();
    if missing.is_empty() {
        GateResult::Pass
    } else {
        GateResult::Reject(GateRejection {
            gate: GateId::Bundle,
            reason: format!("incomplete bundle: {}", missing.join(", ")),
            missing,
        })
    }
}

pub fn gate_approval(has_answerer: bool, needs_approval: bool) -> GateResult {
    if needs_approval && !has_answerer {
        GateResult::Reject(GateRejection {
            gate: GateId::Handoff,
            reason: "approval required but no answerer registered".into(),
            missing: vec![],
        })
    } else {
        GateResult::Pass
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictCode {
    Stale,
    Gone,
    GateOpen { gate: String, missing: Vec<String> },
    BundleIncomplete { fields: Vec<String> },
    OneVariable,
    Corroboration,
}

impl ConflictCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stale => "DI_STALE",
            Self::Gone => "DI_GONE",
            Self::GateOpen { .. } => "DI_GATE_OPEN",
            Self::BundleIncomplete { .. } => "DI_BUNDLE_INCOMPLETE",
            Self::OneVariable => "DI_ONE_VARIABLE",
            Self::Corroboration => "DI_CORROBORATION",
        }
    }
}

pub type GateFn = Box<dyn Fn() -> GateResult + Send + Sync>;

pub fn run_waterfall(gates: Vec<GateFn>) -> GateResult {
    for g in &gates {
        match g() {
            GateResult::Pass => continue,
            r @ GateResult::Reject(_) => return r,
        }
    }
    GateResult::Pass
}
