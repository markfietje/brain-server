//! Fuzz target: domain name validator (v1.3.0 Bedrock M3).
//!
//! Feeds random bytes to `storage_layout::is_valid_domain`. The validator
//! must never panic on any input — it's the security-critical gate that
//! prevents path traversal via domain names.
//!
//! Run: `cargo +nightly fuzz run fuzz_validator -- -max_total_time=60`

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    // is_valid_domain must return a bool, never panic, for any input.
    let _ = brain_server::storage_layout::is_valid_domain(&s);
});
