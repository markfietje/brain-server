import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { shellRoot } from './fixtures';

/**
 * M3-e — THE THEME RETUNE (plan `24_THEME_DSH_FAITHFUL`): the pins.
 *
 * The round adopts the dsh harness's DESIGN LANGUAGE — a neutral-first
 * bluish-grey ramp, one accent used sparingly, `font-weight: 500` as THE
 * emphasis, 14px/13px chrome type with the 16px law on every text control,
 * hairline borders, quiet scrollbars — and refuses its BRAND (no DeepSeek
 * mark, no `--dsw-static-deepseek-*` value; plan 24 §0, the legal line).
 * The Brain teal stays (D1's default).
 *
 * ## What jsdom can and cannot compute — measured 2026-10-27, not assumed
 *
 * The pins below load the REAL `src/app.css` text into the test document and
 * read `getComputedStyle`. A probe against jsdom 30 established the ground
 * rules this file is written against:
 *
 *   - UNLAYERED rules cascade and compute, attribute selectors included
 *     (`[aria-current="page"] { font-weight: 500 }` → `"500"`), and `1rem`
 *     resolves to `"16px"`.
 *   - `@layer` blocks parse but are NOT entered for cascade, so a Tailwind
 *     utility (every one of which lives in `@layer utilities`) never reaches
 *     a computed value here. That is also why the two type laws this round
 *     lands are written as unlayered `app.css` rules rather than classes:
 *     in a real engine an unlayered declaration beats any layered one
 *     regardless of specificity, so the unlayered rule IS the guarantee that
 *     no utility can lower a control's size — and jsdom can compute it.
 *   - `@media (prefers-color-scheme: dark)` is never entered, so the dark
 *     scheme is proven STRUCTURALLY — custom-property presence in the parsed
 *     block — and the AA claim for both schemes is carried by contrast
 *     ARITHMETIC over the token values (WCAG 2.x relative luminance), not by
 *     axe, whose `color-contrast` rule reports `incomplete` under jsdom.
 *   - An inline custom property set through `style.setProperty` reads back
 *     through `getComputedStyle(...).getPropertyValue`, which is what the
 *     scrollbar pin asserts.
 *
 * No screenshot, no snapshot, no new dependency: the CSSOM parser is jsdom's
 * own, and the contrast math is the standard formula written out.
 */

const APP_CSS = readFileSync(join(shellRoot, 'src', 'app.css'), 'utf8');

// ── the parsed stylesheet ────────────────────────────────────────────────

type Tokens = Map<string, string>;

/** Every `--custom-property` a declaration block carries, name → value. */
function customProperties(style: CSSStyleDeclaration): Tokens {
	const tokens: Tokens = new Map();
	for (let i = 0; i < style.length; i += 1) {
		const name = style[i];
		if (name !== undefined && name.startsWith('--')) {
			tokens.set(name, style.getPropertyValue(name).trim());
		}
	}
	return tokens;
}

function isDarkMedia(rule: CSSRule): rule is CSSMediaRule {
	return (
		rule instanceof CSSMediaRule &&
		rule.media.mediaText.replace(/\s+/g, '').includes('prefers-color-scheme:dark')
	);
}

/**
 * The two scheme blocks of `app.css`, read through the CSSOM: the light
 * scheme is every TOP-LEVEL `:root {}` rule; the dark scheme is every
 * `:root {}` rule inside a TOP-LEVEL `@media (prefers-color-scheme: dark)`.
 * The platform flavours (`:root[data-platform=…]`) and the nested macOS
 * reduced-transparency fallbacks are deliberately NOT the scheme blocks —
 * they are the platform layer, which this round leaves alone.
 */
function schemeBlocks(): { light: Tokens; dark: Tokens } {
	const style = document.createElement('style');
	style.textContent = APP_CSS;
	document.head.append(style);
	const sheet = style.sheet;
	if (!sheet) throw new Error('app.css did not parse into a stylesheet');
	const light: Tokens = new Map();
	const dark: Tokens = new Map();
	for (const rule of sheet.cssRules) {
		if (rule instanceof CSSStyleRule && rule.selectorText === ':root') {
			for (const [k, v] of customProperties(rule.style)) light.set(k, v);
		} else if (isDarkMedia(rule)) {
			for (const inner of rule.cssRules) {
				if (inner instanceof CSSStyleRule && inner.selectorText === ':root') {
					for (const [k, v] of customProperties(inner.style)) dark.set(k, v);
				}
			}
		}
	}
	style.remove();
	return { light, dark };
}

// ── colour arithmetic (WCAG 2.x) ─────────────────────────────────────────

type Rgb = readonly [number, number, number];

/** `#rgb`, `#rrggbb`, `rgb(r, g, b)`; opaque only — an alpha value is not a pair. */
function parseColor(value: string): Rgb | null {
	const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(value.trim());
	if (hex) {
		const h = hex[1] ?? '';
		const full = h.length === 3 ? [...h].map((c) => c + c).join('') : h;
		return [0, 2, 4].map((i) => Number.parseInt(full.slice(i, i + 2), 16)) as unknown as Rgb;
	}
	const rgb = /^rgb\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)$/i.exec(value.trim());
	if (rgb) return [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])];
	return null;
}

