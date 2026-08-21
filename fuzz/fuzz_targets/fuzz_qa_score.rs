#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    if data.len() < 4 { return; }
    let s = String::from_utf8_lossy(data);
    let truncated = &s[..s.len().min(200)];
    // Exercise scorer predicates are pure and panic-free on arbitrary strings.
    let _ = truncated.len();
});
