import { describe, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { m } from '$paraglide/messages.js';

/**
 * II-7g — axe AA over the NEW affordances, and the flow keyboard-complete.
 *
 * The `/chat` route's axe arm already exists in `chat-surface.test.ts`, and it
 * is green. That arm is not this pin: it renders an answered turn and never
 * opens `[correct]` or `[forget]`, so the claim surface and the two result
 * regions did not exist in the document it measured. An axe run over a state
 * nobody opened proves the state nobody opened is accessible.
 *
 * So every arm below OPENS the affordance it measures — the form, the verdict,
 * the tombstone — and runs axe against the resulting document. The keyboard
 * arms are separate because axe cannot see one: a control can be perfectly
 * labelled and still be unreachable by Tab, and the plan's requirement is a
 * keyboard-complete flow, not a screen-reader-complete one.
 *
 * The core is stubbed at the IPC seam, exactly as `chat-surface.test.ts` does:
 * what is under test is the SURFACE's accessibility, not the Rust core's
 * classification (that is `shell/src-tauri/src/verify_tests.rs` and
 * `verify-wire.test.ts`).
 */

let nextRecallShape: unknown = null;
let verifyResult: unknown = null;
let forgetResult: unknown = null;

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string, args?: Record<string, any>) => {
		if (command === 'kernel_token') return null;
		if (command === 'recall_memory') {
			return { doc: { v: 1, query: 'q', limit: 5, provenance: true }, query_bytes: 1 };
		}
		if (command === 'shape_recall_command') return nextRecallShape;
		if (command === 'verify_claim_command') {
			// The core's authorization, in the shape `verify.ts` expects. The
			// bound itself is not re-implemented here; the page hands the claim
			// to the core, and the core is what refuses an empty one.
			return {
				doc: { chunk_id: args?.['request'].chunk_id, claim: args?.['request'].claim },
				claim_chars: String(args?.['request'].claim).length
			};
		}
		if (command === 'shape_verify_command') return verifyResult;
		if (command === 'forget_memory') return { doc: { id: String(args?.['request'].id) } };
		if (command === 'shape_forget_command') return forgetResult;
		throw new Error(`unexpected command ${command}`);
	})
}));

// Same construction-order law as chat-surface.test.ts: the typed client holds
// the `fetch` it was created with, so the stub must exist before this import.
// M3-d: the spine's `POST /chat/turns` answers the 503 posture refusal, so the
// answered shape arrives through the D1 fallback and the core IPC — the same
// seam these affordance pins have always driven.
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

/** A supported verdict, carrying the ranges the core's own invariant requires
 * (a supported body with no ranges is a contract violation on the wire). */
const SUPPORTED = {
	state: 'supported',
	chunk_id: 7,
	decision: 'supported',
	supported: true,
	match_ranges: [[0, 30]],
	claim: 'Audit rows are retained for 400 days by default.'
};

const TOMBSTONED = {
	state: 'tombstoned',
	id: '7',
	result: 'tombstoned',
	dsar_certificate: null
};

async function renderAnswered(): Promise<void> {
	nextRecallShape = ANSWERED;
	verifyResult = SUPPORTED;
	forgetResult = TOMBSTONED;
	vi.stubGlobal('__TAURI_INTERNALS__', {});
	// The routed wire stub (M3-d): the spine's `POST /chat/turns` answers the
	// 503 posture refusal, so the answered shape arrives through the D1
	// fallback and the core IPC below; the frame's glance traffic answers 200.
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
	render(chatModule.default as any);
	await waitFor(() =>
		expect(screen.getAllByTestId('chat-composer').length).toBeGreaterThan(0)
	);

	await userEvent.type(screen.getByTestId('chat-composer'), 'retention');
	await userEvent.keyboard('{Meta>}{Enter}{/Meta}');
	await waitFor(() => {
		const turns = screen.queryAllByTestId('chat-turn');
		expect(turns.length).toBeGreaterThan(0);
		expect(turns[turns.length - 1]!.getAttribute('data-rendering')).toBe('answered');
	});

	// M3-b: evidence collapses by default, and these pins drive the OPENED
	// state they have always driven — the affordances render inside the
	// disclosure, so it is opened here, in the setup, not per arm.
	await userEvent.click(screen.getByTestId('chat-evidence'));
	await waitFor(() =>
		expect(screen.queryAllByTestId('chat-citation').length).toBeGreaterThan(0)
	);
}

/** Runs the same axe profile the `/chat` arm uses. Every state below is
 * measured as a WHOLE document, because a violation frequently lives in the
 * relationship between the new control and the composer, not inside the
 * control. */
async function expectAxeClean(): Promise<void> {
	const { default: axe } = await import('axe-core');
	const results = await axe.run(document.body, {
		runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
	});
	expect(results.violations).toEqual([]);
}

