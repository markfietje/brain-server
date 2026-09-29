//! R51 — the taxonomy closure pins (I51.3 docs-truth, I51.5 nesting, R51.1 red-proofs).
//!
//! **What this round corrects.** Three documents describe the same architecture, and
//! before this round they disagreed with each other and with themselves. The kernel's
//! `docs/architecture.md` enumerated Solve / Evolve / Deflect / Delivery and omitted
//! `Create` and `Operate` entirely. The plan's §5.3 and the blueprint's §2.2 were
//! corrected on 2026-09-28 (`619839e`); the kernel doc on the same day (`6007e62`).
//! What was left was **a fifth place each of those fixes missed** — the plan's §2 and
//! §5.1 diagrams and the blueprint's §2.1 "one picture" still drew Operate as the fifth
//! box in a chain, and the r45-0 pin at `:990` was *keeping that contradiction alive* by
//! asserting the five-long chain string. Those are corrected here, under the operator's
//! explicit instruction to correct stale documents.
//!
//! **Why the predicate reads ENUMERATION LINES, not prose — this is load-bearing, not
//! stylistic.** The pre-fix kernel doc already mentioned `Operate` exactly once:
//!
//! ```text
//! D4 --> D5["D5 Operate<br/>observe → attribute → improve"]
//! ```
//!
//! That is `Deliver`'s **software-lifecycle phase 5**, not the knowledge `Operate`, and
//! `Create` was genuinely absent. **A bare name-mention predicate would have false-passed
//! on the very file whose gap this pin closes.** So the predicate below extracts loop
//! names ONLY from enumeration positions — markdown table rows, diagram box rows, and
//! mermaid `subgraph`/node labels — and never from running prose. A document that
//! *mentions* a loop has not thereby *enumerated* it.
//!
//! **Red-first.** The operator's doc fixes are already committed, so the trigger states
//! exist only in git history. The red substrate is therefore committed as fixtures FIRST
//! (`tests/fixtures/`, preregistered as P51.3) and the red-proofs run against those. The
//! fixture-based tests are therefore green against the live tree **by design** — their
//! red-proof is the fixture run, pasted in the evidence file, and the fixture must keep
//! failing to be worth anything.
//!
//! **Cross-repo loader.** Follows the r45-0 precedent (`tests/r45_0_claim_pins.rs:36-42`)
//! and **panics loudly** when the private checkout is absent: a public-only checkout must
//! not silently pass pins whose whole point is cross-document agreement.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The six loops, as named by the authoritative plan
/// `plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md` (Status: AGREED).
const SIX_LOOPS: &[&str] = &["Create", "Solve", "Evolve", "Deflect", "Operate", "Deliver"];

/// The four knowledge STAGES — walked through, in this order, as a ring.
const FOUR_STAGES: &[&str] = &["Create", "Solve", "Evolve", "Deflect"];

fn spine(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../brain-steward-ip")
        .join(rel)
}

fn read_spine(rel: &str) -> String {
    let path = spine(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "the private spine artifact must exist at {}: {e}. R51's taxonomy pins are \
             machine-enforced by this file; a kernel-only checkout that cannot see the \
             private artifacts would otherwise pass vacuously.",
            path.display()
        )
    })
}

fn read_kernel(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} must exist: {e}", path.display()))
}

fn read_fixture(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "the P51.3 fixture must exist at {}: {e}. These are the pre-change document \
             states, extracted from git history; without them the red-proofs cannot run \
             and the green fixture-based pins below prove nothing.",
            path.display()
        )
    })
}

// ── the enumeration-line extractor ─────────────────────────────────────────

