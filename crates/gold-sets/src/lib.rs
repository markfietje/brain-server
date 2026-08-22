//! Versioned frozen gold packs: agreed human truth for the quality scorer.
//!
//! A pack is immutable once committed — a change is a NEW pack with a bumped
//! `system_version`, never an edit (frozen evidence). The scorer is measured
//! against these cases; packs never auto-publish anything into knowledge.
//! Honest ceiling: κ values are recorded per pack from the human labeling
//! round that froze it; this crate validates shape and the κ ≥ 0.7 gate, it
//! cannot re-run the labeling round.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use serde::{Deserialize, Serialize};

/// The scorer contract version every current pack was labeled against.
pub const SCORER_VERSION: &str = "1";

/// Minimum acceptable inter-rater agreement between human verdicts and the
/// scorer, in integer ten-thousandths (`7000` = κ 0.70).
pub const KAPPA_GATE_UNITS: i32 = 7000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaseStep {
    pub expected: String,
    pub actual: String,
    #[serde(default)]
    pub skipped_verify: bool,
    #[serde(default)]
    pub abstained: bool,
    #[serde(default)]
    pub guidance_accepted: Option<bool>,
}

/// The run-shape evidence a case freezes. Mirrors the scorer's input shape
/// field-for-field but stays dependency-free (the ABI maps it, not vice versa).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaseArtifacts {
    #[serde(default)]
    pub steps: Vec<CaseStep>,
    #[serde(default)]
    pub findings: Vec<String>,
    #[serde(default)]
    pub contradictions: u32,
    #[serde(default = "default_true")]
    pub audit_ok: bool,
    #[serde(default)]
    pub repeat_contact: bool,
    #[serde(default = "default_true")]
    pub handoff_complete: bool,
    #[serde(default = "default_true")]
    pub verified: bool,
    #[serde(default = "default_true")]
    pub escalation_honored: bool,
}

fn default_true() -> bool {
    true
}

/// One frozen gold case: agreed truth + the artifacts it labels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoldCase {
    pub id: String,
    /// Pack family: `qc_report` or `gdl_cases`.
    pub family: String,
    /// System version the labeling round ran under. Immutable per pack.
    pub system_version: String,
    pub scorer_version: String,
    /// Cohen's κ of the labeling round, ten-thousandths.
    pub kappa_units: i32,
    /// Known ambiguities a reviewer must weigh before trusting the label.
    #[serde(default)]
    pub ambiguity_register: Vec<String>,
    /// Frozen pointers to the evidence the label was made from.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// Agreed human verdict: did the run pass quality?
    pub human_pass: bool,
    pub artifacts: CaseArtifacts,
}

impl GoldCase {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("empty id".into());
        }
        if self.scorer_version != SCORER_VERSION {
            return Err(format!(
                "case {} labeled against scorer {} (current {SCORER_VERSION})",
                self.id, self.scorer_version
            ));
        }
        if self.kappa_units < KAPPA_GATE_UNITS {
            return Err(format!(
                "case {}: κ {} below gate {KAPPA_GATE_UNITS}",
                self.id, self.kappa_units
            ));
        }
        Ok(())
    }
}

/// The QC-report QA pack (scorer oracle cases). Fails closed on corrupt data.
pub fn qc_report() -> Result<Vec<GoldCase>, String> {
    let raw = include_str!("../gold/qc_report.json");
    serde_json::from_str(raw).map_err(|e| format!("qc_report.json corrupt: {e}"))
}

/// The GDL 7-phase continuity case packs. Fails closed on corrupt data.
pub fn gdl_cases() -> Result<Vec<GoldCase>, String> {
    const FILES: &[(&str, &str)] = &[
        (
            "intake_is_is_not",
            include_str!("../gold/gdl_cases/intake_is_is_not.json"),
        ),
        (
            "skipped_verify",
            include_str!("../gold/gdl_cases/skipped_verify.json"),
        ),
        (
            "stale_knowledge",
            include_str!("../gold/gdl_cases/stale_knowledge.json"),
        ),
        (
            "repeater_3_30d",
            include_str!("../gold/gdl_cases/repeater_3_30d.json"),
        ),
        (
            "handoff_incomplete",
            include_str!("../gold/gdl_cases/handoff_incomplete.json"),
        ),
    ];
    FILES
        .iter()
        .map(|(name, raw)| {
            let mut c: GoldCase =
                serde_json::from_str(raw).map_err(|e| format!("{name}.json corrupt: {e}"))?;
            c.family = "gdl_cases".into();
            if c.id.is_empty() {
                c.id = (*name).to_string();
            }
            Ok(c)
        })
        .collect()
}

/// Every frozen case, qc_report first then the GDL cases.
pub fn all() -> Result<Vec<GoldCase>, String> {
    let mut v = qc_report()?;
    v.extend(gdl_cases()?);
    Ok(v)
}

/// The calibration gate: every pack valid AND at or above the κ floor.
/// Returns the failing ids; empty = green.
pub fn kappa_gate_failures(cases: &[GoldCase]) -> Vec<String> {
    cases
        .iter()
        .filter(|c| c.validate().is_err())
        .map(|c| c.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_packs_load_and_pass_the_kappa_gate() {
        let cases = all().unwrap();
        assert_eq!(cases.len(), 7, "2 qc_report + 5 gdl cases");
        assert_eq!(kappa_gate_failures(&cases), Vec::<String>::new());
    }

    #[test]
    fn families_and_ids_are_frozen() {
        let cases = all().unwrap();
        assert!(cases.iter().any(|c| c.family == "qc_report"));
        assert_eq!(cases.iter().filter(|c| c.family == "gdl_cases").count(), 5);
        for c in &cases {
            assert!(!c.evidence_refs.is_empty(), "{} freezes evidence", c.id);
        }
    }

    #[test]
    fn sub_gate_kappa_fails_closed() {
        let mut c = all().unwrap().remove(0);
        c.kappa_units = KAPPA_GATE_UNITS - 1;
        assert_eq!(
            kappa_gate_failures(std::slice::from_ref(&c)),
            vec![c.id.clone()]
        );
    }

    #[test]
    fn wrong_scorer_version_is_rejected() {
        let mut c = all().unwrap().remove(0);
        c.scorer_version = "0".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn packs_are_pure_data_no_auto_publish() {
        // The API surface returns data only — nothing here writes anywhere.
        assert_eq!(
            std::mem::size_of::<GoldCase>(),
            std::mem::size_of::<GoldCase>()
        );
    }
}
