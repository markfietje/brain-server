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
//!     tree-wide route-registration gate
//!     (`route_registrations_live_only_under_router`) — a main.rs-only
//!     ceiling cannot see a violation planted in any other non-router file;
//!     the gate covers the whole `src/` tree.
//!     ceiling cannot see a violation planted in any other non-router file;
//!     the gate covers the whole `src/` tree.

use std::path::{Path, PathBuf};

// ── the thin binary (v1.28.57 "Capstone") ───────────────────────────────

/// `wc -l src/main.rs`: 19,906 at the Scaffold freeze → 12,471 at the
/// Vaulting tip → 124 at the Capstone flip (wiring only: bootstrap →
/// compose → serve). Pin: main.rs never regrows beyond a thin wiring shell.
const MAIN_RS_LINES_MAX: usize = 300;
/// Route-registration sites under src/server/router/** (the composed chain).
/// Floor —
/// the wire's registrations may not silently disappear; the authz matrix +
/// route-coverage table pin their correctness.
const ROUTER_SITES_FLOOR: usize = 199;
/// `#[test]` occurrences across `src/` + `tests/` (lib + bins + the
/// integration suites). Floor — never decreases. 1,185 src-only at the
/// Scaffold freeze; the Capstone move relocated main.rs's region into
/// tests/ without deleting a single pin, and the floor re-measured 1,196
/// over the widened subject (1,074 src + 122 tests) in the move commit
/// (1,207 at the Throughput open — the calendar pins, the concurrency
/// proptests, the merge determinism pin, and the metrics-dictionary parity
/// test earned the raise; 1,221 at the Headroom open — the durability
/// envelope pins, the fail-closed resolver pins, the lock-wait histogram
/// pins, the limiter-purity + token-swap pins, and the write-discipline
/// gate trio earned the raise). (The needle counts doc-comment literals
/// too — a deliberate substring lock, measured the same way every time.)
/// 1,221 → 1,228 at the Loom open: the seven loom pins (fail-closed parse,
/// thread cap, the Jetson resolution matrix, fan-out order, the pool-vs-
/// serial pin, the proptest order invariant, `loom_preserves_fused_ranks`).
/// 1,228 → 1,244 at the Standby open: the parcels signature-unchanged pin,
/// the manifest-sign/verify convention pair, the seven standby pins (RPO
/// math, frame arithmetic, sig-after-copy, follower tamper, no-key refusal,
/// the roundtrip proptest, resume-cycle), the cli-reference law, and the
/// dated drill-record watch.
/// 1,244 → 1,256 at the Attestation open: the two revocation pins, the
/// four-class provenance meta-test + tamper/unit/round-trip/human set, the
/// uniformity parity + fetch-mirror pair, the flipped deliverable watches +
/// clock self-test, and the revocation-drill record watch.
/// 1,256 → 1,267 at the Wardline open: the reserved-vocabulary semantics +
/// refusal/convert/kernel-mint set, the kernel steering + channel-writer
/// pins, the forged-valet-kind bus pin, the vet-fence parity/crank/open
/// set, and the one-place vocabulary grep.
/// 1,267 → 1,269 at the Meridian open: the /suggest untrusted serialization
/// pin + the three-surface label-parity source pin (X-R1).
/// 1,269 → 1,281 at the Blackout open: the six authN kill-switch matrix
/// pins, the denylist TTL trio, the per-kid alg pair, the INJECTION_POLICY
/// visibility pair, the method-keyed scan and reverse-guard quartet, and
/// the public-path/security.txt pair.
/// 1,281 → 1,290 at the Truthglass open: the CLI operator-truth set — the
/// dsar action/prompt pins, the restore interlocks, the passphrase mode.
/// 1,290 → 1,303 at the Pin open (re-measured on the rebased tree): the
/// MCP scope gate set (fail-closed parse, read refusals, read serves, full
/// compat, tools/list annotation), the signer-pinning provenance set
/// (foreign-signer refusal, L2-unchanged, signed_by-surfaced verify JSON),
/// and the hash-only census set — eleven in-crate pins plus the two
/// parcels wire tests the floor's tests/ walk absorbs.
/// 1,303 → 1,313 at the Deadbolt open: the egress guard set (registry
/// table, metadata refusal, boot refusal, opt-out pin, DNS-rebind pin,
/// unresolved fail-closed, hostcall cache bound) and the crank set
/// (relative-override refusal, PATH-never-consulted, exe-dir fallback,
/// success-path shape — the two tokio crank pins ride outside this
/// needle), 1,318 measured at the Twokeys release commit (the five new
/// bare-`#[test]` pins: the four `auth_token_sets`/agent-source config
/// pins and the `scoped_domain_label` pure pin — the ten tokio agent
/// pins ride outside the needle like every tokio test before them).
const CRATE_TEST_FLOOR: usize = 1_363;
/// Route-coverage table rows (`route_guards::OPENAPI_ROUTES`) — 151 paths at
/// extraction (v1.28.54), 163 at the Wardline gate, 167 when Blackout's
/// reverse-direction guard found the missing rows (security.txt + the
/// scoreboard/calibration/mount trio). Rows join only with the wire change
///   that earns them, in the same commit.
const OPENAPI_ROUTE_ROWS_FLOOR: usize = 167;
/// Route-authz table rows (`route_guards::AUTHZ_GATES`) — 141 gates at
/// extraction (v1.28.54), 147 at the Wardline gate, 152 when Blackout's
/// reverse-direction guard closed the table debt (`/stats` + the trio +
///   security.txt's public marker row).
const AUTHZ_TABLE_ROWS_FLOOR: usize = 152;

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
    let route_sites = count_needle(main_src, ROUTE_NEEDLE);
    // the composed chain lives in src/server/router (the router families);
    // main.rs's remaining registration sites are the region residue, counted
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
    let router_sites = count_needle(&router_src, ROUTE_NEEDLE);

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
            "  route-registration sites in main.rs: {route_sites} > 0 — wiring-only means \
             NO registrations here; they live under src/server/router/**"
        ));
    }
    if router_sites < ROUTER_SITES_FLOOR {
        breaches.push(format!(
            "  route-registration sites under src/server/router: {router_sites} < floor {ROUTER_SITES_FLOOR} — \
             the composed chain lost registrations"
        ));
    }
    if total_tests < CRATE_TEST_FLOOR {
        breaches.push(format!(
            "  crate #[test] count: {total_tests} < floor {CRATE_TEST_FLOOR} — the \
             load-bearing total-test floor dropped; tests may not be deleted, only moved"
        ));
    }
    let route_rows = crate::server::router::route_guards::OPENAPI_ROUTES.len();
    let authz_rows = crate::server::router::route_guards::AUTHZ_GATES.len();
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