/// True when a line is an ENUMERATION position rather than running prose.
///
/// Accepted:
/// * markdown table rows — `| **Create** | ... |`, `| Create | ... |`
/// * ascii diagram box rows — `│  │ Create  │───▶│ ... │`
/// * mermaid `subgraph X["LABEL"]` headers and node labels `Z1["LABEL"]`
/// * fenced-diagram list rows — `  Create ──▶ Solve ...` (indented, in a fence)
///
/// Rejected: everything else, which is where a passing *mention* lives.
fn is_enumeration_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    // markdown table row
    if t.starts_with('|') {
        return true;
    }
    // ascii diagram box row (the plan's §2 / §5.3 use box-drawing)
    if t.contains('│') || t.contains('┌') || t.contains('└') {
        return true;
    }
    // mermaid `subgraph X["LABEL"]` headers. NOTE: bare node labels (`Z1["…"]`) are
    // deliberately NOT accepted — the kernel doc's software loop is a mermaid graph
    // whose `D5["D5 Operate"]` node is the *software lifecycle's phase 5*, not an
    // enumeration of the knowledge `Operate`. Counting node labels is exactly the
    // false-pass this predicate exists to prevent.
    if t.starts_with("subgraph ") {
        return true;
    }
    false
}

/// Extract the loop names a document ENUMERATES (never merely mentions).
///
/// Deliberately narrow: a name counts only on an enumeration line, and only when it
/// appears as a whole word. The point is that a document which merely *talks about*
/// Operate — as `D5 Operate` does — does not thereby enumerate it.
fn enumerated_loops(text: &str) -> BTreeSet<&'static str> {
    let mut found = BTreeSet::new();
    for line in text.lines() {
        if !is_enumeration_line(line) {
            continue;
        }
        for name in SIX_LOOPS {
            // whole-word match, so `Delivery` never counts as `Deliver`
            let mut from = 0usize;
            while let Some(rel) = line[from..].find(name) {
                let start = from + rel;
                let end = start + name.len();
                let before_ok = start == 0
                    || !line[..start]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric());
                let after_ok = end >= line.len()
                    || !line[end..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_alphanumeric());
                if before_ok && after_ok {
                    found.insert(*name);
                    break;
                }
                from = start + 1;
            }
        }
    }
    found
}

fn expect_all_six(label: &str, doc: &str, set: &BTreeSet<&'static str>) {
    let missing: Vec<&&str> = SIX_LOOPS.iter().filter(|n| !set.contains(*n)).collect();
    assert!(
        missing.is_empty(),
        "{label} ({doc}) does not ENUMERATE loop(s) {missing:?}. All six must appear in an \
         enumeration position — a table row, a diagram box, or a mermaid label. A loop that is \
         only *mentioned* in prose has not been enumerated, and a document that enumerates \
         five of six is worse than one that enumerates none: the reader trusts the set."
    );
}

// ── I51.3 · the loop SET agrees across all three documents ─────────────────

/// **I51.3 — the three documents enumerate the SAME loop set.** Load-bearing, not
/// stylistic: no existing pin read `docs/architecture.md` at all, and a guard that never
/// opens the third document cannot fail when the third document drifts.
#[test]
fn r51_all_three_documents_enumerate_the_same_six_loops() {
    let kernel = read_kernel("docs/architecture.md");
    let plan = read_spine("plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md");
    let blueprint = read_spine("docs/blueprint/02-SYSTEM_ARCHITECTURE.md");

    let sets = [
        ("docs/architecture.md", enumerated_loops(&kernel)),
        (
            "plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md",
            enumerated_loops(&plan),
        ),
        (
            "docs/blueprint/02-SYSTEM_ARCHITECTURE.md",
            enumerated_loops(&blueprint),
        ),
    ];

    for (doc, set) in &sets {
        expect_all_six(doc, doc, set);
    }

    // Set EQUALITY, not just completeness. A document that invents a seventh loop is
    // as wrong as one missing a sixth.
    let authoritative = &sets[1].1;
    for (doc, set) in &sets {
        assert_eq!(
            set, authoritative,
            "{doc} enumerates a different loop set than the authoritative plan. The plan is \
             normative (Status: AGREED); the others summarise it and must not diverge."
        );
    }
}

/// The non-vacuity twin. The extractor must be able to return a SHORT set, or the
/// equality pin above could pass because everything matched.
#[test]
fn r51_loop_set_extractor_is_non_vacuous() {
    let plan = read_spine("plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md");
    let full = enumerated_loops(&plan);
    assert_eq!(full.len(), SIX_LOOPS.len(), "the plan enumerates all six");

    // Prose-only text enumerates nothing — this is the property that makes the
    // pre-fix kernel doc (which merely MENTIONED Operate as D5) come out short.
    let prose = "Troubleshooting here is not one process. Solve finds answers, \
                 Evolve publishes them, and Deliver ships the software. \
                 Operate is the D5 software phase, not a knowledge stage.";
    assert!(
        enumerated_loops(prose).is_empty(),
        "prose must never count as enumeration — a passing mention must not be able to \
         fake a complete set. Extracted: {:?}",
        enumerated_loops(prose)
    );
}

