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

/// Every client `src/**/*.rs` surface, recursively.
fn surfaces() -> Vec<(String, String)> {
    fn walk_rs(dir: &Path, out: &mut Vec<(String, String)>) {
        for e in
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        {
            let p = e.expect("dir entry").path();
            if p.is_dir() {
                walk_rs(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let rel = p
                    .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                    .expect("under manifest")
                    .display()
                    .to_string();
                let src = std::fs::read_to_string(&p)
                    .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
                out.push((rel, src));
            }
        }
    }
    let mut out = Vec::new();
    walk_rs(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

/// focus_never_obscured_by_docks — WCAG 2.2 AA 2.4.11. The stylesheet must
/// give every focused node a scroll-margin that clears the sticky header
/// (h-14 = 3.5rem) and the bottom tab bar (~4rem + safe area), and the shell
/// must actually use sticky chrome (so the margin is load-bearing, and any
/// taller dock added later fails this test until the margin grows with it).
#[test]
fn focus_never_obscured_by_docks() {
    let css =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("styles/input.css"))
            .expect("input.css readable");
    // The guard rule exists…
    let guard = css
        .split("*:focus-visible")
        .nth(1)
        .unwrap_or_else(|| panic!("*:focus-visible scroll-margin guard missing from input.css"));
    let block = &guard[..guard.find('}').unwrap_or(guard.len())];
    // …with margins strictly larger than the docks they clear.
    let top = parse_rem(block, "scroll-margin-top");
    let bottom = parse_rem(block, "scroll-margin-bottom");
    assert!(
        top >= 4.0,
        "scroll-margin-top {top}rem does not clear the h-14 (3.5rem) sticky header"
    );
    assert!(
        bottom >= 5.0,
        "scroll-margin-bottom {bottom}rem does not clear the ~4rem+safe-area tab bar"
    );
    // The docks it clears are pinned at their known heights: a taller dock
    // must raise the margin in the same change.
    assert!(css.contains("h-14"), "sticky header height unpinned");
    assert!(
        css.contains("min-height: 44px"),
        "tab-link floor (44px) unpinned — the bottom dock grew without this test noticing"
    );
}

fn parse_rem(block: &str, prop: &str) -> f64 {
    let idx = block
        .find(&format!("{prop}:"))
        .unwrap_or_else(|| panic!("{prop} missing in focus guard"));
    let n: String = block[idx + prop.len() + 1..]
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    n.parse::<f64>()
        .unwrap_or_else(|_| panic!("{prop} is not a rem value near {:?}", &block[idx..idx + 40]))
}

/// drag_alternatives_exist_for_every_drag — WCAG 2.2 AA 2.5.7. No drag-only
/// affordance may ship: any drag/pointer-move handler in a render surface
/// must carry a `// drag-alt:` marker on the same line naming its click
/// alternative. The tree currently ships ZERO drag interactions (keyboard-
/// first J/K navigation instead) — this gate keeps it that way honestly.
#[test]
fn drag_alternatives_exist_for_every_drag() {
    let mut drags = 0usize;
    for (name, src) in surfaces() {
        if name.ends_with("a11y.rs") {
            continue; // this gate's own handler-name literals
        }
        for (i, line) in src.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            if ["ondrag", "onpointermove", "ontouchmove"]
                .iter()
                .any(|h| line.contains(h))
            {
                drags += 1;
                assert!(
                    line.contains("// drag-alt:"),
                    "{name}:{}: drag interaction without a `// drag-alt:` click alternative",
                    i + 1
                );
            }
        }
    }
    assert_eq!(
        drags, 0,
        "drag interactions appeared — each needs a marked click alternative"
    );
}

/// target_size_floor_24px_enforced_by_classes — WCAG 2.2 AA 2.5.8. The
/// component layer's interactive classes carry an explicit size floor:
/// btn-sm ≥ h-7 (28px), btn-md ≥ h-9 (36px), tab-link ≥ 44px; no interactive
/// component class may shrink below the 24px criterion.
#[test]
fn target_size_floor_24px_enforced_by_classes() {
    let css =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("styles/input.css"))
            .expect("input.css readable");
    let components = css
        .split("@layer components")
        .nth(1)
        .expect("component layer present");
    let height_of = |cls: &str| -> f64 {
        let needle = format!(".{cls}");
        let seg = components
            .split(&needle)
            .nth(1)
            .unwrap_or_else(|| panic!(".{cls} missing from the component layer"));
        let block = &seg[..seg.find('}').unwrap_or(seg.len())];
        let h = block
            .split_whitespace()
            .find_map(|tok| tok.strip_prefix("h-").map(|s| s.to_string()))
            .or_else(|| {
                block.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix("min-height:")
                        .map(|s| s.trim().to_string())
                })
            });
        match h.as_deref() {
            Some("7") => 1.75,
            Some("8") => 2.0,
            Some("9") => 2.25,
            Some(px) if px.starts_with(|c: char| c.is_ascii_digit()) => {
                let n: String = px
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                n.parse::<f64>().unwrap_or(0.0) / 16.0
            }
            other => panic!(".{cls} carries no parsable height floor (found {other:?})"),
        }
    };
    for cls in ["btn-sm", "btn-md"] {
        let r = height_of(cls);
        assert!(
            r >= 1.5,
            ".{cls} height {r}rem is below the 24px target-size floor"
        );
    }
    let tab = height_of("tab-link");
    assert!(
        tab >= 2.75,
        ".tab-link height {tab}rem below its 44px floor"
    );
    // The select used as the locale switcher rides .select (py-1.5 + text-sm
    // ≈ 33px) — pin that no component-layer class sets an explicit sub-floor.
    assert!(
        !components.contains("h-4") && !components.contains("h-5"),
        "an interactive component class shrank below the 24px floor"
    );
}

