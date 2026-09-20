//! Fuzz target: the consumption accounting (token budgets and the
//! compaction quota share the saturating-arithmetic discipline). Arbitrary
//! spends against an arbitrary ceiling must saturate — never overflow,
//! never widen a limit, fail closed at the boundary.
//!
//! Run (networked operator machine):
//!   cargo install cargo-fuzz && cargo fuzz run fuzz_budget_predicate -- -max_total_time=60

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Ceiling from the first 8 bytes (LE); the rest is (input, output)
    // u64 pairs as spends against it.
    if data.len() < 8 {
        let _ = brain_server::workflow::fuzz_budget_predicate(0, Vec::new());
        return;
    }
    let limit = u64::from_le_bytes(data[..8].try_into().expect("8 bytes"));
    let rest = &data[8..];
    let spends: Vec<(u64, u64)> = rest
        .chunks_exact(16)
        .map(|c| {
            (
                u64::from_le_bytes(c[..8].try_into().expect("8 bytes")),
                u64::from_le_bytes(c[8..].try_into().expect("8 bytes")),
            )
        })
        .collect();
    let _ = brain_server::workflow::fuzz_budget_predicate(limit, spends);
});
