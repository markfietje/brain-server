//! Fuzz target: the kind-prefix corroboration law. Hypothesis sources are
//! model-supplied strings; the status labeler must classify any source
//! soup into the closed status set — never panic, never invent a kind.
//!
//! Run (networked operator machine):
//!   cargo install cargo-fuzz && cargo fuzz run fuzz_hypothesis_status -- -max_total_time=60

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Split the bytes on newlines into candidate source strings; any
    // malformed soup is the point.
    let sources: Vec<String> = String::from_utf8_lossy(data)
        .split('\n')
        .map(|s| s.trim().to_string())
        .collect();
    let _ = brain_server::workflow::fuzz_hypothesis_status(sources);
});
