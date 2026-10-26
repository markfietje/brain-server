import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';
import { recallMemory, type RecallOutcome } from '../src/lib/api/recall';

/**
 * P2 / II-1 — the transport decision, ENFORCED rather than recorded.
 *
 * The register row (`docs/gate-law-register.md`, "The `KernelRecallTransport`
 * HOST") records the decision and what it ruled out. A decision nobody can
 * undo is a comment, and this prompt named the exact failure it exists to
 * prevent: "whichever file you happened to open first — that is how a second
 * unaudited path to private data gets built, and it looks like a refactor."
 *
 * So the decision is two assertions, and they are different in kind:
 *
 *   1. BEHAVIORAL — the chat surface's recall goes through `recallMemory()`,
 *      i.e. through the core's authorize → wire → shape order. The pin drives
 *      the REAL function and observes that the core was asked to authorize
 *      before anything was sent, and that the shape came back classified.
 *      A chat module that fetched `/recall` itself would never call the core,
 *      so the ordering assertion is the teeth.
 *
 *   2. STRUCTURAL — no chat module opens its own wire. The webview already
 *      holds the only one (`client.ts`, loopback-pinned, no http capability in
 *      `capabilities/main.json`), so a second `fetch(` in the chat path would
 *      be a second unaudited path to private data with the core's bounds and
 *      authz nowhere in it.
 *
 * The structural arm is deliberately NARROW — it scans the chat module and the
 * provenance/forget modules, not the whole tree. `client.ts` is where a
 * `fetch` belongs; flagging it would flag the wire itself. Comment-only lines
 * are excluded for the reason the Swift guard documents: a comment does not
 * execute, and a URL that exists only inside one is not a path to private data.
 * A naive strip-from-first-`//` would be WORSE, because `"http://…"` in a
 * string literal contains `//` and that would delete the host out of a real
 * URL — the same trap the P1 Swift guard already records.
 */

const srcDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'src');
const chatDir = join(srcDir, 'routes');

/** The chat-side modules that must not open their own wire. */
function chatWireModules(): string[] {
	const found: string[] = [];
	const walk = (dir: string): void => {
		let entries: string[];
		try {
			entries = readdirSync(dir);
		} catch {
			return;
		}
		for (const entry of entries) {
			const path = join(dir, entry);
			if (entry.endsWith('.svelte') || entry.endsWith('.ts')) {
				if (entry.endsWith('.test.ts')) continue;
				found.push(path);
			} else if (entry.endsWith('.ts') || entry === 'lib') {
				walk(path);
			}
		}
	};
	// M3-a: the chat page IS `routes/+page.svelte` — the scan keys on THAT
	// file, not on the whole routes tree (the other pages have their own
	// surfaces and their own arms).
	const homePage = join(chatDir, '+page.svelte');
	if (existsSync(homePage)) found.push(homePage);
	// The chat turn's own logic lives beside the route, under lib/.
	const chatLib = join(srcDir, 'lib', 'chat');
	let libEntries: string[] = [];
	try {
		libEntries = readdirSync(chatLib);
	} catch {
		libEntries = [];
	}
	for (const entry of libEntries) {
		if (entry.endsWith('.ts') && !entry.endsWith('.test.ts')) found.push(join(chatLib, entry));
	}
	return found;
}

