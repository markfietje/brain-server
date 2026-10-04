//! A test-only classifier probe.
//!
//! WHY THIS EXISTS. `procedural::subject_matter_terms()` caches in a
//! `OnceLock`, so a test cannot prove the pool both fires AND stays inert by
//! mutating the environment in-process — the first read wins for the whole
//! process. The pool's two properties are therefore each proven in a CHILD
//! process with the variable set differently, and a child needs something to
//! run.
//!
//! It is a separate `[[bin]]` rather than a hidden `brain` verb so the shipped
//! CLI surface gains nothing: `brain classify` talks to a running server, which
//! a unit test must not require.
//!
//! Prints `<category>\t<comma-separated matched keywords>` for one input line
//! and exits 0. Not a supported operator interface.

use std::io::Write;

fn main() {
    // The FIRST ARGV is the text. An earlier draft read stdin, which made every
    // caller pass an argument and silently classified the empty string — the
    // probe returned `general` for everything, and it looked like a lexicon bug.
    // The test that would have caught it is the one asserting the probe's output
    // is NOT `general`.
    let text = std::env::args()
        .nth(1)
        .expect("usage: classify-probe \"<text>\"");
    let result = brain_server::procedural::classify(text.trim());
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{}\t{}",
        result.category,
        result.matched_keywords.join(",")
    )
    .ok();
}
