/**
 * The canonical invisible-Unicode boundary (D9). Renderer-facing text passes
 * through this pure, idempotent removal step before it can be put on screen.
 * This is deliberately not an HTML sanitizer: it neither parses nor emits
 * markup, and the existing `{@html}` lint ban remains the markup boundary.
 */

// This expression is the exact scalar membership set in
// plugin/fixtures/invisible-classes.json and src/strip_invisible.rs. The `u`
// flag makes the supplementary ranges operate on code points rather than
// UTF-16 code units.
const INVISIBLE =
	/[\u00AD\u034F\u061C\u115F-\u1160\u180E\u200B-\u200F\u202A-\u202E\u2060-\u2063\u2066-\u2069\uFE00-\uFE0F\uFEFF\uFFF9-\uFFFB\u{E0000}-\u{E007F}\u{E0100}-\u{E01EF}]/gu;

export function stripInvisible(text: string): string {
	return text.replace(INVISIBLE, '');
}

/** The one door rendered pack text passes through. */
export function sanitizeTemplateText(text: string): string {
	return stripInvisible(text);
}
