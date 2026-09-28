//! R46 — the byte-range evidence verifier: the SCOPE, SUPPLY-CHAIN, and
//! BOUNDARY half of the plan's §5 battery, red-first.
//!
//! **Why this file is half a battery, and where the other half is.** The plan
//! (§3) puts the whole §5 suite in `tests/r46_*.rs`. That is not simultaneously
//! satisfiable with the round's own scope law, and the wall is measured, not
//! assumed: `delivery_r42_adds_no_dependency`, `delivery_r43_adds_no_dependency`,
//! and `delivery_r44_adds_no_table_no_stamp_no_dependency`
//! (`tests/main_suite.rs`) each parse the ROOT manifest's `[dependencies]`
//! section and assert `names.len() == 51`. A kernel test that `use`s a crate
//! needs a root path-dependency, which moves that count to 52 and turns three
//! SHIPPED release pins red. The round answers the execution prompt's OPEN
//! QUESTION 1 with **no** — the kernel does not consume the crate this round —
//! so the battery SPLITS along the only line that splits honestly:
//!
//!   * **behavioural pins** (the arithmetic, the CID, discrimination,
//!     purity) live INSIDE `crates/brain-evidence-core/src/lib.rs`, where they
//!     call the real functions on the `engine-crates` CI leg;
//!   * **scope / supply-chain / boundary pins** (everything below) live HERE,
//!     reading the crate and the manifests as TEXT — the
//!     `tests/r45_0_claim_pins.rs` idiom — so they run on the `lint-test` leg.
//!
//! A behavioural pin that asserted over source text instead of calling the code
//! would be exactly the "asserted in prose" failure the plan forbids
//! (§4.4). A scope pin that needed a live call would drag the kernel's
//! dependency graph along to check that a graph did not grow. Neither
//! compromise is made here.
//!
//! **RED-first.** Every pin that reads the crate is RED at this commit because
//! `crates/brain-evidence-core` does not exist yet; a missing path is a loud
//! `panic!` naming the path, never a silent pass. The pins that assert an
//! ABSENCE (`src/workflow/create.rs`, the floor, the route surfaces) are GREEN
//! from this commit onward and must stay green — that is their job, and the
//! R45-0 file states the same split for the same reason.
//!
//! **The count is 37, not the 30 the execution prompt prints.** The plan's §5
//! lists 32; five pins are added here and two renamed, each for a reason stated
//! at its own definition. Renames touch PIN names only. The verdict-cause
//! vocabulary — the thing the prompt freezes — is untouched and is enumerated
//! in `EXPECTED_VERDICT_CAUSES` below.
//!
//! **Disclosed ceiling on the source scans.** The three source-scan pins strip
//! line comments and `//!` doc lines before scanning, so the crate's own honest
//! boundary table — which must say "no LLM, no semantic contradiction" in prose
//! — cannot trip a guard that bans the token. That strip is line-level and NOT
//! string-aware (the house's `handler_body` is string-aware for the same class
//! of problem at a larger scale). A needle placed inside a string literal in
//! this crate's code would false-positive. The three scans are therefore
//! corroborated by the MANIFEST assertions, which are structural and cannot
//! false-positive: a crate whose only dependency is a hash function cannot
//! reach a model, a clock, or a network, whatever its source says.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const CRATE_DIR: &str = "crates/brain-evidence-core";

// ─────────────────────────────────────────────────────────────────────────────
// the closed verdict vocabulary (the prompt freezes these; E2's deny-wins)
// ─────────────────────────────────────────────────────────────────────────────

/// Every variant the closed enum may carry. A seventh is a scope change and a
/// compile error in every consumer; a rename is a break.
const EXPECTED_VERDICT_CAUSES: &[&str] = &[
    "Resolved",
    "UnresolvedSource",
    "CidMismatch",
    "QuoteMismatch",
    "RangeOutOfBounds",
    "EmptyEvidence",
];

/// The snake_case vocabulary E9 requires a failure to be itemized under. One
/// entry per refusal cause; `Resolved` is not a failure and is absent.
const EXPECTED_FAILURE_CAUSES: &[&str] = &[
    "out_of_range",
    "quote_mismatch",
    "cid_mismatch",
    "unresolved_source",
    "empty_evidence",
];

// ─────────────────────────────────────────────────────────────────────────────
// read helpers (the r45_0_claim_pins.rs idiom: loud panic, never a silent pass)
// ─────────────────────────────────────────────────────────────────────────────

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn crate_path(rel: &str) -> PathBuf {
    repo_root().join(CRATE_DIR).join(rel)
}

fn read_crate(rel: &str) -> String {
    let path = crate_path(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "the R46 resolver crate must exist at {}: {e}. R46's scope and supply-chain \
             battery reads it as text; a public-only or pre-R46 checkout that cannot \
             see it would otherwise pass vacuously.",
            path.display()
        )
    })
}

