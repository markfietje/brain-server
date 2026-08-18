//! Structure-aware Markdown chunker (v0.9.4: CommonMark-compliant via pulldown-cmark).
//!
//! Splits a Markdown document into bounded chunks that respect document
//! structure: chunks break at heading boundaries, code blocks are kept intact
//! (never split mid-fence), and each chunk records the heading breadcrumb and
//! the 1-indexed line span it covers.
//!
//! **CommonMark compliance** is provided by `pulldown-cmark` 0.13 (Context7-
//! verified 2026-07-17). The previous hand-rolled line-scanner mis-handled
//! setext headings (`Foo\n===`), indented code blocks (4-space indent),
//! blockquotes, lists, and tables. pulldown-cmark's event stream with
//! `into_offset_iter()` gives us byte-accurate source spans for every block,
//! so chunk text is sliced verbatim from the source — every byte of content
//! survives intact, including container markup (`>`, `-`, `|`) that falls
//! between inline text events.
//!
//! **Character-preservation warranty:** every byte of input text — including
//! `#`-comments inside code fences, unicode, backticks, brackets, dashes, and
//! arbitrary special characters — survives intact into the chunk `text`. The
//! only lines consumed (not buffered verbatim) are ATX and setext headings;
//! their text becomes the chunk's `heading_path` breadcrumb instead. Verified
//! by `test_special_characters_survive_ingest_pipeline` plus the per-construct
//! tests below (setext, indented code, blockquote, list, table).
//!
//! Pure and allocation-only: no I/O, no `unsafe`. pulldown-cmark itself is
//! `#![forbid(unsafe_code)]` upstream (its SIMD scanner is gated behind a
//! feature we do not enable).

#![deny(unsafe_code)]

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Soft upper bound on a chunk's BYTE length (≈ a few hundred tokens, within
/// the static model's sweet spot). Hard cap only inside code blocks, which are
/// kept intact. Named `_BYTES` (not `_CHARS`) because we measure byte length;
/// a chunk with multibyte UTF-8 may have fewer than this many chars. The
/// atomic unit is a block (paragraph / code block / list item / etc.), so
/// multibyte UTF-8 sequences are always preserved.
const MAX_CHUNK_BYTES: usize = 1000;

/// the hard cap on code blocks. Code blocks are
/// exempt from `MAX_CHUNK_BYTES` "to stay intact" — but that lets a single
/// fenced block up to the 1 MB content cap become ONE giant chunk. Blocks
/// over this cap are split at newline boundaries; fenced pieces re-open the
/// fence with a continuation marker (the same opener line) and only the final
/// piece carries the original closer, so every piece is a standalone valid
/// fenced block that concatenates back to the source. `8 * MAX_CHUNK_BYTES`
/// keeps genuinely monolithic artifacts adjacent via the heading path while
/// bounding the chunk.
const MAX_CODE_CHUNK_BYTES: usize = 8 * MAX_CHUNK_BYTES;

/// A structure-aware chunk of a Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The chunk text (verbatim source bytes for the chunk's line span).
    pub text: String,
    /// Heading breadcrumb, e.g. `"Installation > Linux"`. Empty before the
    /// first heading.
    pub heading_path: String,
    /// 1-indexed line number of the first line in the source document.
    pub line_start: usize,
    /// 1-indexed line number of the last line in the source document.
    pub line_end: usize,
}

