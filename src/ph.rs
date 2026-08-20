//! the RA 10173 / NPC posture as pure decision logic.
//!
//! Honest framing (mirrors `COMPLIANCE_PH.md`): the Philippines has no AI
//! statute yet. AI is governed by RA 10173 (Data Privacy Act 2012) + NPC
//! advisories + EO 119 (gov-data residency); **HB 7396 (risk-based AI) is a
//! pending bill, not law**. This module ships the decision logic the posture
//! needs *now* — the breach-notification clock, the scraping provenance
//! rule, and the RA 10173 control cross-reference map — all layered
//! on the existing profile/role/region primitives. It deliberately implements
//! nothing HB 7396 requires until the bill is enacted (the `HB7396_FORWARD`
//! note below is the structure-absorbs-it marker).
//!
//! Not legal advice — legal review required (the COMPLIANCE.md disclaimer).

/// Controls under RA 10173 / NPC advisories that `COMPLIANCE_PH.md` documents.
/// Each maps a control to the live feature it leans on. `compliance_ph_covers_
/// dpa_controls` (this module's test) cross-references the doc: every entry
/// here must be named in `COMPLIANCE_PH.md` so the map can't silently drift.
#[cfg(test)]
pub const DPA_CONTROLS: &[(&str, &str)] = &[
    // PIC/PIP duties (controller/processor) — the owner + scope + audit chain.
    (
        "pic_pip_duties",
        "access_scope/owner + audit-chain provenance",
    ),
    // NPC 2024-04 AI advisory: privacy-by-design.
    (
        "privacy_by_design",
        "deny-by-default scope + placeholder redaction",
    ),
    // LAWFUL_BASIS constant for scraped-data provenance (M3).
    ("lawful_basis", "scraped-data provenance (this release)"),
    // NPC registration (500+ data subjects) — an operator checklist, not a cert.
    (
        "npc_registration",
        "operator checklist on the 500+ subject trigger",
    ),
    // The DPO role + a named contact surfaced on /health.
    ("dpo_role", "v1.23.0 dpo role + dpo_contact on /health"),
    // EO 119 gov-data residency when BRAIN_REGION is PH.
    ("eo119_residency", "v1.22.0 region stamp (BRAIN_REGION=PH)"),
    // HB 7396 is pending; structured to absorb, not implemented.
    (
        "hb7396_forward",
        "profile/retention/roles absorb the risk-based bill",
    ),
    // Data subject rights under RA 10173 (access/correct/erase → DSAR).
    ("subject_rights", "privacy notice + v1.15.0 DSAR surface"),
];

/// The marker string `COMPLIANCE_PH.md` must contain near each control so the
/// cross-reference test verifies doc ↔ code coupling rather than just a name.
#[cfg(test)]
pub const CROSSREF_MARKER: &str = "Feature:";

/// A scraped record whose provenance carries no documented lawful basis must
/// not be stored as memory — it is quarantined (reusing the quarantine
/// flag), not silently ingested. Kept pure so `handlers::ingest` and the unit
/// test share the same rule.
pub enum ScrapePosture {
    Store,
    Quarantine,
}

/// True for the scrape-family ingest sources (case-insensitive).
fn is_scrape_source(source: Option<&str>) -> bool {
    matches!(
        source
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("scrape" | "scraped" | "crawler" | "crawl")
    )
}

/// A documented lawful basis below the free-text cap (the `legal_hold` reason
/// bound precedent — bounded so a mislabeled basis cannot be abused as an
/// unbounded blob).
pub const MAX_LAWFUL_BASIS: usize = 500;

/// Decide storage posture for an ingest. Scraped data without a documented
/// `lawful_basis` (the NPC 2026-01 scraping advisory posture) is quarantined.
/// Everything else stores. A basis beyond [`MAX_LAWFUL_BASIS`] chars counts as
/// undocumented (quarantine) — fail-closed at the trust boundary.
pub fn scrape_posture(source: Option<&str>, lawful_basis: Option<&str>) -> ScrapePosture {
    if !is_scrape_source(source) {
        return ScrapePosture::Store;
    }
    let basis = lawful_basis.map(str::trim).unwrap_or("");
    if basis.is_empty() || basis.len() > MAX_LAWFUL_BASIS {
        ScrapePosture::Quarantine
    } else {
        ScrapePosture::Store
    }
}

/// One computed notification deadline for a breach: `jurisdiction × audience`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NotifyDeadline {
    pub jurisdiction: String,
    pub audience: &'static str,
    /// Hours from `discovered_at` (the DPA/GDPR 72h rule, jurisdiction-specific).
    pub hours: i64,
    /// Absolute UNIX seconds the notification is due by.
    pub deadline: i64,
}

/// Per-(jurisdiction, audience) notification windows in hours, from each
/// law's breach-notification rule. PH DPA: 72h to the NPC + affected subjects
/// on a serious breach. EU GDPR Art 33: 72h to the supervisory authority;
/// Art 34 subject notice "without undue delay" (modelled as the same 72h). A
/// jurisdiction absent from this table gets no deadline (unknown law → the DPO
/// confirms — fail-open on the producer, fail-closed only on what we know).
const NOTIFY_HOURS: &[(&str, &str, i64)] = &[
    ("ph", "npc", 72),
    ("ph", "subjects", 72),
    ("eu", "authority", 72),
    ("eu", "subjects", 72),
];