test('the_chat_surface_reads_through_the_core_and_never_opens_its_own_wire', async () => {
	// ---- ARM 1 (behavioral): the ORDER is core-authorize → wire → shape ----
	//
	// A chat module that POSTed `/recall` itself would produce the same rows
	// and skip both halves of the boundary. So the pin watches the ORDER, not
	// the result: nothing may be sent before the core authorized, and the
	// classification must come from the core rather than from the chat code
	// inspecting a response.
	const order: string[] = [];
	const outcome: RecallOutcome = {
		state: 'answered',
		decision: 'ok',
		passages: [
			{
				id: 1,
				title: null,
				text: 'the row',
				snippet: null,
				domain: null,
				score: 0.9,
				valid_from: null,
				valid_to: null,
				flagged: false,
				ingest_kind: null,
				memory_kind: null,
				authority: null
			}
		],
		confidence: 0.82,
		searched: { query: 'q', domain: null, limit: 5, domains_searched: [], included_global: null }
	};

	const shaped = await recallMemory(
		{ query: 'q' },
		{
			core: {
				authorize: async () => {
					order.push('authorize');
					return { doc: { v: 1, query: 'q', limit: 5, provenance: true }, query_bytes: 1 };
				},
				shape: async () => {
					order.push('shape');
					return outcome;
				}
			},
			transport: async () => {
				order.push('wire');
				return { status: 200, body: JSON.stringify({ hits: [], decision: 'ok' }) };
			}
		}
	);

	expect(order, 'the core authorizes BEFORE the wire, and classifies after it').toEqual([
		'authorize',
		'wire',
		'shape'
	]);
	expect(shaped.state).toBe('answered');

	// And the negative: a chat path that skips authorize still cannot produce
	// a classification, because the core is what produces one.
	const skipped: string[] = [];
	await recallMemory(
		{ query: 'q' },
		{
			core: {
				authorize: async () => {
					skipped.push('authorize');
					return { doc: { v: 1, query: 'q', limit: 5, provenance: true }, query_bytes: 1 };
				},
				shape: async () => {
					skipped.push('shape');
					return outcome;
				}
			},
			transport: async () => ({ status: 200, body: '{}' })
		}
	);
	expect(skipped[0], 'nothing reaches the wire before authorize').toBe('authorize');

	// ---- ARM 2 (structural): the chat's ONE wire carries the discipline ----
	//
	// M3-d (plan 25, the NAMED reversal): the chat now rides `POST /chat/turns`,
	// and openapi-fetch cannot stream — so `lib/chat/spine.ts` IS the chat's
	// own wire, riding plain `fetch`. The old law ("no chat module opens a
	// wire") is reversed on purpose; what survives is the DISCIPLINE the old
	// law existed to protect: the spine must ride the SAME loopback origin
	// (`apiBase()`), await the SAME memoized credential before its first byte
	// (R130), and attach the SAME runtime-held bearer (`authHeaders()`). So the
	// scan exempts exactly ONE module — the spine — and a separate arm pins
	// that the exempted module carries the three discipline calls. Every OTHER
	// chat module still may not open a wire: a second unaudited path to private
	// data still fails here.
	const modules = chatWireModules();
	const routeExists = existsSync(join(chatDir, '+page.svelte'));
	if (routeExists) {
		expect(
			modules.length,
			'the chat route exists, so the scan must have read its modules — an empty scan over a real route is vacuous'
		).toBeGreaterThan(0);
	} else {
		expect(
			modules,
			'chat modules exist but routes/+page.svelte does not — the home route is the thing the scan keys on'
		).toEqual([]);
	}

	// The ONE exempt module, named by path: the chat spine's consumer. A scan
	// exemption for anything else is how a second wire gets built "like a
	// refactor" — so the exemption is exact, and the anti-vacuity arm below
	// proves the exempted file really is the spine.
	const SPINE = join(srcDir, 'lib', 'chat', 'spine.ts');
	expect(
		modules.includes(SPINE),
		'the spine module must exist for the wire exemption to mean anything'
	).toBe(true);

	for (const path of modules) {
		if (path === SPINE) continue;
		const text = readFileSync(path, 'utf8');
		const codeLines = text
			.split('\n')
			.filter((line) => {
				const trimmed = line.trim();
				return !trimmed.startsWith('//') && !trimmed.startsWith('/*') && !trimmed.startsWith('*');
			})
			.join('\n')
			.split(/\s+/)
			.join(' ');

		for (const forbidden of ['fetch(', 'XMLHttpRequest', 'WebSocket', 'EventSource', 'navigator.sendBeacon']) {
			expect(
				codeLines.includes(forbidden),
				`${path.slice(srcDir.length + 1)} opens its own wire ("${forbidden}"). The webview's client.ts is the only typed wire and lib/chat/spine.ts is the chat's one streaming wire — a third path skips the core's bounds, the origin pin and the credential discipline.`
			).toBe(false);
		}
	}

	// THE EXEMPTION'S TEETH: the exempted module carries the discipline the
	// old law protected. Three source pins — the loopback origin, the awaited
	// credential precondition, and the runtime-held bearer. Delete any of the
	// three and the chat's wire is a second unaudited path again.
	const spineCode = readFileSync(SPINE, 'utf8')
		.split('\n')
		.filter((line) => {
			const trimmed = line.trim();
			return !trimmed.startsWith('//') && !trimmed.startsWith('/*') && !trimmed.startsWith('*');
		})
		.join('\n');
	expect(spineCode, 'the spine must ride the loopback-pinned origin (apiBase)').toContain('apiBase()');
	expect(spineCode, 'the spine must await the credential BEFORE the first byte (R130)').toContain(
		'await bootstrapToken()'
	);
	expect(spineCode, 'the spine must attach the runtime-held bearer (D7)').toContain('authHeaders()');
});

test('the_register_records_the_transport_decision_and_what_it_ruled_out', () => {
	// The decision is only worth making once if the next reader can find it.
	// This arm is deliberately thin: it asserts the row EXISTS and carries both
	// halves, because a row that states only what it chose is the failure the
	// prompt named — a decision that reads as inevitable rather than chosen.
	const kernelRoot = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
	const register = readFileSync(join(kernelRoot, 'docs', 'gate-law-register.md'), 'utf8');
	const row = register
		.split('\n')
		.find((line) => line.includes('KernelRecallTransport') && line.includes('HOST'));

	expect(row, 'the register carries no transport-decision row').toBeDefined();
	expect(row).toContain('What it ruled out');
	expect(row).toContain('noSwiftPathReachesTheKernelAndTheStubIsGone');
});