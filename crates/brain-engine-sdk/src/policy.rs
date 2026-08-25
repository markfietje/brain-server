//! Law/compliance vocabulary as pure data: the single owner of the P-class
//! SLA clock and the default per-kind retention table. The server facades
//! these verbatim (env overrides layer on top there); engines read the same
//! truth — policy is never duplicated across the ABI.

/// Priority class for an inbound request; the SLA clock it buys.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Priority {
    P1,
    P2,
    P3,
    P4,
}

impl Priority {
    /// Time-to-live seconds per class: P1 4h, P2 24h, P3 72h, P4 7d.
    pub fn ttl_secs(&self) -> i64 {
        match self {
            Priority::P1 => 4 * 3600,
            Priority::P2 => 24 * 3600,
            Priority::P3 => 72 * 3600,
            Priority::P4 => 168 * 3600,
        }
    }
}

/// An SLA-stamped envelope: priority + derived deadline (+ optional law
/// version stamped at intake so every downstream artifact cites the law that
/// was in force when the case opened).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Envelope {
    pub p_class: Priority,
    pub sla_deadline: i64,
    /// Acknowledgment clock. Non-complaint classes keep a single clock
    /// (`ack_deadline == sla_deadline`); complaints carry the ISO 10002
    /// posture where acknowledgment is its own, always-tighter deadline.
    pub ack_deadline: i64,
    pub created_at: i64,
    /// Empty string = unknown jurisdiction / unstamped.
    pub law_version: &'static str,
}

/// Complaint acknowledgment budget (ISO 10002 posture): acknowledged within
/// the hour, by policy.
pub const COMPLAINT_ACK_SECS: i64 = 3600;

/// Complaint response clock: a P2-class response window.
pub const COMPLAINT_RESPONSE_SECS: i64 = 72 * 3600;

/// Stamp a complaint envelope: distinct priority map (P2 minimum) and its
/// own acknowledgment deadline, always tighter than the response deadline.
pub fn stamp_complaint_envelope(created_at: i64) -> Envelope {
    Envelope {
        sla_deadline: created_at + COMPLAINT_RESPONSE_SECS,
        ack_deadline: created_at + COMPLAINT_ACK_SECS,
        p_class: Priority::P2,
        created_at,
        law_version: "",
    }
}

/// The curated law-version table: one version label per jurisdiction code,
/// re-checked on each release. Single owner — the server facade and the
/// legal-rule seeds read this same truth.
pub const LAW_VERSIONS: &[(&str, &str)] = &[
    ("ph", "npc-advisory-2024-04"),
    ("eu", "gdpr-consolidated-2021"),
    ("uk", "uk-gdpr-idta-2021"),
    ("us", "ccpa-cpra-2023-amended"),
    ("au", "privacy-act-apps-2019"),
    ("sg", "pdpa-2020-amended"),
    ("ca", "pipeda-2019-amended"),
    ("nl", "wet-ob-1968-rev2024"),
];

/// Resolve a jurisdiction code (lowercase) to its law-version label.
pub fn law_version_for(jurisdiction: &str) -> Option<&'static str> {
    let c = jurisdiction.trim().to_ascii_lowercase();
    LAW_VERSIONS.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)
}

/// Stamp an envelope with its SLA deadline from the class TTL table.
pub fn stamp_envelope(created_at: i64, p_class: Priority) -> Envelope {
    Envelope {
        sla_deadline: created_at + p_class.ttl_secs(),
        ack_deadline: created_at + p_class.ttl_secs(),
        p_class,
        created_at,
        law_version: "",
    }
}

/// Intake stamping: SLA deadline + the law version in force for the case's
/// jurisdiction at open time. Unknown jurisdictions stamp empty (fail-open on
/// labeling only — nothing enforces on it downstream).
pub fn stamp_envelope_for_jurisdiction(
    created_at: i64,
    p_class: Priority,
    jurisdiction: &str,
) -> Envelope {
    let mut e = stamp_envelope(created_at, p_class);
    e.law_version = law_version_for(jurisdiction).unwrap_or("");
    e
}

/// A post-sale worktype: the run `kind` an intent class routes to. One
/// substrate, many worktypes — each carries its own SLA envelope class
/// (the deterministic policy row), so intake and engines read the same
/// clock table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Worktype {
    Troubleshoot,
    CareInquiry,
    Account,
    Return,
    WarrantyClaim,
    RepairField,
    Complaint,
    /// GPSR 2023/988 posture: safety work is P1-class, always.
    SafetyRecall,
    RetentionOutreach,
}

impl Worktype {
    /// Stable string for run `kind` storage and crew routing.
    pub fn as_str(&self) -> &'static str {
        match self {
            Worktype::Troubleshoot => "troubleshoot",
            Worktype::CareInquiry => "care_inquiry",
            Worktype::Account => "account",
            Worktype::Return => "return",
            Worktype::WarrantyClaim => "warranty_claim",
            Worktype::RepairField => "repair_field",
            Worktype::Complaint => "complaint",
            Worktype::SafetyRecall => "safety_recall",
            Worktype::RetentionOutreach => "retention_outreach",
        }
    }

    /// The deterministic routing table: worktype → SLA envelope class.
    pub fn priority_class(&self) -> Priority {
        match self {
            Worktype::SafetyRecall => Priority::P1,
            Worktype::Account | Worktype::RepairField | Worktype::Complaint => Priority::P2,
            Worktype::Troubleshoot
            | Worktype::CareInquiry
            | Worktype::Return
            | Worktype::WarrantyClaim => Priority::P3,
            Worktype::RetentionOutreach => Priority::P4,
        }
    }
}

