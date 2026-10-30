import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, expect, test, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { shellRoot } from './fixtures';
import { APP_CSS, schemeBlocks, type Tokens } from './theme-css';

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

// ── the render harness ───────────────────────────────────────────────────
//
// The frame and the home page read the wire at mount; the construction-order
// law (chat-surface.test.ts) applies: the typed client captures `fetch` at
// creation, so the stub is installed at module top and every component
// module is imported DYNAMICALLY below it. The responder answers the glance
// reads with empty-but-well-formed bodies and refuses everything else.

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));
vi.mock('$app/state', () => ({ page: { url: new URL('http://127.0.0.1/'), params: {} } }));
vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string) => {
		if (command === 'kernel_token') return null;
		throw new Error(`unexpected command ${command}`);
	})
}));

const json = (body: unknown, status = 200): Response =>
	new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
vi.stubGlobal(
	'fetch',
	vi.fn(async (input: RequestInfo | URL) => {
		const path = new URL(input instanceof Request ? input.url : String(input)).pathname;
		if (path === '/proposals') return json([]);
		if (path === '/chat/sessions') return json({ sessions: [], total: 0 });
		if (path === '/recall') return json({ hits: [], decision: 'ok' });
		return new Response('not found', { status: 404 });
	})
);

/** The real stylesheet, attached for one test; removed after it. */
let attachedSheet: HTMLStyleElement | null = null;
function attachAppCss(): void {
	attachedSheet = document.createElement('style');
	attachedSheet.textContent = APP_CSS;
	document.head.append(attachedSheet);
}

afterEach(() => {
	attachedSheet?.remove();
	attachedSheet = null;
	document.body.style.removeProperty('font-size');
});

async function renderFrame(): Promise<void> {
	const host = (await import('./AppShellHost.svelte')).default;
	render(host);
	await waitFor(() => expect(screen.getByTestId('host-content')).toBeTruthy());
}

async function renderHome(): Promise<void> {
	const page = (await import('../src/routes/+page.svelte')).default;
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	render(page as any);
	await waitFor(() => expect(screen.getByTestId('chat-composer')).toBeTruthy());
}

const computed = (el: Element): CSSStyleDeclaration => window.getComputedStyle(el);

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

// ── E-2 · D3: the type scale — 500 is THE emphasis, 16px is the control law ─

/**
 * The 500 law (plan 24 D3; dsh's Figma-510 rule): the ACTIVE sidebar row —
 * the current surface in the nav, the open conversation in the history — is
 * the one place the chrome carries `font-weight: 500`; every other row is
 * regular. The weight is one unlayered `app.css` rule keyed on
 * `[aria-current]` inside the sidebar, so emphasis follows the current
 * state and cannot be sprinkled per element.
 */
test('the_active_sidebar_row_computes_the_emphasis_weight_and_the_rest_do_not', async () => {
	attachAppCss();
	await renderFrame();
	const active = screen.getByTestId('nav-chat');
	const idle = screen.getByTestId('nav-memory');
	expect(active.getAttribute('aria-current'), 'the harness renders at `/`, so Chat is the current row').toBe('page');
	expect(idle.getAttribute('aria-current')).toBeNull();
	expect(computed(active).fontWeight, 'the current row is the emphasis weight').toBe('500');
	expect(computed(idle).fontWeight, 'an idle row is regular — 500 is reserved for the current state').toBe('400');
});

/**
 * The 16px law (app.css's "no zoom-on-focus" platform law): the composer
 * textarea and every input and select compute at least 16px WHATEVER
 * surrounds them. The harness sets the body to 13px — the size a UA
 * small-control default or a shrunk chrome ancestor would hand down — and
 * proves it bit (a plain span inherits 13px) before asserting that the
 * controls do not: the unlayered `button, select, input, textarea` rule
 * resets them to `1rem`, which is why no `text-sm` utility on a control can
 * lower it in a real engine either (unlayered beats layered).
 */
function fontPx(el: Element): number {
	const value = computed(el).fontSize;
	const px = /^([\d.]+)px$/.exec(value);
	if (!px) throw new Error(`font-size did not resolve to px: "${value}"`);
	return Number(px[1]);
}

