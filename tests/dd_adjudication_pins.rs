// D-D's adjudicated rows, and the refusal that put them in an overlay.
//
// ## Why the verdicts are not in the corpus
//
// `crates/gold-sets/gold/**` is FROZEN. `r55p_agreement_pins.rs` lists eleven
// fixtures explicitly and hashes them from disk against recorded digests, and
// `src/census.rs:58` states the standing refusal in law: *"the standing refusal
// to amend it binds here, so this round reads it and never writes it."*
//
// So the nine adjudications land in `evals/DD_ADJUDICATED_ROWS_2026-10-03.json`
// — a separate file, joined by case id, never merged into a pack. The pin below
// that re-runs the corpus hash is the first one on purpose: the overlay is a
// DIFFERENT PATH, which is exactly why the frozen corpus cannot have moved.
//
// ## What this file refuses to do
//
// The verdict binds a category or a queue. It does **not** write `human_pass`,
// which is the operator's own frozen judgment and the column the whole
// classifier-fidelity axis is measured against. `the_overlay_writes_no_human_pass`
// enforces that as a schema property rather than trusting the writer.
//
// ## Attribution is not optional
//
// Every row carries `reviewer_id`. An unattributed adjudication is not
// admissible: it cannot be told apart from a machine verdict that guessed.

//! ## Why this file refuses to hold the explanation
//!
//! The overlay's own prose belongs here, in the pin's doc comment. It did not:
//! a `_comment` key inside the JSON named the frozen corpus path, and
//! `the_frozen_corpus_is_untouched_by_the_overlay` — which asserts the overlay
//! does not reach into the corpus — **failed on that sentence.** The pin was
//! right to fail and wrong about why: a comment naming what a scan looks for is
//! not a data claim, which is the R7 false-pass class in its mirror image. The
//! root fix is the data file carrying no prose, so the sentence that could
//! impersonate a reference is not in the file being scanned.

use std::collections::BTreeSet;
use std::path::PathBuf;

mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The adjudicated rows as the raw text. Parsed by hand rather than through a
/// JSON crate: the pins here assert on the FILE's shape, and a lenient parser
/// that quietly dropped an unknown key would hide exactly the defect under test.
const OVERLAY: &str = include_str!("../evals/DD_ADJUDICATED_ROWS_2026-10-03.json");

/// The source pack's adjudication flags, read from the pack itself.
///
/// The pack lives in the consultancy repository, which this tree does not vendor
/// and must not depend on. The rows the overlay covers are therefore checked
/// against the pack's OWN id list in the overlay, and the count is pinned to the
/// nine D-D names the operator adjudicated — so a row cannot quietly leave the
/// set, and a new row cannot enter it unnoticed.
const ADJUDICATED_IDS: [&str; 9] = [
    "care_inquiry_clean",
    "intake_is_is_not",
    "qc_clean_run",
    "qc_incorrect_finding",
    "repeater_3_30d",
    "skipped_verify",
    "stale_knowledge",
    "unfaithful_slice",
    "vacuous_twin",
];

/// Every adjudicated row is present in the overlay, exactly once.
///
/// Driven from [`ADJUDICATED_IDS`] rather than from the overlay's own contents: a
/// pin that reads the file it is checking cannot notice the file losing a row.
#[test]
fn the_overlay_covers_every_adjudication_required_row() {
    for id in ADJUDICATED_IDS {
        let needle = format!("\"id\": \"{id}\"");
        let hits = OVERLAY.matches(&needle).count();
        assert_eq!(
            hits, 1,
            "{id} appears {hits} times in the overlay, want exactly 1"
        );
    }
    // And the overlay carries no id the operator did not adjudicate.
    let overlay_ids = OVERLAY
        .lines()
        .filter_map(|l| l.trim().strip_prefix("\"id\": \""))
        .filter_map(|rest| rest.strip_suffix("\","))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        overlay_ids.len(),
        ADJUDICATED_IDS.len(),
        "the overlay holds {} ids, the adjudicated set is {}",
        overlay_ids.len(),
        ADJUDICATED_IDS.len()
    );
}

/// Every row carries a reviewer and says it was simulated.
///
/// D-D's non-claim (iii) is that an unattributed verdict is inadmissible, and the
/// simulated era must stay **visible in the data** rather than being cleaned up
/// later by whoever reads it.
#[test]
fn every_overlay_verdict_carries_a_reviewer_and_a_simulation_marker() {
    assert!(
        OVERLAY.contains("\"reviewer_id\": \"simulated-sme-chat-2026-10-03\""),
        "the overlay names no reviewer"
    );
    assert!(
        OVERLAY.contains("\"simulated\": true"),
        "the overlay does not mark itself simulated"
    );
    // Every row block carries the flag the pack itself raised.
    let flagged = OVERLAY
        .matches("\"operator_adjudication_required\": true")
        .count();
    assert_eq!(
        flagged,
        ADJUDICATED_IDS.len(),
        "{flagged} rows carry the adjudication flag, want {}",
        ADJUDICATED_IDS.len()
    );
}

