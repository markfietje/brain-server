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

    /// Throughput v1.28.58: every `/metrics` series the core router emits
    /// carries a dictionary row in docs/metrics.md — the scoreboard
    /// docs↔code parity applied to ops telemetry. Scans the metrics handler's
    /// source for `brain_*` series literals (the substring-lock idiom: a
    /// series added in code without its dictionary row fails here).
    #[test]
    fn metrics_series_have_dictionary_rows() {
        let core = doc("src/server/router/core.rs");
        let metrics = doc("docs/metrics.md");
        // collect every `brain_[a-z0-9_]+` literal in the metrics surface
        let mut series: Vec<String> = Vec::new();
        let bytes = core.as_bytes();
        let mut i = 0usize;
        while let Some(rel) = core[i..].find("brain_") {
            let start = i + rel;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_lowercase()
                    || bytes[end].is_ascii_digit()
                    || bytes[end] == b'_')
            {
                end += 1;
            }
            let name = core[start..end].to_string();
            if !series.contains(&name) {
                series.push(name);
            }
            i = start + 6;
        }
        // anti-vacuous: the scan must see the real surface — the pre-Throughput
        // series floor. If this fires, the scanner (or the handler) is broken.
        assert!(
            series.len() >= 10,
            "metrics-series scan found only {} names — the scanner or the handler is broken",
            series.len()
        );
        for name in &series {
            assert!(
                metrics.contains(&format!("`{name}`")),
                "metrics dictionary is missing a row for series `{name}` — add it to \
                 docs/metrics.md §\"Server telemetry series\" in the same commit"
            );
        }
    }
}