test('the_composer_and_every_text_control_compute_at_least_16px_whatever_surrounds_them', async () => {
	attachAppCss();
	document.body.style.fontSize = '13px';
	const probe = document.createElement('span');
	document.body.append(probe);
	expect(fontPx(probe), 'the harness must bite: a plain span inherits the 13px ancestor').toBe(13);
	probe.remove();

	await renderHome();
	await renderFrame();
	// Open the settings dialog so its input and native select are in the tree.
	screen.getByTestId('nav-settings').click();
	await waitFor(() => expect(document.getElementById('setting-origin')).toBeTruthy());

	const controls = [...document.querySelectorAll<HTMLElement>('input, select, textarea')].filter(
		(el) => !(el instanceof HTMLInputElement && ['hidden', 'submit', 'reset', 'button', 'image'].includes(el.type))
	);
	const names = controls.map((el) => `${el.tagName.toLowerCase()}${el.id ? `#${el.id}` : ''}`);
	expect(names, 'the composer, the origin input and the locale select must all be in the tree').toEqual(
		expect.arrayContaining(['textarea#chat-composer', 'input#setting-origin', 'select#setting-locale'])
	);

	const small = controls
		.map((el) => ({ el, px: fontPx(el) }))
		.filter(({ px }) => px < 16)
		.map(({ el, px }) => `${el.tagName.toLowerCase()}${el.id ? `#${el.id}` : ''} computes ${px}px`);
	expect(small, 'text controls below 16px — the no-zoom-on-focus law').toEqual([]);
});

// ── E-3 · D4: quiet scrollbars on exactly the two measured scroll columns ──

const APPSHELL_SOURCE = readFileSync(
	join(shellRoot, 'src', 'lib', 'components', 'AppShell.svelte'),
	'utf8'
);

/**
 * A scrollbar is a pointer affordance (plan 24 D4, the dsh linger): the two
 * scroll columns of the frame — the session region and the main column,
 * measured at AppShell.svelte:701 and :832 on 2026-10-27 — draw their thumb
 * only while the pointer is inside the column and keep it ~2 s after it
 * leaves. The state is ONE custom property (`--scrollbar-thumb`) the
 * stylesheet reads into `scrollbar-color`; the pin asserts the property
 * flip, never pixels (jsdom lays nothing out). Nothing listens to scroll:
 * pointer presence is the affordance, not scroll position.
 */
test('the_two_scroll_columns_draw_their_bar_only_while_the_pointer_is_inside_and_linger_two_seconds', async () => {
	const { flushSync } = await import('svelte');
	await renderFrame();

	const columns = [...document.querySelectorAll<HTMLElement>('[data-slot="scroll-column"]')];
	expect(
		columns.map((el) => el.getAttribute('data-testid') ?? el.tagName.toLowerCase()),
		'exactly the two measured regions are scroll columns: the session region and <main>'
	).toEqual(['session-region', 'main']);

	const thumb = (el: HTMLElement): string => computed(el).getPropertyValue('--scrollbar-thumb').trim();

	for (const column of columns) {
		const name = column.getAttribute('data-testid') ?? column.tagName.toLowerCase();
		expect(thumb(column), `${name}: idle, the thumb is transparent`).toBe('transparent');

		column.dispatchEvent(new Event('pointerenter'));
		flushSync();
		expect(thumb(column), `${name}: pointer inside, the thumb is drawn`).toBe('var(--scrollbar-thumb-live)');

		vi.useFakeTimers();
		try {
			column.dispatchEvent(new Event('pointerleave'));
			flushSync();
			expect(thumb(column), `${name}: just left, the bar lingers`).toBe('var(--scrollbar-thumb-live)');
			vi.advanceTimersByTime(1999);
			flushSync();
			expect(thumb(column), `${name}: 1999 ms after leaving, still lingering`).toBe('var(--scrollbar-thumb-live)');
			vi.advanceTimersByTime(1);
			flushSync();
			expect(thumb(column), `${name}: 2000 ms after leaving, quiet again`).toBe('transparent');
		} finally {
			vi.useRealTimers();
		}
	}

	// No scroll listener anywhere in the frame — the affordance is pointer presence.
	expect(
		/onscroll|addEventListener\(\s*['"]scroll['"]/.test(APPSHELL_SOURCE),
		'AppShell must register no scroll listener — plan 24 D4 forbids a scroll-driven bar'
	).toBe(false);
	// Both scheme blocks carry the live colour the drawn state resolves to.
	const { light, dark } = schemeBlocks();
	expect(light.get('--scrollbar-thumb-live'), 'the light scheme names the drawn thumb colour').toBeDefined();
	expect(dark.get('--scrollbar-thumb-live'), 'the dark scheme names the drawn thumb colour').toBeDefined();
});

// ── E-4 · D5: the text audit — "not much text", five catalogs, eight families ─

/**
 * Plan 21 §3's friction law, made structural: "labels ≤ 2 words; verdicts
 * one line". The audit (plan 24 D5) classifies EVERY key in the eight chrome
 * families and the pin enforces the class's bound across all five catalogs:
 *
 *   - **label**   — visible chrome labels (nav, buttons, headings, chips,
 *                   titles): ≤ 2 CONTENT words. Articles and prepositions do
 *                   not count — "Palette de commandes" is a two-concept
 *                   label in French exactly as "Command palette" is in
 *                   English, and the law is about what the eye parses, not
 *                   about a language's function words.
 *   - **hint**    — placeholders, helper lines, dialog descriptions, group
 *                   headings and accessible names: ≤ 6 words, every word.
 *   - **verdict** — one-line state reports (saved, cleared, configured, up
 *                   to date, the version line): ≤ 12 words.
 *   - **law**     — refusals, faults and facts the chrome must state in
 *                   full (the facts foot, wire faults, refusal reasons, the
 *                   destructive-delete description): allowlisted BY NAME,
 *                   no bound.
 *
 * A per-key `max` tightens a class where the design says so (the status
 * word is ONE word: the dot already says retrieval). The table is the audit
 * the register row carries; the two must agree, and the pin fails closed in
 * both directions — a key in these families with no row, or a row whose key
 * is gone, is a red with the key's name.
 */
type TextClass = 'label' | 'hint' | 'verdict' | 'law';
type AuditRow = TextClass | { readonly kind: TextClass; readonly max: number };

const CHROME_FAMILIES = [
	'shell.',
	'nav.',
	'palette.',
	'settings.',
	'chat.empty.',
	'chat.composer.',
	'chat.sessions.',
	'common.'
] as const;

const CLASS_BOUND: Record<Exclude<TextClass, 'law'>, number> = { label: 2, hint: 6, verdict: 12 };

const TEXT_AUDIT: Readonly<Record<string, AuditRow>> = {
	// shell.*
	'shell.brand': 'label',
	'shell.sidebar': 'hint',
	'shell.nav': 'hint',
	'shell.kernel': 'law',
	'shell.kernel.unknown': 'law',
	'shell.new_chat': 'label',
	'shell.sidebar.collapse': 'hint',
	'shell.sidebar.expand': 'hint',
	'shell.facts.rows': 'law',
	'shell.facts.integrity.verified': 'law',
	'shell.facts.integrity.broken': 'law',
	'shell.facts.integrity.unverified': 'law',
	'shell.status.healthy': { kind: 'label', max: 1 },
	'shell.status.model': 'law',
	// nav.*
	'nav.chat': 'label',
	'nav.memory': 'label',
	'nav.review': 'label',
	'nav.audit': 'label',
	'nav.governance': 'label',
	'nav.recall': 'label',
	'nav.decisions': 'label',
	'nav.settings': 'label',
	// palette.*
	'palette.title': 'label',
	'palette.placeholder': 'hint',
	'palette.empty': 'hint',
	'palette.navigate': 'label',
	'palette.more': 'label',
	'palette.open': 'hint',
	// settings.*
	'settings.title': 'label',
	'settings.description': 'hint',
	'settings.origin.label': 'label',
	'settings.origin.save': 'label',
	'settings.origin.saved': 'verdict',
	'settings.origin.invalid': 'law',
	'settings.origin.hint': 'hint',
	'settings.locale.label': 'label',
	'settings.token.heading': 'label',
	'settings.token.label': 'label',
	'settings.token.hint': 'hint',
	'settings.token.save': 'label',
	'settings.token.clear': 'label',
	'settings.token.status.configured': 'verdict',
	'settings.token.status.anonymous': 'verdict',
	'settings.token.saved': 'verdict',
	'settings.token.rotated': 'verdict',
	'settings.token.pending': 'verdict',
	'settings.token.stale': 'law',
	'settings.token.cleared': 'verdict',
	'settings.token.empty': 'law',
	'settings.token.error': 'law',
	'settings.update.heading': 'label',
	'settings.update.current': 'verdict',
	'settings.update.check': 'label',
	'settings.update.install': 'label',
	'settings.update.uptodate': 'verdict',
	'settings.update.available': 'verdict',
	'settings.update.unreachable': 'law',
	'settings.update.refused': 'law',
	'settings.update.error': 'law',
	// The retrieval model's download surface (the S-4 onboarding round):
	// heading + the three postures + the hint; the model NAME interpolates,
	// so the loading/ready sentences are HINT-class (a name makes them
		// open-ended) — the bounds law applies to the FIXED prose only.
	'settings.model.heading': 'label',
	'settings.model.loading': 'hint',
	'settings.model.ready': 'hint',
	'settings.model.failed': 'verdict',
	'settings.model.fallback_name': 'label',
	'settings.model.hint': 'hint',
	// chat.empty.*
	'chat.empty.greeting': { kind: 'hint', max: 8 },
	'chat.empty.chip.ask': 'label',
	'chat.empty.chip.trace': 'label',
	'chat.empty.chip.review': 'label',
	// chat.composer.*
	'chat.composer.label': 'hint',
	'chat.composer.placeholder': 'hint',
	'chat.composer.hint': 'hint',
	// chat.sessions.*
	'chat.sessions.heading': 'label',
	'chat.sessions.unlabelled': 'label',
	'chat.sessions.rename': 'hint',
	'chat.sessions.rename.label': 'hint',
	'chat.sessions.delete': 'hint',
	'chat.sessions.delete.title': 'hint',
	'chat.sessions.delete.description': 'law',
	'chat.sessions.delete.confirm': 'label',
	// common.*
	'common.back': 'label',
	'common.cancel': 'label',
	'common.retry': 'label'
};

/** Function words that do not count toward a LABEL's two content words. */
const STOP_WORDS: Readonly<Record<string, ReadonlySet<string>>> = {
	en: new Set(['the', 'a', 'an', 'of', 'to', 'for', 'or', 'and', 'this', 'in', 'on']),
	de: new Set([
		'der', 'die', 'das', 'den', 'dem', 'des', 'ein', 'eine', 'einen', 'einem', 'eines',
		'und', 'oder', 'für', 'zu', 'von', 'im', 'am', 'nach', 'auf', 'dieses', 'diese', 'dieser'
	]),
	fr: new Set([
		'le', 'la', 'les', 'l', 'de', 'du', 'des', 'd', 'un', 'une', 'et', 'ou', 'à', 'au', 'aux',
		'pour', 'en', 'ce', 'cette', 'ces', 'sur'
	]),
	es: new Set([
		'el', 'la', 'los', 'las', 'un', 'una', 'de', 'del', 'y', 'o', 'a', 'al', 'para', 'en',
		'por', 'este', 'esta', 'estos', 'estas'
	]),
	nl: new Set(['de', 'het', 'een', 'van', 'en', 'of', 'voor', 'op', 'te', 'in', 'aan', 'deze', 'dit', 'naar'])
};

const LOCALES = ['en', 'de', 'fr', 'es', 'nl'] as const;

function catalog(locale: string): Record<string, string> {
	return JSON.parse(readFileSync(join(shellRoot, 'src', 'lib', 'i18n', `${locale}.json`), 'utf8')) as Record<
		string,
		string
	>;
}

/** A token is a word when it carries a letter, a digit or a symbol (⌘⏎ counts; a lone "·" or ";" does not). */
const isWord = (token: string): boolean => /[\p{L}\p{N}\p{S}]/u.test(token);

function wordCount(value: string, kind: TextClass, locale: string): number {
	if (kind === 'label') {
		const stop = STOP_WORDS[locale] ?? new Set<string>();
		return value
			.split(/[\s'’]+/)
			.filter(isWord)
			.filter((t) => !stop.has(t.toLowerCase().replace(/[.,:;!?…]+$/u, '')))
			.length;
	}
	return value.split(/\s+/).filter(isWord).length;
}

test('the_chrome_text_audit_holds_in_all_five_catalogs_and_the_table_is_exhaustive', () => {
	const en = catalog('en');
	const chromeKeys = Object.keys(en).filter((k) => CHROME_FAMILIES.some((f) => k.startsWith(f)));
	// Anti-vacuity: the eight families are a real population (77 measured 2026-10-27).
	expect(chromeKeys.length, 'the eight chrome families carry fewer keys than the measured 77').toBeGreaterThanOrEqual(77);

	// Exhaustive in both directions: every chrome key is classified, every row names a live key.
	const unclassified = chromeKeys.filter((k) => !(k in TEXT_AUDIT));
	const stale = Object.keys(TEXT_AUDIT).filter((k) => !(k in en));
	expect(unclassified, 'chrome keys with no audit row — classify them (label / hint / verdict / law) before they ship').toEqual([]);
	expect(stale, 'audit rows whose key no longer exists in en.json').toEqual([]);

	const failures: string[] = [];
	for (const locale of LOCALES) {
		const values = catalog(locale);
		for (const [key, row] of Object.entries(TEXT_AUDIT)) {
			const kind = typeof row === 'string' ? row : row.kind;
			if (kind === 'law') continue;
			const max = typeof row === 'string' ? CLASS_BOUND[kind] : row.max;
			const value = values[key];
			if (value === undefined) {
				failures.push(`${locale}: ${key} is missing`);
				continue;
			}
			const words = wordCount(value, kind, locale);
			if (words > max) {
				failures.push(`${locale}: ${key} (${kind} ≤ ${max}) is ${words} words — "${value}"`);
			}
		}
	}
	expect(failures, 'chrome text over its bound — shorten the value or reclassify the key in the audit table (never silently)').toEqual([]);
});
