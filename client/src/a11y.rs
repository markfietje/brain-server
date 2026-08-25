//! Accessibility as a release gate (G3): the WCAG 2.2 AA checklist is
//! machine-checked and the ACR/VPAT artifact must list its ceilings honestly.
//! A criterion that loses its PASS — or gains an unexplained status — fails
//! the build; the ACR must never claim more than it can evidence.

use std::path::Path;

fn trust_doc(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/trust")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("docs/trust/{name} must exist and be readable: {e}"))
}

/// wcag_22_aa_gate_blocks_release — every `- <num> <name> — STATUS:` line in
/// the checklist must be `PASS` or `CEILING`; a CEILING must cite
/// acr-vpat.md's Known Ceilings section. Anything else blocks release.
#[test]
fn wcag_22_aa_gate_blocks_release() {
    let checklist = trust_doc("wcag22-aa-checklist.md");
    let mut criteria = 0usize;
    for line in checklist
        .lines()
        .filter(|l| l.trim_start().starts_with("- "))
    {
        let t = line.trim_start().trim_start_matches("- ");
        if !t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        criteria += 1;
        let status = t.split("—").nth(1).map(str::trim).unwrap_or("");
        let ok = status.starts_with("PASS:") || status.starts_with("CEILING:");
        assert!(
            ok,
            "WCAG 2.2 AA gate: `{}` carries no PASS/CEILING verdict — \
             verify it or document its ceiling before releasing",
            t.split("—").next().unwrap_or(t).trim()
        );
        if status.starts_with("CEILING:") {
            assert!(
                status.to_ascii_lowercase().contains("acr-vpat.md"),
                "a CEILING must cite acr-vpat.md (where the ceiling is explained): {t}"
            );
        }
    }
    assert!(
        criteria >= 20,
        "the checklist collapsed to {criteria} criteria — the gate lost coverage"
    );
}

/// acr_lists_known_ceilings_honestly — the ACR claims "partially supports",
/// names the axe-gate scope limit, and admits the focus-restoration gap.
#[test]
fn acr_lists_known_ceilings_honestly() {
    let acr = trust_doc("acr-vpat.md");
    for needle in [
        "Partially supports",
        "axe browser gate covers the web console only",
        "Focus restoration after modal close is not yet guaranteed everywhere",
        "EN 301 549",
    ] {
        assert!(acr.contains(needle), "ACR must honestly state `{needle}`");
    }
}
