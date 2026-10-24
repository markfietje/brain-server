import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { shellRoot } from './fixtures';

/**
 * II-3 / II-4 / II-5 / II-7 — the SURFACE, not the model.
 *
 * `chat-turn.test.ts` proves the pure modules are right. This file proves the
 * COMPONENT renders them, which is the step where a correct model still
 * collapses: a template that branches on `passages.length` produces the right
 * union and the wrong picture. So every state is asserted THROUGH THE DOM.
 *
 * Copy is never the assertion target — the catalogs are five locales deep and a
 * translated string is not a stable hook. Each state carries a
 * `data-rendering` token instead, which is what the components bind.
 */

const routeSource = readFileSync(join(shellRoot, 'src/routes/+page.svelte'), 'utf8');

/** Strips comment-only lines — a comment does not execute. */
function codeOf(source: string): string {
	return source
		.split('\n')
		.filter((line) => {
			const trimmed = line.trim();
			return !trimmed.startsWith('//') && !trimmed.startsWith('*') && !trimmed.startsWith('/*');
		})
		.join('\n');
}

/**
 * Stubs the Tauri IPC so a turn can be driven WITHOUT a runtime.
 *
 * The stub stands in for the IPC, never for the subject: `authorize` and
 * `shape` are the Rust core's own commands, and what is under test is the
 * component's rendering of what they returned — the classification itself is
 * pinned in `chat-turn.test.ts` and in `recall-wire.test.ts`.
 */
/**
 * The shape the next render will receive, set before each mount.
 *
 * It is a module-level MUTABLE rather than a `vi.doMock` per test, and that is
 * deliberate: `vi.resetModules()` + `vi.doMock` before each `import()` made
 * every render fail with Svelte's `effect_orphan`, because the component was
 * re-instantiated outside a mount context. Stubs are global and the module is
 * imported ONCE; `@testing-library/svelte`'s `cleanup()` in setup.ts unmounts
 * between tests, so each test still gets a fresh component.
 */
let nextShape: unknown = null;

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string) => {
		if (command === 'kernel_token') return null;
		if (command === 'recall_memory') {
			return { doc: { v: 1, query: 'q', limit: 5, provenance: true }, query_bytes: 1 };
		}
		if (command === 'shape_recall_command') return nextShape;
		throw new Error(`unexpected command ${command}`);
	})
}));

// `client.ts` constructs the openapi-fetch client at module load and captures
// `fetch` then. Stub BEFORE the route import: a stub installed in renderChat is
// too late, which turns this into a live-loopback-kernel probe — green on a
// developer box with a running kernel, `unreachable` on the Ubuntu lane.
vi.stubGlobal('__TAURI_INTERNALS__', {});
vi.stubGlobal(
	'fetch',
	vi.fn(async () => new Response(JSON.stringify({ hits: [], decision: 'ok' }), { status: 200 }))
);

const chatModule = await import('../src/routes/+page.svelte');

/**
 * What `render` hands back, plus the typed scope each assertion reads through.
 *
 * `WithinMatcher` is not exported by @testing-library/svelte, so the scope is
 * typed as the query surface the tests actually use. Declaring it structurally
 * rather than importing a name that does not exist keeps the assertions typed
 * without a cast.
 */
interface Scope {
	getByTestId(id: string): HTMLElement;
	queryByTestId(id: string): HTMLElement | null;
	queryAllByTestId(id: string): HTMLElement[];
}

interface Rendered {
	unmount: () => void;
	/** The real container `within()` needs — a plain object is not accepted. */
	container: HTMLElement;
}

async function renderChat(shape: unknown): Promise<Rendered> {
	nextShape = shape;
	vi.stubGlobal('__TAURI_INTERNALS__', {});
	vi.stubGlobal(
		'fetch',
		vi.fn(async () => new Response(JSON.stringify({ hits: [], decision: 'ok' }), { status: 200 }))
	);
	const rendered = render(chatModule.default as any);
	await waitFor(() =>
		expect(screen.getAllByTestId('chat-composer').length).toBeGreaterThan(0)
	);
	return rendered as unknown as Rendered;
}