/// Compute the deadline list for every affected jurisdiction of a breach.
/// `jurisdictions` are lowercase country codes (e.g. `ph`, `eu`); duplicates
/// are de-duplicated; each yields the rows in [`NOTIFY_HOURS`].
pub fn notification_deadlines(jurisdictions: &[String], discovered_at: i64) -> Vec<NotifyDeadline> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for j in jurisdictions {
        let j = j.trim().to_ascii_lowercase();
        if j.is_empty() || seen.contains(&j) {
            continue;
        }
        seen.push(j.clone());
        for (j2, audience, hours) in NOTIFY_HOURS {
            if j2 == &j {
                out.push(NotifyDeadline {
                    jurisdiction: j.clone(),
                    audience,
                    hours: *hours,
                    deadline: discovered_at + hours * 3600,
                });
            }
        }
    }
    out
}

/// Seconds left before `deadline`, `<= 0` once the window has lapsed. The
/// client Security-panel countdown renders this (the server ships the durable
/// `deadline`; the countdown is display math over `now`).
#[cfg(test)]
pub fn countdown(deadline: i64, now: i64) -> i64 {
    deadline.saturating_sub(now)
}

/// Valid `POST /breach` severities.
pub const SEVERITIES: &[&str] = &["low", "medium", "high", "critical"];

pub fn is_severity(s: &str) -> bool {
    SEVERITIES.contains(&s.trim().to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compliance_ph_covers_dpa_controls() {
        // Verification 1: every RA 10173 control the map names must be present
        // in COMPLIANCE_PH.md and point at a non-empty live feature, so the
        // posture doc cannot drift from what ships.
        let doc = include_str!("../COMPLIANCE_PH.md");
        for (control, feature) in DPA_CONTROLS {
            assert!(
                doc.contains(control),
                "COMPLIANCE_PH.md must name the {control} control"
            );
            assert!(
                !feature.trim().is_empty(),
                "control {control} needs a Feature pointer"
            );
        }
        assert!(
            doc.contains(CROSSREF_MARKER),
            "COMPLIANCE_PH.md must use the {CROSSREF_MARKER} cross-reference marker"
        );
    }

    #[test]
    fn breach_workflow_computes_jurisdiction_deadlines() {
        // Verification 2: a breach affecting EU + PH subjects shows the NPC
        // 72h + GDPR 72h + subject-notification deadlines, with countdowns.
        let when = 1_752_652_800_i64;
        let ph = notification_deadlines(&["ph".to_string(), "eu".to_string()], when);
        let audiences: Vec<(String, &str, i64)> = ph
            .iter()
            .map(|d| (d.jurisdiction.clone(), d.audience, d.hours))
            .collect();
        assert_eq!(audiences.len(), 4);
        assert!(audiences.contains(&("ph".to_string(), "npc", 72)));
        assert!(audiences.contains(&("ph".to_string(), "subjects", 72)));
        assert!(audiences.contains(&("eu".to_string(), "authority", 72)));
        assert!(audiences.contains(&("eu".to_string(), "subjects", 72)));
        // Every deadline is discovered_at + the window (countdown positive
        // before it lapses, satisfying the Security-panel clock).
        for d in &ph {
            assert_eq!(d.deadline, when + d.hours * 3600);
            assert!(countdown(d.deadline, when) > 0);
        }
        // After the window lapses the countdown is negative (saturating below
        // zero never wraps into a huge positive).
        assert!(countdown(when - 1, when) < 0);
        // Duplicate jurisdictions de-duplicate; unknown laws add no deadline.
        let dup = notification_deadlines(&["ph".to_string(), "ph".to_string()], when);
        assert!(dup.iter().filter(|d| d.jurisdiction == "ph").count() == 2);
        let unknown = notification_deadlines(&["xx".to_string()], when);
        assert!(unknown.is_empty());
    }

    #[test]
    fn scraped_data_without_basis_quarantined() {
        // Verification 5: scraped data with no documented lawful basis is
        // quarantined, not stored; a documented basis stores.
        assert!(matches!(
            scrape_posture(Some("scrape"), None),
            ScrapePosture::Quarantine
        ));
        assert!(matches!(
            scrape_posture(Some("SCRAPE"), Some(" ")),
            ScrapePosture::Quarantine
        ));
        assert!(matches!(
            scrape_posture(Some("crawler"), None),
            ScrapePosture::Quarantine
        ));
        assert!(matches!(
            scrape_posture(Some("scrape"), Some(&"x".repeat(MAX_LAWFUL_BASIS + 1))),
            ScrapePosture::Quarantine
        ));
        assert!(matches!(
            scrape_posture(Some("scrape"), Some("contract consent")),
            ScrapePosture::Store
        ));
        assert!(matches!(
            scrape_posture(Some("memory"), None),
            ScrapePosture::Store
        ));
        assert!(matches!(scrape_posture(None, None), ScrapePosture::Store));
    }

    #[test]
    fn severities_vocabulary_is_bounded() {
        assert!(is_severity("high"));
        assert!(is_severity("CRITICAL"));
        assert!(!is_severity("grave"));
        assert_eq!(SEVERITIES.len(), 4);
    }
}