fn read_repo(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The production region: everything before the crate's test module.
///
/// Scoping is not decoration. A whole-file scan passes on the scanner's own
/// literal strings, which is the R38 `blast_radius` failure mode and the reason
/// `crates/brain-executor-core` cuts the region before asserting (lib.rs:314-326).
fn production_region(src: &str) -> &str {
    src.split_once("#[cfg(test)]").map_or(src, |(head, _)| head)
}

/// The CODE region: production, minus every `//!` doc line and every line whose
/// first non-space characters are `//`.
///
/// The crate's honest boundary table is a `//!` header and it MUST say "no LLM"
/// and "no semantic contradiction" in prose. Scanning that prose for those
/// tokens is a guard that fires on correct code, which this repo treats as worse
/// than no guard at all.
fn code_region(src: &str) -> String {
    production_region(src)
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("//") || t.starts_with("#[doc"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn walk_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Every `.rs` file in the crate, production regions included — `src/` only.
fn crate_sources() -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk_rs_files(&crate_path("src"), &mut files);
    files
}

/// Every `.rs` file in the WHOLE crate — `src/` and `tests/` alike.
///
/// The distinction matters and both halves are deliberate. The SOURCE pins
/// scan `crate_sources()` (src/ only): a measurement harness in `tests/` reads
/// files and prints, which the purity law forbids in production code, so
/// scanning it for `std::fs` would be a guard firing on correct code. The
/// CENSUS scans the whole crate, because `CRATE_TEST_FLOOR`'s needle walks the
/// kernel's `src/` + `tests/` and never reaches `crates/` at all — a census
/// reading only `src/` would report a suite whose `tests/` half it never sees.
fn crate_all_sources() -> Vec<PathBuf> {
    let mut files = crate_sources();
    walk_rs_files(&crate_path("tests"), &mut files);
    files
}

/// The crate's `[dependencies]` names, read as text from its manifest.
fn crate_dependency_names() -> Vec<String> {
    let manifest = read_crate("Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .expect("the crate manifest must carry a [dependencies] section");
    deps.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && l.contains('='))
        .filter_map(|l| l.split('=').next())
        .map(str::trim)
        .map(str::to_string)
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// the census (shared by the real assertion and its in-band red-proof)
// ─────────────────────────────────────────────────────────────────────────────

/// The names of every test function defined in `src` — parsed, not grepped.
///
/// The house needle is a raw substring count and the spire guard says so
/// itself (`src/spire_inventory.rs:58-59`: "the needle counts doc-comment
/// literals too"). A census built on substring matching would count this very
/// file's `EXPECTED_PINS` string literals and could not tell a deleted pin from
/// a renamed one, so it parses `#[test]` followed by `fn <name>(` instead.
fn test_fn_names(src: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut cursor = 0usize;
    while let Some(rel) = src[cursor..].find("#[test]") {
        let after_attr = cursor + rel + "#[test]".len();
        let rest = src[after_attr..].trim_start();
        if let Some(tail) = rest.strip_prefix("fn ") {
            let name: String = tail
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                names.insert(name);
            }
        }
        cursor = after_attr;
    }
    names
}

/// Every test name across BOTH homes the round's battery spans.
///
/// Both homes, deliberately: `CRATE_TEST_FLOOR`'s needle walks the kernel's
/// `src/` and `tests/` and does NOT reach `crates/` at all
/// (`src/spire_inventory.rs:232-249`), so a census reading only `tests/` would
/// report a suite whose behavioural half it never sees — which is the "a guard
/// that silently stops covering looks exactly like coverage" failure the R45-0
/// file names at `tests/r45_0_claim_pins.rs:1105-1109`.
fn battery_census() -> BTreeSet<String> {
    let mut names = test_fn_names(&read_repo("tests/r46_evidence_pins.rs"));
    for f in crate_all_sources() {
        let text = std::fs::read_to_string(&f)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", f.display()));
        names.extend(test_fn_names(&text));
    }
    names
}

/// The symmetric difference between the contract and what is on disk.
///
/// Both directions matter: a missing pin is a hole, and an UNEXPECTED pin is
/// either an undocumented addition or a renamed one that left the old name
/// behind. Either must be a visible failure, not a silent superset.
fn census_diff(expected: &[&str], actual: &BTreeSet<String>) -> Result<(), String> {
    let mut missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|n| !actual.contains(*n))
        .collect();
    let actual_refs: BTreeSet<&str> = actual.iter().map(String::as_str).collect();
    let mut unexpected: Vec<&str> = actual_refs
        .iter()
        .copied()
        .filter(|n| !expected.contains(n))
        .collect();
    missing.sort_unstable();
    unexpected.sort_unstable();
    if missing.is_empty() && unexpected.is_empty() {
        return Ok(());
    }
    Err(format!("missing: {missing:?}; unexpected: {unexpected:?}"))
}

/// The round's MEASUREMENT HARNESS — reported in the census, but deliberately
/// NOT part of the battery contract.
///
/// The spike is a measurement, not a pin: it prints a number and asserts one
/// thing (the KILL-3 rate is zero). A low resolve rate is a FINDING to report,
/// not a test failure, so folding it into `EXPECTED_PINS` would imply the
/// battery requires a particular number. It is listed here instead so the
/// census still cannot miss it — "a guard that silently stops covering looks
/// exactly like coverage".
const MEASUREMENT_HARNESS: &[&str] = &["r46_spike_resolve_and_discrimination_rates"];

/// Every pin R46 must ship, across both homes.
///
/// Thirty-two names are the plan's §5 list. Three are RENAMED and each rename
/// is justified at its definition below; the renames touch pin names only and
/// no verdict cause. Two are the plan's own `blake3` pins, renamed because a
/// pin whose name names a primitive the round's own zero-edge law forbids it to
/// use is a pin that lies — this repo's honest-labelling law outranks name
/// stability for a name that would be false on arrival. Five are ADDED, each
/// closing a fail-open or an honesty hole the plan's own §0.3 obligation
/// (discrimination) and §2 E9 (itemize every failure by cause) require closed.
const EXPECTED_PINS: &[&str] = &[
    // ── the plan's §5 list, verbatim ──
    "r46_byte_range_out_of_bounds_refuses_rather_than_panics",
    "r46_quote_mismatch_refuses_even_when_the_range_is_well_formed",
    "r46_cid_recomputed_from_a_tampered_source_is_refused",
    "r46_resolvable_evidence_resolves_with_no_model_in_the_path",
    "r46_quote_resolves_only_at_its_exact_declared_range",
    "r46_a_narrowed_range_containing_the_quote_still_refuses",
    "r46_empty_evidence_set_refuses",
    "r46_inverted_range_start_greater_than_end_refuses",
    "r46_range_past_end_of_source_refuses",
    "r46_quote_matching_a_different_position_in_the_same_source_refuses",
    // (renamed: the plan's `..._cid_is_a_blake3_multihash_...` — see the pin)
    "r46_cid_is_a_sha256_multihash_over_the_exact_source_bytes",
    "r46_cid_is_stable_across_runs_and_is_lowercase_base32",
    "r46_any_single_flipped_byte_changes_the_cid",
    // (renamed: the plan's `..._uses_the_existing_blake3_...` — see the pin)
    "r46_cid_uses_the_locked_hash_and_the_house_base32_and_adds_no_hash_implementation",
    "r46_consolidation_that_rewrites_a_source_breaks_the_stored_reference_visibly",
    "r46_a_rewritten_source_cannot_silently_satisfy_the_original_evidence",
    "r46_resolver_crate_declares_no_provider_or_model_dependency",
    "r46_resolver_source_contains_no_model_or_prompt_reference",
    "r46_resolver_depends_on_no_clock_store_or_network",
    "r46_verdict_enum_is_closed_and_exhaustively_matched",
    "r46_documented_boundary_excludes_semantic_contradiction",
    "r46_documented_boundary_is_present_in_the_module_doc_comment",
    "r46_no_approximate_vector_search_is_introduced_by_this_round",
    "r46_resolution_is_deterministic_across_runs",
    "r46_resolution_does_not_depend_on_any_clock_or_environment",
    "r46_no_audit_row_is_emitted_and_no_key_is_minted",
    "r46_create_rs_does_not_exist_yet",
    "r46_round_adds_no_route_no_table_and_no_schema_stamp",
    "r46_resolver_crate_contains_no_unsafe",
    "r46_crate_test_floor_is_never_lowered",
    "r46_pin_suite_is_non_vacuous_and_fails_when_pins_are_removed",
    "r46_dependency_delta_is_exactly_nothing_and_the_workspace_crate",
    // ── ADDED: five, each for a reason stated at its definition ──
    "r46_empty_quote_never_resolves",
    "r46_resolved_requires_byte_identical_range_content",
    "r46_verdict_precedence_is_fixed_and_by_cause",
    "r46_resolver_never_derives_a_cid_from_the_bytes_it_was_handed",
    "r46_boundary_doc_names_the_verify_offset_skew",
];

// ─────────────────────────────────────────────────────────────────────────────
// the battery — scope, supply chain, boundary
// ─────────────────────────────────────────────────────────────────────────────

/// E1/E4's structural half: the crate can reach a model, a provider, or an
/// inference client ONLY through a dependency, and its only dependency is a
/// hash function. Red-proofed by the plan: planting a provider dev-dependency
/// and re-reading this manifest turns it red.
#[test]
fn r46_resolver_crate_declares_no_provider_or_model_dependency() {
    let names = crate_dependency_names();
    assert_eq!(
        names,
        vec!["sha2".to_string()],
        "the resolver's only dependency must be the already-locked hash; a provider, \
         client, or inference trait would put a model in the path by construction"
    );
    let manifest = read_crate("Cargo.toml");
    let dev = manifest
        .split("[dev-dependencies]")
        .nth(1)
        .map(|rest| rest.split("\n[").next().unwrap_or(""));
    assert!(
        dev.is_none_or(|d| d.lines().all(|l| {
            let t = l.trim();
            t.is_empty() || t.starts_with('#')
        })),
        "the resolver declares no dev-dependency either: a dev-edge is still an edge, \
         and a test-only provider would be the easiest way to reintroduce one"
    );
}

/// §4.7. The scan covers the crate's whole `src/` tree and asserts the
/// anti-vacuity floor, because a walk that finds nothing has found nothing
/// (the lipstyk lesson, `tests/write_discipline.rs:82-89`).
///
/// **The needle list is the EXECUTABLE forms, not the bare word.** The house's
/// zero-unsafe idiom is the literal attribute `#![forbid(unsafe_code)]`, so a
/// scan for the bare word `unsafe` matches `unsafe_code` inside the honest call
/// site and pins correct code as a defect.
#[test]
fn r46_resolver_crate_contains_no_unsafe() {
    const EXECUTABLE_UNSAFE_FORMS: &[&str] = &[
        "unsafe {",
        "unsafe fn",
        "unsafe impl",
        "unsafe trait",
        "unsafe extern",
        "get_unchecked",
        "transmute",
        "from_raw_parts",
        "as_mut_ptr",
        "assume_init",
        "MaybeUninit",
        "static mut",
    ];

    let lib = read_crate("src/lib.rs");
    let production = production_region(&lib);
    assert!(
        production.contains("#![forbid(unsafe_code)]"),
        "the resolver crate must carry the un-overridable lint at its root — `forbid` \
         cannot be silenced by a downstream `#[allow]`, `deny` can (the \
         brain-executor-core / brain-consensus-core precedent)"
    );

    let files = crate_sources();
    assert!(
        files.len() >= 2,
        "the crate's src/ walk found {} files — the scan is vacuous",
        files.len()
    );
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let production = production_region(&text);
        for needle in EXECUTABLE_UNSAFE_FORMS {
            assert!(
                !production.contains(needle),
                "{}: the resolver crate contains `{needle}`",
                file.display()
            );
        }
    }
}

