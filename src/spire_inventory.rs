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

/// `wc -l src/main.rs`: 19,906 at session start, −624 net across Scaffold's
/// extractions, relocations, and the dedup of five stale main.rs copies the
/// handlers-family commit left behind, then −397 net from the Buttress
/// http_limit promotion (family + 9 unit pins, use/mod wiring added back)
/// and −245 net from the blocklist joining screen.rs (fn + 7 pins), then
/// −82 net as the quarantine-flag + read-seam-suppression pair joined them
/// (flag_if_quarantined, suppress_flagged_evidence) with the snippet pin,
/// then −108 net as the graph read mappers (clamp, row mapper, explanation
/// paths) moved to graph_read.rs with the two explanation pins — the
/// AppError-typed graph SQL fns stay for Vaulting (transport-shaped),
/// then −2 more as the boot guards moved to boot.rs (ct_eq pin + the
/// loopback-bind predicates pin), then −489 net at the Vaulting open as the
/// middleware stack + auth middlewares staged into server/router/{mod,auth}.rs
/// (fns + CSP consts verbatim; the poisoned-lock pin repointed same-commit),
/// then −95 net as `app(state)` was lifted out of main_inner (the inline
/// chain became the composed fn; AppState construction + watcher spawns
/// hoisted to the main side of the seam) and the three middleware oneshot
/// suites moved into server/router/auth.rs's test module with their subjects
/// (the +102 AppState-initializer fixture lines across the composed-state
/// test sites came in the same commit — net monolith motion still down).
/// Ceiling.
/// 17,710 at the C2 open, then −841 net as the boot region (fail-closed
/// checks, pool/model/migration, watchdogs, JWT wiring, state construction,
/// bind guard) moved into server/bootstrap.rs and boot.rs folded in
/// (ct_eq + argv + worker-threads + bind predicates, pins traveling).
const MAIN_RS_LINES_CEIL: usize = 16_869;
/// Lines from the `#[cfg(test)] mod tests` boundary to EOF. Ceiling.
/// 13,342 at session start − 630 net (Scaffold) − 156 net (Buttress: the
/// http_limit unit pins left the region, then 126 net as the 7 blocklist
/// pins and their docs left, then 32 net as the snippet pin left, then 34
/// net as the explanation pins left, then 67 net as the ct_eq + loopback
/// bind pins left, then −113 net as the three middleware oneshot suites
/// moved to server/router/auth.rs with their subjects, +102 back as the
/// composed-app test sites gained the middleware-stack fixture fields.
const TEST_REGION_LINES_CEIL: usize = 12_189;
/// Textual `.route(` occurrences in main.rs (the registration chain +
/// the authz scan's own literals — counted identically every time). Ceiling.
/// 234 at the Vaulting open; −5 as the three middleware oneshot suites
/// (stub-router literals, not real registrations) moved to
/// server/router/auth.rs with their subjects. The real registrations leave
/// only in the family commits, each lowering this toward 0.
const ROUTE_CALL_SITES_CEIL: usize = 229;
/// `#[test]` occurrences in main.rs. Floor (lowered only when a pin moves,
/// in the same commit): 139 at session start − 10 relocated pure-unit pins
/// (6 → handlers/mod.rs; auth_tokens, temporal, trace_caps, eval → their
/// modules) = 129, then −8 more as the Buttress http_limit family moved
/// (its 9th pin is a `#[tokio::test]` — outside this substring counter's
/// needle, so it lowers the file's test mass without moving this floor),
/// then −7 more as the layer-1 blocklist moved to screen.rs with its pins,
/// then −1 more as the snippet-suppression pin followed the suppression fn,
/// then −2 more as the explanation-path pins moved to graph_read.rs,
/// then −2 more as the boot guards moved (test_ct_eq + the bind pin).
const MAIN_RS_TEST_FLOOR: usize = 109;
/// `#[test]` occurrences across all of `src/` (lib + bins + main).
/// Floor — never decreases. 1,178 at the Scaffold freeze; re-measured to
/// 1,185 at the Buttress open (tests legitimately added between the lines)
/// so the never-decreases guard stays tight rather than trailing by seven.
const TOTAL_SRC_TEST_FLOOR: usize = 1_185;
/// Route-coverage table rows (`handlers::route_guards::OPENAPI_ROUTES`)
/// — 151 paths at extraction (v1.28.54), re-measured to 161 at the Buttress
/// open (rows joined only with the wire changes that earned them). Rows join
/// only with the wire change that earns them, in the same commit.
const OPENAPI_ROUTE_ROWS_FLOOR: usize = 161;
/// Route-authz table rows (`handlers::route_guards::AUTHZ_GATES`) — 141
/// gates at extraction (v1.28.54), re-measured to 145 at the Buttress open.
const AUTHZ_TABLE_ROWS_FLOOR: usize = 145;

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
    let route_rows = crate::route_guards::OPENAPI_ROUTES.len();
    let authz_rows = crate::route_guards::AUTHZ_GATES.len();
    if route_rows < OPENAPI_ROUTE_ROWS_FLOOR {
        breaches.push(format!(
            "  route-coverage rows: {route_rows} < floor {OPENAPI_ROUTE_ROWS_FLOOR} — a \
             documented route left the table without its wire change"
        ));
    }
    if authz_rows < AUTHZ_TABLE_ROWS_FLOOR {
        breaches.push(format!(
            "  route-authz rows: {authz_rows} < floor {AUTHZ_TABLE_ROWS_FLOOR} — an \
             authz gate row was dropped without its wire change"
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
         {TOTAL_SRC_TEST_FLOOR} · coverage rows {route_rows}≥{OPENAPI_ROUTE_ROWS_FLOOR} · \
         authz rows {authz_rows}≥{AUTHZ_TABLE_ROWS_FLOOR}"
    );
}
