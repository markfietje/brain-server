//! Fuzz target stub: QueryDoc parser (v1.3.0 Bedrock M3).
//!
//! NOTE: `QueryDoc::from_text` lives in the server binary's `search/query.rs`.
//! To fuzz it, the function needs to be exposed via the lib crate. The
//! QueryDoc parser's safety is currently covered by unit tests in `query.rs`.
//!
//! Run: `cargo +nightly fuzz run fuzz_query_doc -- -max_total_time=60`

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Exercises JSON parsing of arbitrary input — the same first step
    // QueryDoc::from_text takes when deserializing from a request body.
    let s = String::from_utf8_lossy(data);
    let _: Result<serde_json::Value, _> = serde_json::from_str(&s);
});