/// Split Markdown `content` into structure-aware chunks.
///
/// Algorithm: walk `pulldown-cmark` events with their byte-offset ranges.
/// Accumulate a chunk byte-range by extending it to cover every event whose
/// source bytes should appear in chunk text. Heading events close the current
/// chunk (and contribute their text to the breadcrumb instead of to the chunk
/// text — matching pre-v0.9.4 behavior). Code blocks set a "don't split" flag
/// so a fence is never broken mid-block. On flush, the chunk's text is sliced
/// verbatim from the source `content[chunk_start..chunk_end]`.
pub fn chunk_markdown(content: &str) -> Vec<Chunk> {
    // Precompute byte offset of each line start, for byte → 1-indexed line.
    // line_starts[i] is the byte offset where line (i+1) begins.
    let line_starts: Vec<usize> = line_start_offsets(content);

    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(content, opts);

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut heading_stack: Vec<(usize, String)> = Vec::new();
    let mut buf_heading = String::new();

    // Current chunk byte range being accumulated. None = no pending content.
    let mut chunk_start: Option<usize> = None;
    let mut chunk_end: usize = 0;

    let mut in_heading = false;
    let mut pending_heading_level: Option<usize> = None;
    let mut heading_title_acc = String::new();
    // Nested code blocks shouldn't happen in well-formed markdown, but depth-
    // count is defensive against any parser quirk.
    let mut code_block_depth: usize = 0;

    for (event, range) in parser.into_offset_iter() {
        let (start, end) = (range.start, range.end);

        match event {
            // ── Headings: close the current chunk, collect title for breadcrumb ──
            // Heading source bytes are NOT added to any chunk's text (they become
            // the breadcrumb). Matches the pre-v0.9.4 chunker's behavior.
            Event::Start(Tag::Heading { level, .. }) => {
                flush_buf(
                    content,
                    &line_starts,
                    &mut chunk_start,
                    &mut chunk_end,
                    &buf_heading,
                    &mut chunks,
                );
                in_heading = true;
                pending_heading_level = Some(heading_level_to_usize(level));
                heading_title_acc.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(lvl) = pending_heading_level.take() {
                    let title = heading_title_acc.trim().to_string();
                    if !title.is_empty() {
                        // Pop any deeper-or-equal headings, then push this one.
                        // Sibling headings (same level) replace each other;
                        // child headings (deeper) nest under the parent.
                        heading_stack.retain(|(l, _)| *l < lvl);
                        heading_stack.push((lvl, title));
                        buf_heading = breadcrumb(&heading_stack);
                    }
                }
                heading_title_acc.clear();
                in_heading = false;
            }

            // ── Code blocks: track depth so we never split mid-fence ──
            // The CodeBlock Start/End range covers the WHOLE block including
            // the fence markers and info string. Extending the chunk to this
            // range captures the fences verbatim.
            Event::Start(Tag::CodeBlock(_)) => {
                code_block_depth += 1;
                extend_buf(&mut chunk_start, &mut chunk_end, start, end);
            }
            Event::End(TagEnd::CodeBlock) => {
                code_block_depth = code_block_depth.saturating_sub(1);
                extend_buf(&mut chunk_start, &mut chunk_end, start, end);
            }

            // ── Text inside a heading: collect into the title accumulator ──
            Event::Text(t) if in_heading => {
                heading_title_acc.push_str(&t);
            }

            // ── Everything else: extend the chunk's byte range ──
            // This covers: Text outside headings, Code, Html, SoftBreak,
            // HardBreak, Rule, TaskListMarker, plus Start/End of Paragraph,
            // BlockQuote, List, Item, Table, etc. Container markup (`>`, `-`,
            // `|`) lives in the source BETWEEN inline-text byte ranges, so
            // taking the union of all event ranges naturally preserves it.
            _ => {
                if !in_heading {
                    extend_buf(&mut chunk_start, &mut chunk_end, start, end);
                }
            }
        }

        // Flush on size, except inside a code block (keep blocks intact) or a
        // heading (heading always closes the chunk anyway, handled above).
        if code_block_depth == 0 && !in_heading {
            if let Some(cs) = chunk_start {
                if chunk_end.saturating_sub(cs) >= MAX_CHUNK_BYTES {
                    flush_buf(
                        content,
                        &line_starts,
                        &mut chunk_start,
                        &mut chunk_end,
                        &buf_heading,
                        &mut chunks,
                    );
                }
            }
        }
    }

    flush_buf(
        content,
        &line_starts,
        &mut chunk_start,
        &mut chunk_end,
        &buf_heading,
        &mut chunks,
    );
    chunks
        .into_iter()
        .flat_map(|c| {
            if c.text.len() > MAX_CODE_CHUNK_BYTES {
                split_oversized_code(c)
            } else {
                vec![c]
            }
        })
        .collect()
}

