import { beforeEach, describe, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { m } from '$paraglide/messages.js';
import { clearWireFault } from '../src/lib/chat/wire-status.svelte';

/**
 * M3-b — the chat home state: the empty state, the exception-chrome fault
 * banner, and the collapsed evidence disclosure (plan `21_CHAT_FIRST_IA`
 * §1/§5, plan 22 R2).
 *
 * Three laws, and each pin below is named after the one it polices:
 *
 * 1. **The empty state is the first paint.** A greeting and exactly three
 *    ACTING chips (ask / recall history / review queue) — the chips are the
 *    onboarding, and the first turn replaces them. The composer keeps its
 *    mount focus (the M3-a pin, untouched).
 * 2. **Chrome by exception.** No banner node exists while no wire fault was
 *    classified; a classified fault renders the fault's OWN sentence above
 *    the composer with a Restart that re-runs the EXISTING gate flow. A
 *    refusal is an answer and never draws the banner — the exact conflation
 *    the six-state taxonomy exists to prevent.
 * 3. **Evidence collapses.** An answered turn shows the verdict line and a
 *    disclosure (collapsed by default); the affordances live INSIDE it; the
 *    zero-passage states grow no disclosure (fabricated evidence chrome).
 *
 * The core is stubbed at the IPC seam exactly as `chat-surface.test.ts`
 * stubs it, with the M3-b commands added: `kernel_wire_status` (the wire
 * fault's NAME — the renderer cannot tell a blocked response from a dead
 * port), and `kernel_status`/`kernel_start` (the gate flow the banner's
 * Restart re-runs).
 */

let nextShape: unknown = null;
/** What `kernel_wire_status` answers at the next classification probe. */
let wireStatusKind: string | null = null;
/** The `kernel_status` answers, consumed in order; a Promise hangs the probe. */
let kernelStatusAnswers: unknown[] = [];
/** What `kernel_start` answers. */
let kernelStartOutcome: unknown = null;
/** Every command the page issued, in order — the Restart flow's witness. */
const invokeLog: string[] = [];

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string) => {
		invokeLog.push(command);
		if (command === 'kernel_token') return null;
		if (command === 'recall_memory') {
			return { doc: { v: 1, query: 'q', limit: 5, provenance: true }, query_bytes: 1 };
		}
		if (command === 'shape_recall_command') return nextShape;
		if (command === 'kernel_wire_status') {
			return { kind: wireStatusKind ?? 'unknown' };
		}
		if (command === 'kernel_status') {
			const next = kernelStatusAnswers.shift();
			if (next === undefined) throw new Error(`unexpected kernel_status #${invokeLog.length}`);
			return next;
		}
		if (command === 'kernel_start') {
			if (kernelStartOutcome === null) throw new Error('unexpected kernel_start');
			return kernelStartOutcome;
		}
		throw new Error(`unexpected command ${command}`);
	})
}));

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));

// The construction-order law (chat-surface.test.ts): the typed client holds
// the `fetch` it was created with, so the stub must exist before this import.
// M3-d: the stub routes by path — the spine's `POST /chat/turns` answers the
// 503 posture refusal (the D1 fallback driver: the deterministic shapes then
// arrive through the core IPC, exactly as the M3-b pins drove them), and the
// frame's own glance traffic answers a harmless 200.
vi.stubGlobal('__TAURI_INTERNALS__', {});
vi.stubGlobal(
	'fetch',
	vi.fn(async (input: RequestInfo | URL) => {
		const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
		if (url.includes('/chat/turns')) {
			return new Response(
				JSON.stringify({
					error: { code: 'recall_unavailable', message: 'chat_bridge_unavailable — fixture posture' }
				}),
				{ status: 503 }
			);
		}
		return new Response(JSON.stringify({ sessions: [], total: 0 }), { status: 200 });
	})
);

import { invoke } from '@tauri-apps/api/core';
import { goto } from '$app/navigation';
const chatModule = await import('../src/routes/+page.svelte');

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
			valid_to: null,
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
const REFUSED = { state: 'refused', status: 403, scope: 'recall' };
const UNREACHABLE = { state: 'unreachable', status: 0, detail: 'the recall transport failed: wire_error' };

async function renderChat(shape: unknown): Promise<void> {
	nextShape = shape;
	render(chatModule.default as any);
	await waitFor(() =>
		expect(screen.getAllByTestId('chat-composer').length).toBeGreaterThan(0)
	);
}

async function sendQuery(): Promise<void> {
	await userEvent.type(screen.getByTestId('chat-composer'), 'retention');
	await userEvent.keyboard('{Meta>}{Enter}{/Meta}');
	await waitFor(() => {
		const turns = screen.queryAllByTestId('chat-turn');
		expect(turns.length).toBeGreaterThan(0);
		expect(turns[turns.length - 1]!.getAttribute('data-rendering')).not.toBe('pending');
	});
}

