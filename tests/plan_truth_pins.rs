//! F23 — the plan set's load-bearing citations must still resolve.
//!
//! **The finding this pins is demonstrated by the documents proposing it.** The
//! Phase G plan and prompt were written with citations; verifying them found
//! THREE already stale — including the F15 evidence sites that had been
//! corrected hours earlier in the same round, which the F15 commit then shifted
//! by 27 lines. A citation that points at the wrong code does not read as
//! broken; it reads as *correct*, and the wrong code is usually plausible.
//!
//! **This is deliberately a BOUNDED table, not a census.** Resolving every
//! `file.rs:NNN` in the plan set is F17c, declined at `3a093872`: it cannot
//! mechanically separate a load-bearing citation from a narrative one, and a
//! rule whose exceptions live in prose is the R123 rule-3 mistake again. So the
//! set here is the handful of sites a CLOSED finding names as its evidence —
//! the citations a reader would act on — and the pin's job is to say which ones
//! went stale so the correction is a re-derivation rather than a guess.
//!
//! **Cross-repo, with the two-door rule.** The plan set is private IP. On a
//! public runner it does not exist, and a pin that silently passed there would
//! be the F18 shape one layer up — so this follows
//! `loop_taxonomy_pins.rs`: sibling absent is a documented skip, sibling present
//! is a real assertion that panics on a moved artifact.
//!
//! What this does NOT do: re-audit the loops, or verify the plans' prose claims.
//! It checks that a line number still holds the construct it was cited for.

use std::path::Path;

/// The bounded load-bearing set: (what a finding depends on, file, line, the
/// construct that must be ON that line).
///
/// Re-derive with `sed -n '<line>p' <file>` — never from memory. A line that
/// moved is this pin's whole job.
const ANCHORS: &[(&str, &str, usize, &str)] = &[
    (
        "F15 — the main phase exchange",
        "src/workflow/gdl.rs",
        3371,
        "run_turns_owned",
    ),
    (
        "F15 — the adversarial re-check delegation",
        "src/workflow/gdl.rs",
        3126,
        "delegate_owned_budgeted",
    ),
    (
        "F15 — the second-verification confirmation",
        "src/workflow/gdl.rs",
        3671,
        "run_turns_owned",
    ),
    (
        "F8 — accept requires a SERVER hit",
        "src/workflow/gdl.rs",
        1255,
        "no server knowledge hits",
    ),
    (
        "F8 — a cited id must be one the server returned",
        "src/workflow/gdl.rs",
        1272,
        "unknown knowledge id",
    ),
    (
        "F9 — locator resolution",
        "src/workflow/gdl.rs",
        1597,
        "fn locator_resolves",
    ),
    (
        "F11 — the child reservation seam",
        "src/agentloop/subagents.rs",
        333,
        "fn reserve_child",
    ),
    (
        "LL-01/T2 — the header's NOT-do list admits the one re-check child by name",
        "src/workflow/gdl.rs",
        33,
        "ADVERSARIAL_RECHECK",
    ),
];

/// True when the private plan checkout exists at all. Absent on a public CI
/// runner, which is the case this rule exists for.
fn plans_present() -> bool {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../brain-steward-ip/plans/brain-shell")
        .is_dir()
}

#[test]
fn the_plan_sets_load_bearing_citations_still_resolve() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    // 1 — every anchor still holds the construct it is cited for.
    let mut moved: Vec<String> = Vec::new();
    for (label, file, line, needle) in ANCHORS {
        let path = root.join(file);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file} must exist: {e}"));
        let actual = text
            .lines()
            .nth(line - 1)
            .unwrap_or_else(|| panic!("{file} has no line {line} — it shrank?"));
        if !actual.contains(needle) {
            moved.push(format!(
                "{label}: {file}:{line} was cited for `{needle}` and now reads {:?}",
                actual.trim()
            ));
        }
    }
    assert!(
        moved.is_empty(),
        "{} of {} load-bearing plan citations no longer point where they say. A plan \
         describing a mechanism the tree does not contain is the defect Phase F existed \
         to remove, one layer up. Re-derive each with `sed -n '<line>p' <file>` — do not \
         widen the needle to make this pass, because a plausible wrong line is the \
         failure mode:\n  - {}",
        moved.len(),
        ANCHORS.len(),
        moved.join("\n  - ")
    );

    // 2 — the plan set, when present, cites the same lines this table pins. This
    // is the claim that makes the table more than an internal consistency check:
    // if the plan moves a citation and this table does not, the drift is visible
    // rather than silent.
    if !plans_present() {
        eprintln!(
            "F23: private plan checkout absent (public runner) — the ANCHORS table is \
             still checked; the plan-side cross-check is skipped by the two-door rule."
        );
        return;
    }
    let plan = std::fs::read_to_string(
        root.join("../brain-steward-ip/plans/brain-shell/11_LOOP_FINDINGS_LEDGER_2026-10-16.md"),
    )
    .unwrap_or_else(|e| panic!("the ledger must exist once the sibling does: {e}"));

    // The ledger's F15 row names its three evidence sites. They must agree with
    // the table above, or one of the two has been re-derived without the other.
    for (_, file, line, _) in ANCHORS.iter().filter(|(l, ..)| l.starts_with("F15")) {
        let citation = format!("`:{line}`");
        assert!(
            plan.contains(&citation),
            "the ledger's F15 row must cite {citation} for {file} — the table and the \
             plan disagree about where the evidence lives, which is exactly the drift \
             this pin exists to catch. Re-derive BOTH in one commit."
        );
    }
}