/// Security control 1, belt to the manifest's braces. The crate's boundary
/// table must SAY "no model" in prose; the code must not BE one.
#[test]
fn r46_resolver_source_contains_no_model_or_prompt_reference() {
    const MODEL_TOKENS: &[&str] = &[
        "openai",
        "anthropic",
        "ollama",
        "llm",
        "Completion",
        "Prompt",
        "temperature",
        "top_p",
    ];
    let files = crate_sources();
    assert!(
        !files.is_empty(),
        "the crate's src/ walk found nothing to scan"
    );
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let code = code_region(&text);
        for needle in MODEL_TOKENS {
            assert!(
                !code.contains(needle),
                "{}: the resolver's CODE region names `{needle}` — the no-model claim \
                 (E1/E2) is structural, and a model reference in code breaks it",
                file.display()
            );
        }
    }
}

/// E2: "pure, total, and I/O-free; no clock, no store, no network, no
/// provider" — the contract `brain-delivery-core` already states. These are
/// the APIs that would break it.
#[test]
fn r46_resolver_depends_on_no_clock_store_or_network() {
    const IO_TOKENS: &[&str] = &[
        "std::time",
        "SystemTime",
        "Instant",
        "std::env",
        "std::fs",
        "std::net",
        "rusqlite",
        "Connection",
        "tokio",
        "reqwest",
        "std::process",
    ];
    let files = crate_sources();
    assert!(
        !files.is_empty(),
        "the crate's src/ walk found nothing to scan"
    );
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let code = code_region(&text);
        for needle in IO_TOKENS {
            assert!(
                !code.contains(needle),
                "{}: the resolver's CODE region names `{needle}` — the purity law (E2) \
                 says no clock, no store, no network, and this is the seam it would \
                 arrive through",
                file.display()
            );
        }
    }
}

