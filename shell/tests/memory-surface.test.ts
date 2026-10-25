import { afterEach, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { currentStatusContext, recordKernelFacts } from '../src/lib/shell/frame.svelte.js';

/**
 * The memory surface's pins — the sidebar frame's "Memory" entry over the
 * `/recall` wire (recall.test.ts holds the hit/abstain/trace laws; these
 * hold what the frame added).
 *
 *   M1  the cards carry the kernel's OWN facts as badges: the score with
 *       its band, the memory kind and origin, the validity window, the
 *       untrusted mark and the quarantine mark — each only when the kernel
 *       stated it
 *   M2  the kind filter narrows CLIENT-SIDE and never re-asks the kernel
 *   M3  "Add memory" files a PROPOSAL to /ingest/proposal with content and
 *       kind, and says so with the proposal's id; it never posts a memory
 *   M4  an empty draft is refused locally; a kernel refusal is named
 *   M5  the status bar's context states what is on screen, and clears
 *
 * Why M3 is the one that matters: a memory typed into a webview must enter
 * the store through Review like every other candidate. The route-parity
 * census holds the same law structurally (the call site is read); this pin
 * holds it behaviourally (the request that leaves is the proposal).
 */

const state = vi.hoisted(() => ({ wireStatus: 'unreachable' }));

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string) => {
		if (command === 'kernel_wire_status') return { kind: state.wireStatus };
		throw new Error(`unexpected command ${command}`);
	})
}));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));

type Responder = (url: string, body: string | null, method: string) => Promise<Response>;
const noWire: Responder = async () => new Response('no wire', { status: 599 });
let responder: Responder = noWire;
// openapi-fetch hands the stub a Request object, so the body is read off
// the request itself (cloned, so the responder may still consume it).
const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
	const url = input instanceof Request ? input.url : String(input);
	const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
	const body = input instanceof Request ? await input.clone().text() : (init?.body as string | null) ?? null;
	return responder(url, body, method);
});
vi.stubGlobal('fetch', fetchMock);

afterEach(() => {
	cleanup();
	responder = noWire;
	state.wireStatus = 'unreachable';
	recordKernelFacts(null);
	vi.clearAllMocks();
});

