//! The Spire Line's structural ledger — `#[cfg(test)]`. Born at Scaffold
//! (v1.28.54) as the freeze-on-the-monolith: measured constants with date +
//! release comments, CI fails on ceiling growth or floor drop. At the
//! Capstone flip (v1.28.57) the monolith is structurally impossible, so the
//! ceilings RETIRE instead of zeroing (the Cornerstone precedent — the
//! enforcing guard replaces the table row). What survives is what the thin
//! binary must never lose: the main.rs ≤ 300 pin, the router's registration
//! floor, the crate-wide test floor (`src/` + `tests/` — the load-bearing
//! never-decreases pin), and the route-coverage/authz table row floors.
//!
//! Like the SQL baseline before it, the counters are deliberate substring
//! locks, not precision instruments — they are measured the same way every
//! time, which is what a freeze needs.
//!
//! Retired at Capstone, and why (the move commit is the record):
//!   * `MAIN_RS_LINES_CEIL` (16,869, Scaffold freeze) → replaced by
//!     `MAIN_RS_LINES_MAX` (300): the pin IS the ceiling now.
//!   * `TEST_REGION_LINES_CEIL` (12,294, Vaulting tip) → absence-pinned:
//!     main.rs must not regrow a `#[cfg(test)]` region at all; the mass
//!     lives in `tests/main_suite.rs` (moved verbatim, nothing deleted).
//!   * `MAIN_RS_TEST_FLOOR` (109) → its 109 pins relocated verbatim to
//!     `tests/main_suite.rs` in the Capstone move commit (the ledger's own
//!     relocation convention — lowered only when pins physically move, in
//!     that commit); the crate-wide floor below is load-bearing.
//!   * `ROUTE_CALL_SITES_CEIL` (35) → retired at the move commit: main.rs
//!     sites are pinned to 0 below, and the enforcing successor is the
//!     tree-wide `.route(` gate
//!     (`route_registrations_live_only_under_router`) — a main.rs-only
//!     ceiling cannot see a violation planted in any other non-router file;
//!     the gate covers the whole `src/` tree.

use std::path::{Path, PathBuf};

// ── the thin binary (v1.28.57 "Capstone") ───────────────────────────────

/// `wc -l src/main.rs`: 19,906 at the Scaffold freeze → 12,471 at the
/// Vaulting tip → 113 at the Capstone flip (wiring only: bootstrap →
/// compose → serve). Pin: main.rs never regrows beyond a thin wiring shell.
const MAIN_RS_LINES_MAX: usize = 300;
/// `.route(` sites under src/server/router/** (the composed chain). Floor —
/// the wire's registrations may not silently disappear; the authz matrix +
/// route-coverage table pin their correctness.
const ROUTER_SITES_FLOOR: usize = 199;
/// `#[test]` occurrences across `src/` + `tests/` (lib + bins + the
/// integration suites). Floor — never decreases. 1,185 src-only at the
/// Scaffold freeze; the Capstone move relocated main.rs's region into
/// tests/ without deleting a single pin, and the floor re-measured 1,196
/// over the widened subject (1,074 src + 122 tests) in the move commit.
/// (The needle counts doc-comment literals too — a deliberate substring
/// lock, measured the same way every time.)
const CRATE_TEST_FLOOR: usize = 1_196;
/// Route-coverage table rows (`route_guards::OPENAPI_ROUTES`) — 151 paths at
/// extraction (v1.28.54), re-measured to 161 at the Buttress open. Rows join
/// only with the wire change that earns them, in the same commit.
const OPENAPI_ROUTE_ROWS_FLOOR: usize = 161;
/// Route-authz table rows (`route_guards::AUTHZ_GATES`) — 141 gates at
/// extraction (v1.28.54), re-measured to 145 at the Buttress open.
const AUTHZ_TABLE_ROWS_FLOOR: usize = 145;