const ANSWERED = {
	state: 'answered',
	decision: 'ok',
	passages: [
		{
			id: 7,
			title: 'Retention policy',
			text: 'Audit rows are retained for 400 days by default.',
			snippet: null,
			domain: 'infra',
			score: 0.82,
			valid_from: '2026-03-02',
			valid_to: '2021-06-30',
			flagged: false,
			ingest_kind: null,
			memory_kind: null,
			authority: null
		}
	],
	confidence: 0.82,
	searched: { query: 'retention', domain: null, limit: 5, domains_searched: [], included_global: null }
};
const ABSTAINED = {
	state: 'abstained',
	decision: 'low_confidence',
	passages: [],
	confidence: 0.04,
	searched: { query: 'retention', domain: null, limit: 5, domains_searched: [], included_global: null }
};
const ABSENT = {
	state: 'absent',
	decision: 'ok',
	passages: [],
	confidence: 0.1,
	searched: { query: 'retention', domain: null, limit: 5, domains_searched: [], included_global: null }
};
const REFUSED = { state: 'refused', status: 403, scope: 'recall' };
const UNREACHABLE = { state: 'unreachable', status: 0, detail: 'the kernel is not running' };

/**
 * Types a question and sends it with ⌘⏎.
 *
 * It waits for a turn to LEAVE `pending`, not merely to exist. The first draft
 * waited for a turn to appear, which passed while the turn was still the
 * placeholder — so four arms went green against a surface that had rendered
 * nothing at all. A turn appearing is not a turn having arrived.
 */
async function sendQuery(scope: Scope = screen as unknown as Scope): Promise<void> {
	await userEvent.type(scope.getByTestId('chat-composer'), 'retention');
	await userEvent.keyboard('{Meta>}{Enter}{/Meta}');
	await waitFor(() => {
		const turns = scope.queryAllByTestId('chat-turn');
		expect(turns.length).toBeGreaterThan(0);
		expect(turns[turns.length - 1]!.getAttribute('data-rendering')).not.toBe('pending');
	});
}

/**
 * Opens an answered turn's evidence disclosure (M3-b: collapsed is the
 * DEFAULT). The pins that assert citation content drive the OPENED state
 * they have always driven — this helper is their setup step, so the
 * collapsed-default law lives in `chat-home-state.test.ts`, not in a
 * template accident here.
 */
async function openEvidence(scope: Scope = screen as unknown as Scope): Promise<void> {
	await userEvent.click(scope.getByTestId('chat-evidence'));
	await waitFor(() =>
		expect(scope.queryAllByTestId('chat-citation').length).toBeGreaterThan(0)
	);
}

describe('II-3 the composer', () => {
	test('the_composer_is_focused_with_zero_navigation_before_typing', async () => {
		await renderChat(ANSWERED);
		// The focus EFFECT is asserted by source and the binding by the DOM,
		// because jsdom's focus handling for a component-bound `ref` is not the
		// browser's: the element exists and the route calls `.focus()` on mount,
		// which is what Plan 02 §7 asks for. Asserting `document.activeElement`
		// here would be asserting jsdom, not the shell.
		const code = codeOf(routeSource);
		expect(code, 'the composer is never focused').toContain('.focus()');
		expect(code, 'focus must happen on mount, not after a turn').toMatch(
			/\$effect\([\s\S]{0,120}focus\(\)[\s\S]{0,40}\}\);/
		);
		// And the composer is a real focusable control with a label.
		const composer = screen.getByTestId('chat-composer');
		expect(composer.tagName).toBe('TEXTAREA');
		expect(document.querySelector('label[for="chat-composer"]')).toBeTruthy();
	});

	test('the_turn_is_fully_keyboard_operable', async () => {
		await renderChat(ANSWERED);
		// Typed, sent and read without a single click: ⌘⏎ sends (Plan 02 §3).
		await sendQuery();
		// Focus returns to the composer after a turn, so the next question needs
		// no mouse — asserted as the source call, for the jsdom reason above.
		expect(codeOf(routeSource)).toContain('composerEl?.focus()');
	});
});

