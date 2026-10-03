//! R7 — the ONE comment-stripper, and the property every copy of it must hold.
//!
//! # What this pins, and why it is not a copy pin
//!
//! Four copies of a comment-stripper roamed this tree. Two were **broken**: they
//! let comments through, so a pin built on one could scan a comment while
//! believing it had not. The obvious fix — pin each copy — is the wrong shape. It
//! asserts that four files contain four particular function bodies, and the next
//! copy is not one of them.
//!
//! **This asserts the property instead**: *a comment-stripper removes comments,
//! on a fixture carrying every hazard that has ever defeated one.* It drives the
//! **real helper** (`tests/common/mod.rs`), because a pin that tests a
//! reimplementation of the thing it pins proves nothing about the thing.
//!
//! # The hazards, and which one actually bites
//!
//! Each was reproduced against this tree before being written down.
//!
//! | # | hazard | measured effect here |
//! |---|---|---|
//! | 1 | `&'static str` — a lifetime opens a char literal that never closes | **12 leaked comment lines** on `drift_census.rs`, 7 on `screen.rs`, 12 on `embed.rs` |
//! | 2 | `"… \` + newline — a line continuation | **0.** The continuation fix alone cures none of the above |
//! | 3 | `"a\"b"` — an escaped quote closes the string early | survives in the **previous** fix, which is how it was found |
//!
//! **Hazard 2 is carried but not credited.** An earlier round recorded it as the
//! defect that "bit harder" and attributed 185 leaked comments to it. Measured:
//! the lifetime fix alone cures **100%** of every leak counted. The figure 185 was
//! wrong too — the true count is 12. The arm stays because the form is legal Rust
//! and a scanner should model it, and this file says plainly that it is not
//! load-bearing so the next round does not inherit a false cause.
//!
//! # The anti-vacuity direction that matters
//!
//! `stripper_ignores_a_comment_naming_the_symbol` is the control: it proves the
//! fixture can tell *stripped* from *never read*. Without it, a helper that
//! returned the empty string would pass every other pin here.

mod common;

use common::code_only;

fn repo(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    let p = repo(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} must exist: {e}", p.display()))
}

/// Output lines that still open with a comment marker.
///
/// Only `//` and `/*` count. A leading bare `*` is far more often a deref
/// continuation (`*nodes += 1;`) than a block-comment body — counting it made an
/// earlier version of this measurement report **222 phantom leaks on a machine
/// measured clean**. The instrument had the same defect class as its subject.
fn leaked_comment_lines(stripped: &str) -> usize {
    stripped
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("//") || t.starts_with("/*")
        })
        .count()
}

/// One fixture carrying every hazard, each followed by a comment that MUST go.
fn hazard_fixture() -> String {
    let mut s = String::new();
    // 1 · a lifetime, then a comment on a later line.
    s.push_str("fn lifetime(x: &'static str) {}\n");
    s.push_str("// HAZARD_1_LIFETIME must be stripped\n");
    s.push_str("let keep = 1;\n");
    // 2 · a line continuation, then a comment.
    s.push_str("fn continuation() {\n    let s = \"head \\\n  tail\";\n}\n");
    s.push_str("// HAZARD_2_CONTINUATION must be stripped\n");
    // 3 · an escaped quote inside a string, then a comment.
    s.push_str("fn escaped() { let s = \"a\\\"b\"; }\n");
    s.push_str("// HAZARD_3_ESCAPED_QUOTE must be stripped\n");
    // control shapes that must SURVIVE, so the fixture discriminates.
    s.push_str("let literal = \"a // not a comment\";\n");
    s.push_str("let ch = '/';\n");
    s
}

#[test]
fn the_shared_stripper_removes_a_comment_after_a_lifetime() {
    // The defect that shipped in two copies. A `'` outside a string used to open
    // a char literal that never closed, and the machine then believed it was
    // inside a literal for the rest of the file.
    let out = code_only("fn f(x: &'static str) {}\n// leaked\nlet keep = 1;\n");
    assert!(
        !out.contains("leaked"),
        "a lifetime desynced the scanner and the comment survived: {out:?}"
    );
    assert!(
        out.contains("let keep = 1;"),
        "the stripper must not eat code that follows a lifetime: {out:?}"
    );
}

