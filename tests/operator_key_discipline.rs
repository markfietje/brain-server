//! Operator-key discipline — **a phase pass signs its attestation link, so a
//! test that advances a phase must HOLD the key it signs with.**
//!
//! **The gap this closes, and it cost a release.** The key law is the tree's
//! own: *"a phase pass signs its attestation link, and a pass with no usable
//! operator key REFUSES"* (`delivery_phase_advance_writes_step_row_cas_and_audit_atomically`).
//! The operator key is therefore AMBIENT state — `BRAIN_UMP_KEY_DIR` is one
//! process-global environment variable — and the resolver falls back to
//! `~/.config/brain-server/ump/`, where a developer machine holds a real
//! `operator.ed25519`.
//!
//! A test that calls `seed()` gets the crate-wide guard for the SEEDING and
//! not for the rest of its body. So a test that then advances a phase and
//! expects SUCCESS was resolving the key from whatever the machine happened to
//! have: green on a laptop, RED on a clean CI runner with
//! `AttestationRefused { reason: "operator_key_absent" }`.
//!
//! That is not hypothetical. It is what refused the `v1.29.25` publication:
//! the `multivec` lane failed on
//! `promote_refuses_when_the_run_moved_since_the_approval`, and the release
//! workflow's fail-closed gate (correctly) published nothing.
//!
//! **The convention already existed and nothing policed it.** `delivery.rs`
//! states it in prose — *"every CALLER must hold `operator_key_guard()`"* —
//! and the sibling census in `releases.rs`
//! (`no_test_holds_the_operator_key_guard_and_then_seeds`) polices the
//! OPPOSITE hazard (holding the guard and then calling `seed()`, which
//! deadlocks the thread against itself). Between them, the hazard that
//! actually broke a release was unpoliced. This is that pin.
//!
//! **What it asserts.** For every phase-advance call site in `src/workflow/`:
//!
//! 1. the enclosing function HOLDS the operator key (it takes
//!    `operator_key_guard()`, or it seeds through `seed_locked`), OR
//! 2. the call EXPECTS a refusal (`expect_err` / `unwrap_err` — the arms
//!    that pin the refusal are allowed to be keyless), OR
//! 3. the enclosing function is a HELPER, and every TEST that calls it holds
//!    the key (the convention `delivery.rs` states).
//!
//! A test that satisfies none of the three is the failure this exists to
//! catch: it is green on the developer's machine and red on the runner.
//!
//! The scan reads source, so it is STRUCTURAL — it cannot prove a phase pass
//! really signed, and it does not try to. What it proves is that no test
//! depends on a key it does not own, which is the whole defect.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The workflow modules that can advance a phase. Scanned as a DIRECTORY, so
/// a new module that advances one is covered without editing this list — a
/// checker that only knows the files it was written against is a checker that
/// goes blind exactly where a new file appears.
const SCAN_DIR: &str = "src/workflow";

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    let p = repo(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

/// Strip COMMENTS from a source block.
///
/// This is not tidiness — it is the difference between a pin and a decoration.
/// The first draft of this scan read the body RAW, and the doc comment it
/// was written next to (`the seed runs through \`seed_locked\``) satisfied
/// the "holds the key" check on its own. The drill planted the exact defect
/// the pin exists to catch and the pin stayed GREEN: a scan that reads prose
/// about a guard is a scan that reads prose.
///
/// So: full-line comments are dropped, and a trailing `// ...` is cut. A
/// `//` inside a string literal would truncate that line, which can only
/// shorten a body — it cannot invent a guard, and it cannot hide a call site
/// that is not there.
fn strip_comments(block: &str) -> String {
    let mut out = String::new();
    for line in block.lines() {
        let t = line.trim_start();
        if t.starts_with("//") {
            continue;
        }
        let cut = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        out.push_str(cut);
        out.push('\n');
    }
    out
}

/// One parsed function, in the test-module shape (a `fn` at indent 4).
struct Fn {
    name: String,
    file: String,
    /// 1-based line of the `fn` declaration.
    line: usize,
    body: String,
    is_test: bool,
    holds_key: bool,
}

impl Fn {
    /// The phase-advance calls this function makes, with the statement each
    /// one belongs to (the call plus the lines that finish it).
    fn advances(&self) -> Vec<(usize, String)> {
        let lines: Vec<&str> = self.body.lines().collect();
        let mut out = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            let is_call = (l.contains("delivery::advance(") || l.contains("advance_one("))
                // The HELPER's own declaration is not a call site.
                && !l.trim_start().starts_with("fn ");
            if !is_call {
                continue;
            }
            // The statement: from the call to the line that ends it.
            let end = (i..lines.len().min(i + 20))
                .find(|j| lines[*j].trim_end().ends_with(';'))
                .unwrap_or(i);
            out.push((self.line + i, strip_comments(&lines[i..=end].join("\n"))));
        }
        out
    }

    /// The statement EXPECTS a refusal — the arms that pin the refusal are
    /// allowed to be keyless, because refusing IS their subject.
    fn expects_refusal(&self, stmt: &str) -> bool {
        stmt.contains("expect_err") || stmt.contains("unwrap_err")
    }
}