/// E2's closed vocabulary. The risk this closes is a PERMISSIVE variant: an
/// enum that grows a `ProbablyResolved` arm is a compile error in every
/// consumer only if the enum stays closed, and closure is checkable by
/// parsing the declaration rather than by reading it.
///
/// Exhaustiveness is enforced by the COMPILER, not here: the crate matches
/// every variant in its own code with no wildcard, so dropping an arm is a
/// build failure. This pin's job is the other half — that the SET is exactly
/// the six, and no seventh.
#[test]
fn r46_verdict_enum_is_closed_and_exhaustively_matched() {
    let lib = read_crate("src/lib.rs");
    let code = code_region(&lib);
    let start = code
        .find("pub enum EvidenceVerdict")
        .expect("the crate must declare `pub enum EvidenceVerdict`");
    let body = &code[start..];
    let open = body
        .find('{')
        .expect("the enum declaration must have a body");
    let close = body[open..]
        .find('}')
        .map(|i| i + open)
        .expect("the enum body must close");
    let decl = &body[open + 1..close];

    let mut found: Vec<String> = decl
        .split(',')
        .filter_map(|seg| {
            let t = seg.trim();
            let name: String = t
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() { None } else { Some(name) }
        })
        .collect();
    found.sort();

    let mut expected: Vec<String> = EXPECTED_VERDICT_CAUSES
        .iter()
        .map(|s| s.to_string())
        .collect();
    expected.sort();
    assert_eq!(
        found, expected,
        "EvidenceVerdict must carry EXACTLY the six closed variants; a permissive \
         seventh is the KILL-3 bypass wearing a new name"
    );

    for cause in EXPECTED_FAILURE_CAUSES {
        assert!(
            lib.contains(cause),
            "the crate must name the `{cause}` failure cause — E9 requires every \
             failure to be itemized under a cause, and a cause no code spells is a \
             cause no consumer can aggregate"
        );
    }
}