function jsonResponse(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

/** The kernel's RecallHit shape with the provenance facts the cards show. */
const HITS = [
	{
		id: 4,
		title: 'Green tea — Camellia sinensis',
		snippet: 'Tea made from leaves steamed and dried without oxidation.',
		score: 0.75,
		domain: 'global',
		source: 'vector',
		memory_kind: 'fact',
		origin: 'human',
		flagged: false,
		evidence: { valid_from: '2026-10-25T09:43:00Z', valid_to: null }
	},
	{
		id: 2,
		title: 'Espresso machine — descaling',
		snippet: 'Descale every four weeks.',
		score: 0.08,
		domain: 'global',
		source: 'fts',
		memory_kind: 'procedure',
		origin: 'agent',
		flagged: true
	},
	{
		id: 3,
		title: 'Backup rotation policy',
		snippet: 'Nightly snapshots to the local NAS.',
		score: 0.0,
		domain: 'global',
		memory_kind: 'fact'
	}
];

/** Record what every request posted, by path. */
const posted: Array<{ path: string; body: unknown }> = [];

function serveMemory(proposal: () => Response = () => jsonResponse({ id: 77, status: 'pending' })): void {
	posted.length = 0;
	responder = async (url, body, method) => {
		const path = new URL(url).pathname;
		if (method === 'POST' && path === '/recall') {
			return jsonResponse({ hits: HITS, decision: 'ok', domains_searched: ['global'], trace_id: null });
		}
		if (method === 'POST' && path === '/ingest/proposal') {
			posted.push({ path, body: body === null || body.length === 0 ? null : JSON.parse(body) });
			return proposal();
		}
		if (method === 'GET' && path === '/health') return jsonResponse({ status: 'ok', version: '1.29.32' });
		if (method === 'GET' && path === '/stats') return jsonResponse({ count: 8745, model: 'test-model' });
		return new Response('not found', { status: 404 });
	};
}

async function renderMemory(): Promise<ReturnType<typeof userEvent.setup>> {
	const mod = await import('../src/routes/recall/+page.svelte');
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	render(mod.default as any);
	return userEvent.setup();
}

async function search(user: ReturnType<typeof userEvent.setup>): Promise<void> {
	await user.type(screen.getByTestId('recall-input'), 'tea');
	await user.click(screen.getByRole('button', { name: 'Recall' }));
	await waitFor(() => expect(screen.getAllByTestId('recall-hit').length).toBeGreaterThan(0));
}

test('M1 the cards carry the kernel\'s own facts as badges, each only when stated', async () => {
	serveMemory();
	const user = await renderMemory();
	await search(user);

	const cards = screen.getAllByTestId('recall-hit');
	expect(cards).toHaveLength(3);

	// #4 — a stated fact from a person, valid since a date, high band.
	const tea = cards[0]!;
	expect(tea.textContent).toContain('score 0.750 · high');
	expect(tea.textContent).toContain('fact · human');
	expect(tea.textContent).toContain('valid from 2026-10-25T09:43:00Z');
	expect(tea.textContent).toContain('#4');
	expect(tea.textContent).toContain('global');
	expect(within(tea).queryByTestId('memory-untrusted')).toBeNull();
	expect(tea.textContent).not.toContain('quarantined');

	// #2 — an agent's procedure, quarantined: untrusted AND quarantined, low band.
	const espresso = cards[1]!;
	expect(espresso.textContent).toContain('score 0.080 · low');
	expect(within(espresso).getByTestId('memory-untrusted').textContent).toContain('untrusted · fenced');
	expect(espresso.textContent).toContain('quarantined');
	// The origin is NOT repeated beside the kind when it is the untrusted mark.
	expect(espresso.textContent).not.toContain('procedure · agent');

	// #3 — nothing stated beyond the kind: no validity line, no origin, no marks.
	const backup = cards[2]!;
	expect(backup.textContent).not.toContain('valid');
	expect(within(backup).queryByTestId('memory-untrusted')).toBeNull();
});

test('M2 the kind filter narrows client-side and never re-asks the kernel', async () => {
	serveMemory();
	const user = await renderMemory();
	await search(user);
	const recallCalls = (): number =>
		fetchMock.mock.calls.filter((call) => {
			const u = call[0] instanceof Request ? (call[0] as Request).url : String(call[0]);
			return new URL(u).pathname === '/recall';
		}).length;
	const before = recallCalls();

	await user.click(screen.getByTestId('memory-filter-facts'));
	expect(screen.getAllByTestId('recall-hit')).toHaveLength(2);
	expect(screen.getByTestId('memory-filter-facts').getAttribute('aria-pressed')).toBe('true');

	await user.click(screen.getByTestId('memory-filter-procedures'));
	expect(screen.getAllByTestId('recall-hit')).toHaveLength(1);
	expect(screen.getAllByTestId('recall-hit')[0]!.textContent).toContain('Espresso');

	await user.click(screen.getByTestId('memory-filter-all'));
	expect(screen.getAllByTestId('recall-hit')).toHaveLength(3);

	expect(recallCalls(), 'the filter is a view over the last answer, not a new query').toBe(before);
});

test('M3 add memory files a PROPOSAL with content and kind, and says so', async () => {
	serveMemory();
	const user = await renderMemory();

	await user.click(screen.getByTestId('memory-add'));
	const dialog = await screen.findByRole('dialog');
	await user.type(within(dialog).getByTestId('memory-add-content'), 'Descale every four weeks.');
	await user.selectOptions(within(dialog).getByTestId('memory-add-kind'), 'procedure');
	await user.click(within(dialog).getByTestId('memory-add-submit'));

	await waitFor(() => expect(screen.getByTestId('memory-add-filed')).toBeTruthy());
	expect(screen.getByTestId('memory-add-filed').textContent).toContain('#77');
	expect(screen.getByTestId('memory-add-filed').textContent).toContain('Review');

	// The request that LEFT: the proposal verb, content and kind, nothing else.
	expect(posted).toHaveLength(1);
	expect(posted[0]!.path).toBe('/ingest/proposal');
	expect(posted[0]!.body).toEqual({ content: 'Descale every four weeks.', kind: 'procedure' });

	// And no memory verb was ever called.
	const paths = fetchMock.mock.calls.map((call) => {
		const u = call[0] instanceof Request ? (call[0] as Request).url : String(call[0]);
		return new URL(u).pathname;
	});
	expect(paths).not.toContain('/add');
	expect(paths).not.toContain('/ingest/memory');
});

test('M4 an empty draft is refused locally and a kernel refusal is named', async () => {
	serveMemory(() => jsonResponse({ error: 'forbidden' }, 403));
	const user = await renderMemory();

	await user.click(screen.getByTestId('memory-add'));
	const dialog = await screen.findByRole('dialog');
	await user.click(within(dialog).getByTestId('memory-add-submit'));
	expect(within(dialog).getByRole('alert').getAttribute('data-outcome')).toBe('empty');
	expect(posted, 'an empty draft never reaches the wire').toHaveLength(0);

	await user.type(within(dialog).getByTestId('memory-add-content'), 'something');
	await user.click(within(dialog).getByTestId('memory-add-submit'));
	await waitFor(() =>
		expect(within(dialog).getByRole('alert').getAttribute('data-outcome')).toBe('refused')
	);
	expect(within(dialog).getByRole('alert').textContent).toContain('403');
	expect(screen.queryByTestId('memory-add-filed')).toBeNull();
});

test('M5 the status context states what is on screen and clears on the way out', async () => {
	serveMemory();
	const user = await renderMemory();
	expect(currentStatusContext()).toBeNull();
	await search(user);
	await waitFor(() => expect(currentStatusContext()).toBe('3 memories'));
	await user.click(screen.getByTestId('memory-filter-procedures'));
	await waitFor(() => expect(currentStatusContext()).toBe('1 memories'));
	cleanup();
	expect(currentStatusContext()).toBeNull();
});
