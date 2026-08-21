use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    SystemEventLog,
    DiagnosticBundle,
    HardwareInventory,
    SensorTelemetry,
    LinkTopology,
    ConfigurationState,
    WorkloadCounters,
    FaultCode,
}

impl EvidenceType {
    pub fn all() -> &'static [EvidenceType] {
        &[
            Self::SystemEventLog,
            Self::DiagnosticBundle,
            Self::HardwareInventory,
            Self::SensorTelemetry,
            Self::LinkTopology,
            Self::ConfigurationState,
            Self::WorkloadCounters,
            Self::FaultCode,
        ]
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SystemEventLog => "system_event_log",
            Self::DiagnosticBundle => "diagnostic_bundle",
            Self::HardwareInventory => "hardware_inventory",
            Self::SensorTelemetry => "sensor_telemetry",
            Self::LinkTopology => "link_topology",
            Self::ConfigurationState => "configuration_state",
            Self::WorkloadCounters => "workload_counters",
            Self::FaultCode => "fault_code",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub evidence_type: EvidenceType,
    pub locator: String,
    pub digest: String,
    pub captured_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_symptom_ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorProfile {
    pub vendor: String,
    pub artifacts: Vec<VendorArtifact>,
    pub escalation_tiers: Vec<String>,
    #[serde(default)]
    pub entitlement: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorArtifact {
    pub evidence_type: EvidenceType,
    pub capture_command: String,
    pub description: String,
}
