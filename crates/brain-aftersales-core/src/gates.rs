//! Fulfillment gates — troubleshoot-core's gate/waterfall shape over the
//! aftersales decision path: ENTITLEMENT (does a governed registry row
//! grant coverage?) → WINDOW (is the claim inside its legal window?) →
//! DISPOSITION (is the ranked disposition a HITL proposal, ready for the
//! human?). Gates run in order; the first rejection is THE answer — a later
//! gate never papers over an earlier failure.

/// The gate ids, stable strings for evidence rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateId {
    Entitlement,
    Window,
    Disposition,
}

impl GateId {
    pub fn as_str(&self) -> &'static str {
        match self {
            GateId::Entitlement => "G_ENTITLEMENT",
            GateId::Window => "G_WINDOW",
            GateId::Disposition => "G_DISPOSITION",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateResult {
    Pass,
    Reject(String),
}

pub type GateFn = Box<dyn Fn() -> GateResult + Send + Sync>;

/// The waterfall: first rejection wins, order is the law.
pub fn run_waterfall(gates: Vec<GateFn>) -> GateResult {
    for g in &gates {
        match g() {
            GateResult::Pass => continue,
            r @ GateResult::Reject(_) => return r,
        }
    }
    GateResult::Pass
}