/// LL-01 (doc truth pass, 2026-10-28): the corrected plan sentences must stay
/// corrected. Each assertion is two-door (the private plan checkout, like the
/// cross-check above) and each carries both halves — the LIE must be absent and
/// the TRUTH must be present — because a doc that simply drops the false claim
/// without stating the real machine would satisfy a one-sided pin while leaving
/// the reader with no working description. The lies this pins against are the
/// ones LL-01 removed: the Verify→Hypothesize edge the forward-only machine
/// does not have, the per-case weights/threshold pinning the GDL chain does not
/// do, and the roadmap header's "1.29–1.31 unshipped".
#[test]
fn the_loop_learning_doc_truth_corrections_hold() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if !plans_present() {
        eprintln!(
            "LL-01: private plan checkout absent (public runner) — the doc-truth \
             corrections are skipped by the two-door rule."
        );
        return;
    }

    let read = |rel: &str| -> String {
        let path = root.join("../brain-steward-ip").join(rel);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{rel} must exist once the sibling does: {e}"))
    };
    // Whitespace-collapsed so a markdown reflow cannot pass or fail a phrase.
    let flat = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");

    // T1 — the assessment draws the machine that exists: a gate-fail retries
    // the SAME phase, attempt-3 routes to a human, and a law-clean pass:false
    // is the TERMINAL VerifyFailed. No V→H edge, and the correction is stated.
    // (`&lt;` is the mermaid file's own `<` escape; a reintroduced edge matches
    // under either spelling. The LIE-half is scoped to the diagram itself,
    // because the correction's own blockquote QUOTES the removed edge — a
    // whole-document check would fail on the correction, row 166's lesson in a
    // second key: scope a phrase check to where the claim lives.)
    let assessment = read("plans/ASSESSMENT_SIX_LOOPS_2026-10-18.md");
    let section = assessment
        .split("### 2.3")
        .nth(1)
        .unwrap_or_else(|| panic!("the assessment must keep its `### 2.3` section"));
    let diagram: String = section
        .split("```mermaid")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap_or_else(|| panic!("the 2.3 section must keep its mermaid diagram"))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !diagram.contains("fails, attempts < 3| H")
            && !diagram.contains("fails, attempts &lt; 3| H"),
        "the assessment's GDL diagram reintroduced the Verify→Hypothesize edge — \
         the machine is forward-only (gdl_checkpoint.rs advance: phases += 1 on \
         pass, next.attempt = change.attempt on fail); iteration within a case \
         is LL-07's preregistered work, not a drawn edge"
    );
    assert!(
        diagram.contains("gate-fail, attempt &lt; 3| V"),
        "the assessment's GDL diagram must draw the real gate-fail path — the \
         SAME-phase retry — not a backward edge"
    );
    let flat_assessment = flat(&assessment);
    assert!(
        flat_assessment.contains("There is no Verify→Hypothesize edge at HEAD"),
        "the assessment must keep the explicit no-backward-edge sentence under \
         §2.3 — the sentence is what tells a reader the iteration question is \
         LL-07's preregistered work, not a drawn edge"
    );

    // T3 — the spec addendum claims only the pinning the chain actually does,
    // and names the owed item.
    let addendum = flat(&read(
        "plans/SPEC_ADDENDUM_GDL_DIAGNOSTIC_GAPS_2026-09-21.md",
    ));
    assert!(
        !addendum.contains(
            "pinned per-case (model version + weights digest + thresholds) on the audit chain"
        ),
        "the spec addendum reintroduced the unqualified per-case pinning claim — \
         the chain pins policy_identity, per-exchange usage, and eval-row prompt \
         hashes; the weights digest and sampling thresholds are OWED (LL-05), and \
         the checkpoint's own comment reads \"validate shape, not a pinned digest\""
    );
    assert!(
        addendum.contains("Not yet pinned per case:") && addendum.contains("owed under LL-05"),
        "the spec addendum must carry the measured-truth sentence with the owed \
         item named"
    );

    // T4 — the roadmap's lanes are lane names: the stale unshipped header is
    // gone, the column is Lane, and the Lane-NN scheme with its mapping is
    // stated.
    let roadmap = flat(&read("plans/ROADMAP_1.30_to_2.0_Loop.md"));
    assert!(
        !roadmap.contains("1.29–1.31 unshipped") && !roadmap.contains("| Version · Name |"),
        "the roadmap reintroduced the stale \"1.29–1.31 unshipped\" chain header or \
         the \"Version · Name\" column — 1.29.x shipped (crate 1.29.x, tags prove \
         the series) and the lane labels are names, not versions"
    );
    assert!(
        roadmap.contains("| Lane · Name |")
            && roadmap.contains("Lane-00 · Loop-core")
            && roadmap.contains("1.32.N → Lane-0N")
            && roadmap.contains("unrelated to schema 1.32.28"),
        "the roadmap must carry the Lane-NN scheme, the mechanical mapping from \
         the retired 1.32.N labels, and the schema-axis disclaimer"
    );
}
