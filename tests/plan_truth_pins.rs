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
        3366,
        "run_turns_owned",
    ),
    (
        "F15 — the adversarial re-check delegation",
        "src/workflow/gdl.rs",
        3121,
        "delegate_owned_budgeted",
    ),
    (
        "F15 — the second-verification confirmation",
        "src/workflow/gdl.rs",
        3666,
        "run_turns_owned",
    ),
    (
        "F8 — accept requires a SERVER hit",
        "src/workflow/gdl.rs",
        1250,
        "no server knowledge hits",
    ),
    (
        "F8 — a cited id must be one the server returned",
        "src/workflow/gdl.rs",
        1267,
        "unknown knowledge id",
    ),
    (
        "F9 — locator resolution",
        "src/workflow/gdl.rs",
        1592,
        "fn locator_resolves",
    ),
    (
        "F11 — the child reservation seam",
        "src/agentloop/subagents.rs",
        333,
        "fn reserve_child",
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