describe('II-4 / II-5 the surface renders every state distinctly', () => {
	test('an_answered_turn_renders_its_provenance_with_a_historical_marker', async () => {
		await renderChat(ANSWERED);
		await sendQuery();
		await openEvidence();
		await waitFor(() => expect(screen.getAllByTestId('chat-turn').length).toBe(1));

		const turn = screen.getAllByTestId('chat-turn')[0]!;
		expect(turn.getAttribute('data-rendering')).toBe('answered');
		// The citation is present: a turn that recalls cannot render without one.
		expect(screen.getByTestId('chat-citation')).toBeTruthy();
		// THE BI-TEMPORAL ARM. This passage's `valid_to` is 2021 and the turn
		// is rendered in 2026, so it must be marked historical in the DOM --
		// not merely worded that way, so the marker survives a stylesheet
		// failure and reaches a screen reader.
		const historical = document.querySelector('[data-historical="true"]');
		expect(historical, 'an expired valid_to must be marked historical in the DOM').toBeTruthy();
	});

	test('abstention_renders_as_an_answer_not_an_error', async () => {
		await renderChat(ABSTAINED);
		await sendQuery();
		await waitFor(() => expect(screen.getAllByTestId('chat-turn').length).toBe(1));

		expect(screen.getAllByTestId('chat-turn')[0]!.getAttribute('data-rendering')).toBe('abstained');
		expect(screen.getByTestId('chat-abstained')).toBeTruthy();
		// Not an error surface: no alert role, no violation testid.
		expect(screen.queryByTestId('chat-violation')).toBeNull();
		expect(screen.queryByTestId('chat-refused')).toBeNull();
	});

	// TWO renders in one test is deliberate and it is the point: the refusal
	// and the absence must be DIFFERENT renderings of the same surface, so
	// comparing them requires them to coexist in one document. Queries are
	// therefore scoped to each turn element rather than to the screen, which
	// would find both composers at once.
	test('refusal_and_absence_are_different_renderings', async () => {
		// TWO renders in one test is deliberate: the two states must be
		// DIFFERENT renderings of the SAME surface. Each half unmounts before
		// the next mounts, because a shared document would leave two composers
		// and make every unscoped query ambiguous.
		const refused = within((await renderChat(REFUSED)).container);
		await sendQuery(refused as unknown as Scope);
		expect(refused.getByTestId('chat-turn').getAttribute('data-rendering')).toBe('refused');
		expect(refused.getByTestId('chat-refused')).toBeTruthy();
		// THE LEAK, in the DOM: a refusal must not be drawn as an absence, and
		// the other two zero-passage states must not stand in for it either.
		expect(refused.queryByTestId('chat-absent')).toBeNull();
		expect(refused.queryByTestId('chat-abstained')).toBeNull();
		cleanup();

		const absent = within((await renderChat(ABSENT)).container);
		await sendQuery(absent as unknown as Scope);
		expect(absent.getByTestId('chat-turn').getAttribute('data-rendering')).toBe('absent');
		expect(absent.getByTestId('chat-absent')).toBeTruthy();
		expect(absent.queryByTestId('chat-refused')).toBeNull();
	});

	test('an_unreachable_kernel_renders_as_degraded_not_empty', async () => {
		await renderChat(UNREACHABLE);
		await sendQuery();
		await waitFor(() => expect(screen.getAllByTestId('chat-turn').length).toBe(1));
		expect(screen.getAllByTestId('chat-turn')[0]!.getAttribute('data-rendering')).toBe('unreachable');
		expect(screen.getByTestId('chat-unreachable')).toBeTruthy();
	});
});

describe('II-7 accessibility', () => {
	test('axe_AA_clean_on_the_chat_route', async () => {
		await renderChat(ANSWERED);
		await sendQuery();
		// The swept document is the OPENED one, as before M3-b: the collapsed
		// default's own sweep lives in chat-home-state.test.ts.
		await openEvidence();
		await waitFor(() => expect(screen.getAllByTestId('chat-turn').length).toBe(1));
		const { default: axe } = await import('axe-core');
		const results = await axe.run(document.body, {
			runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
		});
		expect(results.violations).toEqual([]);
	});

	test('the_template_branches_on_the_rendering_kind_never_on_passages', () => {
		const code = codeOf(routeSource);
		expect(code, 'the template branches on passages length').not.toMatch(/passages\s*\??\.\s*length/);
		for (const kind of [
			'pending',
			'answered',
			'absent',
			'abstained',
			'refused',
			'unreachable',
			'contract_violation'
		]) {
			expect(code, `no branch for ${kind}`).toContain(`'${kind}'`);
		}
	});

	test('the_route_uses_no_html_escape_hatch_and_the_ban_still_exists', () => {
		expect(codeOf(routeSource), 'the chat route uses {@html}').not.toContain('@html');
		// A disabled rule is not a passing one, so the guard asserts the ban is
		// still configured as well as the surface obeying it.
		expect(readFileSync(join(shellRoot, 'eslint.config.js'), 'utf8')).toContain('@html');
	});

	test('the_four_renderings_carry_their_own_copy_not_colour_alone', () => {
		const code = codeOf(routeSource);
		expect(code, 'the refusal branch has its own copy').toContain('chat.refused.body');
		expect(code, 'the abstention branch has its own copy').toContain('chat.abstained.title');
		expect(code, 'the absence branch has its own copy').toContain('chat.absent.body');
		expect(code, 'the unreachable branch has its own copy').toContain('chat.unreachable.body');
		expect(code).toContain('data-historical');
	});
});