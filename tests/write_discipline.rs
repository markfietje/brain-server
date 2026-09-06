//! The write-discipline gate (Headroom M1) — `write_paths_are_immediate`.
//!
//! The law: the IMMEDIATE transaction discipline (`WorkflowTx::begin` in
//! `src/workflow/tx.rs`, the workflow lane, the audit-chain settle) is the
//! sanctioned shape for read-modify-write transitions, and a future PR cannot
//! quietly grow raw transaction construction outside the sanctioned seams.
//!
//! Idiom: the `no_sql_in_handlers_enforced` / transport-free greps — a
//! source-tree scan with `#[cfg(test)]` regions stripped via the house
//! `.split("#[cfg(test)]")` idiom, substring needles, and an anti-vacuous
//! sanity so a scan that finds nothing fails loudly instead of smiling.
//!
//! TWO inventories, opposite ratchets:
//!   * DEFERRED (`.transaction(` + `.unchecked_transaction(`): the FROZEN
//!     inventory. Per-file ceilings measured at the Headroom open
//!     (2026-09-05); any file ABOVE its frozen count, or any UNLISTED file
//!     with a hit, fails CI with the file:line list; below-baseline progress
//!     prints a delta (the Plumb debt-lock pattern: the lock stops regrowth,
//!     it does not force pace — the burn is follow-up work).
//!   * IMMEDIATE (`.transaction_with_behavior(` + `"BEGIN IMMEDIATE"`): the
//!     FLOORED inventory. The compliant discipline may only grow.
//!
//! Honesty note (the plan's verification did not survive re-verification):
//! IMPLEMENTATION_PLAN_v1.28.59 claimed "the allowlist is empty on arrival —
//! write discipline already routes through tx.rs". Empirically false: the
//! Headroom open measured 38 deferred + 20 immediate production sites across
//! the tree. Handler-side transaction CONSTRUCTION delegating its statements
//! to service cores is the established Foundation-Line pattern (the no-SQL
//! gate counts statements, not BEGINs), so the gate ships as a ratchet, not
//! a zero. See CHANGELOG §[1.28.59].
//!
//! Known ceiling (shared with the house idiom): code hidden behind a
//! MID-FILE `#[cfg(test)]` block (between two test regions) escapes the
//! scan — the convention of trailing test regions is the fence, same as the
//! service-layer transport-free gate.

use std::path::{Path, PathBuf};

const NEEDLES_DEFERRED: [&str; 2] = [".transaction(", ".unchecked_transaction("];
const NEEDLES_IMMEDIATE: [&str; 2] = [".transaction_with_behavior(", "\"BEGIN IMMEDIATE\""];

/// Whole-file test modules: the file is included by a `#[cfg(test)] mod`
/// declaration in its parent (e.g. `src/search/tests.rs` from `search/mod.rs`),
/// so the file itself carries no attribute to split on. Each entry is fenced
/// by `cfg_test_fence_holds` below, which asserts the parent really does
/// include it under cfg(test) — if the fence moves, the gate refuses to
/// guess and fails.
const CFG_TEST_INCLUDED_FILES: [&str; 1] = ["src/search/tests.rs"];

/// The frozen DEFERRED inventory at the Headroom open (2026-09-05, v1.28.59):
/// production-only counts of `.transaction(` + `.unchecked_transaction(`
/// per file. A row may only be LOWERED in the commit that earns the burn.
const DEFERRED_BASELINE: &[(&str, usize)] = &[
    ("src/handlers/breaches.rs", 3),
    ("src/handlers/clients.rs", 5),
    ("src/handlers/compliance.rs", 1),
    ("src/handlers/consolidate.rs", 2),
    ("src/handlers/domains.rs", 1),
    ("src/handlers/forget.rs", 1),
    ("src/handlers/holds.rs", 2),
    ("src/handlers/ingest.rs", 1),
    ("src/handlers/procedure.rs", 1),
    ("src/handlers/sources.rs", 2),
    ("src/handlers/transfers.rs", 1),
    ("src/handlers/ump_ops.rs", 2),
    ("src/handlers/webhooks.rs", 1),
    ("src/handlers/workflow.rs", 1),
    ("src/migration.rs", 2),
    ("src/server/bootstrap.rs", 1),
    ("src/server/router/memory.rs", 5),
    ("src/service/domains_admin.rs", 1),
    ("src/service/dsar.rs", 1),
    ("src/service/lifecycle/purge.rs", 1),
    ("src/webhook.rs", 1),
];