/// Stamp the envelope a worktype's policy row buys. The complaint class
/// keeps its own two-clock ISO 10002 envelope ([`stamp_complaint_envelope`]);
/// every other worktype keeps a single clock.
pub fn stamp_worktype_envelope(created_at: i64, wt: &Worktype) -> Envelope {
    if *wt == Worktype::Complaint {
        return stamp_complaint_envelope(created_at);
    }
    stamp_envelope(created_at, wt.priority_class())
}

/// Default retention (days) per `memory_kind` for chunks with no explicit
/// `expires_at`. Per-chunk `expires_at` always wins; this table governs whole
/// classes. Server config layers env overrides on top — these numbers live
/// here and nowhere else.
pub const DEFAULT_RETENTION_KIND_DAYS: &[(&str, i64)] = &[
    ("fact", 365),
    ("episodic", 30),
    ("procedure", 730),
    ("step", 730),
    ("decision", 730),
    ("entitlement", 1825),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p_class_ttl_table() {
        assert_eq!(
            stamp_envelope(1000, Priority::P2).sla_deadline,
            1000 + 24 * 3600
        );
        let e = stamp_envelope(0, Priority::P1);
        assert_eq!(e.sla_deadline, 4 * 3600);
        let e2 = stamp_envelope(0, Priority::P4);
        assert!(e2.sla_deadline > e.sla_deadline);
    }

    #[test]
    fn retention_table_shape() {
        // Every kind maps to a positive day count; the six governed kinds are present.
        assert_eq!(DEFAULT_RETENTION_KIND_DAYS.len(), 6);
        assert!(DEFAULT_RETENTION_KIND_DAYS.iter().all(|(_, d)| *d > 0));
        assert!(
            DEFAULT_RETENTION_KIND_DAYS
                .iter()
                .any(|(k, _)| *k == "episodic")
        );
    }

    #[test]
    fn complaint_envelope_ack_leads_response() {
        let e = stamp_complaint_envelope(1000);
        assert_eq!(e.ack_deadline, 1000 + COMPLAINT_ACK_SECS);
        assert_eq!(e.sla_deadline, 1000 + COMPLAINT_RESPONSE_SECS);
        assert!(
            e.ack_deadline < e.sla_deadline,
            "acknowledgment is always the tighter clock"
        );
        // Non-complaint stamps keep a single clock (ack == response).
        let plain = stamp_envelope(1000, Priority::P2);
        assert_eq!(plain.ack_deadline, plain.sla_deadline);
    }

    #[test]
    fn worktype_sla_table_is_deterministic() {
        // Safety recall is P1-class, always — GPSR posture.
        let recall = stamp_worktype_envelope(0, &Worktype::SafetyRecall);
        assert_eq!(recall.p_class, Priority::P1);
        assert_eq!(recall.sla_deadline, 4 * 3600);
        // Complaint keeps its own tighter ack clock; other worktypes keep one.
        assert_eq!(
            stamp_worktype_envelope(1000, &Worktype::Complaint).sla_deadline,
            1000 + COMPLAINT_RESPONSE_SECS
        );
        let care = stamp_worktype_envelope(1000, &Worktype::CareInquiry);
        assert_eq!(care.ack_deadline, care.sla_deadline);
        // Retention outreach is the loosest clock.
        assert!(
            stamp_worktype_envelope(0, &Worktype::RetentionOutreach).sla_deadline
                > stamp_worktype_envelope(0, &Worktype::Return).sla_deadline
        );
        for (wt, kind) in [
            (Worktype::CareInquiry, "care_inquiry"),
            (Worktype::WarrantyClaim, "warranty_claim"),
            (Worktype::SafetyRecall, "safety_recall"),
            (Worktype::RetentionOutreach, "retention_outreach"),
        ] {
            assert_eq!(wt.as_str(), kind);
        }
    }

    #[test]
    fn law_version_stamped_at_intake() {
        let e = stamp_envelope_for_jurisdiction(0, Priority::P2, "PH");
        assert_eq!(e.law_version, "npc-advisory-2024-04");
        let eu = stamp_envelope_for_jurisdiction(0, Priority::P2, "eu");
        assert_eq!(eu.law_version, "gdpr-consolidated-2021");
        // Unknown jurisdiction stamps empty; SLA clock unaffected.
        let unk = stamp_envelope_for_jurisdiction(0, Priority::P2, "zz");
        assert_eq!(unk.law_version, "");
        assert_eq!(unk.sla_deadline, e.sla_deadline);
        // The plain stamp stays law-free (legacy callers unchanged).
        assert_eq!(stamp_envelope(0, Priority::P1).law_version, "");
    }
}
