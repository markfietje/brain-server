import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, vi, afterEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

// ONE hoisted mock for $app/state with a MUTABLE params holder — the page
// module (and this mock) evaluates once per test file; per-test navigation
// happens by mutating, never by re-registering the factory.
const pageState = { params: { trace_id: '4242' } as Record<string, string> };
vi.mock('$app/state', () => ({ page: pageState }));

// ONE identity-stable fetch mock for the whole file: openapi-fetch binds
// `fetch` ONCE (the createClient default-param capture of globalThis.fetch),
// so the stubbed global's IDENTITY must never change — per-test wires swap
// the responder underneath, never the global itself.
type Responder = (url: string) => Promise<Response>;
const noWire: Responder = async () => new Response('no wire', { status: 599 });
let responder: Responder = noWire;
const fetchMock = vi.fn(async (input: RequestInfo | URL) =>
	responder(input instanceof Request ? input.url : String(input))
);
vi.stubGlobal('fetch', fetchMock);

/** Points the (identity-stable) wire at a trace response for this test. */
function serveTraceWire(body: string | null, status = 200): void {
	responder = async (url) =>
		url.includes('/recall/') && url.includes('/trace')
			? new Response(body, { status, headers: { 'content-type': 'application/json' } })
			: new Response('not found', { status: 404 });
}

afterEach(() => {
	responder = noWire;
});

/**
 * The R25 named tests (R25_PREREG §8): the recall-trace replay view —
 * the defensive parse (parse-or-refuse, nothing invented), the Dioxus
 * parity rendering with invisible chars stripped BEFORE render, the
 * verbatim-bytes export (the byte-parity gate, filename trace-{id}.json),
 * and the honest 404 state. The wire is stubbed at fetch (the openapi-fetch
 * seam) so the typed client and the capture middleware run FOR REAL.
 */

// The ONE-source-of-truth fixture: the committed R24 wire shape.
const fixtureBytes = readFileSync(
	join(process.cwd(), 'tests', 'fixtures', 'recall-trace.json'),
	'utf8'
);
const fixture = JSON.parse(fixtureBytes) as Record<string, unknown>;

async function renderTracePage(traceId: string): Promise<void> {
	pageState.params = { trace_id: traceId };
	const { default: TracePage } = await import('../src/routes/recall/[trace_id]/trace/+page.svelte');
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	render(TracePage as any);
	// The view resolves after the stubbed wire round-trip.
	await waitFor(
		() => {
			expect(
				screen.queryByTestId('trace-view') ?? screen.queryByTestId('trace-not-found')
			).not.toBeNull();
		},
		{ timeout: 5000 }
	);
}

test('recall_trace_parses_defensively_from_the_wire', async () => {
	const { parseRecallTrace } = await import('../src/lib/trace');

	// The committed fixture parses to the pinned parity schema, exactly.
	const view = parseRecallTrace(fixture);
	expect(view.decision).toBe('Ok');
	expect(view.actor).toBe('loopback');
	expect(view.query_hash).toBe(fixture['query_hash']);
	expect(view.domains_searched).toEqual(['global']);
	expect(view.scope).toBeNull(); // honest absence — the fixture's scope is null
	expect(view.hits).toEqual([
		{
			id: '1',
			score: '0.03333333507180214',
			source: 'both',
			relevance: 'low',
			assertion_kind: 'stated',
			decayed: null // absent/non-bool decayed → null, never invented
		}
	]);

	// Hostile inputs parse-or-refuse honestly: no throw, no invented value.
	expect(parseRecallTrace(null)).toEqual({
		decision: null,
		actor: null,
		query_hash: null,
		scope: null,
		domains_searched: null,
		hits: null
	});
	expect(parseRecallTrace(42).decision).toBeNull();
	expect(parseRecallTrace('trace').hits).toBeNull();
	expect(parseRecallTrace({ decision: 42 }).decision).toBeNull();
	expect(parseRecallTrace({ hits: 'all of them' }).hits).toBeNull();
	expect(parseRecallTrace({ hits: [null, 'x', { id: 1 }] })).toEqual({
		decision: null,
		actor: null,
		query_hash: null,
		scope: null,
		domains_searched: null,
		hits: [{ id: '1', score: null, source: null, relevance: null, assertion_kind: null, decayed: null }]
	});
	// Non-integer id and non-finite score are refused, not stringified.
	expect(parseRecallTrace({ hits: [{ id: 1.5 }, { score: Number.NaN }] }).hits).toEqual([
		{ id: null, score: null, source: null, relevance: null, assertion_kind: null, decayed: null },
		{ id: null, score: null, source: null, relevance: null, assertion_kind: null, decayed: null }
	]);
	// Lists: strings kept (stripped), non-strings dropped; a stripped-empty
	// string is still the wire's own value — kept, never invented away.
	expect(parseRecallTrace({ scope: ['global', 5, ''] }).scope).toEqual(['global', '']);
	expect(parseRecallTrace({ scope: [] }).scope).toBeNull();
	expect(parseRecallTrace({ scope: 'global' }).scope).toBeNull();
});