fn count_needle(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// Lines from the (unique) `#[cfg(test)]\nmod tests {` boundary, inclusive,
/// to EOF — the region the Spire Line dismantled. Returns `None` once the
/// region is gone; the thin-binary pin below asserts it stays gone.
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
fn spire_inventory_freezes_the_thin_binary() {
    let main_src = include_str!("main.rs");
    let total_lines = main_src.lines().count();
    let route_sites = count_needle(main_src, ".route(");
    // the composed chain lives in src/server/router (the router families);
    // main.rs's remaining `.route(` sites are the region residue, counted
    // toward 0 by the same needle that froze the monolith. Count the router
    // files to keep the anti-vacuous guard meaningful.
    let router_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server/router");
    let mut router_src = String::new();
    let mut router_files = 0usize;
    if let Ok(entries) = std::fs::read_dir(&router_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|e| e == "rs") {
                router_src.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
                router_files += 1;
            }
        }
    }
    assert!(
        router_files >= 6,
        "spire: router family files missing ({router_files} found, need core+memory+ump+compliance+workflow+auth+mod)"
    );
    let router_sites = count_needle(&router_src, ".route(");

    // Anti-vacuous sanity: the counters must be looking at the real thing
    // (the Cornerstone lesson — a guard that can pass on nothing guards
    // nothing). main.rs is the wiring file (it has `fn main`), the router
    // chain is where the registrations live, and the test region stays GONE
    // from main.rs — it lives in tests/main_suite.rs now.
    assert!(
        main_src.contains("fn main("),
        "spire: include_str!(\"main.rs\") does not look like the wiring file"
    );
    assert!(
        router_sites >= ROUTER_SITES_FLOOR,
        "spire: counters look broken (router routes {router_sites})"
    );
    assert!(
        test_region_lines(main_src).is_none(),
        "spire: main.rs regrew a `#[cfg(test)]` region — the test mass lives \
         in tests/; a region in main.rs is the monolith's first vertebra"
    );

    // The crate-wide floor walks BOTH trees: the lib/bins (src/) and the
    // integration suites (tests/) — the Capstone move relocated main.rs's
    // region into tests/ without deleting a pin, and neither tree may shrink
    // its mass quietly.
    let mut files = Vec::new();
    walk_rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    walk_rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"),
        &mut files,
    );
    assert!(
        files.len() >= 50,
        "spire: src+tests walk found only {} files — the tree walker is broken",
        files.len()
    );
    let total_tests = files
        .iter()
        .map(|p| count_needle(&std::fs::read_to_string(p).unwrap_or_default(), "#[test]"))
        .sum::<usize>();

    let mut breaches: Vec<String> = Vec::new();
    if total_lines > MAIN_RS_LINES_MAX {
        breaches.push(format!(
            "  main.rs lines: {total_lines} > pin {MAIN_RS_LINES_MAX} — the thin binary \
             is regrowing; wiring belongs in server/bootstrap + server/router"
        ));
    }
    if route_sites > 0 {
        breaches.push(format!(
            "  .route( sites in main.rs: {route_sites} > 0 — wiring-only means \
             NO registrations here; they live under src/server/router/**"
        ));
    }
    if router_sites < ROUTER_SITES_FLOOR {
        breaches.push(format!(
            "  .route( sites under src/server/router: {router_sites} < floor {ROUTER_SITES_FLOOR} — \
             the composed chain lost registrations"
        ));
    }
    if total_tests < CRATE_TEST_FLOOR {
        breaches.push(format!(
            "  crate #[test] count: {total_tests} < floor {CRATE_TEST_FLOOR} — the \
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
        "SPIRE INVENTORY VIOLATION — the thin binary regrew or a pin was lost; \
         fix the code, or lower/raise the constant in the same reviewed commit:\n{}",
        breaches.join("\n")
    );

    println!(
        "spire: main.rs {total_lines}≤{MAIN_RS_LINES_MAX} · region absent · \
         main routes {route_sites}=0 · router routes {router_sites}≥{ROUTER_SITES_FLOOR} · \
         crate tests {total_tests}≥{CRATE_TEST_FLOOR} · coverage rows {route_rows}≥\
         {OPENAPI_ROUTE_ROWS_FLOOR} · authz rows {authz_rows}≥{AUTHZ_TABLE_ROWS_FLOOR}"
    );
}
