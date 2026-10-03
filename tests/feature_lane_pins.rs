//! R67 — **every feature the crate declares must be built by a CI lane.**
//!
//! **The gap this closes.** Six of the crate's eleven `[features]` were built
//! by no lane in `.github/workflows/`. Two of those six did not compile:
//! `injection-classifier` (9 errors, found and fixed while executing R63a) and
//! `neural-embed` (2 errors). A lane that does not exist is indistinguishable
//! from a lane that passes, so the breakage accumulated unnoticed and then
//! surfaced only when a round happened to touch that surface.
//!
//! **The pin is the deliverable, not the six lanes.** Lanes decay back to zero;
//! a pin does not. Without this file, adding the lanes fixes today's breakage
//! and guarantees nothing about next month.
//!
//! **The enumeration is DERIVED, never hand-listed** (`P67.2`). A hard-coded
//! list of eleven names would rot exactly the way the frozen schema-stamp
//! snapshots did — and that rot cost a real round to diagnose. Both sides are
//! parsed from source: features from `Cargo.toml`'s `[features]` table, lanes
//! from the workflow files.
//!
//! **Anti-vacuity runs in both directions** (`P67.3`): a scan that sees zero
//! features must fail, and a scan that sees zero lanes must fail. A coverage
//! check that passes when it sees nothing is worse than no check.
//!
//! Preregistration: `IMPL_R67_FEATURE_LANES_2026-09-29.md` (P67.1–P67.5).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

mod common;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    let p = repo(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

/// Parse the `[features]` table out of a `Cargo.toml`.
///
/// Handles the shapes actually present: single-line (`bench = []`) and
/// multi-line arrays (`otel = [` … `]`). Keys only — the `dep:` and
/// `feature/` prefixes that appear inside an array are values, not names.
fn declared_features(cargo: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_features = false;
    for line in cargo.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // A new table ends the features table. `[features]` itself starts it.
            in_features = trimmed == "[features]";
            continue;
        }
        if !in_features || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = trimmed.split_once('=') {
            let name = key.trim();
            // A feature key is a bare identifier; anything else is not one.
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Feature names that appear on a `cargo` command line in any workflow file.
///
/// The `cargo` requirement is what keeps this from false-passing on a comment
/// that merely names a feature: only a real command line counts. This is still
/// a textual check and is labelled as such — it proves a lane *mentions* the
/// feature, not that the lane passes.
fn laned_features(workflows: &[String]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for text in workflows {
        for line in text.lines() {
            if !line.contains("cargo") {
                continue;
            }
            for token in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            {
                if !token.is_empty() {
                    found.insert(token.to_string());
                }
            }
        }
    }
    found
}

fn workflow_files() -> Vec<String> {
    let dir = repo(".github/workflows");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} must be a readable directory: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "yml" || e == "yaml")
        })
        .collect();
    files.sort();
    files
        .iter()
        .map(|p| {
            std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", p.display()))
        })
        .collect()
}

/// `P67.1` + `P67.2` + `P67.3` — the coverage law.
#[test]
fn every_declared_feature_has_a_ci_lane() {
    let features = declared_features(&read("Cargo.toml"));
    let workflows = workflow_files();

    // ── anti-vacuity, direction 1: the feature parse must see the real table.
    assert!(
        features.len() >= 10,
        "the feature scan found only {} names — the `[features]` parser is broken, and a \
         coverage check that sees nothing passes while proving nothing (found {features:?})",
        features.len()
    );

    // ── anti-vacuity, direction 2: the lane scan must see real workflows.
    assert!(
        !workflows.is_empty() && workflows.iter().any(|w| w.contains("cargo")),
        "the lane scan found no `cargo` command lines in .github/workflows — the lane parser is \
         broken, and every feature would vacuously read as 'covered' by nothing"
    );

    let laned = laned_features(&workflows);
    let uncovered: Vec<&String> = features.iter().filter(|f| !laned.contains(*f)).collect();

    assert!(
        uncovered.is_empty(),
        "features declared in Cargo.toml that NO CI lane builds: {uncovered:?}.\n\
         A feature with no lane is a feature nobody knows is broken — that is how \
         `injection-classifier` (9 errors) and `neural-embed` (2 errors) both rotted unnoticed. \
         Add a lane per feature (the `otel-gate` job in .github/workflows/ci.yml is the \
         closest precedent), or record an explicit, justified exemption here. Silence is not \
         an exemption."
    );
}

/// The helpers are exercised on hostile input so the coverage check cannot pass
/// by parsing nothing. This is `R67.3` as a permanent, repeatable guard rather
/// than a one-off transcript.
#[test]
fn the_feature_parser_rejects_junk_instead_of_matching_it() {
    // A manifest with no features table yields nothing — which is what the
    // anti-vacuity assertion above exists to catch, not to tolerate.
    assert!(
        declared_features("[package]\nname = \"x\"\n").is_empty(),
        "a manifest with no [features] table must parse to zero features, not to guesses"
    );
    // A features table with content yields exactly that content.
    let parsed = declared_features("[features]\nalpha = []\nbeta = [\"dep:z\"]\ngamma = [\n]\n");
    assert_eq!(
        parsed,
        vec!["alpha", "beta", "gamma"],
        "keys only: {parsed:?}"
    );
    // Keys that are not bare identifiers are not features.
    let junk = declared_features("[features]\n\"quoted\" = []\n# comment = []\nreal = []\n");
    assert_eq!(
        junk,
        vec!["real"],
        "junk keys must not count as features: {junk:?}"
    );
}

/// `P67.5` — **the `neural-embed` fix must be the import, not a deletion.**
///
/// The build was broken because `mod neural` (a CHILD module) calls
/// `embed_input`, which is defined in the PARENT. The fix is to import it. The
/// tempting alternative — dropping the call so the build goes green — would
/// **silently disable the input budget** (`MAX_EMBED_CHARS`), a bounded-input
/// control. Nothing in the existing suite would notice, because
/// `embed_input_is_budgeted` tests the helper directly and never the call
/// sites.
///
/// So this law exists: the call sites must route through the budgeted helper.
/// A source-level check is the honest tool here — the feature needs a model
/// artifact to exercise at runtime, and a pin that pretended otherwise would
/// be its own false claim.
#[test]
fn neural_embed_routes_through_the_budgeted_input() {
    let code = code_only(&read("src/embed.rs"));
    assert!(
        code.contains("use super::{EmbedError, Embedder, embed_input}"),
        "mod neural must IMPORT the parent's budgeted helper. A child module does not inherit \
         the parent's items; this import is the fix for the 2 build errors, and removing it \
         restores the red build rather than resolving it."
    );
    // Both call sites must still go through the budget. Two occurrences inside
    // `mod neural` (`embed_multi` and `GteEmbedder::encode`) plus the parent's
    // own site = 3. Fewer means a call was bypassed, which is the failure this
    // law exists to catch: a green build with the budget switched off.
    let routed = code.matches("embed_input(s).to_string()").count();
    assert_eq!(
        routed, 3,
        "every embed call site must route through `embed_input` (the MAX_EMBED_CHARS budget). \
         Found {routed}; a site was bypassed, which compiles cleanly and silently removes the \
         bounded-input control."
    );
}

/// Strip `//` and `/* */` comments, string-aware — a source pin that greps raw
/// text can pass on a COMMENT naming the symbol.
///
/// The body lives in `tests/common/mod.rs`: four copies of this stripper
/// existed and two were broken. **This copy was one of the two**, and measured
/// **12 leaked comment lines** on `src/embed.rs`, the only file below it scans.
fn code_only(src: &str) -> String {
    common::code_only(src)
}