/// §3's "NO new hash" half, re-scoped to the property the round can actually
/// hold. **The plan's original name is `r46_cid_uses_the_existing_blake3_...`
/// and it is renamed here, deliberately.** The plan is internally inconsistent
/// about its primitive — §0.3 and the execution prompt both specify `sha256`,
/// while E1, §3, and the two original pin names say `blake3` — and `blake3` is
/// unreachable from a zero-edge crate: it is absent from `crates/Cargo.lock`
/// entirely, and depending on it would add four new external packages
/// (`blake3`, `arrayref`, `arrayvec`, `constant_time_eq`) to a workspace whose
/// defining property is that it has none. `sha2 0.11.0` is already in
/// `crates/Cargo.lock` (via `brain-delivery-core` and three sibling cores), so
/// using it adds ZERO new `[[package]]` entries. A pin whose NAME says `blake3`
/// and whose BODY asserts `sha256` would be a guard that lies; the rename is
/// the honest-labelling law applied to a name that would be false on arrival.
#[test]
fn r46_cid_uses_the_locked_hash_and_the_house_base32_and_adds_no_hash_implementation() {
    // (a) the crate adds no hash crate beyond the one already in the lockfile
    let names = crate_dependency_names();
    assert_eq!(
        names,
        vec!["sha2".to_string()],
        "the CID rides the hash the crates workspace ALREADY locks; a second hash \
         crate would be a new external edge, and the plan's zero-edge law outranks \
         its own primitive choice"
    );

    // (b) no hand-rolled primitive smuggled into the crate's CODE regions.
    //     Scanned across EVERY module, not just lib.rs — the base32 alphabet
    //     lives in cid.rs, and a scan of one file would miss it.
    let files = crate_sources();
    assert!(
        files.len() >= 2,
        "the crate's src/ walk found {} files",
        files.len()
    );
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let code = code_region(&text);
        for needle in ["const K:", "rotate_right", "wrapping_add(", "0x6a09e667"] {
            assert!(
                !code.contains(needle),
                "{}: the resolver must not carry its own hash implementation \
                 (`{needle}`): E4 declines adding a hash, and a hand-rolled \
                 primitive in the trust path of a provenance verifier inverts the \
                 risk calculus",
                file.display()
            );
        }
    }

    // (c) the base32 encoding is the house's — RFC 4648, no padding, lowercase
    let joined: String = files
        .iter()
        .map(|f| {
            std::fs::read_to_string(f)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", f.display()))
        })
        .map(|t| code_region(&t))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("abcdefghijklmnopqrstuvwxyz234567"),
        "the CID encodes over the kernel's alphabet (src/ump_integrity.rs:23) — \
         RFC 4648, no padding, LOWERCASE. A second alphabet would make R46 CIDs \
         incomparable with every content hash the tree already stores."
    );
}

