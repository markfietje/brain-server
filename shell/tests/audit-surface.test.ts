import { afterEach, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { pageState } from './fixtures/page-state.svelte.js';

/**
 * The audit surface's pins. The kernel's hash chain is the product's
 * self-verifying evidence, and this surface DISPLAYS the verdict — so the
 * pins hold the one behaviour a display can get catastrophically wrong:
 * saying "verified" when the chain was not verified.
 *
 *   A1  the live render: the verified chip, the events table, the count
 *   A2  a false verdict reads "BROKEN"; a refused verify reads "not
 *       verified" — and never the other words
 *   A3  a refused events read renders the REFUSAL with its reason, not an
 *       empty log; 403 names Admin, 401 names the token
 *   A4  an unreachable kernel is the OFFLINE state, not an empty log
 *   A5  a body the surface cannot prove is MALFORMED
 *   A6  Load more pages with offset and APPENDS
 *   A7  the surface is read-only: no write verb anywhere it reaches
 */

const state = vi.hoisted(() => ({ wireStatus: 'unreachable' }));

// The `$app/state` page holder: a REACTIVE fixture (page-state.svelte.ts) —
// the M3-g principal filter reads the URL at mount, the since filter's Apply
// writes it and re-reads, and the deriveds must follow the write the way
// they follow the router's own navigation. The `$app/navigation` mock moves
// the same proxy, exactly like the real goto.
vi.mock('$app/state', async () => ({
	page: (await import('./fixtures/page-state.svelte.js')).pageState
}));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));
const gotoMock = vi.hoisted(() =>
	vi.fn(async (to: string) => {
		const { pageState: reactive } = await import('./fixtures/page-state.svelte.js');
		reactive.url = new URL(to, 'http://127.0.0.1/');
	})
);
vi.mock('$app/navigation', () => ({ goto: gotoMock }));

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string) => {
		if (command === 'kernel_wire_status') return { kind: state.wireStatus };
		throw new Error(`unexpected command ${command}`);
	})
}));

type Responder = (url: string, init: RequestInit | undefined, method: string) => Promise<Response>;
const noWire: Responder = async () => new Response('no wire', { status: 599 });
let responder: Responder = noWire;
const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
	const url = input instanceof Request ? input.url : String(input);
	const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
	return responder(url, init, method);
});
vi.stubGlobal('fetch', fetchMock);

afterEach(() => {
	cleanup();
	responder = noWire;
	state.wireStatus = 'unreachable';
	pageState.url = new URL('http://127.0.0.1/audit');
	gotoMock.mockClear();
	vi.clearAllMocks();
});

