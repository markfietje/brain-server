// Quick demo: prints chunks for a given markdown file.
// Usage: cargo run --example chunk_demo -- <path>
use std::io::Read;

#[path = "../src/chunker.rs"]
mod chunker;

fn main() {
    let path = std::env::args().nth(1).expect("usage: chunk_demo <path>");
    let mut content = String::new();
    std::fs::File::open(&path)
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    let chunks = chunker::chunk_markdown(&content);
    println!("=== {} → {} chunks ===", path, chunks.len());
    for (i, c) in chunks.iter().enumerate() {
        println!(
            "\n--- chunk {} [lines {}..{}] heading={:?} ---",
            i + 1,
            c.line_start,
            c.line_end,
            c.heading_path
        );
        println!("{}", c.text);
    }
}