/// help_entry_consistent_across_panels — WCAG 2.2 AA 3.2.6. Exactly ONE help
/// entry exists, rendered by the AppShell (identical position/content on
/// every panel); panels define none of their own beyond the per-run sheet.
#[test]
fn help_entry_consistent_across_panels() {
    let main = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"));
    let triggers = main.matches("help_open.toggle()").count();
    assert_eq!(
        triggers, 1,
        "the shell must render exactly one help trigger (found {triggers})"
    );
    assert!(
        main.contains("if help_open() {"),
        "the shared help sheet is rendered conditionally on the single shell signal"
    );
    // Panels never register a competing global help entry.
    for (name, src) in surfaces() {
        if name.ends_with("main.rs")
            || name.ends_with("a11y.rs")
            || name.ends_with("conversation.rs")
        {
            continue; // main.rs owns the entry; a11y.rs pins it; conversation owns the per-run sheet
        }
        assert!(
            !src.contains("help_open") && !src.contains("nav_help"),
            "{name} defines its own help entry — 3.2.6 requires ONE consistent entry"
        );
    }
}

/// acr_remarks_cover_every_non_support — every ACR claim short of full
/// support ("partial" / "does not support") either lives in Known Ceilings
/// or explicitly points there (checked per paragraph, so multi-line
/// sentences count). An unexplained shortfall fails the build.
#[test]
fn acr_remarks_cover_every_non_support() {
    let acr = trust_doc("acr-vpat.md");
    let mut in_ceilings = false;
    for para in acr.split("\n\n") {
        if para.starts_with("## ") {
            in_ceilings = para.to_ascii_lowercase().contains("known ceilings");
            continue;
        }
        if para.starts_with('#') {
            continue; // other headings never carry claims
        }
        let low = para.to_ascii_lowercase();
        if low.contains("partial") || low.contains("does not support") {
            assert!(
                in_ceilings || low.contains("ceiling"),
                "ACR paragraph makes a non-support claim without a ceiling reference: {para:?}"
            );
        }
    }
}
