#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    let words: Vec<&str> = s.split_whitespace().take(20).collect();
    for w in words {
        let mut out = String::new();
        for word in w.split_whitespace() {
            if !out.is_empty() { out.push(' '); }
            out.push_str(word);
        }
        out.make_ascii_lowercase();
        let _ = out;
    }
});
