//! Fuzz target: chunker multibyte safety (v1.3.0 Bedrock M3).
//!
//! NOTE: the chunker lives in the server binary crate (`src/chunker.rs`), not
//! the lib crate. cargo-fuzz can only call into the lib. The chunker's
//! multibyte safety is covered by:
//!   1. The proptest in `src/chunker.rs` (256 random cases)
//!   2. The hand-rolled fuzz test (2000 cases)
//!   3. This stub — to activate, move `chunk_markdown` to the lib crate or
//!      add a `#[cfg(fuzz)] pub fn chunk_markdown_fuzz()` wrapper.
//!
//! Run: `cargo +nightly fuzz run fuzz_chunker -- -max_total_time=60`

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Exercises String::from_utf8_lossy — the same conversion the ingest path
    // uses when reading vault files. Ensures the lossy conversion itself is
    // panic-free under arbitrary byte inputs.
    let _lossy = String::from_utf8_lossy(data);
});