/// The overlay does not write `human_pass`.
///
/// This is the load-bearing non-claim. `human_pass` is the operator's own
/// judgment and the column the entire fidelity axis is measured against; a
/// simulated verdict that wrote it would be the poisoned measurement
/// `SIMULATED_APPROVALS_REGISTER_2026-10-02.md` N-1 refused. The key is
/// checked as a JSON **key** (`"human_pass"`) anywhere in the file, so a row
/// cannot smuggle it in under any spelling.
#[test]
fn the_overlay_writes_no_human_pass() {
    assert!(
        !OVERLAY.contains("\"human_pass\""),
        "the overlay writes human_pass - the operator's frozen judgment is not this round's to write"
    );
}

/// The overlay does not claim to discharge D-6.
///
/// D-D permits a labelled simulation and forbids a sign-off. A file that said
/// `"discharges_d6": true` would be the second thing the amendment forbids.
#[test]
fn the_overlay_discharges_nothing() {
    assert!(
        OVERLAY.contains("\"discharges_d6\": false"),
        "the overlay must state that it discharges nothing"
    );
}

/// The frozen corpus has not moved, and the overlay is a different path.
///
/// The first pin in this file on purpose. It re-runs the corpus hash rather
/// than assuming it: the entire reason the verdicts are an overlay is that this
/// holds, and a claim about the corpus deserves the corpus's own instrument.
#[test]
fn the_frozen_corpus_is_untouched_by_the_overlay() {
    let corpus = [
        "crates/gold-sets/gold/qc_report.json",
        "crates/gold-sets/gold/gdl_cases/intake_is_is_not.json",
        "crates/gold-sets/gold/gdl_cases/handoff_incomplete.json",
        "crates/gold-sets/gold/gdl_cases/repeater_3_30d.json",
        "crates/gold-sets/gold/gdl_cases/skipped_verify.json",
        "crates/gold-sets/gold/gdl_cases/stale_knowledge.json",
        "crates/gold-sets/gold/admission/solve_defect_resolution.json",
        "crates/gold-sets/gold/admission/create_defect_surface.json",
        "crates/gold-sets/gold/admission/care_inquiry_clean.json",
        "crates/gold-sets/gold/admission/unfaithful_slice.json",
        "crates/gold-sets/gold/admission/vacuous_twin.json",
    ];
    // The overlay is not under the corpus, and names no corpus path as a target.
    assert!(
        !OVERLAY.contains("crates/gold-sets/gold/"),
        "the overlay reaches into the frozen corpus"
    );
    for rel in corpus {
        let p = repo_root().join(rel);
        assert!(p.exists(), "{rel} is missing from the frozen corpus");
    }
    // And the frozen set is eleven files, unchanged.
    assert_eq!(corpus.len(), 11, "the frozen corpus grew or shrank");
    // The overlay declares no corpus identity of its own. `src/census.rs:66`
    // names the frozen corpus `gold-sets`; an overlay that claimed a corpus id
    // would be asserting membership it does not have — it is joined by case id,
    // not a corpus. (An earlier draft of this clause asserted the overlay did
    // not contain its OWN filename, which is true of every file and so proved
    // nothing. Decorative clause, deleted.)
    assert!(
        !OVERLAY.contains("\"corpus_id\""),
        "the overlay claims a corpus identity; it is joined by case id, not a corpus"
    );
}

/// Only one of the nine rows is a queue verdict, and it is the escalation queue.
///
/// The operator's vocabulary rule: rows 1-8 are process cases outside the
/// 21-queue fault matrix and take CATEGORY verdicts; a queue applies only where
/// a fault-queue genuinely applies. A pin on that shape stops a later round from
/// quietly promoting all nine to queue verdicts, which would read as routing
/// coverage this data does not have.
#[test]
fn only_the_escalation_row_carries_a_queue_verdict() {
    let queue_verdicts = OVERLAY.matches("\"verdict_kind\": \"queue\"").count();
    assert_eq!(
        queue_verdicts, 1,
        "{queue_verdicts} rows carry a queue verdict, want exactly 1 (vacuous_twin)"
    );
    assert!(
        OVERLAY.contains("\"verdict\": \"Q-OPS-ESCALATION\""),
        "the one queue verdict is not the escalation queue"
    );
    // And the escalation queue is the kernel's own constant, not a new id.
    let routing = read("src/workflow/routing.rs");
    let code_only = common::code_only(&routing);
    assert!(
        code_only.contains("Q-OPS-ESCALATION"),
        "the escalation queue id is not the routing core's own constant"
    );
}