// ── R51.1(a) · set divergence, red-proofed against the FIXTURE ─────────────

/// **R51.1(a) — the red-proof.** Against the PRE-FIX kernel doc, the loop-set
/// predicate must report `Create` and `Operate` missing. This is the substrate the pin
/// above cannot provide on its own, because the operator already fixed the live file.
#[test]
fn r51_redproof_prefix_kernel_doc_is_missing_create_and_operate() {
    let prefix = read_fixture("prefix_kernel_architecture.md");
    let set = enumerated_loops(&prefix);

    // The pre-fix doc genuinely lacked these two.
    assert!(
        !set.contains("Create"),
        "the pre-fix kernel doc must NOT enumerate Create — that absence is the red-proof \
         substrate. If this fixture now enumerates Create, the fixture is stale and the \
         red-proof below proves nothing. Extracted: {set:?}"
    );
    assert!(
        !set.contains("Operate"),
        "the pre-fix kernel doc must NOT enumerate the knowledge Operate — it carried only \
         `D5 Operate`, the SOFTWARE lifecycle's phase 5. That near-miss is exactly why the \
         predicate must read enumeration lines and not mentions. Extracted: {set:?}"
    );

    // And the live doc has them — the fix is real, not asserted.
    let live = read_kernel("docs/architecture.md");
    let live_set = enumerated_loops(&live);
    assert!(
        live_set.contains("Create") && live_set.contains("Operate"),
        "the LIVE docs/architecture.md must enumerate both Create and Operate after the \
         2026-09-28 fix. Extracted: {live_set:?}"
    );
}

// ── R51.1(b) · chain red-proof: Operate is NOT chain-position 5 ────────────

/// True when the text draws Operate as the terminal box of the knowledge chain —
/// i.e. `Deflect` immediately followed by `Operate` in a chain row.
///
/// The two documents draw the chain in different styles, so the gap between the two
/// names is matched as "arrow, optionally interrupted by box-drawing characters":
/// * the plan (box-drawing): `Deflect │───▶│Operate`
/// * the blueprint (plain fence): `Deflect ──▶ Operate`
///
/// Matching the arrow alone would false-positive on a *feedback* edge such as
/// `Deflect → Create`, so `Operate` must be the box that immediately follows.
fn draws_operate_as_fifth_chain_box(text: &str) -> bool {
    const ARROWS: [&str; 4] = ["───▶", "──▶", "→", "▶"];
    const BOX_NOISE: [char; 5] = ['│', '|', ' ', '─', '▶'];
    text.lines().any(|line| {
        let Some(pos) = line.find("Deflect") else {
            return false;
        };
        let rest = &line[pos + "Deflect".len()..];
        // skip box-drawing noise until an arrow begins
        let arrow_start = rest.find(|c: char| ARROWS.iter().any(|a| a.starts_with(c)));
        let Some(arrow_start) = arrow_start else {
            return false;
        };
        let arrow = &rest[arrow_start..];
        if !ARROWS.iter().any(|a| arrow.starts_with(a)) {
            return false;
        }
        // after the arrow, allow box noise, then require Operate
        let after = &arrow[ARROWS
            .iter()
            .find(|a| arrow.starts_with(**a))
            .map_or(0, |a| a.len())..];
        let after = after.trim_start_matches(|c: char| BOX_NOISE.contains(&c));
        after.starts_with("Operate")
    })
}