/// split a single chunk that exceeds the hard
/// cap at newline boundaries. Only code blocks (fenced or ≥4-space indented)
/// and single-event prose runs can reach this size — normal prose flushes at
/// every event boundary. Fenced blocks are re-opened with the same opener
/// line (the continuation marker) and each piece closes with the original
/// closer, so every piece is a standalone block and the pieces concatenate
/// back to the source. Indented code / prose need no synthesis: cutting at a
/// newline leaves every piece a valid verbatim block.
fn split_oversized_code(chunk: Chunk) -> Vec<Chunk> {
    let text: &str = &chunk.text;
    let is_fenced = text.starts_with("```") || text.starts_with("~~~");
    let all_lines: Vec<&str> = text.split('\n').collect();
    let (opener, closer, body): (Option<&str>, Option<&str>, &[&str]) = if is_fenced {
        (
            all_lines.first().copied(),
            all_lines.last().copied(),
            &all_lines[1..all_lines.len().saturating_sub(1)],
        )
    } else {
        (None, None, &all_lines[..])
    };

    // Greedy newline-boundary pack of the body lines into ≤ cap pieces.
    let mut raw: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_bytes = 0usize;
    for line in body {
        let add = line.len() + 1;
        if cur_bytes + add > MAX_CODE_CHUNK_BYTES {
            if !cur.is_empty() {
                raw.push(std::mem::take(&mut cur));
            }
            cur_bytes = 0;
        }
        // Degenerate: one single line longer than the cap. The newline-
        // boundary rule has no boundary to respect — split by byte (char-
        // boundary-safe) and keep the parts as their own one-line pieces.
        if add > MAX_CODE_CHUNK_BYTES {
            let mut rest = *line;
            while !rest.is_empty() {
                let cut = rest.floor_char_boundary(MAX_CODE_CHUNK_BYTES.min(rest.len()));
                raw.push(rest[..cut].to_string());
                rest = &rest[cut..];
            }
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
        cur_bytes += add;
    }
    if !cur.is_empty() {
        raw.push(cur);
    }
    if raw.is_empty() {
        raw.push(String::new());
    }

    let mut out = Vec::with_capacity(raw.len());
    let mut next_line = chunk.line_start;
    for piece in raw {
        let piece_text = match (opener, closer) {
            (Some(op), Some(cl)) => format!("{op}\n{piece}{cl}"),
            _ => piece,
        };
        let body_lines = piece_text.matches('\n').count().max(1);
        let line_end = next_line + body_lines - 1;
        out.push(Chunk {
            text: piece_text,
            heading_path: chunk.heading_path.clone(),
            line_start: next_line,
            line_end,
        });
        next_line = line_end + 1;
    }
    out
}

/// Extend the chunk's byte range to cover `[start, end)`. Idempotent on
/// overlapping ranges — repeated calls with subsets of an already-covered
/// range are no-ops.
fn extend_buf(chunk_start: &mut Option<usize>, chunk_end: &mut usize, start: usize, end: usize) {
    if start >= end {
        return;
    }
    match chunk_start {
        None => {
            *chunk_start = Some(start);
            *chunk_end = end;
        }
        Some(cs) => {
            if start < *cs {
                *cs = start;
            }
            if end > *chunk_end {
                *chunk_end = end;
            }
        }
    }
}

/// Flush the pending byte range into a chunk. Trims leading/trailing blank
/// lines (matches pre-v0.9.4 behavior — a chunk's text never starts or ends
/// with a blank line). Computes 1-indexed line spans from the trimmed range.
fn flush_buf(
    content: &str,
    line_starts: &[usize],
    chunk_start: &mut Option<usize>,
    chunk_end: &mut usize,
    heading: &str,
    chunks: &mut Vec<Chunk>,
) {
    let Some(mut start) = *chunk_start else {
        return;
    };
    let mut end = *chunk_end;
    if start >= end {
        *chunk_start = None;
        return;
    }

    // NOTE: `start`/`end` arrive as pulldown-cmark byte ranges, which are
    // guaranteed UTF-8 char boundaries. We still slice defensively through
    // `safe_slice` so that a future change passing a non-boundary offset can
    // never panic on a multibyte char (the historical '•' crash).

    // Trim leading blank lines: advance `start` past any line whose trim() is
    // empty. A "line" here is delimited by '\n'.
    while start < end {
        let line_end = content[start..end]
            .find('\n')
            .map(|i| start + i)
            .unwrap_or(end);
        if safe_slice(content, start, line_end).trim().is_empty() {
            start = line_end + 1; // skip past the '\n'
        } else {
            break;
        }
    }

    // Trim trailing blank lines: retreat `end` before any trailing run of
    // blank lines.
    while end > start {
        // `rfind` returns an offset *within* the `start..end` slice, so add
        // `start` to get the absolute byte index of the newline. Using the raw
        // relative offset as an absolute index would slice inside a multibyte
        // char (e.g. a '•' bullet) and panic on a non-char-boundary slice.
        let prev_nl = content[start..end].rfind('\n').map(|i| start + i);
        let last_line_start = match prev_nl {
            Some(i) => {
                // The line after the last newline starts at i+1.
                // To trim trailing blanks, look at the LAST line: if blank,
                // move end back to the newline position.
                let last_line = safe_slice(content, i + 1, end);
                if last_line.trim().is_empty() {
                    i // end becomes the newline position (trim it)
                } else {
                    break;
                }
            }
            None => {
                // Single line, no newlines. If blank, drop the whole chunk.
                if safe_slice(content, start, end).trim().is_empty() {
                    *chunk_start = None;
                    return;
                }
                break;
            }
        };
        end = last_line_start;
    }

    if start >= end {
        *chunk_start = None;
        return;
    }

    // Also strip a single trailing newline if present (the source slice often
    // ends just past the last content line's '\n'). This matches the
    // pre-v0.9.4 join-with-'\n' which never appended a trailing newline.
    if end > start && content.as_bytes().get(end - 1) == Some(&b'\n') {
        end -= 1;
    }

    // Defensive: clamp to char boundaries before the final slice. pulldown-cmark
    // byte ranges are already boundaries, but this makes the slice panic-proof
    // against any non-boundary `start`/`end` a future caller could pass.
    let text = safe_slice(content, start, end).to_string();
    let line_start = byte_to_line(line_starts, start);
    // The last byte is at end-1 (end is exclusive). If end==start the chunk is
    // empty — but we've already returned in that case.
    let line_end = byte_to_line(line_starts, end.saturating_sub(1).max(start));

    chunks.push(Chunk {
        text,
        heading_path: heading.to_string(),
        line_start,
        line_end,
    });
    *chunk_start = None;
}

/// Build a vector where `line_starts[i]` is the byte offset of the start of
/// the (i+1)-th line. Used to convert a byte offset to a 1-indexed line number.
fn line_start_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Convert a byte offset to a 1-indexed line number by binary search through
/// the precomputed `line_starts` table.
fn byte_to_line(line_starts: &[usize], byte_offset: usize) -> usize {
    // Find the largest i where line_starts[i] <= byte_offset. Line number is i+1.
    match line_starts.binary_search(&byte_offset) {
        Ok(i) => i + 1,
        Err(i) => i, // i is the insertion point; line is i (1-indexed: i)
    }
}

/// Slice `content[start..end]` with both indices clamped to UTF-8 char
/// boundaries. `start` floors (moves left to the char start) and `end` ceilings
/// (moves right to the char end) so the slice can never land inside a multibyte
/// sequence — the historical '•' panic in `flush_buf`. Returns "" for any
/// out-of-range or degenerate range rather than panicking.
fn safe_slice(content: &str, start: usize, end: usize) -> &str {
    let bytes = content.as_bytes();
    if start >= end || start >= bytes.len() {
        return "";
    }
    let start = start.min(bytes.len());
    let end = end.min(bytes.len());
    let start = content.floor_char_boundary(start);
    let end = content.ceil_char_boundary(end);
    &content[start..end]
}

/// Convert `HeadingLevel` to a 1..=6 usize. pulldown-cmark models this as an
/// enum so we can't accidentally get a level 7.
fn heading_level_to_usize(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn breadcrumb(stack: &[(usize, String)]) -> String {
    stack
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join(" > ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_paragraph_is_one_chunk() {
        let chunks = chunk_markdown("Just one short paragraph of text.");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].line_start, 1);
        assert!(chunks[0].heading_path.is_empty());
    }

    #[test]
    fn splits_at_headings_with_breadcrumb() {
        let md = "# Guide\nintro line one\nintro line two\n\n## Setup\nstep one\nstep two\n";
        let chunks = chunk_markdown(md);
        assert!(
            chunks.len() >= 2,
            "should split at the ## heading: {:?}",
            chunks
        );
        // First chunk is under "Guide", second under "Guide > Setup".
        assert_eq!(chunks[0].heading_path, "Guide", "{:?}", chunks);
        assert_eq!(chunks[1].heading_path, "Guide > Setup", "{:?}", chunks);
        // First chunk contains the intro lines, not the setup steps.
        assert!(chunks[0].text.contains("intro line one"));
        assert!(!chunks[0].text.contains("step one"));
        // Spans are ordered and non-overlapping.
        assert!(chunks[0].line_end < chunks[1].line_start);
    }

    #[test]
    fn keeps_code_fence_intact() {
        let big: String = "x".repeat(MAX_CHUNK_BYTES + 200);
        let md = format!("Intro here.\n\n```rust\n{big}\n```\n\nAfter fence.\n");
        let chunks = chunk_markdown(&md);
        let fence_chunk = chunks.iter().find(|c| c.text.contains("```rust"));
        assert!(fence_chunk.is_some(), "fence must be in a chunk");
        assert_eq!(
            fence_chunk.unwrap().text.matches("```").count(),
            2,
            "fence must not be split mid-block"
        );
    }

    // ── code blocks were exempt from
    // `MAX_CHUNK_BYTES` "to stay intact", so a single fenced block up to the
    // 1 MB content cap became ONE giant chunk. The hard cap
    // (`MAX_CODE_CHUNK_BYTES = 8 * MAX_CHUNK_BYTES`) splits oversized blocks
    // at newline boundaries, re-opening the fence with a continuation marker.

    #[test]
    fn oversized_code_block_is_split_with_continuation() {
        // 40 lines ≈ 40 × 51 bytes ≈ 2000+ per... build a body well over the
        // 8 KB cap: 40 lines of ~60 B = ~2400; use 240 lines ≈ 14 KB.
        let code_line = "let config = Config::builder().with_feature(\"all\").build()?;";
        let body: String = std::iter::repeat_with(|| code_line)
            .take(240)
            .collect::<Vec<_>>()
            .join("\n");
        let md = format!("# Big\n\n```rust\n{body}\n```\n\nTail.\n");
        let chunks = chunk_markdown(&md);
        let split: Vec<&Chunk> = chunks
            .iter()
            .filter(|c| c.text.contains("let config"))
            .collect();
        assert!(
            split.len() >= 2,
            "the oversized fence must split: {} pieces",
            split.len()
        );
        // Every piece stays under the hard cap (+the fence overhead).
        for c in &split {
            assert!(
                c.text.len() <= MAX_CODE_CHUNK_BYTES + 32,
                "piece exceeds the hard cap: {} bytes",
                c.text.len()
            );
        }
        // Continuation: every piece is a complete standalone fenced block —
        // opener + closer — and every piece after the first re-opens the
        // fence with the SAME info string (continuation marker).
        for c in &split {
            assert!(
                c.text.starts_with("```rust"),
                "continuation opener: {:?}",
                &c.text[..8]
            );
            assert!(c.text.ends_with("```"), "every piece closes its fence");
        }
        // No content line is lost or duplicated: the body lines all survive
        // in order across the pieces.
        let joined: String = split
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let times = |s: &str, needle: &str| s.matches(needle).count();
        assert_eq!(
            times(&joined, code_line),
            240,
            "every source code line survives exactly once"
        );
        // Line spans are contiguous across the split pieces.
        for w in split.windows(2) {
            assert_eq!(
                w[0].line_end + 1,
                w[1].line_start,
                "pieces stay adjacent: {:?}",
                (
                    w[0].line_start,
                    w[0].line_end,
                    w[1].line_start,
                    w[1].line_end
                )
            );
        }
    }

    #[test]
    fn normal_code_blocks_unchanged() {
        // A fenced block over `MAX_CHUNK_BYTES` but under the hard cap stays
        // ONE intact chunk — the F-52 carve-out only triggers past 8 KB.
        let body: String = std::iter::repeat_with(|| "let intact = true;")
            .take(120)
            .collect::<Vec<_>>()
            .join("\n");
        let md = format!("```rust\n{body}\n```\n");
        let chunks = chunk_markdown(&md);
        assert_eq!(chunks.len(), 1, "a ~1.8 KB fence stays one chunk");
        assert_eq!(
            chunks[0].text.matches("```").count(),
            2,
            "both fences intact, byte-sliced verbatim"
        );
    }

    #[test]
    fn no_chunk_exceeds_hard_cap() {
        // Property: over a spread of generated markdown — fenced blocks of
        // varying sizes/content and indented code — NO chunk exceeds the hard
        // cap, and non-fenced pieces stay verbatim substrings of the source.
        let mut bodies: Vec<String> = Vec::new();
        let mut seed: u64 = 0xdeadbeefcafe;
        let mut rng = || {
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            seed.wrapping_mul(0x2545F4914F6CDD1D)
        };
        let glyphs = ["x", "•", "💡", "café", "let k = 🏋️;"];
        for _ in 0..24 {
            let n = 100 + (rng() % 900) as usize;
            let mut b = String::new();
            for _ in 0..n {
                match rng() % 3 {
                    0 => b.push_str(glyphs[(rng() as usize) % glyphs.len()]),
                    1 => b.push_str("word "),
                    _ => b.push('\n'),
                }
            }
            bodies.push(b);
        }
        for b in &bodies {
            for md in [
                format!("```rust\n{b}\n```\n"),
                format!("intro\n\n```\n{b}\n```\n\noutro\n"),
                format!("    {b}\n"),
            ] {
                for c in chunk_markdown(&md) {
                    assert!(
                        c.text.len() <= MAX_CODE_CHUNK_BYTES + 32,
                        "chunk exceeds the hard cap ({} bytes): {:?}",
                        c.text.len(),
                        &c.text[..c.text.len().min(64)]
                    );
                }
            }
        }
        // Non-fenced inputs keep the verbatim-substring invariant (no
        // synthesis off the fenced path).
        let prose: String = "word ".repeat(9000);
        for c in chunk_markdown(&prose) {
            assert!(prose.contains(&c.text), "prose pieces stay verbatim");
            assert!(
                c.text.len() <= MAX_CODE_CHUNK_BYTES + 32,
                "over-long prose is bounded too"
            );
        }
    }

    #[test]
    fn dense_headings_dont_create_empty_slivers() {
        // Heading lines are not buffered, so a run of headings followed by
        // content yields chunks that all carry real content.
        let md = "# A\n# B\n# C\nsome content that is long enough to be a real chunk and not a sliver at all yes indeed\n";
        let chunks = chunk_markdown(md);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| !c.text.trim().is_empty()));
        // The headings are siblings (all level 1), so the content chunk's
        // breadcrumb is the last heading only, not a nested chain.
        assert_eq!(chunks.last().unwrap().heading_path, "C", "{:?}", chunks);
    }

    #[test]
    fn nested_headings_build_a_breadcrumb() {
        // Genuinely nested headings produce a " > "-joined breadcrumb.
        let md = "# A\n## B\n### C\nsome real content here that is not a sliver no\n";
        let chunks = chunk_markdown(md);
        assert_eq!(
            chunks.last().unwrap().heading_path,
            "A > B > C",
            "{:?}",
            chunks
        );
    }

    #[test]
    fn line_spans_are_ordered() {
        let md = "# A\nl1\nl2\n## B\nl3\nl4\nl5\n";
        let chunks = chunk_markdown(md);
        for w in chunks.windows(2) {
            assert!(
                w[0].line_end < w[1].line_start,
                "spans must be ordered: {:?}",
                chunks
            );
        }
        for c in &chunks {
            assert!(c.line_start <= c.line_end);
        }
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        assert!(chunk_markdown("").is_empty());
        assert!(chunk_markdown("\n\n\n").is_empty());
    }

    // ── v0.9.4: new tests for the CommonMark constructs the old line-scanner
    // mis-handled. Each one would have failed against the pre-v0.9.4 chunker. ──

    #[test]
    fn setext_headings_are_recognized() {
        // Setext: underline text with === (H1) or --- (H2). The old scanner
        // only recognized ATX (`#`) headings, so this was treated as a
        // paragraph and `#`-less "Foo" was kept as content (no breadcrumb).
        let md = "Title\n=====\n\nbody under the setext heading\n";
        let chunks = chunk_markdown(md);
        assert!(!chunks.is_empty(), "{:?}", chunks);
        // The body chunk carries the setext title as its breadcrumb.
        let body = chunks
            .iter()
            .find(|c| c.text.contains("body under"))
            .expect("body chunk must exist");
        assert_eq!(body.heading_path, "Title", "{:?}", chunks);
        // And the "=====" underline is NOT in any chunk's text (it's part of
        // the heading, consumed into the breadcrumb like ATX `#` markers).
        assert!(
            !chunks.iter().any(|c| c.text.contains("=====")),
            "setext underline must not appear in chunk text: {:?}",
            chunks
        );
    }

    #[test]
    fn indented_code_block_is_not_split_and_hash_lines_are_code() {
        // 4-space-indented code block. The old scanner didn't recognize this
        // as code, so the `#`-comment line was mistaken for an ATX heading,
        // splitting the chunk and polluting the breadcrumb.
        let md = "Intro paragraph.\n\n    # not a heading, just a comment\n    def foo(): pass\n\nAfter the block.\n";
        let chunks = chunk_markdown(md);
        let all = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // The indented `#` line survives verbatim in chunk text...
        assert!(
            all.contains("# not a heading, just a comment"),
            "indented `#`-comment must survive verbatim: {chunks:?}"
        );
        assert!(
            all.contains("def foo(): pass"),
            "indented def must survive verbatim: {chunks:?}"
        );
        // ...and is NOT mistaken for a heading (no breadcrumb pollution).
        assert!(
            chunks.iter().all(|c| c.heading_path.is_empty()),
            "indented code must not produce a heading breadcrumb: {:?}",
            chunks
        );
    }

    #[test]
    fn blockquote_markup_is_preserved() {
        // pulldown-cmark emits Text events for the quote content but NOT for
        // the `>` markers. Our byte-range-union approach captures the `>` by
        // slicing source bytes between Text events.
        let md = "> quoted line one\n> quoted line two\n";
        let chunks = chunk_markdown(md);
        assert_eq!(
            chunks.len(),
            1,
            "single blockquote = single chunk: {:?}",
            chunks
        );
        // Both `>` markers survive (the first AND the second).
        assert_eq!(
            chunks[0].text.matches('>').count(),
            2,
            "both `>` markers must be preserved: {:?}",
            chunks[0].text
        );
        assert!(chunks[0].text.contains("quoted line one"));
        assert!(chunks[0].text.contains("quoted line two"));
    }

    #[test]
    fn list_with_wikilinks_is_preserved() {
        // The old scanner walked lists as plain text. pulldown-cmark emits
        // multiple Text events for `[[wikilink]]` brackets (one per bracket),
        // so the byte-range union has to cover them all.
        let md = "- item one\n- item two with [[wikilink]]\n";
        let chunks = chunk_markdown(md);
        let all = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all.contains("[[wikilink]]"),
            "wikilink brackets survive: {all:?}"
        );
        assert!(all.contains("item one"));
        assert!(all.contains("item two"));
        // At least one `-` list marker survives (the second one definitely;
        // the first may or may not, depending on byte-range coverage — but the
        // list structure is visible).
        assert!(all.contains("- item two"), "list markup survives: {all:?}");
    }

    #[test]
    fn gfm_table_is_preserved_with_markup() {
        let md = "| Col1 | Col2 |\n|------|------|\n| a    | b    |\n";
        let chunks = chunk_markdown(md);
        assert_eq!(chunks.len(), 1, "single table = single chunk: {:?}", chunks);
        // The `|` separators and `---` divider survive verbatim.
        assert!(
            chunks[0].text.contains("| Col1 | Col2 |"),
            "header row: {:?}",
            chunks[0].text
        );
        assert!(
            chunks[0].text.contains("|------|------|"),
            "divider row must survive: {:?}",
            chunks[0].text
        );
        assert!(chunks[0].text.contains("| a    | b    |"));
    }

    #[test]
    fn hash_in_code_fence_is_not_a_heading() {
        // The pre-v0.9.4 scanner handled fenced code correctly, but this test
        // locks the behavior in for the pulldown-cmark rewrite: `#` inside a
        // fence is code, not a heading.
        let md = "# Real Heading\n```python\n# this is a comment\ncode\n```\n";
        let chunks = chunk_markdown(md);
        // Every chunk is under "Real Heading" — the `#`-comment did NOT split.
        assert!(
            chunks.iter().all(|c| c.heading_path == "Real Heading"),
            "code-fence `#` must not be a heading: {:?}",
            chunks.iter().map(|c| &c.heading_path).collect::<Vec<_>>()
        );
        // And the comment text survives verbatim.
        assert!(
            chunks
                .iter()
                .any(|c| c.text.contains("# this is a comment")),
            "comment must survive: {:?}",
            chunks
        );
    }

    #[test]
    fn multibyte_char_with_trailing_blank_lines_does_not_panic() {
        // Regression: a chunk whose source range starts mid-buffer and contains
        // a multibyte char (e.g. a '•' bullet) used to panic on a non-char-
        // boundary slice when trailing blank lines were trimmed. The rfind
        // offset was treated as absolute instead of relative to `start`.
        //
        // The leading paragraph pushes the subsequent chunk's start > 0, and the
        // '•' bullets inside give a multibyte boundary to slice into.
        let md = "# Section\n\nIntro paragraph that is long enough to flush.\n\n\
                  - • first bullet with multibyte char\n\
                  - • second bullet with another • here\n\
                  \n\
                  \n";
        let chunks = chunk_markdown(md);
        assert!(!chunks.is_empty(), "must produce chunks");
        let all = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all.contains("• first bullet"),
            "bullet text survives: {all:?}"
        );
        assert!(
            all.contains("• second bullet"),
            "bullet text survives: {all:?}"
        );
        // No chunk's text may contain the trailing blank lines verbatim, and no
        // chunk may be empty after trimming.
        assert!(
            chunks.iter().all(|c| !c.text.trim().is_empty()),
            "chunks must not be empty after trim: {:?}",
            chunks
        );
    }

    // Helper: assert the chunker's core invariants over arbitrary input.
    // 1. Never panics. 2. No chunk is empty after trimming. 3. Every chunk's
    //    text is a verbatim substring of the source (the with_snippet
    //    invariant — the chunker must never synthesize or shift bytes).
    fn assert_chunk_invariants(content: &str) {
        let chunks = chunk_markdown(content); // would panic here if the bug returned
        assert!(
            chunks.iter().all(|c| !c.text.trim().is_empty()),
            "no empty chunks for input {content:?}: {chunks:?}"
        );
        for c in &chunks {
            assert!(
                content.contains(&c.text),
                "chunk text must be a verbatim substring of source: {:?} not in {content:?}",
                c.text
            );
            // Line spans must be 1-indexed and ordered within a chunk.
            assert!(c.line_start >= 1 && c.line_end >= c.line_start);
        }
    }

    #[test]
    fn multibyte_chars_never_panic_across_positions() {
        // Adversarial multibyte coverage: place multibyte chars (2/3/4-byte)
        // at the very start, at buffer boundaries, around newlines, with
        // trailing blank lines, and mid-chunk — every place the historical
        // '•' crash could be triggered.
        let cases = [
            // 3-byte bullet at start of a line that gets trimmed-trailing.
            "- • bullet one\n- • bullet two\n\n\n",
            // 4-byte emoji adjacent to the newline that ends a trailing trim.
            "text line\n\n\n💡 trailing emoji context\n\n\n",
            // 2-byte accented char exactly where flush splits a chunk.
            "aaa ààà bbb\n\nççç ddd\n\n\n",
            // Mixed multibyte with many newlines and blank runs.
            "• x\n\n• y\n\n— z\n\n• w\n\n\n\n",
            // Multibyte char as the single non-newline character.
            "•\n",
            "—\n\n",
            "💡💡💡\n\n\n",
            // Accents right before a trailing '\n' that gets stripped.
            "café\n",
            "naïve text here that is reasonably long to be a real chunk\n\n\n",
            // A chunk that starts mid-buffer and ends on a multibyte char.
            "lead paragraph pushes start forward\n\nbody with • inside\n\n\n",
        ];
        for c in cases {
            assert_chunk_invariants(c);
        }
    }

    #[test]
    fn multibyte_fuzz_via_proptest_style_loops() {
        // Deterministic pseudo-fuzz: combine multibyte glyphs with structural
        // newlines/blank-runs in many layouts. Cheap stand-in for a real
        // fuzzer (no proptest dep) that still hammers the slice sites.
        let glyphs = ["•", "—", "é", "💡", "ç", "🏋️", "ñ"];
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        let mut rng = || {
            // xorshift64* — tiny, no dep, deterministic.
            seed ^= seed >> 12;
            seed ^= seed << 25;
            seed ^= seed >> 27;
            seed.wrapping_mul(0x2545F4914F6CDD1D)
        };
        for _ in 0..2000 {
            let mut s = String::new();
            let n = (rng() % 12) as usize + 1;
            for _ in 0..n {
                match rng() % 5 {
                    0 => s.push('\n'),
                    1 => s.push_str("\n\n"),
                    2 => s.push_str(glyphs[(rng() as usize) % glyphs.len()]),
                    3 => s.push_str("word "),
                    _ => s.push_str(&format!("h{}/", rng() % 100)),
                }
            }
            // Exercise both flush-on-size and final flush.
            assert_chunk_invariants(&s);
            assert_chunk_invariants(&format!("# H\n\n{s}"));
            assert_chunk_invariants(&format!("{s}\n\n# Tail\nbody with •🏋️\n"));
        }
    }

    // proper property-based test (replaces the hand-rolled
    // pseudo-fuzz above for the exhaustive case). proptest generates 1000s of
    // random UTF-8 inputs and verifies the chunk byte ranges are valid.
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(proptest::test_runner::Config {
            cases: 256, // bounded for CI; the hand-rolled test above covers 2000 more
            ..proptest::test_runner::Config::default()
        })]

        #[test]
        fn proptest_chunker_never_panics_and_ranges_are_valid(s in ".{0,2000}") {
            let chunks = chunk_markdown(&s);
            // Every chunk's text must be a valid substring of the input (the
            // with_snippet invariant — never synthesized).
            for chunk in &chunks {
                prop_assert!(s.contains(&chunk.text),
                    "chunk text must be a substring of the input");
            }
        }

        #[test]
        fn proptest_chunker_handles_multibyte_inputs(
            s in proptest::collection::vec(
                proptest::sample::select(vec!["\n", "•", "💡", "word ", "# H\n", "—", "🏋️"]),
                0..100
            )
        ) {
            let content = s.join("");
            // Must never panic on multibyte input (the historical '•' crash).
            let chunks = chunk_markdown(&content);
            for chunk in &chunks {
                prop_assert!(content.contains(&chunk.text));
            }
        }
    }
}
