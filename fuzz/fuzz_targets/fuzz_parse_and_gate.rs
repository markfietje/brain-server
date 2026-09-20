//! Fuzz target: the loop's model-artifact parser (the untrusted-input
//! surface). Every artifact the model emits goes through parse-and-gate;
//! it must return a named verdict for ANY byte string — never panic,
//! never truncate silently, never accept malformed JSON.
//!
//! Run (networked operator machine):
//!   cargo install cargo-fuzz && cargo fuzz run fuzz_parse_and_gate -- -max_total_time=60

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // First byte selects the phase (deterministic coverage steering);
    // the rest is the model artifact verbatim, lossy-decoded.
    let (phase_idx, rest) = data.split_first().unwrap_or((&0, &[]));
    let phases = ["intake", "triage", "hypothesize", "plan", "act", "verify", "handoff"];
    let phase = phases[usize::from(*phase_idx) % phases.len()];
    let text = String::from_utf8_lossy(rest);
    let _ = brain_server::workflow::fuzz_parse_and_gate(phase, &text);
});
