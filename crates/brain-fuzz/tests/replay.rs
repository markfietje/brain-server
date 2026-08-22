//! Corpus replay: every byte in `corpus/<target>/` runs through the SAME
//! total functions the libFuzzer targets call. A fuzzer crash becomes a
//! regression by committing its bytes here.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use brain_fuzz::{fuzz_evidence, fuzz_hostcall, fuzz_meta, fuzz_scorer};

fn replay(dir: &str, f: fn(&[u8])) {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("corpus");
    path.push(dir);
    let entries = std::fs::read_dir(&path).unwrap_or_else(|e| panic!("corpus dir {dir}: {e}"));
    let mut count = 0;
    for e in entries.flatten() {
        if e.path().is_file()
            && let Ok(bytes) = std::fs::read(e.path())
        {
            f(&bytes);
            count += 1;
        }
    }
    assert!(count > 0, "corpus {dir} must not be empty");
}

#[test]
fn corpus_replays_evidence() {
    replay("evidence", fuzz_evidence);
}

#[test]
fn corpus_replays_meta() {
    replay("meta", fuzz_meta);
}

#[test]
fn corpus_replays_hostcall() {
    replay("hostcall", fuzz_hostcall);
}

#[test]
fn corpus_replays_scorer() {
    replay("scorer", fuzz_scorer);
}
