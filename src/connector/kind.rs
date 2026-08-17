//! the connector kind vocabulary.
//!
//! The `connectors` table already carries a free-form `kind TEXT` column; this
//! module pins the shipped vocabulary the vertical set (USE_CASES.md) uses, and
//! validates registration so a typo'd kind can't silently register. The
//! registered kinds are the ones a profile's `connectors_allowed` gates against
//! (see `profile::connector_allowed`).
//!
//! `github` predates the v1.24 set; it is preserved so the v0.9.6 connector
//! keeps working. Everything else is a v1.24 vertical.

/// Shipped connector kinds. `github` is the v0.9.6 original; the rest are the
/// v1.24 vertical set (read-only; each is a translate+ingest module).
pub const CONNECTOR_KINDS: &[&str] = &[
    "github",
    "crm-salesforce",
    "crm-hubspot",
    "slack",
    "email-imap",
    "jira",
    "linear",
    "notion",
    "hris-readonly",
    "ehr-readonly",
];

/// Family prefix of a kind (`crm-salesforce` → `crm`). Used by profile gating
/// so an allowed `crm` grants every `crm-*` sub-kind. Kinds without a `-`
/// return the whole kind (family matches only exact).
pub fn family(kind: &str) -> &str {
    match kind.find('-') {
        Some(i) => &kind[..i],
        None => kind,
    }
}

/// True when `kind` is a shipped connector kind (exact match, case-sensitive
/// lowercase). Registration refuses anything not in the vocabulary.
pub fn is_connector_kind(kind: &str) -> bool {
    CONNECTOR_KINDS.contains(&kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_includes_all_shipped_kinds() {
        for k in [
            "github",
            "crm-salesforce",
            "crm-hubspot",
            "slack",
            "email-imap",
            "jira",
            "linear",
            "notion",
            "hris-readonly",
            "ehr-readonly",
        ] {
            assert!(is_connector_kind(k), "{k} should be a valid kind");
        }
    }

    #[test]
    fn unknown_kind_is_rejected() {
        for k in ["slacko", "crm", "", "Crm-salesforce", "github "].iter() {
            assert!(!is_connector_kind(k), "{k:?} should be rejected");
        }
    }

    #[test]
    fn family_splits_sub_kind_and_passes_flat_kind_through() {
        assert_eq!(family("crm-salesforce"), "crm");
        assert_eq!(family("slack"), "slack");
        assert_eq!(family("ehr-readonly"), "ehr");
    }
}