beforeEach(() => {
	cleanup();
	clearWireFault();
	invokeLog.length = 0;
	kernelStatusAnswers = [];
	kernelStartOutcome = null;
	wireStatusKind = null;
});

// ── 1 · the empty state ───────────────────────────────────────────────────

describe('M3-b the empty state', () => {
	test('the_empty_state_is_a_greeting_and_exactly_three_acting_chips', async () => {
		await renderChat(ANSWERED);

		const empty = screen.getByTestId('chat-empty');
		expect(empty.textContent).toContain(m['chat.empty.greeting']());

		// The COUNT is the pin (plan 21 §3.8): three chips, each a real
		// button, each an ACT — never a link to a page that explains.
		const chips = empty.querySelectorAll('button');
		expect(chips.length, `the empty state carries ${chips.length} chips`).toBe(3);

		// Ask ACTS on this surface: the composer takes focus.
		await userEvent.click(screen.getByTestId('chat-empty-ask'));
		expect(document.activeElement).toBe(screen.getByTestId('chat-composer'));

		// The other two navigate to the two surfaces the operator reaches for.
		vi.mocked(goto).mockClear();
		await userEvent.click(screen.getByTestId('chat-empty-trace'));
		await waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalledWith('/recall'));
		await userEvent.click(screen.getByTestId('chat-empty-review'));
		await waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalledWith('/review'));
	});

	test('the_first_turn_replaces_the_empty_state', async () => {
		await renderChat(ANSWERED);
		expect(screen.getByTestId('chat-empty')).toBeTruthy();
		await sendQuery();
		// A transcript with chips below it is clutter: the first turn — even
		// pending — is the conversation now.
		expect(screen.queryByTestId('chat-empty')).toBeNull();
	});

	test('the_empty_state_is_axe_clean', async () => {
		await renderChat(ANSWERED);
		const { default: axe } = await import('axe-core');
		const results = await axe.run(document.body, {
			runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
		});
		expect(results.violations).toEqual([]);
	});
});

// ── 2 · the fault banner (exception chrome) ───────────────────────────────