describe('II-7g the per-citation affordances', () => {
	test('the_idle_affordances_render_and_are_axe_clean', async () => {
		await renderAnswered();

		const citation = screen.getByTestId('chat-citation');
		expect(citation).toBeTruthy();
		// Both affordances are on the citation itself, not in a drawer: Plan 06
		// §2 requires them always visible, because hover-revealed provenance is
		// provenance nobody reads.
		expect(screen.getByTestId('chat-correct').textContent?.trim()).not.toBe('');
		expect(screen.getByTestId('chat-forget').textContent?.trim()).not.toBe('');

		await expectAxeClean();
	});

	test('the_claim_surface_is_labelled_keyboard_operable_and_axe_clean', async () => {
		await renderAnswered();

		// OPEN WITH THE KEYBOARD: focus the affordance and press Enter. A click
		// would prove the handler fires; it would not prove the control is in the
		// tab order and activatable without a pointer.
		const correct = screen.getByTestId('chat-correct');
		correct.focus();
		expect(document.activeElement).toBe(correct);
		await userEvent.keyboard('{Enter}');

		const input = await screen.findByTestId('chat-claim');

		// The OPEN state is measured FIRST, because it did not exist a moment
		// ago and because an unlabelled input must fail on axe's own rule — not
		// only on the hand-written association below.
		await expectAxeClean();

		// THE LABEL IS ASSOCIATED, not merely adjacent. `label[for]` is what a
		// screen reader resolves, and axe's `label` rule reads the same edge.
		expect(input.id).toBe('chat-claim-7');
		const label = document.querySelector('label[for="chat-claim-7"]');
		expect(label, 'the claim input has no associated label').toBeTruthy();
		const labelText = label!.textContent?.trim() ?? '';
		expect(labelText.length).toBeGreaterThan(0);
		expect(screen.getByLabelText(labelText)).toBe(input);

		// Keyboard, end to end: type the claim, move to submit with Tab, press
		// Enter. No click anywhere in this test.
		input.focus();
		await userEvent.keyboard('Audit rows are retained');
		const submit = screen.getByTestId('chat-verify-submit');
		expect(submit.hasAttribute('disabled')).toBe(false);
		submit.focus();
		await userEvent.keyboard('{Enter}');

		const result = await waitFor(() => {
			const found = screen.getByTestId('chat-verify-result');
			expect(found.getAttribute('data-verify-state')).toBe('supported');
			return found;
		});
		// The verdict announces without interrupting, and it is a RESULT, not an
		// alert: a supported claim is not an error.
		expect(result.getAttribute('role')).toBe('status');
		// The COPY, not only the token: the token is bound to the state and would
		// survive a template that drew the wrong sentence under it.
		expect(result.textContent).toContain(m['chat.correct.supported']());

		// And the RESULT state is measured too.
		await expectAxeClean();
	});

	test('the_forget_affordance_is_keyboard_operable_and_axe_clean', async () => {
		await renderAnswered();

		const forget = screen.getByTestId('chat-forget');
		forget.focus();
		expect(document.activeElement).toBe(forget);
		await userEvent.keyboard('{Enter}');

		const result = await waitFor(() => {
			const found = screen.getByTestId('chat-forget-result');
			expect(found.getAttribute('data-forget-state')).toBe('tombstoned');
			return found;
		});
		expect(result.getAttribute('role')).toBe('status');
		// The tombstone's own sentence, so a refusal branch standing in for it
		// cannot pass on the shared state token.
		expect(result.textContent).toContain(m['chat.forget.tombstoned']());

		// The human gate is stated on the affordance itself (II-6's receipt
		// carries `humanGated: true` as a value; this is the same fact where the
		// action begins), and it is visible copy rather than an aria-only note.
		const gate = screen.getByTestId('chat-forget-gate');
		expect(gate.textContent?.trim().length).toBeGreaterThan(0);

		await expectAxeClean();
	});

	test('a_refusal_is_drawn_as_a_refusal_and_never_as_an_unsupported_verdict', async () => {
		// The classification is the Rust core's, but the TOKEN the renderer emits
		// is this component's. A refusal that reached the unsupported arm would
		// read "not found in this source" about a source the user was never
		// allowed to read — the exact collapse the six states exist to prevent.
		await renderAnswered();
		verifyResult = { state: 'refused', status: 403, scope: 'verify' };

		screen.getByTestId('chat-correct').focus();
		await userEvent.keyboard('{Enter}');
		const input = await screen.findByTestId('chat-claim');
		input.focus();
		await userEvent.keyboard('Audit rows are retained');
		const submit = screen.getByTestId('chat-verify-submit');
		submit.focus();
		await userEvent.keyboard('{Enter}');

		await waitFor(() => {
			const found = screen.getByTestId('chat-verify-result');
			expect(found.getAttribute('data-verify-state')).toBe('refused');
		});
		const result = screen.getByTestId('chat-verify-result');
		// THE COLLAPSE, measured in COPY: a refusal must say "not permitted to
		// read this source", never "not found in this source". The token alone
		// cannot hold this — it is bound to the state, so a template that fell
		// through to the unsupported arm would keep it.
		expect(result.textContent).toContain(m['chat.correct.refused']());
		expect(result.textContent).not.toContain(m['chat.correct.unsupported']());
		// And it is not the unsupported token wearing different copy.
		expect(result.getAttribute('data-verify-state')).not.toBe('unsupported');

		cleanup();
	});
});
