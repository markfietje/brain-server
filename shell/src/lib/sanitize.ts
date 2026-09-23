/**
 * The invis-char/spoof-stripper behavior (D9) — a shared sanitize helper
 * from day one (the Dioxus discipline). Renderer-facing text (the packs'
 * instruction/description strings) passes through BEFORE it can be put on
 * screen: invisible/zero-width/bidi-control characters are stripped, so a
 * hostile template cannot spoof labels or smuggle rendering directives.
 * This helper only ever REMOVES — it never injects markup (the {@html} ban
 * is lint-enforced).
 */

// BiDi controls + zero-width/invisible characters (Unicode TR9 + ZWSP
// family). Deliberately a closed, reviewed set — not a blanket category
// filter that would eat legitimate scripts.
const INVISIBLE = /[\u200B-\u200F\u202A-\u202E\u2060-\u2064\u206A-\u206F\uFEFF\u00AD]/g;

export function stripInvisible(text: string): string {
	return text.replace(INVISIBLE, '');
}

/** The one door rendered pack text passes through. */
export function sanitizeTemplateText(text: string): string {
	return stripInvisible(text);
}
