//! The subject-matter pool: the vocabulary is DEPLOYMENT-SUPPLIED, and this
//! tree must carry no engagement name.
//!
//! WHY A SEPARATE FILE. `subject_matter_terms()` caches in a `OnceLock`, so the
//! two properties below cannot be proven in one process — the first read of the
//! variable wins for the life of the process. Each therefore runs in its own
//! CHILD process with the variable set differently, and `CARGO_BIN_EXE_*` is
//! only defined for integration tests, which is why these are here and not in
//! `procedural.rs`'s own test module.
//!
//! The probe binary exists so a child has something to run; `brain classify`
//! talks to a running server, which a unit test must not require.

use std::process::Command;

const PROBE: &str = env!("CARGO_BIN_EXE_classify-probe");

/// The hardware-estate text used by both tests. Deliberately names product
/// lines rather than a vendor, so this file adds nothing to the engagement-name
/// surface it is protecting.
const HARDWARE_CASE: &str =
    "PowerEdge R760 is down, the idrac is unresponsive and the perc shows a failed VD";

#[test]
fn the_subject_matter_pool_fires_when_terms_are_bound() {
    // A category nothing emits is a category that routes nowhere, which is what
    // the routing-axis drift pin exists to prevent — so the variant needs a
    // positive proof, not just the pin.
    let out = Command::new(PROBE)
        .env("BRAIN_SUBJECT_MATTER_TERMS", "idrac,poweredge,perc")
        .arg(HARDWARE_CASE)
        .output()
        .expect("child classify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(SUBJECT_MATTER),
        "a case naming a bound hardware line must reach the subject-matter pool. Got {stdout:?}"
    );
    assert!(
        stdout.contains("idrac") || stdout.contains("poweredge"),
        "the matched keywords must name the terms the deployment bound. Got {stdout:?}"
    );
}

#[test]
fn the_subject_matter_pool_is_inert_until_terms_are_bound() {
    // THE PIN THAT KEEPS THE PUBLIC TREE HONEST. Unset terms must score zero,
    // never fall back to a built-in list: a lexicon that quietly refilled itself
    // from a hard-coded array is exactly what the deployment-supplied split
    // exists to prevent.
    let out = Command::new(PROBE)
        .env_remove("BRAIN_SUBJECT_MATTER_TERMS")
        .arg(HARDWARE_CASE)
        .output()
        .expect("child classify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(SUBJECT_MATTER),
        "with no terms bound the pool must score zero, not fall back to a built-in \
         vocabulary. Got {stdout:?}"
    );
}

#[test]
fn the_deployment_terms_reach_the_matched_keywords() {
    // The scoring path and the matched-keyword path must agree about which
    // terms were in force — they read `effective_lexicon`, and this is what
    // would catch one of them drifting back to the bare `LEXICON`.
    let out = Command::new(PROBE)
        .env("BRAIN_SUBJECT_MATTER_TERMS", "flangewidget")
        .arg("the flangewidget reports a fault")
        .output()
        .expect("child classify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("flangewidget"),
        "a bound term that fires must appear in matched_keywords. Got {stdout:?}"
    );
}

const SUBJECT_MATTER: &str = "subject_matter";