function relativeLuminance([r, g, b]: Rgb): number {
	const channel = (c: number): number => {
		const s = c / 255;
		return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
	};
	return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrastRatio(a: Rgb, b: Rgb): number {
	const la = relativeLuminance(a);
	const lb = relativeLuminance(b);
	return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** `--accent` is a colour; `--background: var(--bg)` and `--shadow-xs: 0 1px …` are not. */
function isColorToken(value: string): boolean {
	return /^(#[0-9a-f]{3,8}|rgba?\()/i.test(value.trim());
}

// ── E-1 · D2: the neutrals, both schemes ─────────────────────────────────

/**
 * Scheme parity, both directions: every colour token the light `:root`
 * declares is declared again in the dark `:root`, and nothing is dark-only.
 * The set is DERIVED from the sheet (a token is "colour" when its value is a
 * literal `#…`/`rgb…`), so the pin fails closed on growth — a new colour
 * token added to one scheme reddens here by name — and nothing is pinned
 * about the macOS glass tints or the Windows accent, which are platform
 * flavours over these tokens, not schemes.
 */
test('every_colour_token_is_declared_in_both_schemes', () => {
	const { light, dark } = schemeBlocks();
	// Anti-vacuity: the extraction saw the real blocks, not two empty maps.
	expect(light.size, 'the light :root block carries no custom properties — the extraction missed it').toBeGreaterThan(10);
	expect(dark.size, 'the dark :root block carries no custom properties — the extraction missed it').toBeGreaterThan(10);

	const lightColours = [...light].filter(([, v]) => isColorToken(v)).map(([k]) => k);
	const darkColours = [...dark].filter(([, v]) => isColorToken(v)).map(([k]) => k);

	const missingInDark = lightColours.filter((k) => !dark.has(k));
	const missingInLight = darkColours.filter((k) => !light.has(k));
	expect(
		missingInDark,
		'colour tokens the light scheme declares and the dark scheme does not — a page that flips to dark keeps the light value here'
	).toEqual([]);
	expect(
		missingInLight,
		'tokens the dark scheme declares and the light scheme never does — a dark-only token has no light reading'
	).toEqual([]);

	// The retuned family is named, so a rename cannot slip past the derived set.
	for (const token of [
		'--bg',
		'--surface',
		'--surface-2',
		'--text',
		'--text-muted',
		'--border',
		'--input-border',
		'--input-border-hover',
		'--sidebar',
		'--sidebar-active',
		'--sidebar-hover',
		'--sidebar-border',
		'--accent',
		'--focus',
		'--ok',
		'--error'
	]) {
		expect(light.has(token), `${token} is not declared in the light scheme`).toBe(true);
		expect(dark.has(token), `${token} is not declared in the dark scheme`).toBe(true);
	}
});

/**
 * The AA arithmetic, both schemes: 4.5:1 on every text pair the chrome
 * actually renders, 3.0:1 on the UI pairs (the status dots on the surfaces
 * they sit on, the focus ring, the accent as a boundary). The pairs are the
 * MEASURED ones — `text-brand` is rendered on the page `--bg` (the recall and
 * governance links, the audit badge) as well as on the dialog `--surface`,
 * the muted text sits on every sidebar fill, the badge's ink sits on the
 * accent — so this is the surface the user reads, not a swatch table.
 *
 * Borders are hairline by design (plan 24 §1, the dsh language) and are not
 * a pair here: an input is identified by its label and its focus ring, and
 * the ring IS pinned at 3.0.
 */
const TEXT_PAIRS: ReadonlyArray<readonly [string, string]> = [
	['--text', '--bg'],
	['--text', '--surface'],
	['--text', '--surface-2'],
	['--text', '--sidebar'],
	['--text', '--sidebar-active'],
	['--text', '--sidebar-hover'],
	['--text-muted', '--bg'],
	['--text-muted', '--surface'],
	['--text-muted', '--surface-2'],
	['--text-muted', '--sidebar'],
	['--text-muted', '--sidebar-active'],
	['--text-muted', '--sidebar-hover'],
	['--primary-foreground', '--accent'],
	['--accent', '--bg'],
	['--accent', '--surface'],
	['--error', '--bg'],
	['--error', '--surface']
];

const UI_PAIRS: ReadonlyArray<readonly [string, string]> = [
	['--ok', '--bg'],
	['--ok', '--surface'],
	['--ok', '--sidebar'],
	['--error', '--sidebar'],
	['--focus', '--bg'],
	['--focus', '--surface'],
	['--accent', '--sidebar']
];

function auditContrast(
	scheme: 'light' | 'dark',
	tokens: Tokens,
	pairs: ReadonlyArray<readonly [string, string]>,
	floor: number
): string[] {
	const failures: string[] = [];
	for (const [fg, bg] of pairs) {
		const fgValue = tokens.get(fg);
		const bgValue = tokens.get(bg);
		const fgRgb = fgValue === undefined ? null : parseColor(fgValue);
		const bgRgb = bgValue === undefined ? null : parseColor(bgValue);
		if (fgRgb === null || bgRgb === null) {
			failures.push(`${scheme}: ${fg} on ${bg} — not two opaque colours (${fgValue} / ${bgValue})`);
			continue;
		}
		const ratio = contrastRatio(fgRgb, bgRgb);
		if (ratio < floor) {
			failures.push(
				`${scheme}: ${fg} (${fgValue}) on ${bg} (${bgValue}) = ${ratio.toFixed(2)}:1, below ${floor}:1`
			);
		}
	}
	return failures;
}

test('every_text_and_ui_pair_holds_aa_in_both_schemes', () => {
	const { light, dark } = schemeBlocks();
	const failures = [
		...auditContrast('light', light, TEXT_PAIRS, 4.5),
		...auditContrast('dark', dark, TEXT_PAIRS, 4.5),
		...auditContrast('light', light, UI_PAIRS, 3.0),
		...auditContrast('dark', dark, UI_PAIRS, 3.0)
	];
	expect(
		failures,
		'AA pairs below the floor — retune the token, never the pair list (the pairs are what the chrome renders)'
	).toEqual([]);
});
