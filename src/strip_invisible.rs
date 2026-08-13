//! The one invisible-Unicode strip boundary, shared by every binary.
//!
//! v1.20.24 "Sweep": [`strip_invisible`]/[`is_invisible`] moved out of the
//! server-private `screen.rs` into the lib so the MCP binary + `brain` CLI
//! close the same bidi/zero-width smuggling class the server screen and the
//! wasm client already close. One definition, four surfaces.
//!
//! ponytail: stripping happens at render/output boundaries only — storage
//! stays verbatim (a legitimate user's invisible Unicode is preserved at rest).

/// Strip every invisibly-smuggled Unicode char. The canonical set: tag-block
/// (U+E0000–E007F), variation-selectors (U+FE00–U+FE0F), the zero-width set
/// (U+200B/200C/200D/2060), the legacy BOM / soft-hyphen / grapheme-joiner
/// members, and the Unicode `Bidi_Control` set (U+200E/200F marks,
/// U+202A–202E embed/override, U+2066–2069 isolates) — the Trojan Source /
/// W3C TR#20 bidi smuggling class. Idempotent + pure.
pub fn strip_invisible(input: &str) -> String {
    input.chars().filter(|&c| !is_invisible(c)).collect()
}

/// True for a char that is invisible in normal rendering and used to smuggle
/// instruction/exfiltration bytes or defeat substring matching.
pub fn is_invisible(c: char) -> bool {
    let cp = c as u32;
    // Tag block (U+E0000–E007F) — smuggles arbitrary bytes invisibly.
    (0xE0000..=0xE007F).contains(&cp)
        // Variation selectors (U+FE00–FE0F) — variant smuggling.
        || (0xFE00..=0xFE0F).contains(&cp)
        // Bidi controls (U+200E–U+200F, U+202A–U+202E, U+2066–U+2069) — the
        // Trojan Source / W3C TR#20 directional-override + isolate class.
        || (0x200E..=0x200F).contains(&cp)
        || (0x202A..=0x202E).contains(&cp)
        || (0x2066..=0x2069).contains(&cp)
        // Zero-width space / non-joiner / joiner + word joiner.
        || matches!(cp, 0x200B | 0x200C | 0x200D | 0x2060)
        // Legacy members (BOM, function/abbreviation/invisible separators,
        // soft hyphen, combining grapheme joiner).
        || matches!(cp, 0xFEFF | 0x2061 | 0x2062 | 0x2063 | 0x00AD | 0x034F)
}