/// **R51.1(b) — the chain red-proof.** Neither the plan nor the blueprint may present
/// Operate as chain-position 5. This is the half P51.1 pins: the *relationship* was what
/// was wrong, not the taxonomy.
#[test]
fn r51_redproof_prefix_docs_drew_operate_as_the_fifth_chain_box() {
    for (label, fixture) in [
        (
            "plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md",
            "prefix_plan_six_loops.md",
        ),
        (
            "docs/blueprint/02-SYSTEM_ARCHITECTURE.md",
            "prefix_blueprint_system_architecture.md",
        ),
    ] {
        let prefix = read_fixture(fixture);
        assert!(
            draws_operate_as_fifth_chain_box(&prefix),
            "{label} (pre-fix fixture) must draw Operate as the fifth box in the chain — \
             that mis-drawing is the red-proof substrate. If this fixture no longer does, \
             the fixture is stale."
        );
    }

    // And neither LIVE document does any more.
    let plan = read_spine("plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md");
    let blueprint = read_spine("docs/blueprint/02-SYSTEM_ARCHITECTURE.md");
    for (label, text) in [
        ("plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md", &plan),
        ("docs/blueprint/02-SYSTEM_ARCHITECTURE.md", &blueprint),
    ] {
        assert!(
            !draws_operate_as_fifth_chain_box(text),
            "{label} still draws Operate as the fifth box in the knowledge chain. Operate is \
             the RETURN PATH: four stages turn in a ring (Create → Solve → Evolve → Deflect) \
             and Operate is what closes it. A stage is either walked through or it is the \
             thing that closes the walk — never both."
        );
    }
}

// ── I51.5 · the four timescales appear in all three documents ─────────────

/// The four nesting levels and their cadences, pinned verbatim (P51.1 wording).
const CADENCES: &[(&str, &str)] = &[
    ("Business", "days"),
    ("Feedback", "continuous"),
    ("Operational", "minutes"),
    ("Execution", "seconds"),
];

/// **I51.5 — the four-level nesting diagram with its cadences appears in ALL THREE
/// documents.** A reader must never have to guess which timescale a statement is about.
#[test]
fn r51_four_timescales_appear_in_all_three_documents() {
    let docs = [
        ("docs/architecture.md", read_kernel("docs/architecture.md")),
        (
            "plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md",
            read_spine("plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md"),
        ),
        (
            "docs/blueprint/02-SYSTEM_ARCHITECTURE.md",
            read_spine("docs/blueprint/02-SYSTEM_ARCHITECTURE.md"),
        ),
    ];

    for (label, text) in &docs {
        for (level, cadence) in CADENCES {
            assert!(
                text.contains(level),
                "{label} is missing the `{level}` nesting level. All four levels (business, \
                 feedback, operational, execution) must be named, with their cadences, in \
                 every document that describes the architecture — otherwise a reader has to \
                 guess which timescale a statement is about."
            );
            assert!(
                text.contains(cadence),
                "{label} names the `{level}` level but not its cadence (`{cadence}`). A \
                 cadence quoted without its level is not a measurement."
            );
        }
    }
}

/// The four STAGES appear in ring order in all three documents (I51.1/I51.2 shape).
#[test]
fn r51_four_stages_appear_in_ring_order_in_all_three_documents() {
    let docs = [
        ("docs/architecture.md", read_kernel("docs/architecture.md")),
        (
            "plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md",
            read_spine("plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md"),
        ),
        (
            "docs/blueprint/02-SYSTEM_ARCHITECTURE.md",
            read_spine("docs/blueprint/02-SYSTEM_ARCHITECTURE.md"),
        ),
    ];
    for (label, text) in &docs {
        // Ring order in EITHER drawing style: the arrow form the plan/blueprint use,
        // or the mermaid stage boxes the kernel doc uses (LOOP 0/1/2/3 subgraphs).
        let arrow_form = FOUR_STAGES.join(" ──▶ ");
        let arrow_form_alt = FOUR_STAGES.join(" → ");
        let mermaid_form = text.contains("subgraph L0[\"CREATE")
            && text.contains("subgraph L1[\"LOOP 1 · SOLVE")
            && text.contains("subgraph L2[\"LOOP 2 · EVOLVE")
            && text.contains("subgraph L3[\"LOOP 3 · DEFLECT");
        assert!(
            text.contains(&arrow_form) || text.contains(&arrow_form_alt) || mermaid_form,
            "{label} must show the four knowledge stages in ring order \
             (Create → Solve → Evolve → Deflect) — as an arrow chain or as the ordered \
             LOOP 0..3 stage boxes. Deliver is the separate software loop, not a fifth \
             stage in this ring."
        );
    }
}