function jsonResponse(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

function auditEvent(id: number, kind = 'recall', status = 'ok'): Record<string, unknown> {
	return {
		id,
		ts: `2026-10-25T13:41:0${id % 10}Z`,
		kind,
		actor: 'operator',
		target_hash: 'a91f'.padEnd(64, '0'),
		status,
		detail_hash: 'd'.repeat(64),
		tenant_id: 'global',
		domain: 'global'
	};
}

/** Serve a list of events split across pages of `per` rows. */
function serveEvents(rows: Record<string, unknown>[], verify: unknown, verifyStatus = 200): void {
	responder = async (url, _init, method) => {
		const path = new URL(url).pathname;
		if (method === 'GET' && path === '/audit') {
			const params = new URL(url).searchParams;
			const offset = Number.parseInt(params.get('offset') ?? '0', 10);
			const limit = Number.parseInt(params.get('limit') ?? '100', 10);
			return jsonResponse({ events: rows.slice(offset, offset + limit) });
		}
		if (method === 'GET' && path === '/audit/verify') {
			return jsonResponse(verify, verifyStatus);
		}
		return new Response('not found', { status: 404 });
	};
}

async function renderAudit(): Promise<void> {
	const mod = await import('../src/routes/audit/+page.svelte');
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	render(mod.default as any);
}

test('A1 the live render shows the verified chain, the events and the loaded count', async () => {
	serveEvents([auditEvent(7, 'recall'), auditEvent(8, 'ingest')], { ok: true, domains: { global: true } });
	await renderAudit();

	await waitFor(() => expect(screen.getAllByTestId('audit-row')).toHaveLength(2));
	const chain = screen.getByTestId('audit-chain');
	expect(chain.getAttribute('data-chain')).toBe('verified');
	expect(chain.textContent).toContain('chain verified');
	expect(screen.getByTestId('audit-count').textContent).toContain('2');
	const rows = screen.getAllByTestId('audit-row');
	expect(rows).toHaveLength(2);
	expect(rows[0]!.textContent).toContain('recall');
	expect(rows[0]!.textContent).toContain('a91f');
	expect(rows[1]!.textContent).toContain('ingest');
});

test('A2 a false verdict reads BROKEN and a refused verify reads not verified', async () => {
	// ── ok:false — the chain is BROKEN, named ──
	serveEvents([auditEvent(7)], { ok: false, domains: { global: false } });
	await renderAudit();
	await waitFor(() =>
		expect(screen.getByTestId('audit-chain').getAttribute('data-chain')).toBe('broken')
	);
	expect(screen.getByTestId('audit-chain').textContent).toContain('BROKEN');
	expect(screen.getByTestId('audit-chain').textContent).not.toContain('verified ✓');
	cleanup();

	// ── the verify read refused — NOT verified, never "verified" on a guess ──
	serveEvents([auditEvent(7)], { error: 'forbidden' }, 403);
	await renderAudit();
	await waitFor(() =>
		expect(screen.getByTestId('audit-chain').getAttribute('data-chain')).toBe('unverified')
	);
	expect(screen.getByTestId('audit-chain').textContent).toContain('not verified');
	expect(screen.getByTestId('audit-chain').textContent).not.toContain('verified ✓');
	expect(screen.getByTestId('audit-chain').textContent).not.toContain('BROKEN');
});

test('A3 a refused events read renders the refusal, never an empty log', async () => {
	// 403 — this bearer may not read the audit log; Admin is named.
	responder = async (url, _init, method) =>
		method === 'GET' && new URL(url).pathname === '/audit'
			? jsonResponse({ error: 'forbidden' }, 403)
			: new Response('not found', { status: 404 });
	await renderAudit();
	await waitForTestId('audit-refused');
	expect(screen.getByTestId('audit-refused-reason').textContent).toContain('Admin');
	// The empty state is a DIFFERENT fact and is not rendered over a refusal.
	expect(screen.queryByTestId('audit-empty')).toBeNull();
	// And the retry affordance exists: the state names the fact and offers
	// the next read.
	expect(screen.getByTestId('audit-retry')).toBeTruthy();
	cleanup();

	// 401 — no credential was accepted; the token is named.
	responder = async (url, _init, method) =>
		method === 'GET' && new URL(url).pathname === '/audit'
			? jsonResponse({ error: 'unauthorized' }, 401)
			: new Response('not found', { status: 404 });
	await renderAudit();
	await waitForTestId('audit-refused');
	expect(screen.getByTestId('audit-refused-reason').textContent).toContain('token');
});

test('A4 an unreachable kernel is the offline state, not an empty log', async () => {
	// A REAL wire failure: fetch rejects, and the core classifies the fault.
	responder = async () => {
		throw new TypeError('Failed to fetch');
	};
	await renderAudit();
	await waitForTestId('audit-offline');
	expect(screen.queryByTestId('audit-empty')).toBeNull();
});

test('A5 a body the surface cannot prove is malformed, not rows', async () => {
	responder = async (url, _init, method) =>
		method === 'GET' && new URL(url).pathname === '/audit'
			? jsonResponse({ events: [{ id: 'not-a-number' }] })
			: new Response('not found', { status: 404 });
	await renderAudit();
	await waitForTestId('audit-malformed');
	expect(screen.queryByTestId('audit-row')).toBeNull();
});

test('A6 load_more_pages_with_offset_and_appends', async () => {
	// A FULL first page (the surface's own page width) means the kernel may
	// hold more; a short page means the log is exhausted and the button is
	// not offered. 101 rows: one full page, then one more.
	const { AUDIT_PAGE } = await import('../src/lib/api/audit.js');
	const rows = Array.from({ length: AUDIT_PAGE + 1 }, (_, i) => auditEvent(i + 1));
	serveEvents(rows, { ok: true, domains: { global: true } });
	await renderAudit();
	await waitFor(() => expect(screen.getAllByTestId('audit-row')).toHaveLength(AUDIT_PAGE));

	const user = userEvent.setup();
	await user.click(screen.getByTestId('audit-load-more'));
	await waitFor(() => expect(screen.getAllByTestId('audit-row')).toHaveLength(AUDIT_PAGE + 1));
	// The second read asked for the page AFTER what is on screen, and the
	// short page it got retires the button.
	const offsets = fetchMock.mock.calls
		.map((call) => {
			const u = call[0] instanceof Request ? (call[0] as Request).url : String(call[0]);
			return new URL(u).pathname === '/audit'
				? new URL(u).searchParams.get('offset')
				: null;
		})
		.filter((v) => v !== null);
	expect(offsets).toEqual(['0', String(AUDIT_PAGE)]);
	expect(screen.queryByTestId('audit-load-more')).toBeNull();
});

test('A7 the audit surface reaches no write verb', async () => {
	// The audit surface is a READ of the kernel's evidence. A write here
	// would make it a different surface with a different law, so the page
	// and the api module it imports are scanned for one.
	const { readFileSync } = await import('node:fs');
	const { join } = await import('node:path');
	const { shellRoot } = await import('./fixtures');
	const sources = [
		readFileSync(join(shellRoot, 'src', 'routes', 'audit', '+page.svelte'), 'utf8'),
		readFileSync(join(shellRoot, 'src', 'lib', 'api', 'audit.ts'), 'utf8')
	];
	for (const source of sources) {
		expect(
			source.match(/client\.(POST|PUT|DELETE|PATCH)\(/g),
			'the audit surface must reach no write verb — it displays the kernel\'s evidence'
		).toBeNull();
	}
});

async function waitForTestId(testid: string): Promise<void> {
	await waitFor(() => expect(screen.getByTestId(testid)).toBeTruthy());
}

test('A4 the URL principal filter reaches the wire and names itself on the page', async () => {
	// The retired GUI's `/audit?principal=` promise, kept: the URL is the
	// source, the request carries the filter, and the page NAMES the filter
	// (the chip) beside its clear link — a filter that silently changed the
	// rows would be indistinguishable from a smaller chain.
	let requested: URL | null = null;
	const rows = [auditEvent(7), auditEvent(8)];
	responder = async (url, _init, method) => {
		const u = new URL(url);
		if (method === 'GET' && u.pathname === '/audit') {
			requested = u;
			const offset = Number.parseInt(u.searchParams.get('offset') ?? '0', 10);
			const limit = Number.parseInt(u.searchParams.get('limit') ?? '100', 10);
			return jsonResponse({ events: rows.slice(offset, offset + limit) });
		}
		if (method === 'GET' && u.pathname === '/audit/verify') {
			return jsonResponse({ ok: true, domains: { global: true } });
		}
		return new Response('not found', { status: 404 });
	};
	pageState.url = new URL('http://127.0.0.1/audit?principal=edu-plugin');
	await renderAudit();
	await waitFor(() => expect(screen.getAllByTestId('audit-row')).toHaveLength(2));
	expect(requested).not.toBeNull();
	expect(requested!.searchParams.get('principal')).toBe('edu-plugin');
	const chip = screen.getByTestId('audit-principal-filter');
	expect(chip.textContent).toContain('edu-plugin');
	expect(screen.getByTestId('audit-principal-clear').getAttribute('href')).toBe('/audit');
	// and the verify read rode the SAME page load, filter-free (the verdict
	// surface is a separate endpoint the filter never touches)
	expect(screen.getByTestId('audit-chain').getAttribute('data-chain')).toBe('verified');
});

test('A5 no filter in the URL sends no principal param and renders no chip', async () => {
	let requested: URL | null = null;
	responder = async (url, _init, method) => {
		const u = new URL(url);
		if (method === 'GET' && u.pathname === '/audit') {
			requested = u;
			return jsonResponse({ events: [auditEvent(7)] });
		}
		if (method === 'GET' && u.pathname === '/audit/verify') {
			return jsonResponse({ ok: true, domains: { global: true } });
		}
		return new Response('not found', { status: 404 });
	};
	await renderAudit();
	await waitFor(() => expect(screen.getAllByTestId('audit-row')).toHaveLength(1));
	expect(requested).not.toBeNull();
	expect(requested!.searchParams.get('principal')).toBeNull();
	expect(screen.queryByTestId('audit-principal-filter')).toBeNull();
});

// ═══ THE SINCE ARM (the audit-filter round) ══════════════════════════════
// The retired GUI's other URL promise. The URL is the source; the request
// carries the bound; the chip names it; it composes with the principal
// filter; and the input's Apply WRITES the URL and re-reads — a filter that
// silently changed the rows would be indistinguishable from a smaller chain.

const serveSince = (): { requested: () => URL | null } => {
	const holder: { u: URL | null } = { u: null };
	responder = async (url, _init, method) => {
		const u = new URL(url);
		if (method === 'GET' && u.pathname === '/audit') {
			holder.u = u;
			return jsonResponse({ events: [auditEvent(7)] });
		}
		if (method === 'GET' && u.pathname === '/audit/verify') {
			return jsonResponse({ ok: true, domains: { global: true } });
		}
		return new Response('not found', { status: 404 });
	};
	return { requested: () => holder.u };
};

test('A8 the URL since filter reaches the wire, composes with principal, and names itself', async () => {
	const { requested } = serveSince();
	pageState.url = new URL(
		'http://127.0.0.1/audit?since=2026-10-25T13%3A00%3A00Z&principal=edu-plugin'
	);
	await renderAudit();
	await waitFor(() => expect(screen.getAllByTestId('audit-row')).toHaveLength(1));
	expect(requested()!.searchParams.get('since')).toBe('2026-10-25T13:00:00Z');
	// COMPOSITION: both filters ride the SAME request.
	expect(requested()!.searchParams.get('principal')).toBe('edu-plugin');
	// The chip NAMES the bound; its clear link rides the page's own "show
	// all" vocabulary (the composition lives at the wire and at the Apply,
	// which keeps the principal filter when it writes the bound).
	const chip = screen.getByTestId('audit-since-filter');
	expect(chip.textContent).toContain('2026-10-25T13:00:00Z');
	expect(screen.getByTestId('audit-since-clear').getAttribute('href')).toBe('/audit');
	// And the verify read rode the SAME page load, filter-free.
	expect(screen.getByTestId('audit-chain').getAttribute('data-chain')).toBe('verified');
});

test('A9 the since input writes the URL as ISO-8601 and the rows follow the write', async () => {
	const { requested } = serveSince();
	await renderAudit();
	await waitFor(() => expect(screen.getAllByTestId('audit-row')).toHaveLength(1));
	// No bound yet: no param, no chip.
	expect(requested()!.searchParams.get('since')).toBeNull();
	expect(screen.queryByTestId('audit-since-filter')).toBeNull();

	const user = userEvent.setup();
	// datetime-local speaks the local wall clock; the wire gets ISO-8601.
	await user.type(screen.getByTestId('audit-since-input'), '2026-10-25T13:00');
	await user.click(screen.getByTestId('audit-since-apply'));
	// The write landed on the URL, and the re-read that follows it carried
	// the bound to the wire — the rows cannot silently disagree with the
	// filter the URL now carries.
	await waitFor(() => {
		expect(gotoMock).toHaveBeenCalledTimes(1);
		expect(requested()!.searchParams.get('since')).not.toBeNull();
	});
	const since = requested()!.searchParams.get('since')!;
	expect(Number.isNaN(Date.parse(since))).toBe(false);
	expect(since.endsWith('Z')).toBe(true);
	// And the chip names the bound the URL now carries.
	await waitFor(() => expect(screen.getByTestId('audit-since-filter')).toBeTruthy());
	expect(screen.getByTestId('audit-since-filter').textContent).toContain(since);
});

test('A10 an empty since input applied clears the bound from the URL', async () => {
	const { requested } = serveSince();
	pageState.url = new URL('http://127.0.0.1/audit?since=2026-10-25T13%3A00%3A00Z');
	await renderAudit();
	await waitFor(() => expect(screen.getAllByTestId('audit-row')).toHaveLength(1));
	expect(requested()!.searchParams.get('since')).not.toBeNull();

	const user = userEvent.setup();
	await user.click(screen.getByTestId('audit-since-apply'));
	await waitFor(() => expect(gotoMock).toHaveBeenCalledTimes(1));
	expect(gotoMock.mock.calls[0]?.[0]).toBe('/audit');
});