describe('M3-b the fault banner', () => {
	test('the_page_holds_no_banner_node_while_no_fault_was_classified', async () => {
		await renderChat(ANSWERED);
		await sendQuery();
		// Silent while green is STRUCTURAL: not a hidden banner, an ABSENT
		// one. A `<div hidden>` would still be a node this query finds.
		expect(screen.queryByTestId('chat-fault-banner')).toBeNull();
	});

	test('a_wire_fault_renders_a_banner_naming_the_fault_above_the_composer', async () => {
		wireStatusKind = 'unreachable';
		await renderChat(UNREACHABLE);
		await sendQuery();

		const banner = await screen.findByTestId('chat-fault-banner');
		// The fault's OWN sentence, never a generic "something went wrong":
		// the copy is `wireFaultMessage`'s, keyed by the NAMED fault.
		expect(banner.getAttribute('data-fault')).toBe('unreachable');
		expect(banner.textContent).toContain(m['wire.error.unreachable']());
		expect(banner.getAttribute('role')).toBe('alert');
		// ABOVE the composer, in DOM order — the banner is the one interrupt
		// plan 21 §1 allows itself.
		const composer = screen.getByTestId('chat-composer');
		expect(
			(banner.compareDocumentPosition(composer) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0,
			'the banner must precede the composer'
		).toBe(true);
	});

	test('a_refusal_is_an_answer_and_draws_no_banner', async () => {
		// THE TRAP (plan 21 §1's taxonomy): a 403 turn is the kernel SAYING
		// NO. A banner here would call the kernel sick for refusing.
		await renderChat(REFUSED);
		await sendQuery();
		expect(screen.getByTestId('chat-refused')).toBeTruthy();
		expect(screen.queryByTestId('chat-fault-banner')).toBeNull();
	});

	test('restart_re_runs_the_gate_flow_and_a_successful_probe_dismisses_the_banner', async () => {
		wireStatusKind = 'unreachable';
		await renderChat(UNREACHABLE);
		await sendQuery();
		await screen.findByTestId('chat-fault-banner');

		// The EXISTING gate flow, verbatim: status → start (consent and the
		// data root already exist from the first run) → status. The probe's
		// `ready` is the gate's own health read, and it is what dismisses.
		kernelStatusAnswers = [{ state: 'notRunning' }, { state: 'ready' }];
		kernelStartOutcome = { state: 'started' };
		invokeLog.length = 0;

		await userEvent.click(screen.getByTestId('chat-fault-restart'));
		await waitFor(() => expect(screen.queryByTestId('chat-fault-banner')).toBeNull());
		expect(invokeLog).toEqual(['kernel_status', 'kernel_start', 'kernel_status']);
		expect(
			vi.mocked(invoke).mock.calls.some(
				(call) => call[0] === 'kernel_start' && (call[1] as { consent?: unknown })?.consent === true
			),
			'the banner restarts over the existing consent — there is no second consent dialog'
		).toBe(true);
	});

	test('a_failed_restart_keeps_the_banner_and_the_fault_stays_named', async () => {
		wireStatusKind = 'unreachable';
		await renderChat(UNREACHABLE);
		await sendQuery();
		await screen.findByTestId('chat-fault-banner');

		// The kernel never comes up: start answers refused, the probe never
		// says ready — the banner is still the live fact.
		kernelStatusAnswers = [{ state: 'notRunning' }, { state: 'refused', kind: 'startFailed' }];
		kernelStartOutcome = { state: 'refused', kind: 'startFailed' };
		await userEvent.click(screen.getByTestId('chat-fault-restart'));
		await waitFor(() =>
			expect(
				vi.mocked(invoke).mock.calls.filter((call) => call[0] === 'kernel_status').length
			).toBe(2)
		);
		expect(screen.getByTestId('chat-fault-banner').getAttribute('data-fault')).toBe('unreachable');
	});

	test('while_restart_is_busy_the_button_says_so', async () => {
		wireStatusKind = 'unreachable';
		await renderChat(UNREACHABLE);
		await sendQuery();
		await screen.findByTestId('chat-fault-banner');

		// Hang the first gate read on a deferred: while the gate is busy the
		// action says so, and a second click cannot start a second flow.
		let releaseProbe: (value: { state: string }) => void = () => {};
		const held = new Promise<{ state: string }>((resolve) => {
			releaseProbe = resolve;
		});
		kernelStatusAnswers = [held, { state: 'ready' }];
		kernelStartOutcome = { state: 'started' };

		const restart = screen.getByTestId('chat-fault-restart');
		await userEvent.click(restart);
		await waitFor(() =>
			expect(restart.textContent).toContain(m['chat.status.restarting']())
		);
		expect(restart.hasAttribute('disabled')).toBe(true);

		releaseProbe({ state: 'ready' });
		await waitFor(() => expect(screen.queryByTestId('chat-fault-banner')).toBeNull());
	});
});

// ── 3 · the collapsed evidence disclosure ─────────────────────────────────

describe('M3-b the collapsed evidence', () => {
	test('evidence_collapses_by_default_and_expands_on_demand', async () => {
		await renderChat(ANSWERED);
		await sendQuery();

		// The verdict line is the answer; the count is the disclosure's own
		// copy; and until it is opened there is NO citation node in the DOM.
		const disclosure = screen.getByTestId('chat-evidence');
		expect(disclosure.getAttribute('aria-expanded')).toBe('false');
		expect(disclosure.textContent).toContain(m['chat.evidence.count']({ count: 1 }));
		expect(screen.queryAllByTestId('chat-citation')).toHaveLength(0);

		// Keyboard, not pointer: a real disclosure control in the tab order.
		disclosure.focus();
		expect(document.activeElement).toBe(disclosure);
		await userEvent.keyboard('{Enter}');
		await waitFor(() =>
			expect(screen.getByTestId('chat-evidence').getAttribute('aria-expanded')).toBe('true')
		);
		expect(screen.getAllByTestId('chat-citation').length).toBeGreaterThan(0);
		// The affordances live INSIDE the disclosure (plan 21 §3: evidence
		// opens in place — the correct/forget pair arrives with it).
		expect(screen.getByTestId('chat-correct')).toBeTruthy();
		expect(screen.getByTestId('chat-forget')).toBeTruthy();

		// And it closes again — the collapsed state is the DEFAULT, not the
		// resting state after one use.
		await userEvent.click(screen.getByTestId('chat-evidence'));
		await waitFor(() =>
			expect(screen.getByTestId('chat-evidence').getAttribute('aria-expanded')).toBe('false')
		);
		expect(screen.queryAllByTestId('chat-citation')).toHaveLength(0);
	});

	test('zero_passage_states_grow_no_disclosure', async () => {
		// Fabricated evidence chrome: an abstention has no citations, so a
		// disclosure on it would announce evidence that does not exist.
		await renderChat(ABSTAINED);
		await sendQuery();
		expect(screen.getByTestId('chat-abstained')).toBeTruthy();
		expect(screen.queryByTestId('chat-evidence')).toBeNull();
	});
});