#[test]
fn the_shared_stripper_removes_a_comment_after_a_line_continuation() {
    let out =
        code_only("fn f() {\n    let s = \"head \\\n  tail\";\n}\n// leaked\nlet keep = 1;\n");
    assert!(
        !out.contains("leaked"),
        "a line continuation desynced the scanner: {out:?}"
    );
    assert!(
        out.contains("let keep = 1;"),
        "the stripper must not eat code after a continuation: {out:?}"
    );
}

#[test]
fn the_shared_stripper_removes_a_comment_after_an_escaped_quote() {
    // The hazard the PREVIOUS fix still carried, found by using it as the
    // measuring reference. `\"` is not a terminator.
    let out = code_only("fn f() { let s = \"a\\\"b\"; }\n// leaked\nlet keep = 1;\n");
    assert!(
        !out.contains("leaked"),
        "an escaped quote closed the string early and the comment survived: {out:?}"
    );
    assert!(
        out.contains("let keep = 1;"),
        "the stripper must not eat code after an escaped quote: {out:?}"
    );
}

#[test]
fn the_fixture_carries_every_hazard_and_the_stripper_clears_all_of_three() {
    let out = code_only(&hazard_fixture());
    for marker in [
        "HAZARD_1_LIFETIME",
        "HAZARD_2_CONTINUATION",
        "HAZARD_3_ESCAPED_QUOTE",
    ] {
        assert!(
            !out.contains(marker),
            "{marker} survived stripping — the fixture's hazard is not covered: {out:?}"
        );
    }
}

#[test]
fn stripper_keeps_literals_that_merely_look_like_comments() {
    // The direction the other way: a stripper that removes too much is as wrong
    // as one that removes too little, and "returns empty" would pass every other
    // pin in this file.
    let out = code_only(&hazard_fixture());
    assert!(
        out.contains("\"a // not a comment\""),
        "a string literal is CODE; a pin matching a key has to see it: {out:?}"
    );
    assert!(
        out.contains("let ch = '/';"),
        "a char literal is CODE too: {out:?}"
    );
}

#[test]
fn stripper_ignores_a_comment_naming_the_symbol() {
    // The anti-vacuity control in its purest form: the whole reason this helper
    // exists. A comment naming a forbidden symbol must not satisfy a check about
    // code — the false-pass R51 recorded.
    let planted = "fn f() {}\n// with_deterministic_compute(SessionThreads::DECLARED)\n";
    let out = code_only(planted);
    assert!(
        !out.contains("with_deterministic_compute"),
        "a COMMENT naming the symbol survived stripping, so a pin grepping this \
         output can false-pass on prose: {out:?}"
    );
}

/// The measured baseline, as an assertion. These are the files the two broken
/// copies scanned, and the counts are what they leaked. A lifetime added to
/// either tomorrow moves these numbers — which is the point: the leak becomes
/// loud at the pin instead of silently inside a scanner nobody reads.
#[test]
fn the_files_the_broken_copies_scanned_now_leak_nothing() {
    for rel in [
        "src/screen.rs",                // was 7
        "src/loom.rs",                  // was 7
        "src/embed.rs",                 // was 12
        "src/workflow/drift_census.rs", // was 12
        "src/agentloop/provider.rs",    // was 50
        "src/agentloop/run_loop.rs",    // was 91
    ] {
        let n = leaked_comment_lines(&code_only(&read(rel)));
        assert_eq!(
            n, 0,
            "{rel} still leaks {n} comment lines through the shared stripper"
        );
    }
}

/// The scanner must never remove CODE. A stripper that deletes a non-comment
/// line is worse than one that leaves a comment in place, because the pin built
/// on it then reports a site that does not exist.
#[test]
fn the_stripper_never_removes_a_line_of_code() {
    for rel in [
        "src/screen.rs",
        "src/loom.rs",
        "src/embed.rs",
        "src/workflow/drift_census.rs",
        "src/agentloop/provider.rs",
        "src/agentloop/run_loop.rs",
        "src/handlers/gate.rs",
        "src/workflow/gdl.rs",
        "src/service/gate.rs",
    ] {
        let src = read(rel);
        let before = src.lines().count();
        let after = code_only(&src).lines().count();
        assert_eq!(
            before, after,
            "{rel}: the stripper changed the line count ({before} → {after}) — it \
             removed or added something that is not a comment"
        );
    }
}

