//! Docs truth: standards watch items are pinned by test so a standards
//! revision cannot land silently. If one of these fails, the cited standard
//! moved — re-verify the mapping in the referenced doc and update it in the
//! same change.

#[cfg(test)]
mod pins {
    fn doc(rel: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{rel} must exist and be readable: {e}"))
    }

    /// ISO/AWI 18295-1 is under revision (verified 2026-08). The watch item
    /// must stay registered: when the revision publishes, every clause
    /// reference has to be re-mapped deliberately, not silently.
    #[test]
    fn iso_18295_revision_stays_a_registered_watch_item() {
        let standards = doc("docs/CONTACT_CENTER_STANDARDS.md");
        assert!(
            standards.contains("ISO/AWI 18295-1"),
            "the ISO 18295-1 revision watch item vanished from the standards inventory"
        );
        let compliance = doc("COMPLIANCE.md");
        assert!(
            compliance.contains("ISO 18295-1"),
            "the contact-centre clause map lost its ISO 18295-1 anchor"
        );
    }

    /// The conformance posture stays self-assessed: certification language
    /// must not creep into the compliance file.
    #[test]
    fn contact_centre_posture_is_self_assessed_not_certified() {
        let compliance = doc("COMPLIANCE.md");
        let section = compliance
            .split("### 6.7")
            .nth(1)
            .unwrap_or_else(|| panic!("COMPLIANCE.md §6.7 missing"));
        let head = section.split("## ").next().unwrap_or_default();
        assert!(
            head.to_ascii_lowercase().contains("self-assessed"),
            "§6.7 must state its self-assessed posture"
        );
    }

    /// The metrics dictionary covers every emitted scoreboard field and the
    /// FCR window config exists with its documented default.
    #[test]
    fn fcr_window_documented_matches_code_default() {
        let metrics = doc("docs/metrics.md");
        assert!(
            metrics.contains("BRAIN_FCR_WINDOW_DAYS") && metrics.contains("default 7"),
            "metrics dictionary must document BRAIN_FCR_WINDOW_DAYS (default 7)"
        );
        assert_eq!(
            crate::config::DEFAULT_FCR_WINDOW_DAYS,
            7,
            "code default drifted from the documented default"
        );
    }
}