/// The IMMEDIATE floor at the Headroom open (2026-09-05): the sanctioned
/// discipline across the tree (WorkflowTx::begin, the lane, the audit
/// settle, revocation rotation, the IMMEDIATE handler seams). Up-only.
const IMMEDIATE_FLOOR: usize = 20;

/// Total `#[cfg(test)]`-stripped production source must stay above this for
/// the scan to be believed (the lipstyk lesson: a guard that scans nothing
/// has found nothing).
const MIN_SCANNED_LINES: usize = 20_000;

/// Minimum file count for the walk to be believed (src/ has ~150 .rs files
/// at the open).
const MIN_SCANNED_FILES: usize = 100;

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

/// Production-only view of a source file: everything before the first
/// `#[cfg(test)]` (the house split idiom — test regions trail the file).
fn production_view(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

/// Count needle occurrences with their 1-based line numbers (over the
/// production view only).
fn needle_hits(prod: &str, needle: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for line in prod.lines() {
        offset += line.len() + 1;
        if line.contains(needle) {
            lines.push(prod[..offset.min(prod.len())].matches('\n').count());
        }
    }
    lines
}

fn count_needle(prod: &str, needle: &str) -> usize {
    prod.matches(needle).count()
}

#[test]
fn write_paths_are_immediate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = root.join("src");
    let mut files: Vec<PathBuf> = Vec::new();
    walk_rs_files(&src, &mut files);
    files.sort();
    assert!(
        files.len() >= MIN_SCANNED_FILES,
        "sanity: the walk found only {} .rs files — a scan that finds nothing \
         has found nothing (expected 100+ at the open)",
        files.len()
    );

    let mut total_prod_lines = 0usize;
    let mut total_immediate = 0usize;
    let mut deferred_failures: Vec<String> = Vec::new();
    let mut deltas: Vec<String> = Vec::new();

    for f in &files {
        let rel = f
            .strip_prefix(root)
            .unwrap_or(f)
            .to_string_lossy()
            .into_owned();
        let text = std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{rel} readable: {e}"));
        let prod = if CFG_TEST_INCLUDED_FILES.contains(&rel.as_str()) {
            String::new() // fenced whole-file test module (asserted below)
        } else {
            production_view(&text).to_string()
        };
        total_prod_lines += prod.lines().count();

        let deferred: usize = NEEDLES_DEFERRED
            .iter()
            .map(|n| count_needle(&prod, n))
            .sum();
        let immediate: usize = NEEDLES_IMMEDIATE
            .iter()
            .map(|n| count_needle(&prod, n))
            .sum();
        total_immediate += immediate;

        let baseline = DEFERRED_BASELINE
            .iter()
            .find(|(p, _)| *p == rel)
            .map(|(_, n)| *n);
        match (deferred, baseline) {
            (0, None) => {}
            (n, Some(cap)) if n <= cap => {
                if n < cap {
                    deltas.push(format!(
                        "  {rel}: {n} deferred sites vs frozen {cap} — lower the row \
                         in this commit to bank the burn"
                    ));
                }
            }
            (n, cap) => {
                // Growth on a listed file, or any hit on an unlisted one.
                let mut hits: Vec<usize> = Vec::new();
                for needle in NEEDLES_DEFERRED {
                    hits.extend(needle_hits(&prod, needle));
                }
                hits.sort_unstable();
                hits.dedup();
                let ctx = match cap {
                    Some(c) => format!("frozen ceiling {c}"),
                    None => "file not in the frozen inventory".to_string(),
                };
                deferred_failures.push(format!(
                    "  {rel}: {n} deferred transaction sites ({ctx}) at lines {:?}",
                    hits
                ));
            }
        }
    }

    // Anti-vacuous: the scan must have seen the real tree.
    assert!(
        total_prod_lines >= MIN_SCANNED_LINES,
        "sanity: only {total_prod_lines} production lines scanned — the walk or \
         the strip is broken"
    );
    // Positive control: the discipline home itself must carry the needle.
    let tx = std::fs::read_to_string(root.join("src/workflow/tx.rs"))
        .expect("src/workflow/tx.rs must exist");
    assert!(
        tx.contains(".transaction_with_behavior("),
        "positive control failed: src/workflow/tx.rs no longer matches the \
         IMMEDIATE needle — the needles or the discipline moved"
    );

    // The IMMEDIATE ratchet: the compliant discipline may only grow.
    assert!(
        total_immediate >= IMMEDIATE_FLOOR,
        "the IMMEDIATE discipline shrank: {total_immediate} sites < floor \
         {IMMEDIATE_FLOOR} — a sanctioned BEGIN site was removed; if that is a \
         real consolidation, lower the floor in the same commit"
    );

    // Progress deltas are printed, never fatal (the Plumb pattern).
    if !deltas.is_empty() {
        eprintln!("write-discipline burn progress (bank it in this commit):");
        for d in &deltas {
            eprintln!("{d}");
        }
    }

    assert!(
        deferred_failures.is_empty(),
        "write-discipline VIOLATION — raw DEFERRED transaction construction grew \
         past the frozen inventory. New read-modify-write transitions route \
         through `WorkflowTx::begin` (src/workflow/tx.rs, BEGIN IMMEDIATE); a \
         genuine new single-writer deferred seam needs a deliberate \
         baseline-row edit in the same commit:\n{}",
        deferred_failures.join("\n")
    );
}

