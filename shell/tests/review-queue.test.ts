import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { afterEach, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { shellRoot } from './fixtures';
import { CENSUS_ROUTES, ROUTE_FLOOR } from './route-census';
import { QUEUE_CACHE_KEY } from '../src/lib/queue/model.js';

/**
 * III-6 — the human review queue, DISPLAYED and nothing more.
 *
 * Six pins, and each one names the law it holds:
 *
 *   P1  the queue renders the kernel's pending rows, with the digest visible
 *   P2  the offline render shows the last-known METADATA and NO content
 *   P3  the surface contains NO decision-posting path
 *   P4  a refused queue renders a REFUSAL, not an empty queue
 *   P5  the census, the parity row, the sweep arm and the catalogs carry it
 *   P6  the parity rows for /decisions and /decisions/[id] are TRUE
 *
 * **P3 is the pin that matters most.** It is the II-6 `humanGated` pattern
 * applied to a surface: the promise is that this view never decides, and a
 * promise is held by CONSTRAINING IT, not by observing it. A button that
 * posts a decision would make this a different round with a different law (a
 * human-gated write from a webview needs its own authz proof), so the scan
 * covers the route and EVERY module it imports.
 *
 * **P2's guard is a TYPE.** The cache's row type declares `content?: never`,
 * so a row carrying the proposal text is not assignable to it; and the cache
 * READER takes only the fields it knows, so a tampered entry cannot put the
 * text on the screen either. Two layers, and the pin holds both.
 */

const state = vi.hoisted(() => ({ wireStatus: 'unreachable' }));

// The Rust core answers "which fault is the webview hitting?". The queue's
// offline arm is driven by a REAL unreachable, not by a mocked flag.
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

function jsonResponse(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

/** One wire row — the kernel's `ProposalView`, as the queue serves it. */
function proposalRow(overrides: Record<string, unknown> = {}): Record<string, unknown> {
	return {
		id: 42,
		kind: 'capture',
		content: 'the bench wiring is lead-safe and labelled',
		content_digest: 'a'.repeat(64),
		source: 'vector',
		authority: 0.8,
		novelty: 0.9137,
		conflict_with: null,
		salience: 0.512,
		created_at: 1750000000,
		screen_verdict: 'clean',
		edited_at: null,
		decided_at: null,
		expires_at: 1750003600,
		warn_secs: 600,
		critical_secs: 300,
		owner: 'operator',
		qa_note: null,
		qa_score: 90,
		domain: 'global',
		title: 'Lead-safe wiring note',
		...overrides
	};
}

async function renderRoute(): Promise<void> {
	const module = await import('../src/routes/review/+page.svelte');
	render(module.default);
}

/**
 * Wait for a render proof.
 *
 * The waitFor bound (2 s) sits INSIDE vitest's default 5 s test bound on
 * purpose: with the default on both, a route that never renders fails as
 * `Test timed out in 5000ms` — the arm reddens, but the reader learns nothing
 * about WHY. A proof arm whose failure message is a stopwatch has not spoken,
 * and the P4 drill (the 403 arm collapsing into the empty state) is exactly
 * the case that would have been reported that way.
 */
async function waitForTestId(testId: string): Promise<void> {
	await waitFor(
		() => {
			expect(
				screen.queryByTestId(testId),
				`the render proof "${testId}" never appeared — a surface that renders nothing must FAIL here, never read as an empty queue`
			).not.toBeNull();
		},
		{ timeout: 3000 }
	);
}

/**
 * The source's CODE — comment-only lines removed.
 *
 * This is the II-6 precedent, and it is applied for the same reason: a
 * comment does not execute, so a path that exists only inside one is not a
 * path to the wire. Excluding comment-only lines cannot hide a bypass — the
 * CODE line that would carry it is never excluded.
 *
 * The exclusion is deliberately NARROW: only lines that ARE comments go. A
 * naive strip-from-the-first-`//` would be WORSE, because `"http://…"`
 * contains `//` and it would delete the host out of a real URL and quietly
 * weaken the scan.
 */
function codeOf(source: string): string {
	return source
		.split('\n')
		.filter((line) => {
			const trimmed = line.trim();
			return !trimmed.startsWith('//') && !trimmed.startsWith('*') && !trimmed.startsWith('/*');
		})
		.join('\n');
}

afterEach(() => {
	responder = noWire;
	state.wireStatus = 'unreachable';
	try {
		sessionStorage.clear();
	} catch {
		// no storage
	}
	cleanup();
	vi.clearAllMocks();
});

// ── P1 · the queue renders the kernel's pending rows ────────────────────────

test('P1 the queue renders the pending rows with the digest visible', async () => {
	responder = async () => jsonResponse([proposalRow()]);
	await renderRoute();
	await waitForTestId('review-list-row');

	const row = screen.getByTestId('review-list-row');
	expect(row.textContent).toContain('42');
	expect(screen.getByTestId('review-row-kind').textContent).toContain('capture');
	expect(screen.getByTestId('review-row-domain').textContent).toContain('global');
	expect(screen.getByTestId('review-row-novelty').textContent).toContain('0.9137');
	expect(screen.getByTestId('review-row-salience').textContent).toContain('0.5120');
	expect(screen.getByTestId('review-row-clean')).toBeTruthy();
	// The content is the reviewer's copy …
	expect(screen.getByTestId('review-row-content').textContent).toContain('lead-safe');
	// … and the digest is the reviewer's COPY HANDLE, because the approve
	// verb is digest-bound: a decision made elsewhere binds to the bytes the
	// reviewer was shown. A display without the digest cannot be used to make
	// a decision elsewhere with any confidence.
	expect(screen.getByTestId('review-row-digest').textContent).toContain('a'.repeat(64));

	// And the request the surface made is the READ, with the pending filter.
	const requested = fetchMock.mock.calls.map(([input]) =>
		input instanceof Request ? input.url : String(input)
	);
	expect(requested.some((url) => new URL(url).pathname === '/proposals')).toBe(true);
	const query = new URL(requested[0]!).searchParams;
	expect(query.get('status')).toBe('pending');
	expect(query.get('limit')).toBe('50');
});

test('P1 a quarantined row is visibly marked', async () => {
	responder = async () => jsonResponse([proposalRow({ screen_verdict: 'quarantine' })]);
	await renderRoute();
	await waitForTestId('review-list-row');
	expect(screen.getByTestId('review-row-quarantine')).toBeTruthy();
	expect(screen.queryByTestId('review-row-clean')).toBeNull();
});

// ── P2 · the offline render shows METADATA and NO content ───────────────────

test('P2 the offline render shows the last-known METADATA and NO content', async () => {
	// A live read first: the cache is written from it.
	responder = async () => jsonResponse([proposalRow()]);
	await renderRoute();
	await waitForTestId('review-list-row');

	const raw = sessionStorage.getItem(QUEUE_CACHE_KEY);
	expect(raw, 'a live read must leave a cache behind').not.toBeNull();
	// THE LAW, on the bytes: the proposal text is NOT in the cache.
	expect(raw).not.toContain('lead-safe');
	const parsed = JSON.parse(raw!) as { cachedAt: number; rows: Array<Record<string, unknown>> };
	expect(parsed.rows).toHaveLength(1);
	expect('content' in (parsed.rows[0] ?? {})).toBe(false);
	expect('contentDigest' in (parsed.rows[0] ?? {})).toBe(false);

	// Now the kernel goes away.
	cleanup();
	responder = async () => {
		throw new Error('the port is dead');
	};
	state.wireStatus = 'unreachable';
	await renderRoute();
	await waitForTestId('review-offline');

	// The offline state is NAMED and states the age of what it shows.
	expect(screen.getByTestId('review-offline-age').textContent).not.toBe('');
	// The cached METADATA is shown …
	expect(screen.getByTestId('review-list-row').textContent).toContain('42');
	expect(screen.getByTestId('review-row-domain').textContent).toContain('global');
	// … and the content column reads as UNAVAILABLE, never as absent.
	expect(screen.getByTestId('review-row-content').textContent).toContain('Not available offline');
	// And no cached content is anywhere on the screen.
	expect(document.body.textContent).not.toContain('lead-safe');
	// And the offline state is NOT the empty state: "nothing is waiting" and
	// "we could not ask" are different facts and must never share a rendering.
	expect(screen.queryByTestId('review-empty')).toBeNull();
	expect(document.body.textContent).not.toContain('no proposals waiting');
});

test('P2 the cache type cannot hold the content and the reader drops it', async () => {
	// The TYPE is the guard: `content?: never` makes a row carrying the
	// proposal text unassignable to the cache's row type.
	const model = readFileSync(join(shellRoot, 'src/lib/queue/model.ts'), 'utf8');
	expect(model).toMatch(/interface QueueRowMetadata[\s\S]*?readonly content\?: never;/);

	// And the READER is the second layer: a cache entry that was tampered
	// with, or written by a build without the law, contributes its metadata
	// and nothing else.
	sessionStorage.setItem(
		QUEUE_CACHE_KEY,
		JSON.stringify({
			cachedAt: 1750000000,
			rows: [
				{
					id: 42,
					kind: 'capture',
					domain: 'global',
					novelty: 0.9,
					salience: 0.5,
					createdAt: 1750000000,
					screenVerdict: 'clean',
					content: 'the bench wiring is lead-safe and labelled',
					contentDigest: 'a'.repeat(64)
				}
			]
		})
	);
	responder = async () => {
		throw new Error('the port is dead');
	};
	state.wireStatus = 'unreachable';
	await renderRoute();
	await waitForTestId('review-offline');
	expect(document.body.textContent).not.toContain('lead-safe');
	expect(document.body.textContent).not.toContain('a'.repeat(64));
});

test('P2 the cache is sessionStorage and nothing else', async () => {
	const model = codeOf(readFileSync(join(shellRoot, 'src/lib/queue/model.ts'), 'utf8'));
	expect(model).toContain("export const QUEUE_CACHE_KEY = 'brain-queue-cache:'");
	expect(model).toContain('window.sessionStorage');
	expect(model).not.toContain('localStorage');
	expect(model).not.toContain('document.cookie');
	expect(model).not.toContain('indexedDB');
});

// ── P3 · the surface contains NO decision-posting path ──────────────────────

/** The route and EVERY module it imports, resolved from its own specifiers. */
function queueImportClosure(): string[] {
	const routeFile = join(shellRoot, 'src', 'routes', 'review', '+page.svelte');
	const seen = new Set<string>([routeFile]);
	const queue: string[] = [routeFile];
	while (queue.length > 0) {
		const file = queue.pop()!;
		const source = readFileSync(file, 'utf8');
		for (const m of source.matchAll(/from\s*'([^']+)'/g)) {
			const specifier = m[1]!;
			let candidates: string[];
			if (specifier.startsWith('#lib/')) {
				const rel = specifier.slice('#lib/'.length).replace(/\.js$/, '');
				candidates = [
					join(shellRoot, 'src', 'lib', `${rel}.ts`),
					join(shellRoot, 'src', 'lib', rel, 'index.ts')
				];
			} else if (specifier.startsWith('.')) {
				const base = join(dirname(file), specifier.replace(/\.js$/, ''));
				candidates = [`${base}.ts`, join(base, 'index.ts')];
			} else {
				continue;
			}
			for (const candidate of candidates) {
				if (existsSync(candidate) && !seen.has(candidate)) {
					seen.add(candidate);
					queue.push(candidate);
				}
			}
		}
	}
	return [...seen].sort();
}

test('P3 the surface contains NO decision-posting path', () => {
	const files = queueImportClosure();

	// ANTI-VACUITY: the scan must have found the surface at all, and the
	// module that holds the read. A scan over zero files passes every arm.
	expect(files.length).toBeGreaterThan(1);
	expect(files).toContain(join(shellRoot, 'src', 'routes', 'review', '+page.svelte'));
	expect(files).toContain(join(shellRoot, 'src', 'lib', 'queue', 'client.ts'));

	for (const file of files) {
		const source = codeOf(readFileSync(file, 'utf8'));
		const label = file.slice(shellRoot.length + 1);

		// The kernel's decision verbs, as paths.
		expect(source, `${label} names a decision path`).not.toMatch(/\/(?:approve|reject|edit)\b/);
		// The decision verbs, as the client's own write methods.
		expect(source, `${label} can write`).not.toMatch(/\bclient\.(?:POST|PUT|PATCH|DELETE)\b/);
		// The decision verbs, as symbols.
		expect(source, `${label} names a decision verb`).not.toMatch(
			/\b(?:approve|reject|edit)_proposal\b|\b(?:approve|reject|edit)Proposal\b/
		);
	}

	// And the queue's own module tree makes exactly ONE typed-wire call, and
	// it is a READ of the queue.
	const queueModule = codeOf(readFileSync(join(shellRoot, 'src', 'lib', 'queue', 'client.ts'), 'utf8'));
	const calls = [...queueModule.matchAll(/client\.(GET|POST|PUT|PATCH|DELETE)\(\s*'([^']+)'/g)];
	expect(calls).toHaveLength(1);
	expect(calls[0]![1]).toBe('GET');
	expect(calls[0]![2]).toBe('/proposals');
});

// ── P4 · a refused queue renders a REFUSAL ──────────────────────────────────

test('P4 a refused queue renders a REFUSAL, not an empty queue', async () => {
	responder = async () => jsonResponse({ error: 'forbidden' }, 403);
	await renderRoute();
	await waitForTestId('review-refused');

	expect(screen.getByTestId('review-refused-reason').textContent).toContain('403');
	// The refusal is not the empty state, and it does not read like one.
	expect(screen.queryByTestId('review-empty')).toBeNull();
	expect(screen.queryByTestId('review-list')).toBeNull();
	expect(document.body.textContent).not.toContain('no proposals waiting');
});

test('P4 an unauthorized queue is named as such', async () => {
	responder = async () => jsonResponse({ error: 'unauthorized' }, 401);
	await renderRoute();
	await waitForTestId('review-refused');
	expect(screen.getByTestId('review-refused-reason').textContent).toContain('401');
	expect(screen.queryByTestId('review-empty')).toBeNull();
});

test('P4 an empty queue is the kernel saying so, not a refusal', async () => {
	responder = async () => jsonResponse([]);
	await renderRoute();
	await waitForTestId('review-empty');
	expect(screen.queryByTestId('review-refused')).toBeNull();
	expect(screen.queryByTestId('review-offline')).toBeNull();
});

// ── P5 · the four artifacts carry the route ─────────────────────────────────

/** One parity row's source block, as the table holds it. */
function parityRowBlock(source: string, route: string): string {
	const start = source.indexOf(`route: '${route}',`);
	expect(start, `the parity table has no row for ${route}`).toBeGreaterThan(-1);
	const open = source.lastIndexOf('{', start);
	const close = source.indexOf('\n\t},', start);
	return source.slice(open, close === -1 ? undefined : close);
}

test('P5 the census, the parity row, the sweep arm and the catalogs carry the route', () => {
	// The census — and its floor, which is a floor on the walk, not on the list.
	expect([...CENSUS_ROUTES]).toContain('/review');
	expect(ROUTE_FLOOR).toBe(CENSUS_ROUTES.length);

	// The parity row — and its SHAPE, not just its presence.
	const parity = readFileSync(join(shellRoot, 'tests', 'route-parity.test.ts'), 'utf8');
	const row = parityRowBlock(parity, '/review');
	expect(row).toContain("verdict: 'exact'");
	expect(row).toContain("dioxus: '/review'");
	expect(row).toContain("'/proposals': 'Read'");
	expect(row).toContain('mutates: null');

	// The sweep arm.
	const sweep = readFileSync(join(shellRoot, 'tests', 'route-a11y.test.ts'), 'utf8');
	expect(sweep).toContain("'/review': {");
	expect(sweep).toContain("value: 'review-list-row'");

	// The five catalogs.
	for (const locale of ['en', 'de', 'fr', 'es', 'nl']) {
		const catalog = JSON.parse(
			readFileSync(join(shellRoot, 'src', 'lib', 'i18n', `${locale}.json`), 'utf8')
		) as Record<string, string>;
		expect(Object.keys(catalog), `${locale} carries no review queue key`).toContain('review.title');
		expect(Object.keys(catalog), `${locale} carries no review nav key`).toContain('nav.review');
	}
});

// ── P6 · the parity rows for the decision surfaces are TRUE ─────────────────

test('P6 the parity rows for /decisions and /decisions/[id] are TRUE', () => {
	const parity = readFileSync(join(shellRoot, 'tests', 'route-parity.test.ts'), 'utf8');

	for (const route of ['/decisions', '/decisions/[id]']) {
		const block = parityRowBlock(parity, route);

		// The row NAMES the surface it actually serves …
		expect(block, `${route}'s row does not name its own wire path`).toContain(
			'/workflow/decision-runs'
		);
		// … and does NOT claim the proposal queue. This is the claim that was
		// FALSE, and a false `renamed` reads as coverage.
		expect(block.toLowerCase(), `${route}'s row still claims the proposal queue`).not.toContain(
			'review queue'
		);
		// … and its counterpart is a RUN surface, never a proposal path.
		const dioxus = /dioxus: '([^']+)'/.exec(block);
		expect(dioxus, `${route}'s row names no counterpart`).not.toBeNull();
		expect(
			dioxus![1]!.startsWith('/review'),
			`${route} still names the Dioxus proposal surface (${dioxus![1]}) as its counterpart`
		).toBe(false);

		// And the route's OWN wire call is the decision-run listing, not the
		// proposal queue. The row and the code must agree.
		const page = readFileSync(
			join(shellRoot, 'src', 'routes', route.replace(/^\//, ''), '+page.svelte'),
			'utf8'
		);
		expect(page, `${route} does not read the decision-run listing`).toContain(
			"client.GET('/workflow/decision-runs"
		);
		expect(page, `${route} reads the proposal queue`).not.toContain('/proposals');
	}
});