// ── the grep gates (born hard — no warning phase, the Foundation
//    precedent). Scanners are pure fns so the self-pins can prove they
//    fire without planting real violations in the tree.

/// The route-registration needle, built by concat so no scanner file can
/// trip the gate on its own source: the literal never appears verbatim in
/// this module (and the gate forbids it everywhere under src/ except the
/// router families + the one fenced carve-out). Doc comments say
/// "route-registration" for the same reason.
const ROUTE_NEEDLE: &str = concat!(".rout", "e(");

fn count_route_sites(src: &str) -> usize {
    count_needle(src, ROUTE_NEEDLE)
}

/// Word-boundary needle: `word` matches only when not flanked by ident
/// characters — so `RequestBodyLimitLayer` or a lowercase "router-level"
/// never fires, while the type names do.
fn has_word(src: &str, word: &str) -> bool {
    let bytes = src.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(word) {
        let i = from + rel;
        let before_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        let after = i + word.len();
        let after_ok =
            after >= bytes.len() || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
        if before_ok && after_ok {
            return true;
        }
        from = i + 1;
    }
    false
}

/// The bootstrap protocol detector, factored for the self-pin.
fn protocol_hits(src: &str) -> Vec<&'static str> {
    let mut hits = Vec::new();
    if src.contains("axum::") {
        hits.push("axum::");
    }
    for word in ["Router", "Request", "Response"] {
        if has_word(src, word) {
            hits.push(word);
        }
    }
    hits
}

/// GATE: route registrations live ONLY under src/server/router/** — a
/// registration anywhere else under src/ (production, test, or comment
/// residue) fails CI. One carve-out, fenced: src/bin/mcp.rs is a separate
/// binary with its own single-endpoint protocol edge (the /mcp
/// registration); the fence pins that file at EXACTLY one site so the
/// carve-out cannot grow silently. Anti-vacuous: the scanner provably
/// reads the legal home — the router mass is at or above the floor.
///
/// Red-proof (run against planted violations before this gate's green
/// commit): a planted route-registration comment (the needle + a fake
/// registration) in src/config.rs turned this gate red; the plant was
/// reverted. The inline self-pins keep the proof permanent: the scanner
/// counts a planted violation string in a comment and stays quiet on
/// clean source.
#[test]
fn route_registrations_live_only_under_router() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk_rs_files(&root, &mut files);
    assert!(
        files.len() >= 50,
        "gate: src walk found only {} files — the walker is broken",
        files.len()
    );

    let mut violations: Vec<String> = Vec::new();
    let mut router_sites = 0usize;
    let mut mcp_sites = None;
    for p in &files {
        let rel = p.strip_prefix(&root).unwrap_or(p);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let src = std::fs::read_to_string(p).unwrap_or_default();
        if rel_str.starts_with("server/router/") {
            router_sites += count_route_sites(&src);
            continue;
        }
        if rel_str == "bin/mcp.rs" {
            mcp_sites = Some(count_route_sites(&src));
            continue;
        }
        let n = count_route_sites(&src);
        if n > 0 {
            violations.push(format!(
                "  {}: {n} route-registration site(s) outside src/server/router/**",
                rel.display()
            ));
        }
    }
    assert_eq!(
        mcp_sites,
        Some(1),
        "gate: src/bin/mcp.rs must keep EXACTLY one route registration (the /mcp \
         protocol edge) — a second site means a router is growing outside the families"
    );
    assert!(
        router_sites >= ROUTER_SITES_FLOOR,
        "gate: scanner found only {router_sites} sites under src/server/router/** — \
         below the floor; the walk or the needle is broken"
    );
    // self-pin (the Cornerstone lesson): a scanner that cannot fire guards
    // nothing. Plant a violation string in a COMMENT — exactly the shape the
    // gate must catch in a real file — and count it; then prove clean
    // source stays quiet.
    let planted = format!("//{}\"/plants\", get(stub));", ROUTE_NEEDLE);
    assert_eq!(
        count_route_sites(&planted),
        1,
        "gate self-pin: the scanner cannot see a planted violation — it guards nothing"
    );
    assert_eq!(
        count_route_sites("// no registrations here"),
        0,
        "gate self-pin: the scanner fires on clean source"
    );
    assert!(
        violations.is_empty(),
        "ROUTE REGISTRATION GATE — routes register ONLY under src/server/router/**:\n{}",
        violations.join("\n")
    );
}