/// E7: "a verifier whose boundary is unstated will be quoted as covering
/// semantic contradiction." The table is a test asset, not a courtesy.
#[test]
fn r46_documented_boundary_is_present_in_the_module_doc_comment() {
    let lib = read_crate("src/lib.rs");
    let header: String = lib
        .lines()
        .take_while(|l| {
            let t = l.trim_start();
            t.starts_with("//!") || t.is_empty()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        header
            .lines()
            .filter(|l| l.trim_start().starts_with("//!"))
            .count()
            >= 20,
        "the crate must open with a //! boundary table that says what it proves AND \
         what it does not (E7); the table is long because the honest rows are the \
         ones that are easy to leave out"
    );
    for marker in ["does not", "no model", "not a proof"] {
        assert!(
            header.contains(marker),
            "the boundary header must carry the marker `{marker}` — a table of \
             capabilities with no stated limit is the failure E7 exists to prevent"
        );
    }
}

/// E7's specific row, pinned by name because it is the row that gets quoted
/// past its boundary. arXiv:2507.09751: semantic contradiction detection still
/// needs an LLM in the interpretation function — and that stays true here.
#[test]
fn r46_documented_boundary_excludes_semantic_contradiction() {
    let lib = read_crate("src/lib.rs");
    let header: String = lib
        .lines()
        .take_while(|l| {
            let t = l.trim_start();
            t.starts_with("//!") || t.is_empty()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let lower = header.to_lowercase();
    assert!(
        lower.contains("semantic contradiction"),
        "the boundary table must name semantic contradiction explicitly — it is the \
         check this round most invites a reader to assume it performs"
    );
    assert!(
        lower.contains("deterministic"),
        "the boundary table must mark semantic contradiction non-deterministic, so \
         the refusal is legible as a boundary rather than as a bug"
    );
}

/// E8: no HNSW, no ANN, ever, in this line. Approximate ordering and exact
/// reproducibility are incompatible by construction.
#[test]
fn r46_no_approximate_vector_search_is_introduced_by_this_round() {
    let mut files = crate_sources();
    files.push(crate_path("Cargo.toml"));
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let code = code_region(&text);
        for needle in ["hnsw", "annoy", "scann", "usearch", "faiss", "lsh("] {
            assert!(
                !code.to_lowercase().contains(needle),
                "{}: `{needle}` — E8 forbids approximate vector search in this line; \
                 it would break every determinism pin by construction",
                file.display()
            );
        }
    }
}

/// §4.5: "R46 writes nothing, emits no audit row, mints no key, and reads no
/// operator content. The pure crate cannot write: it has no store seam."
#[test]
fn r46_no_audit_row_is_emitted_and_no_key_is_minted() {
    let files = crate_sources();
    assert!(
        !files.is_empty(),
        "the crate's src/ walk found nothing to scan"
    );
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let code = code_region(&text);
        for needle in [
            "Ed25519",
            "ed25519",
            "SigningKey",
            "signing",
            "insert into",
            "audit_events",
            "hash_chain",
        ] {
            assert!(
                !code.contains(needle),
                "{}: the resolver names `{needle}` — R46 writes nothing and mints no \
                 key; the pure crate has no store seam and must not grow one",
                file.display()
            );
        }
    }
}

/// E9 + §6's "dependency delta is verified as ZERO new external edges — the
/// workspace crate plus nothing else."
///
/// The mechanical distinction, which is the honest form of that sentence: **a
/// legitimate workspace-member addition is a lockfile `[[package]]` block with
/// NO `source` line and NO `checksum` line; a new external edge always
/// introduces at least one block WITH a registry `source`.** Path members carry
/// neither, and `sha2` is already present, so the diff must add zero of both.
#[test]
fn r46_dependency_delta_is_exactly_nothing_and_the_workspace_crate() {
    // (a) the kernel does NOT depend on the crate — this is the machine-enforced
    //     form of the execution prompt's OPEN QUESTION 1, and it is what keeps
    //     the three frozen `names.len() == 51` release pins green.
    let root_manifest = read_repo("Cargo.toml");
    assert!(
        !root_manifest.contains("brain-evidence-core"),
        "the root manifest must not mention the crate: R46 has no kernel consumer \
         (R50 is the first), and a path-dependency added for no current consumer is \
         exactly the speculative wiring the plan forbids. It would also move the \
         frozen direct-dependency count 51 -> 52 and turn three SHIPPED release \
         pins red."
    );

    // (b) the crates workspace lists it, exactly once, as a member
    let members = read_repo("crates/Cargo.toml");
    assert_eq!(
        members.matches("\"brain-evidence-core\"").count(),
        1,
        "the crates workspace must list brain-evidence-core as a member exactly once"
    );

    // (c) the crate's lockfile block carries no source and no checksum.
    //     The block is bounded by the surrounding `[[package]]` headers —
    //     splitting on the NAME and scanning forward runs into the NEXT
    //     package, which does carry a registry source, and the first version
    //     of this pin failed for exactly that reason.
    let lock = read_repo("crates/Cargo.lock");
    let block = lock
        .split("[[package]]")
        .find(|b| b.contains("name = \"brain-evidence-core\""))
        .expect("crates/Cargo.lock must carry a [[package]] block for the new member");
    assert!(
        block.contains("\"sha2\""),
        "the member's block must name its dependency: {block}"
    );
    assert!(
        !block.contains("source =") && !block.contains("checksum ="),
        "a workspace member's lockfile block carries no `source` and no `checksum` — \
         the presence of either would mean a NEW EXTERNAL EDGE, which is the one \
         thing this round's dependency claim forbids. Block was: {block}"
    );
}

/// "R46 produces no table, no route, no schema stamp, and no migration." The
/// positive structural form: the crate appears on none of the surfaces that
/// would carry one.
#[test]
fn r46_round_adds_no_route_no_table_and_no_schema_stamp() {
    for surface in [
        "openapi.yaml",
        "src/server/router/route_guards.rs",
        "src/storage_layout.rs",
        "src/migration.rs",
    ] {
        let text = read_repo(surface);
        assert!(
            !text.contains("brain-evidence-core") && !text.contains("brain_evidence"),
            "{surface} names the R46 crate — a route, a table, or a schema stamp would \
             name it there. R46's wire consequence is nil and that is worth pinning."
        );
    }
    // The route census PARSES route keys rather than substringing the document.
    // The first version of this pin searched for `/evidence` in the whole file and
    // went RED on two lines of correct English — "Identity/evidence only" and
    // "raw query/evidence text" — which is the R45-0 lesson exactly: a substring
    // scan over a whole file matches prose the server legitimately carries, and a
    // ban that fires on correct code is a ban that gets deleted.
    let openapi = read_repo("openapi.yaml");
    let route_keys: Vec<&str> = openapi
        .lines()
        .map(str::trim_end)
        .filter(|l| l.starts_with("  /") && l.ends_with(':'))
        .collect();
    assert!(
        route_keys.len() >= 208,
        "the route census found {} keys; openapi.yaml's route floor is 208, so the \
         parse is reading nothing and the absence check below would pass vacuously",
        route_keys.len()
    );
    for key in &route_keys {
        assert!(
            !key.to_lowercase().contains("evidence"),
            "openapi.yaml declares route `{key}` — R46 adds no route, and \
             x-api-version therefore does not move"
        );
    }
}

/// §3: "R46 does NOT create `src/workflow/create.rs`. Verified ABSENT at §0 and
/// MUST be absent at R46's close." RED by creating the file; the pin's green
/// state is its absence.
#[test]
fn r46_create_rs_does_not_exist_yet() {
    let path = repo_root().join("src/workflow/create.rs");
    assert!(
        !path.exists(),
        "{} exists — creating it is R50's work. R46 ships a verifier, not a Create \
         loop, and this pin holds the boundary.",
        path.display()
    );
}

/// The floor is UP ONLY (`src/spire_inventory.rs:139`), so a stale-low value
/// silently WEAKENS the guard instead of failing loudly — which is the trap the
/// plan's own §0 correction names. The pin asserts the floor is at or above the
/// value measured at the round's open, and that the walk still clears it.
#[test]
fn r46_crate_test_floor_is_never_lowered() {
    const ROUND_OPEN_FLOOR: usize = 2_301;

    let spire = read_repo("src/spire_inventory.rs");
    let floor_line = spire
        .lines()
        .find(|l| l.contains("const CRATE_TEST_FLOOR: usize"))
        .expect("CRATE_TEST_FLOOR must still be declared");
    let floor: usize = floor_line
        .split('=')
        .nth(1)
        .and_then(|s| s.trim().trim_end_matches(';').replace('_', "").parse().ok())
        .expect("CRATE_TEST_FLOOR must be a usize literal");
    assert!(
        floor >= ROUND_OPEN_FLOOR,
        "CRATE_TEST_FLOOR fell to {floor}; it was {ROUND_OPEN_FLOOR} at R46's open and \
         is UP ONLY. A lowered floor makes this round's own guard weaker instead of \
         failing loudly."
    );

    // the walk itself, recounted here so the constant cannot drift from reality
    let mut files = Vec::new();
    walk_rs_files(&repo_root().join("src"), &mut files);
    walk_rs_files(&repo_root().join("tests"), &mut files);
    let measured: usize = files
        .iter()
        .map(|p| {
            std::fs::read_to_string(p)
                .unwrap_or_default()
                .matches("#[test]")
                .count()
        })
        .sum();
    assert!(
        measured >= floor,
        "the needle measures {measured} test attributes under src/ + tests/ but the \
         floor is {floor}"
    );
}

/// The non-vacuity pin, and the one the plan calls "the E9 non-vacuity pin [that]
/// is not optional".
///
/// **Why a name census and not the test floor.** The plan's stated mechanism is
/// "RED by deleting three siblings and re-running". Two problems with leaning on
/// the floor for that: at the round's open the walk measures 2,321 against a
/// floor of 2,301, so deleting three pins still clears it — the floor is
/// structurally blind to the exact failure it is supposed to accompany. And a
/// test cannot delete files to re-run itself without mutating the tree mid-test.
///
/// **What makes THIS version honest.** (i) It shares one predicate with the real
/// assertion — the census it drives the red-proof through is the same function
/// the real check uses, so the two cannot drift apart. (ii) The red-proof is
/// IN-BAND: the "three siblings removed" state is constructed inside the test
/// and fed to the real comparison, so it runs on every green build rather than
/// once. (iii) It cannot pass by counting itself: the census PARSES
/// `#[test] fn <name>(` out of both homes, while `EXPECTED_PINS` holds string
/// literals the parser never sees.
///
/// **Disclosed ceiling.** A census is defeatable by editing `EXPECTED_PINS` and
/// deleting a pin in the same commit; no in-test mechanism prevents that, because
/// it is a code-review property rather than a runtime one. `EXPECTED_PINS` is a
/// `const` so the edit is a visible line in the diff, and the count floor below
/// is the second lock on the same door. The R45-0 precedent
/// (`r45_0_blueprint_completeness_pins_are_non_vacuous`) has exactly this
/// ceiling and ships anyway.
#[test]
fn r46_pin_suite_is_non_vacuous_and_fails_when_pins_are_removed() {
    let census = battery_census();
    let expected: Vec<&str> = EXPECTED_PINS
        .iter()
        .copied()
        .chain(MEASUREMENT_HARNESS.iter().copied())
        .collect();

    // (a) the real state must satisfy the same predicate
    census_diff(&expected, &census)
        .unwrap_or_else(|e| panic!("the R46 battery and its contract disagree — {e}"));

    // (b) anti-vacuous floor: a census that can pass on nothing must fail here
    assert!(
        census.len() >= expected.len(),
        "the battery census found {} test names against a contract of {} — the scan \
         is reading nothing",
        census.len(),
        expected.len()
    );
    assert!(
        census.len() == expected.len(),
        "the battery must be EXACTLY the contract's {} pins plus the {} named \
         measurement harness; an unlisted extra is either an undocumented addition \
         or a renamed pin that left its old name behind",
        EXPECTED_PINS.len(),
        MEASUREMENT_HARNESS.len()
    );

    // (c) THE RED-PROOF, in band: the same predicate driven on a suite with
    //     three siblings removed must fail, naming exactly those three.
    let thinned: BTreeSet<String> = {
        let mut t = census.clone();
        for victim in [
            "r46_quote_mismatch_refuses_even_when_the_range_is_well_formed",
            "r46_empty_evidence_set_refuses",
            "r46_round_adds_no_route_no_table_and_no_schema_stamp",
        ] {
            assert!(
                t.remove(victim),
                "the red-proof victim `{victim}` is not in the census — the red-proof \
                 must delete pins that exist, or it proves nothing"
            );
        }
        t
    };
    let err = match census_diff(&expected, &thinned) {
        Ok(()) => panic!(
            "a suite missing three pins MUST fail its own census — it passed, so the \
             census cannot detect deletion and the real assertion above is decorative"
        ),
        Err(e) => e,
    };
    for victim in [
        "r46_quote_mismatch_refuses_even_when_the_range_is_well_formed",
        "r46_empty_evidence_set_refuses",
        "r46_round_adds_no_route_no_table_and_no_schema_stamp",
    ] {
        assert!(
            err.contains(victim),
            "the red-proof must name `{victim}` as missing; it reported: {err}"
        );
    }
    assert_eq!(
        err.matches("missing:").count(),
        1,
        "the red-proof's report must be a single missing-list, not a malformed one: {err}"
    );
}

/// The `/verify` offset skew, disclosed where the code lives.
///
/// `POST /verify` (`src/handlers/verify.rs:160-161`) case-folds BOTH sides and
/// returns `match_ranges` computed over the **lowercased** haystack while
/// documenting them as offsets into the original `content` (`:47-50`). Rust's
/// `str::to_lowercase` is full Unicode and can change byte LENGTH — `İ` (U+0130)
/// is 2 bytes and lowercases to 3 — so every offset after such a character is
/// shifted, and the documented contract is false.
///
/// This is NOT the resolver's bug and R46 does not touch `/verify` (Option C:
/// it is a leaf with no internal consumer). The reason this pin exists is that
/// the skew must not be rediscovered downstream as a resolver failure: a
/// consumer that feeds `/verify` offsets into the resolver gets a FALSE
/// REFUSAL — which is fail-closed and therefore safe — and the boundary table
/// has to say so before someone "fixes" the resolver for it.
#[test]
fn r46_boundary_doc_names_the_verify_offset_skew() {
    let lib = read_crate("src/lib.rs");
    let header: String = lib
        .lines()
        .take_while(|l| {
            let t = l.trim_start();
            t.starts_with("//!") || t.is_empty()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let lower = header.to_lowercase();
    assert!(
        lower.contains("verify"),
        "the boundary header must name the /verify route: it normalizes (case-folds) \
         and its offsets are computed over the LOWERCASED haystack, so a consumer that \
         feeds them here gets a false refusal. Fail-closed, but it must be disclosed \
         at the seam, not discovered in production."
    );
    assert!(
        lower.contains("offset") && (lower.contains("skew") || lower.contains("lowercas")),
        "the /verify disclosure must name the OFFSET skew specifically, not merely \
         mention the route"
    );
}