test('trace_view_renders_parity_fields_with_invisible_chars_stripped', async () => {
	const hostile = {
		...fixture,
		decision: 'O\u200Bk', // zero-width smuggled into the stored decision
		query_hash: '\uFEFF1f3cb94dd34485f8',
		domains_searched: ['glo\u202Ebdom'] // bidi override in a stored label
	};
	serveTraceWire(JSON.stringify(hostile));
	await renderTracePage('4242');

	// The parity field set is visible — with the invisible bytes ALREADY
	// stripped (the v1.20.3 boundary, parser-side).
	const decision = screen.getByTestId('trace-field-decision');
	expect(decision.textContent).toBe('Ok');
	const qhash = screen.getByTestId('trace-field-query-hash');
	expect(qhash.textContent).toBe('1f3cb94dd34485f8');
	expect(screen.getByTestId('trace-field-domains').textContent).toBe('globdom');
	expect(screen.getByTestId('trace-field-actor').textContent).toBe('loopback');
	// The fixture's scope is null → the honest — absence.
	expect(screen.getByTestId('trace-field-scope').textContent).toBe('—');
	// The per-hit row: id/score/source/relevance/assertion_kind/decayed.
	const row = screen.getByTestId('trace-hit-row');
	expect(row.textContent).toContain('0.03333333507180214');
	expect(row.textContent).toContain('stated');
	// decayed null renders the — absence, never a fabricated marker.
	expect(row.textContent).toContain('—');
	// The raw form stays alongside (the evidence surface keeps both).
	expect(screen.getByTestId('trace-json').textContent).toContain('"decision":');
	expect(screen.getByTestId('trace-export')).toBeTruthy();
});

test('trace_export_downloads_verbatim_bytes_named_per_the_parity_law', async () => {
	serveTraceWire(fixtureBytes); // the EXACT committed wire bytes, compact
	const user = userEvent.setup();
	await renderTracePage('4242');

	const created: Blob[] = [];
	Object.defineProperty(URL, 'createObjectURL', {
		configurable: true,
		writable: true,
		value: vi.fn((blob: Blob) => {
			created.push(blob);
			return 'blob:trace-e2e';
		})
	});
	const revoke = vi.fn();
	Object.defineProperty(URL, 'revokeObjectURL', {
		configurable: true,
		writable: true,
		value: revoke
	});
	const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});

	await user.click(screen.getByTestId('trace-export'));

	expect(created).toHaveLength(1);
	expect(await created[0]!.text()).toBe(fixtureBytes); // BYTE PARITY: the exact wire bytes
	expect(click).toHaveBeenCalled();
	const anchor = click.mock.instances[0] as HTMLAnchorElement;
	expect(anchor.download).toBe('trace-4242.json'); // the pinned filename law
	expect(revoke).toHaveBeenCalledWith('blob:trace-e2e');
	click.mockRestore();
	Reflect.deleteProperty(URL, 'createObjectURL');
	Reflect.deleteProperty(URL, 'revokeObjectURL');
});

test('trace_404_renders_the_honest_state', async () => {
	serveTraceWire(null, 404);
	await renderTracePage('999999');

	const notFound = screen.getByTestId('trace-not-found');
	expect(notFound).toBeTruthy();
	expect(notFound.textContent).toContain('No trace is stored for this id.');
	// The honest 404: no view, no table, no export — nothing invented.
	expect(screen.queryByTestId('trace-view')).toBeNull();
	expect(screen.queryByTestId('trace-hits-table')).toBeNull();
	expect(screen.queryByTestId('trace-export')).toBeNull();
	expect(screen.queryByTestId('trace-json')).toBeNull();
});