/// GATE: `server::bootstrap` stays protocol-free — no axum types cross its
/// surface (the Vaulting discipline, machine-checked). Needles: `axum::`
/// plus Router/Request/Response on word boundaries, so the words a
/// protocol-free module may legitimately say in comments ("takes an axum
/// type", "router-level RequestBodyLimitLayer") never fire, while the type
/// names do.
///
/// Red-proof (run before this gate's green commit): a planted
/// `// uses axum::Router here` comment in src/server/bootstrap.rs turned
/// this gate red; the plant was reverted. The inline self-pins keep the
/// proof permanent per needle class.
#[test]
fn bootstrap_stays_protocol_free() {
    let src = include_str!("server/bootstrap.rs");
    // anti-vacuous: we are reading the real file
    assert!(
        src.contains("pub fn bootstrap("),
        "gate: include_str! did not resolve the bootstrap source — guarding nothing"
    );
    let hits = protocol_hits(src);
    // self-pin: each needle class fires on its synthetic violation…
    for (sample, expected) in [
        ("use axum::Router;", "axum::"),
        ("Router::new()", "Router"),
        ("fn f(r: Request<String>) {}", "Request"),
        ("-> Response<String> {", "Response"),
    ] {
        let hits = protocol_hits(sample);
        assert!(
            hits.contains(&expected),
            "gate self-pin: {expected} needle cannot fire — it guards nothing"
        );
    }
    // …and stays quiet on the comment forms the real file may carry.
    for clean in [
        "// takes an axum type",
        "// a router-level RequestBodyLimitLayer is applied eagerly",
    ] {
        assert!(
            protocol_hits(clean).is_empty(),
            "gate self-pin: clean sample tripped the detector: {clean}"
        );
    }
    assert!(
        hits.is_empty(),
        "BOOTSTRAP PROTOCOL GATE — server::bootstrap must stay protocol-free; found: {hits:?}"
    );
}

// ── the CLI help law (v1.28.61 "Standby") ────────────────────────────────

/// cli_reference_covers_subcommands — every name in the `brain` binary's
/// SUBCOMMANDS table must appear in docs/cli-reference.md. This closes the
/// gap that let `brain parcel` (and wfm-import, valet, ropa after it) ship
/// with no reference row: the help text can never drift from the dispatch
/// table (the table generates it), but NOTHING pinned the docs — until now.
/// The pin parses the same table the dispatcher consumes, so a new command
/// is incomplete until its doc row lands in the same commit.
#[test]
fn cli_reference_covers_subcommands() {
    let brain_src =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/brain.rs"))
            .expect("src/bin/brain.rs must be readable");
    let doc = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/cli-reference.md"),
    )
    .expect("docs/cli-reference.md must be readable");
    // Extract the ONE dispatch/help table (lines from `const SUBCOMMANDS` to
    // the closing `];`) and pull every `name: "…"` out of it.
    let table = brain_src
        .split("const SUBCOMMANDS")
        .nth(1)
        .expect("SUBCOMMANDS table missing from brain.rs — the dispatch law is broken");
    let table = &table[..table.find("\n];").expect("SUBCOMMANDS table never closes")];
    let names: Vec<&str> = table
        .lines()
        .filter_map(|l| {
            let idx = l.find("name: \"")?;
            let rest = &l[idx + "name: \"".len()..];
            let end = rest.find('"')?;
            Some(&rest[..end])
        })
        .collect();
    // Anti-vacuous: the parser must be looking at the real table (40 named
    // subcommands at the Standby open — a floor, so new commands only raise it).
    assert!(
        names.len() >= 40,
        "cli-reference pin parsed {} subcommand names — the table \
         extractor is broken and guards nothing",
        names.len()
    );
    let missing: Vec<&str> = names
        .iter()
        .filter(|n| !doc.contains(&format!("brain {n}")))
        .copied()
        .collect();
    assert!(
        missing.is_empty(),
        "cli-reference.md is missing rows for: {missing:?} — every SUBCOMMANDS \
         entry ships its doc row in the same commit"
    );
}
