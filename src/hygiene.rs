//! Ingest capture hygiene: stop the server from silently
//! storing model reasoning traces and foreign systems' synthesis prompts.
//!
//! Two pure, unit-tested transforms applied at the raw-text ingest doors
//! (`/ingest/memory`, `/add`):
//!   - [`strip_reasoning_blocks`]: removes paired reasoning-tag blocks
//!     (`<thinking>…</thinking>`, `<think>`, `<reasoning>`, `<reflection>`,
//!     `<analysis>`) including an unclosed trailing block.
//!   - [`should_skip`]: drops an entry whose text starts with a configured
//!     prefix (`BRAIN_INGEST_SKIP_PATTERNS`) — the dream-prompt mechanism.
//!
//! ponytail: this is an allow-list of tag names the audit proved are leaking,
//! NOT a general "AI-text" detector. Extend [`REASONING_TAGS`] as new
//! delimiters appear; do not build a content classifier. Curated ingest
//! (`/ingest`, `/ingest/markdown`) is deliberately NOT filtered here
//! (operator-authored content; false-positive risk). This stops the bleeding;
//! cleaning the historical store is a separate ROADMAP sweep.

/// Tag names whose `<name>…</name>` blocks are stripped together with their
/// content. Case-insensitive. Sourced from the round-5 audit: Anthropic
/// `<thinking>`, DeepSeek `<think>`, plus the common reasoning/reflection/
/// analysis variants emitted by reasoning models.
const REASONING_TAGS: &[&str] = &["thinking", "think", "reasoning", "reflection", "analysis"];

/// Remove reasoning/trace blocks from `text`, returning the cleaned text.
///
/// A block is `<tag …>` … `</tag>` (open tag may carry attributes); the open
/// tag, its content, and the close tag are all removed. An open tag with **no**
/// matching close is treated as a truncated trace and dropped to end-of-string
/// (the conservative privacy choice — a captured transcript that opens a
/// reasoning block and gets cut off is reasoning, not memory). Non-matching
/// text is passed through verbatim.
pub fn strip_reasoning_blocks(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < text.len() {
        if let Some((after_open, tag)) = match_open_tag(&lower, i) {
            // Look for the matching close tag in the remainder.
            let close = format!("</{tag}>");
            if let Some(rel) = lower[after_open..].find(&close) {
                i = after_open + rel + close.len();
                continue;
            } else {
                // Unclosed reasoning block: drop the rest of the input.
                break;
            }
        } else {
            // Copy one character (UTF-8 safe) and advance.
            let ch = text[i..].chars().next().expect("non-empty slice");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// If a reasoning open tag starts at byte `i` in `lower` (already lowercased),
/// return `(index_just_past_the_open_tag, matched_tag_name)`; else `None`.
/// Accepts `<tag>` and `<tag …>` (attributes), but rejects `<tagx>` so a
/// longer identifier isn't matched as a prefix of a tag.
fn match_open_tag(lower: &str, i: usize) -> Option<(usize, &'static str)> {
    let rest = &lower[i..];
    if !rest.starts_with('<') {
        return None;
    }
    for tag in REASONING_TAGS {
        let prefix = format!("<{tag}");
        if let Some(after) = rest.strip_prefix(&prefix) {
            // The byte after `<tag` must be `>` (plain) or whitespace (attrs).
            let next_ch = after.chars().next();
            match next_ch {
                Some('>') => return Some((i + prefix.len() + 1, tag)),
                Some(c) if c.is_whitespace() => {
                    // Attributes: skip to the closing '>' of the open tag.
                    if let Some(gt) = after.find('>') {
                        return Some((i + prefix.len() + gt + 1, tag));
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// True if `text` (ignoring leading whitespace) begins with any of the
/// configured `patterns`. Case-sensitive — dream/synthesis prompts are exact
/// strings, and case-folding them would widen false positives.
pub fn should_skip(text: &str, patterns: &[String]) -> bool {
    let t = text.trim_start();
    patterns
        .iter()
        .any(|p| !p.trim().is_empty() && t.starts_with(p.trim()))
}

/// Read skip-pattern prefixes from `BRAIN_INGEST_SKIP_PATTERNS`, separated by
/// newlines or commas. Empty/unset → no patterns (opt-in; default behavior
/// unchanged). Read per ingest — cheap, and env is static over the process life.
pub fn skip_patterns() -> Vec<String> {
    std::env::var("BRAIN_INGEST_SKIP_PATTERNS")
        .ok()
        .map(|raw| {
            raw.split([',', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Combined door check for one ingest entry: returns `None` if the entry should
/// be dropped (skip-pattern match), else `Some(cleaned_text)` with reasoning
/// blocks stripped. `patterns` is supplied by the caller (the handlers pass
/// [`skip_patterns`]) so the logic is testable without touching the process env.
pub fn clean(text: &str, patterns: &[String]) -> Option<String> {
    if should_skip(text, patterns) {
        return None;
    }
    Some(strip_reasoning_blocks(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_paired_thinking_block_with_content() {
        let s = "outer <thinking>secret reasoning trace</thinking> tail";
        assert_eq!(strip_reasoning_blocks(s), "outer  tail");
    }

    #[test]
    fn strips_is_think_tag() {
        assert_eq!(strip_reasoning_blocks("a <think>x</think> b"), "a  b");
    }

    #[test]
    fn strips_case_insensitive_and_attributes() {
        assert_eq!(
            strip_reasoning_blocks("<THINKING type=\"deep\">r</THINKING>ok"),
            "ok"
        );
    }

    #[test]
    fn unclosed_block_drops_to_end() {
        // A truncated trace: no close tag → the rest is reasoning, drop it.
        assert_eq!(
            strip_reasoning_blocks("keep <thinking>leaked rest of transcript"),
            "keep "
        );
    }

    #[test]
    fn no_tags_passthrough_unchanged() {
        let s = "plain memory with <html> and ampersand & stuff";
        assert_eq!(strip_reasoning_blocks(s), s);
    }

    #[test]
    fn does_not_match_tag_prefix_of_longer_word() {
        // `<thinkingx>` is NOT the `<thinking>` tag.
        assert_eq!(
            strip_reasoning_blocks("a <thinkingx>b</thinkingx> c"),
            "a <thinkingx>b</thinkingx> c"
        );
    }

    #[test]
    fn multiple_blocks_all_stripped() {
        assert_eq!(
            strip_reasoning_blocks("<thinking>1</thinking>x<reflection>2</reflection>y"),
            "xy"
        );
    }

    #[test]
    fn should_skip_matches_configured_prefix() {
        let p = vec!["Write a dream diary entry from these".to_string()];
        assert!(should_skip(
            "Write a dream diary entry from these fragments: a, b",
            &p
        ));
        assert!(!should_skip("A normal memory about dreams", &p));
    }

    #[test]
    fn should_skip_ignores_leading_whitespace_and_empty_patterns() {
        let p = vec!["  x  ".to_string(), "".to_string()];
        assert!(should_skip("   x is a dream prompt", &p));
    }

    #[test]
    fn clean_drops_skip_matches_and_strips_others() {
        let p = vec!["DREAM::".to_string()];
        assert_eq!(clean("DREAM:: synth", &p), None);
        assert_eq!(
            clean("real <thinking>r</thinking>mem", &p),
            Some("real mem".into())
        );
        // No patterns -> never skip, still strips.
        assert_eq!(
            clean("real <thinking>r</thinking>mem", &[]),
            Some("real mem".into())
        );
    }
}
