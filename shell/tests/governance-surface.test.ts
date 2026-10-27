import { afterEach, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';

/**
 * The governance surface's pins. Three reads with two scopes, and the law
 * is that each section renders ITS OWN answer: a Read bearer must see the
 * domains and the roles live beside a NAMED refusal for retention, never a
 * page that went blank because one of its reads was refused.
 *
 *   G1  the live render: a domain row, a role row, the retention card
 *   G2  a refused retention leaves domains and roles LIVE and names Admin
 *   G3  an unreachable kernel is the OFFLINE state with a retry
 *   G4  a body the surface cannot prove is MALFORMED, per section
 *   G5  the decision-run ledger is LINKED, not folded in
 *   G6  the surface is read-only: no write verb anywhere it reaches
 */

const state = vi.hoisted(() => ({ wireStatus: 'unreachable' }));

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string) => {
		if (command === 'kernel_wire_status') return { kind: state.wireStatus };
		throw new Error(`unexpected command ${command}`);
	})
}));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));

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
	vi.clearAllMocks();
});

function jsonResponse(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

const DOMAINS = {
	domains: [{ name: 'global', entries: 8745, entities: 120, relations: 48, multi_db: false }]
};
const ROLES = {
	roles: [
		{
			name: 'operator',
			description: 'the one who runs this kernel',
			scopes: ['private', 'domain'],
			owner_filter: 'all',
			can: ['read', 'write', 'approve']
		}
	]
};
const RETENTION = { enabled: true, policy: { chunk: 30, procedure: 180 } };

function serve(bodies: Readonly<Record<string, () => Response>>): void {
	responder = async (url) => {
		const path = new URL(url).pathname;
		const body = bodies[path];
		return body === undefined ? new Response('not found', { status: 404 }) : body();
	};
}

async function renderGovernance(): Promise<void> {
	const mod = await import('../src/routes/governance/+page.svelte');
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	render(mod.default as any);
}

test('G1 the live render shows domains, roles and retention', async () => {
	serve({
		'/domains': () => jsonResponse(DOMAINS),
		'/roles': () => jsonResponse(ROLES),
		'/retention': () => jsonResponse(RETENTION)
	});
	await renderGovernance();
	await waitFor(() => expect(screen.getByTestId('governance-domain-row')).toBeTruthy());
	expect(screen.getByTestId('governance-domain-row').textContent).toContain('global');
	expect(screen.getByTestId('governance-domain-row').textContent).toContain('8745');
	expect(screen.getByTestId('governance-role-row').textContent).toContain('operator');
	expect(screen.getByTestId('governance-role-row').textContent).toContain('approve');
	const retention = screen.getByTestId('governance-retention-card');
	expect(retention.getAttribute('data-enabled')).toBe('true');
	expect(retention.textContent).toContain('chunk: 30 days');
});

test('G2 a refused retention leaves domains and roles live and names Admin', async () => {
	serve({
		'/domains': () => jsonResponse(DOMAINS),
		'/roles': () => jsonResponse(ROLES),
		'/retention': () => jsonResponse({ error: 'forbidden' }, 403)
	});
	await renderGovernance();
	await waitFor(() => expect(screen.getByTestId('governance-retention-refused')).toBeTruthy());
	expect(screen.getByTestId('governance-retention-refused').textContent).toContain('Admin');
	// The other two sections are LIVE — one refused read does not blank the page.
	expect(screen.getByTestId('governance-domain-row')).toBeTruthy();
	expect(screen.getByTestId('governance-role-row')).toBeTruthy();
	expect(screen.queryByTestId('governance-retention-card')).toBeNull();
});

test('G3 an unreachable kernel is the offline state with a retry', async () => {
	responder = async () => {
		throw new TypeError('Failed to fetch');
	};
	await renderGovernance();
	await waitFor(() => expect(screen.getByTestId('governance-offline')).toBeTruthy());
	expect(screen.getByTestId('governance-retry')).toBeTruthy();
	expect(screen.queryByTestId('governance-domain-row')).toBeNull();
});

test('G4 a body the surface cannot prove is malformed, per section', async () => {
	serve({
		'/domains': () => jsonResponse({ domains: 'not-a-list' }),
		'/roles': () => jsonResponse(ROLES),
		'/retention': () => jsonResponse(RETENTION)
	});
	await renderGovernance();
	await waitFor(() => expect(screen.getByTestId('governance-role-row')).toBeTruthy());
	// The domains section says so; it does not render an empty list over a
	// body it could not read.
	expect(screen.queryByTestId('governance-domains-empty')).toBeNull();
	expect(screen.queryByTestId('governance-domain-row')).toBeNull();
	const alerts = screen.getAllByRole('alert');
	expect(alerts.some((el) => (el.textContent ?? '').includes('cannot read'))).toBe(true);
});

test('G5 the decision-run ledger is linked, not folded in', async () => {
	serve({
		'/domains': () => jsonResponse(DOMAINS),
		'/roles': () => jsonResponse(ROLES),
		'/retention': () => jsonResponse(RETENTION)
	});
	await renderGovernance();
	await waitFor(() => expect(screen.getByTestId('governance-decisions-link')).toBeTruthy());
	expect(screen.getByTestId('governance-decisions-link').getAttribute('href')).toBe('/decisions');
	// And the page did NOT read the decision-run listing itself.
	const paths = fetchMock.mock.calls.map((call) => {
		const u = call[0] instanceof Request ? (call[0] as Request).url : String(call[0]);
		return new URL(u).pathname;
	});
	expect(paths).not.toContain('/workflow/decision-runs');
});

test('G6 the governance surface reaches no write verb', async () => {
	const { readFileSync } = await import('node:fs');
	const { join } = await import('node:path');
	const { shellRoot } = await import('./fixtures');
	const sources = [
		readFileSync(join(shellRoot, 'src', 'routes', 'governance', '+page.svelte'), 'utf8'),
		readFileSync(join(shellRoot, 'src', 'lib', 'api', 'governance.ts'), 'utf8')
	];
	for (const source of sources) {
		expect(
			source.match(/client\.(POST|PUT|DELETE|PATCH)\(/g),
			'the governance surface must reach no write verb — it displays what the kernel enforces'
		).toBeNull();
	}
});

test('G7 the role row links the audited footprint, answered by the chain', async () => {
	// M3-g (plan 27 §2.3): the roles section LINKS to the audit surface's
	// principal filter — "what did a principal of this role touch?" is
	// answered by the chain, never by a new telemetry, and never by this
	// page reading the audit log itself.
	serve({
		'/domains': () => jsonResponse(DOMAINS),
		'/roles': () => jsonResponse(ROLES),
		'/retention': () => jsonResponse(RETENTION)
	});
	await renderGovernance();
	await waitFor(() =>
		expect(screen.getAllByTestId('governance-role-footprint').length).toBeGreaterThan(0)
	);
	const links = screen.getAllByTestId('governance-role-footprint');
	expect(links).toHaveLength(1); // the fixture carries one role row
	expect(links[0]!.getAttribute('href')).toBe('/audit?principal=operator');
	// and the page did NOT read the chain itself — the link is the whole act
	const paths = fetchMock.mock.calls.map((call) => {
		const u = call[0] instanceof Request ? (call[0] as Request).url : String(call[0]);
		return new URL(u).pathname;
	});
	expect(paths).not.toContain('/audit');
});