/// The whole-file-test fence: every entry in `CFG_TEST_INCLUDED_FILES` must
/// still be included from its parent under `#[cfg(test)]`. If the file
/// becomes production code (or the include moves), this fails and the
/// deferred census must be re-measured deliberately.
#[test]
fn cfg_test_fence_holds() {
    for rel in CFG_TEST_INCLUDED_FILES {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel} must exist: {e}"));
        assert!(
            text.contains(".transaction(") || text.contains("mod tests"),
            "{rel} no longer looks like the test module it is fenced as — \
             re-measure the deferred inventory"
        );
        // The including parent must carry the cfg(test) include. For
        // `src/search/tests.rs` the parent is `src/search/mod.rs`.
        let parent = path.parent().expect("parent dir").join("mod.rs");
        let parent_text = std::fs::read_to_string(&parent)
            .unwrap_or_else(|e| panic!("{} must exist: {e}", parent.display()));
        assert!(
            parent_text.matches("#[cfg(test)]").count() > 0 && parent_text.contains("mod tests"),
            "{} must include its tests module under #[cfg(test)] — the fence \
             behind {}'s exclusion moved; re-measure the deferred inventory",
            parent.display(),
            rel
        );
    }
}

/// The stripper's own pin: production view keeps everything before the first
/// cfg(test) marker, including mid-file markers-as-data (a needle literal in
/// a doc comment still counts — substring locks are deliberately stricter).
#[test]
fn production_view_split_semantics() {
    let src = "fn a() {\n    conn.transaction();\n}\n\n#[cfg(test)]\nmod tests {\n    \
               fn b() { conn.transaction(); }\n}\n";
    let prod = production_view(src);
    assert_eq!(count_needle(prod, ".transaction("), 1);
    assert!(prod.contains("fn a()"));
    assert!(!prod.contains("fn b()"));
    // Files with no test region are fully scanned.
    assert_eq!(
        count_needle(production_view("fn c() {}"), ".transaction("),
        0
    );
}
