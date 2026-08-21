//! The one invisible-Unicode strip boundary, shared by every binary.
//!
//! Moved out of the server-private `screen.rs` into the lib so the MCP binary +
//! `brain` CLI close the same bidi/zero-width smuggling class the server screen
//! and wasm client already close. One definition, four surfaces.
//!
//! ponytail: stripping happens at render/output boundaries only — storage stays
//! verbatim (legitimate invisible Unicode is preserved at rest).

/// Strip every invisibly-smuggled Unicode char. The canonical set: tag-block
/// (U+E0000–E007F), variation-selectors (U+FE00–U+FE0F + supplemental
/// U+E0100–U+E01EF), the zero-width set (U+200B/200C/200D/2060), the legacy
/// BOM / soft-hyphen / grapheme-joiner members, and the `Bidi_Control` set
/// (U+200E/200F, U+202A–202E, U+2066–2069, U+061C) — the Trojan Source / W3C
/// TR#20 bidi smuggling class. Idempotent + pure.
pub fn strip_invisible(input: &str) -> String {
    input.chars().filter(|&c| !is_invisible(c)).collect()
}

/// True for a char that is invisible in normal rendering and used to smuggle
/// instruction/exfiltration bytes or defeat substring matching.
pub fn is_invisible(c: char) -> bool {
    let cp = c as u32;
    // Tag block (U+E0000–E007F) — smuggles arbitrary bytes invisibly.
    (0xE0000..=0xE007F).contains(&cp)
        // Variation selectors (BMP + supplemental) — variant smuggling.
        || (0xFE00..=0xFE0F).contains(&cp)
// the supplementary variation-selector
        // range is used by emoji/ideographic variation sequences.
        || (0xE0100..=0xE01EF).contains(&cp)
        // Bidi controls — the Trojan Source / W3C TR#20 directional class.
        || (0x200E..=0x200F).contains(&cp)
        || (0x202A..=0x202E).contains(&cp)
        || (0x2066..=0x2069).contains(&cp)
// ARABIC LETTER MARK (U+061C) — a
        // `Bidi_Control` codepoint closing the last gap in the documented set.
        || cp == 0x061C
        // Zero-width space / non-joiner / joiner + word joiner.
        || matches!(cp, 0x200B | 0x200C | 0x200D | 0x2060)
        // Legacy members: BOM, separators, soft hyphen, combining grapheme joiner.
        || matches!(cp, 0xFEFF | 0x2061 | 0x2062 | 0x2063 | 0x00AD | 0x034F)
}

/// Strip C0 control chars (except `\t`/`\n`), DEL, and C1 control chars. Used
/// for terminal-facing output (CLI prints + MCP text payloads) where an ANSI
/// escape smuggled through stored content could script the operator's shell.
/// Deliberately narrower than [`strip_invisible`] (keeps tab/newline).
/// Idempotent + pure.
pub fn strip_control_chars(input: &str) -> String {
    input
        .chars()
        .filter(|&c| {
            let cp = c as u32;
            !((cp < 0x20 && c != '\t' && c != '\n') || cp == 0x7F || (0x80..=0x9F).contains(&cp))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // the two added classes (U+061C ALM, the U+E0100–U+E01EF supplemental VS
    // range) must join the existing strip set.
    #[test]
    fn arabic_letter_mark_stripped() {
        assert!(is_invisible('\u{061C}'));
        assert_eq!(strip_invisible("a\u{061C}b"), "ab");
    }

    #[test]
    fn supplementary_variation_selectors_stripped() {
        assert!(is_invisible('\u{E0100}'));
        assert!(is_invisible('\u{E01EF}'));
        assert_eq!(strip_invisible("x\u{E0100}y\u{E01EF}z"), "xyz");
    }

    #[test]
    fn existing_invisible_classes_still_stripped() {
        for c in [
            '\u{200B}',
            '\u{202E}',
            '\u{061C}',
            '\u{E0001}',
            '\u{FE00}',
            '\u{E01EF}',
        ] {
            assert!(is_invisible(c), "expected {c:?} invisible");
        }
    }

    #[test]
    fn control_chars_stripped_preserves_tab_newline() {
        assert_eq!(strip_control_chars("a\u{0000}b\u{001B}\u{007F}c"), "abc");
        assert_eq!(strip_control_chars("a\u{0085}b\u{009F}c"), "abc"); // C1
        // Tab + newline survive (legit whitespace).
        assert_eq!(strip_control_chars("a\tb\nc"), "a\tb\nc");
    }

    #[test]
    fn strip_fns_idempotent() {
        for input in [
            "a\u{061C}b\u{E0100}c\u{202E}d\u{001B}e",
            "plain text with spaces  \t\n",
            "日本語 héllo",
            "",
        ] {
            assert_eq!(
                strip_invisible(&strip_invisible(input)),
                strip_invisible(input)
            );
            assert_eq!(
                strip_control_chars(&strip_control_chars(input)),
                strip_control_chars(input)
            );
        }
    }

    #[test]
    fn control_strip_preserves_visible_unicode() {
        assert_eq!(
            strip_control_chars("héllo wörld 日本語 ✓"),
            "héllo wörld 日本語 ✓"
        );
        // NBSP is NOT a control char and is preserved (legit content).
        assert_eq!(strip_control_chars("a\u{00A0}b"), "a\u{00A0}b");
    }
}