/// The files under `SCAN_DIR`, in a stable order.
fn scan_files() -> Vec<String> {
    let dir = repo(SCAN_DIR);
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| {
            p.strip_prefix(repo(""))
                .unwrap_or(&p)
                .to_string_lossy()
                .to_string()
        })
        .collect();
    out.sort();
    out
}

/// Every function in the file, in the test-module shape.
fn functions(file: &str) -> Vec<Fn> {
    let text = read(file);
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let trimmed = l.trim_start();
        // A test-module function: exactly four spaces of indent, `fn name(`.
        let Some(rest) = trimmed.strip_prefix("fn ") else {
            continue;
        };
        if l.len() - trimmed.len() != 4 {
            continue;
        }
        let name_end = rest.find('(').unwrap_or(rest.len());
        let name = &rest[..name_end];
        // The body runs to this function's closing line at the same indent.
        let end = (i + 1..lines.len())
            .find(|j| lines[*j] == "    }")
            .unwrap_or(lines.len());
        let body = strip_comments(&lines[i..end].join("\n"));
        // `#[test]` sits within the three lines above the declaration.
        let is_test = (i.saturating_sub(3)..i).any(|k| lines[k].trim() == "#[test]");
        let holds_key = body.contains("operator_key_guard()") || body.contains("seed_locked(");
        out.push(Fn {
            name: name.to_string(),
            file: file.to_string(),
            line: i + 1,
            body,
            is_test,
            holds_key,
        });
    }
    out
}

/// The test that closes the class.
///
/// RED-PROOF (run 2026-10-21, reverted): replacing the guard in
/// `promote_refuses_when_the_run_moved_since_the_approval` with a bare
/// `seed()` — the shape the test had before it was fixed — turns this red and
/// names the file and the line. The same drill on the HELPER path (a
/// `seeded_run` caller that drops its guard) turns it red naming the helper
/// and the caller, because the scan follows the call chain rather than
/// reading one function.
#[test]
fn every_test_that_advances_a_phase_holds_the_operator_key() {
    let files = scan_files();
    assert!(
        !files.is_empty(),
        "the operator-key scan found no files under {SCAN_DIR} — a scan that \
         reads nothing proves nothing"
    );

    // Every function in every scanned file, and the call graph between them.
    let mut all: Vec<Fn> = Vec::new();
    for f in &files {
        all.extend(functions(f));
    }
    let names: BTreeSet<String> = all.iter().map(|f| f.name.clone()).collect();

    // callers[callee] = the functions whose body calls `callee`.
    let mut callers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for f in &all {
        for callee in &names {
            if *callee == f.name {
                continue;
            }
            // A call, not a mention: the name followed by `(`.
            if f.body.contains(&format!("{callee}(")) {
                callers
                    .entry(callee.clone())
                    .or_default()
                    .insert(f.name.clone());
            }
        }
    }

    // The functions that advance a phase, and the key question for each.
    let mut offenders: Vec<String> = Vec::new();
    for f in &all {
        for (line, stmt) in f.advances() {
            if f.holds_key || f.expects_refusal(&stmt) {
                continue;
            }
            if f.is_test {
                offenders.push(format!(
                    "{}:{}: `{}` advances a phase and expects it to land, but it \
                     does not hold the operator key — it resolves `BRAIN_UMP_KEY_DIR` \
                     from the MACHINE, so it is green on a developer box and red on a \
                     keyless runner. Hold `operator_key_guard()` (and seed through \
                     `seed_locked`), or expect a refusal.",
                    f.file, line, f.name
                ));
                continue;
            }
            // A HELPER: the convention is that its CALLERS hold the key.
            // Walk the chain, so a helper of a helper is covered too.
            let mut frontier: Vec<String> = vec![f.name.clone()];
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut depth = 0;
            while let Some(callee) = frontier.pop() {
                depth += 1;
                if depth > 4 {
                    break;
                }
                for caller_name in callers.get(&callee).into_iter().flatten() {
                    if !seen.insert(caller_name.clone()) {
                        continue;
                    }
                    let Some(caller) = all.iter().find(|x| x.name == *caller_name) else {
                        continue;
                    };
                    if caller.holds_key {
                        continue;
                    }
                    if caller.is_test {
                        offenders.push(format!(
                            "{}:{}: `{}` calls the helper `{}`, which advances a phase, \
                             and the caller does not hold the operator key — the helper's \
                             key law is the CALLER's to hold (see `seeded_run`).",
                            caller.file, caller.line, caller.name, callee
                        ));
                    } else {
                        frontier.push(caller.name.clone());
                    }
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a test that advances a phase without holding the operator key is green \
         on the developer's machine and RED on a clean runner — the failure \
         that refused the v1.29.25 publication:\n{}",
        offenders.join("\n")
    );
}
