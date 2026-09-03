//! The Spire Line's frozen structural ledger — `#[cfg(test)]`, sibling
//! idiom of the Foundation Line's `sql_inventory_baseline` (Plumb, retired
//! at Cornerstone): measured constants with date + release comments, CI
//! fails on ceiling growth or floor drop. Ceilings may only go DOWN; floors
//! only UP — except `MAIN_RS_TEST_FLOOR`, which is lowered only when a pin
//! physically relocates out of `main.rs` (the crate-wide
//! `TOTAL_SRC_TEST_FLOOR` is the load-bearing never-decreases pin).
//!
//! Like the SQL baseline before it, the counters are deliberate substring
//! locks, not precision instruments — they are measured the same way every
//! time, which is what a freeze needs.

use std::path::{Path, PathBuf};

// ── session-start freeze (2026-09-03, v1.28.54 "Scaffold" opening) ──────
// GREEN VALUES — the measured session-start truth (with the +3-line
// `mod spire_inventory` declaration honestly included in the line count).
// Every shrink from here on is earned by a move and lands in that move's
// commit.

/// `wc -l src/main.rs`: 19,906 at session start + 3 ledger decl lines.
/// Ceiling (may only go down).
const MAIN_RS_LINES_CEIL: usize = 19_909;
/// Lines from the `#[cfg(test)] mod tests` boundary to EOF. Ceiling.
const TEST_REGION_LINES_CEIL: usize = 13_342;
/// Textual `.route(` occurrences in main.rs (the registration chain +
/// the authz scan's own literals — counted identically every time). Ceiling.
/// Routes do not move until Vaulting (M3); this freezes at the start value.
const ROUTE_CALL_SITES_CEIL: usize = 234;
/// `#[test]` occurrences in main.rs. Floor (lowered only when a pin moves).
const MAIN_RS_TEST_FLOOR: usize = 139;
/// `#[test]` occurrences across all of `src/` (lib + bins + main).
/// Floor — never decreases.
const TOTAL_SRC_TEST_FLOOR: usize = 1_178;

fn count_needle(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// Lines from the (unique) `#[cfg(test)]\nmod tests {` boundary, inclusive,
/// to EOF — the region the Spire Line is dismantling.
fn test_region_lines(main_src: &str) -> Option<usize> {
    const MARKER: &str = "#[cfg(test)]\nmod tests {";
    let idx = main_src.find(MARKER)?;
    let start_line = main_src[..idx].matches('\n').count() + 1;
    Some(main_src.lines().count() - start_line + 1)
}

fn walk_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn spire_inventory_freezes_the_monolith() {
    let main_src = include_str!("main.rs");
    let total_lines = main_src.lines().count();
    let region = test_region_lines(main_src)
        .unwrap_or_else(|| panic!("spire: `#[cfg(test)] mod tests` marker not found in main.rs"));
    let route_sites = count_needle(main_src, ".route(");
    let main_tests = count_needle(main_src, "#[test]");

    // Anti-vacuous sanity: the counters must be looking at the real thing
    // (the Cornerstone lesson — a guard that can pass on nothing guards
    // nothing).
    assert!(
        region < total_lines && region > total_lines / 2,
        "spire: test-region derivation looks wrong ({region} of {total_lines})"
    );
    assert!(
        route_sites >= 100 && main_tests >= 100,
        "spire: counters look broken (routes {route_sites}, tests {main_tests})"
    );

    let mut files = Vec::new();
    walk_rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(
        files.len() >= 50,
        "spire: src walk found only {} files — the tree walker is broken",
        files.len()
    );
    let total_tests = files
        .iter()
        .map(|p| count_needle(&std::fs::read_to_string(p).unwrap_or_default(), "#[test]"))
        .sum::<usize>();

    let mut breaches: Vec<String> = Vec::new();
    if total_lines > MAIN_RS_LINES_CEIL {
        breaches.push(format!(
            "  main.rs lines: {total_lines} > ceiling {MAIN_RS_LINES_CEIL}"
        ));
    }
    if region > TEST_REGION_LINES_CEIL {
        breaches.push(format!(
            "  test-region lines: {region} > ceiling {TEST_REGION_LINES_CEIL}"
        ));
    }
    if route_sites > ROUTE_CALL_SITES_CEIL {
        breaches.push(format!(
            "  .route( sites: {route_sites} > ceiling {ROUTE_CALL_SITES_CEIL}"
        ));
    }
    if main_tests < MAIN_RS_TEST_FLOOR {
        breaches.push(format!(
            "  main.rs #[test] count: {main_tests} < floor {MAIN_RS_TEST_FLOOR} — a pin \
             left main.rs without its spire_inventory edit in the same commit"
        ));
    }
    if total_tests < TOTAL_SRC_TEST_FLOOR {
        breaches.push(format!(
            "  crate #[test] count: {total_tests} < floor {TOTAL_SRC_TEST_FLOOR} — the \
             load-bearing total-test floor dropped; tests may not be deleted, only moved"
        ));
    }

    assert!(
        breaches.is_empty(),
        "SPIRE INVENTORY VIOLATION — the monolith regrew or a pin was lost; \
         fix the code, or lower/raise the constant in the same reviewed commit:\n{}",
        breaches.join("\n")
    );

    println!(
        "spire: main.rs {total_lines}≤{MAIN_RS_LINES_CEIL} · region {region}≤\
         {TEST_REGION_LINES_CEIL} · routes {route_sites}≤{ROUTE_CALL_SITES_CEIL} · \
         main tests {main_tests}≥{MAIN_RS_TEST_FLOOR} · crate tests {total_tests}≥\
         {TOTAL_SRC_TEST_FLOOR}"
    );
}