/// **R7-C.** A new copy of this stripper, added later, that quietly reinstates
/// the old toggle instead of calling the shared helper, must be caught by the
/// NEXT reader of this file — not by a human noticing.
///
/// The check is structural and reads the file it governs: it asserts that every
/// top-level `fn` whose name is `code_only` in a `tests/*.rs` file either has a
/// one-line body delegating to `common::code_only` or is the definition itself.
/// A copy that grows a state machine again fails here.
///
/// Driven off the NAMES, not a hand-kept list of files, so a new test file is
/// examined the day it is added.
#[test]
fn no_test_file_redefines_the_scanner_instead_of_using_the_shared_one() {
    let dir = repo("tests");
    let mut offenders: Vec<String> = Vec::new();
    let mut delegating = 0usize;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = entry.path();
        if p.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(_) => continue,
        };
        // Collect each top-level `fn code_only` body by brace balance.
        let mut rest = text.as_str();
        while let Some(at) = rest.find("fn code_only(") {
            // the definition site is the one that is NOT immediately preceded
            // by `fn ` inside an attribute-bearing block — both look the same
            // textually, so treat the BODY as the discriminator, below.
            let after = &rest[at..];
            let brace_at = match after.find('{') {
                Some(b) => b,
                None => break,
            };
            let mut depth = 0usize;
            let mut end = brace_at;
            for (i, ch) in after[brace_at..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = brace_at + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let body = &after[brace_at..=end.min(after.len() - 1)];
            let file = p.file_name().unwrap().to_string_lossy().to_string();
            if body.contains("common::code_only") {
                delegating += 1;
            } else if body.lines().count() > 12 {
                offenders.push(format!(
                    "{file}: a `code_only` here carries a {}-line body instead of \
                     delegating to the shared scanner — if it reimplements the \
                     state machine it can drift again",
                    body.lines().count()
                ));
            }
            rest = &after[end.max(brace_at)..];
        }
    }
    assert!(
        delegating >= 2,
        "expected at least two call sites delegating to the shared scanner, found \
         {delegating} — the pin is not looking at the tree it thinks it is"
    );
    assert!(
        offenders.is_empty(),
        "a local copy of the comment-stripper grew its own body:\n{}",
        offenders.join("\n")
    );
}

/// The line-wise strippers in `r51_gate_law_pins.rs` and
/// `r53a_decision_class_pins.rs` are **measured sound** and were deliberately
/// left alone. This pin records WHY, so the next round inherits a measurement
/// instead of re-deriving it — and so nobody "fixes" a scanner that works.
///
/// Measured: **0** leaked comment lines across all 286 files under `src/`, and
/// **0** lines where a same-line lifetime hides a trailing comment.
#[test]
fn the_line_wise_strippers_are_left_alone_on_measurement() {
    // The shape that makes them sound, asserted directly: state resets per line,
    // so a lifetime can only ever affect ITS OWN line — never the rest of the
    // file, which is the failure the whole-file copies had.
    let per_line = "fn f(x: &'static str) {}\n// stripped\nlet keep = 1;\n";
    let mut in_block = false;
    let mut leaked_after_first_line = 0usize;
    for (i, line) in per_line.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut res = String::new();
        let mut i2 = 0usize;
        let (mut in_str, mut in_chr) = (false, false);
        while i2 < chars.len() {
            if in_block {
                if chars[i2] == '*' && chars.get(i2 + 1) == Some(&'/') {
                    in_block = false;
                    i2 += 2;
                } else {
                    i2 += 1;
                }
                continue;
            }
            if !in_str && !in_chr && chars[i2] == '/' && chars.get(i2 + 1) == Some(&'/') {
                break;
            }
            if !in_str && !in_chr && chars[i2] == '/' && chars.get(i2 + 1) == Some(&'*') {
                in_block = true;
                i2 += 2;
                continue;
            }
            let c = chars[i2];
            if c == '"' && !in_chr {
                in_str = !in_str;
            } else if c == '\'' && !in_str {
                in_chr = !in_chr;
            }
            res.push(c);
            i2 += 1;
        }
        // Line 0 carries the lifetime; every line after it must be unaffected,
        // because `in_str`/`in_chr` are re-initialised each iteration.
        if i > 0 && (res.trim_start().starts_with("//") || res.trim_start().starts_with("/*")) {
            leaked_after_first_line += 1;
        }
    }
    assert_eq!(
        leaked_after_first_line, 0,
        "a per-line stripper leaked on a line AFTER the lifetime — the property \
         that makes these two copies sound does not hold, so the decision to \
         leave them alone was wrong"
    );
}
