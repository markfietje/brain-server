import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import {
	MODEL_REGISTRY_LIMITS,
	REGISTRY_ROW_FIELDS,
	lifecycleProposalContent,
	modelRegistryListQuery,
	parseModelRegistryDetail
} from '../src/lib/model-registry';

/**
 * R36 M6-S2 — the Model Registry view, in test form.
 *
 * Every test below is a security invariant the surface must hold, not a snapshot
 * of markup:
 *
 *  - the listing sends ONLY the filters the server supports, and invents no
 *    filter, cursor, or pagination control
 *  - a 403 is a FORBIDDEN state and is never an empty registry, and a Read that
 *    succeeds is never blanked by a list the same principal cannot make
 *  - the row digest is a content PIN, and the surface says so in its own words
 *  - there is no approve/reject control and no direct status write; the only
 *    actions are lifecycle proposals the kernel's transition table allows
 *  - the outbound lifecycle payload is the VERBATIM wire value, byte-identical
 *    to `lifecycleProposalContent`, never the display-sanitized projection
 *  - an evaluation join is data: it never moves a status and never claims that
 *    `evaluated` is reachable
 *
 * No test touches the network: `fetch` is stubbed at the module boundary, and
 * every responder is a local function.
 */

const LIST_PATH = '/workflow/model-registry';
const DETAIL_PATH_PREFIX = '/workflow/model-registry/';
const PROPOSAL_PATH = '/ingest/proposal';
const LIST_URL = 'http://127.0.0.1/models';
const DETAIL_REF = 'rules-r32-e2e@1.0.0';
const ROW_DIGEST = '9fbcf60dabc26c8905a2eb8820100e6709df27a0a409d49f04eab59d85a60037';
const ARTIFACT_DIGEST = 'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1';
const CONFIG_DIGEST = 'c0ffee00000000000000000000000000000000000000000000000000000000ff';
const ABSENT = '—';

/** Exactly the filters the kernel documents for the listing — no more. */
const SUPPORTED_LIST_FILTERS = ['limit', 'status', 'kind'] as const;

/** U+200B built at runtime; a literal byte is stripped by tooling. */
const ZWSP = String.fromCharCode(0x200b);

const pageState: { url: URL } = { url: new URL(LIST_URL) };
vi.mock('$app/state', () => ({ page: pageState }));

type Seen = { url: string; method: string; body: string | null };
const seen: Seen[] = [];

type Responder = (url: string, init: RequestInit | undefined, method: string) => Promise<Response>;
const noWire: Responder = async () => new Response('no wire', { status: 599 });
let responder: Responder = noWire;

const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
	const url = input instanceof Request ? input.url : String(input);
	const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
	const body =
		input instanceof Request
			? await input.clone().text()
			: typeof init?.body === 'string'
				? init.body
				: null;
	seen.push({ url, method, body });
	return responder(url, init, method);
});
vi.stubGlobal('fetch', fetchMock);

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function fixture(): Record<string, unknown> {
	const parsed: unknown = JSON.parse(
		readFileSync(join(process.cwd(), 'tests', 'fixtures', 'model-registry.json'), 'utf8')
	);
	if (!isRecord(parsed)) throw new Error('model-registry fixture must be an object');
	return parsed;
}

function listBody(): Record<string, unknown> {
	const list = fixture()['list'];
	if (!isRecord(list)) throw new Error('fixture list must be an object');
	return structuredClone(list);
}

function detailBody(): Record<string, unknown> {
	const detail = fixture()['detail'];
	if (!isRecord(detail)) throw new Error('fixture detail must be an object');
	return structuredClone(detail);
}

function withDetail(mutate: (row: Record<string, unknown>) => void): Record<string, unknown> {
	const row = detailBody();
	mutate(row);
	return row;
}

function jsonResponse(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

function notFound(): Response {
	return new Response('not found', { status: 404 });
}

function pathOf(url: string): string {
	return new URL(url).pathname;
}

/** Serves the listing and the single-row read from the shared fixture. */
function serveRegistry(): void {
	responder = async (url) => {
		const path = pathOf(url);
		if (path === LIST_PATH) return jsonResponse(listBody());
		if (path.startsWith(DETAIL_PATH_PREFIX)) return jsonResponse(detailBody());
		return notFound();
	};
}

/** Serves the row read plus a lifecycle-proposal POST of the given disposition. */
function serveProposal(status: number, body: unknown, row?: Record<string, unknown>): void {
	responder = async (url, _init, method) => {
		const path = pathOf(url);
		if (method === 'POST' && path === PROPOSAL_PATH) return jsonResponse(body, status);
		if (path.startsWith(DETAIL_PATH_PREFIX)) return jsonResponse(row ?? detailBody());
		return notFound();
	};
}

/** Points the surface at a `ref` query segment (or the listing when null). */
function refUrl(ref: string | null): void {
	const url = new URL(LIST_URL);
	if (ref !== null) url.searchParams.set('ref', ref);
	pageState.url = url;
}

async function renderRoute(): Promise<void> {
	const module = await import('../src/routes/models/+page.svelte');
	render(module.default);
}

async function waitForTestId(testId: string): Promise<void> {
	// `queryAllBy`: a testid can legitimately appear once per row, and the
	// singular query would throw on a surface that has more than one.
	await waitFor(() => expect(screen.queryAllByTestId(testId).length).toBeGreaterThan(0), {
		timeout: 5000
	});
}

async function expectAxeClean(): Promise<void> {
	const axe = await import('axe-core');
	const results = await axe.default.run(document.body, {
		runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
	});
	expect(results.violations).toEqual([]);
}

/** The lifecycle controls currently offered, in render order. */
function offeredActions(): (string | null)[] {
	return screen.queryAllByTestId('models-action-promote').length === 0 &&
		screen.queryAllByTestId('models-action-retire').length === 0
		? []
		: [
				...screen.queryAllByTestId('models-action-promote'),
				...screen.queryAllByTestId('models-action-retire')
			].map((button) => button.textContent);
}

function seenPosts(): Seen[] {
	return seen.filter((request) => request.method === 'POST');
}

function postBody(index = 0): Record<string, unknown> {
	const post = seenPosts()[index];
	expect(post, 'a lifecycle proposal POST').toBeDefined();
	return JSON.parse(post?.body ?? 'null') as Record<string, unknown>;
}

afterEach(() => {
	responder = noWire;
	pageState.url = new URL(LIST_URL);
	seen.length = 0;
	vi.clearAllMocks();
});

describe('Model Registry view', () => {
	test('models_list_sends_only_server_supported_filters', async () => {
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-list-row');

		const listCalls = seen.filter((request) => pathOf(request.url) === LIST_PATH);
		expect(listCalls.length).toBeGreaterThan(0);

		// The query the surface builds IS the parser's query, byte for byte.
		const expected = modelRegistryListQuery({ limit: 20 });
		expect(expected).toBe('?limit=20');
		for (const call of listCalls) {
			const url = new URL(call.url);
			expect(url.search).toBe(expected);
			for (const key of url.searchParams.keys()) {
				expect(SUPPORTED_LIST_FILTERS).toContain(key as (typeof SUPPORTED_LIST_FILTERS)[number]);
			}
		}

		// And nothing outside that set is ever added.
		for (const unsupported of [
			'cursor',
			'offset',
			'page',
			'after',
			'before',
			'sort',
			'order',
			'q',
			'search',
			'name',
			'id',
			'version',
			'proposed_by',
			'domain',
			'updated_at'
		]) {
			for (const call of listCalls) {
				expect(new URL(call.url).searchParams.has(unsupported)).toBe(false);
			}
		}
	});

	test('models_list_renders_identity_kind_status_and_digest_presence', async () => {
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-list-row');

		const rows = screen.getAllByTestId('models-list-row');
		expect(rows).toHaveLength(2);

		expect(rows[0]?.textContent).toContain('rules-r32-e2e');
		expect(rows[0]?.textContent).toContain('1.0.0');
		expect(rows[0]?.textContent).toContain('Deterministic rules');
		expect(rows[0]?.textContent).toContain('R32 e2e rules');
		expect(rows[0]?.textContent).toContain('Candidate');
		expect(rows[0]?.textContent).toContain('did:key:zOperator');
		expect(rows[0]?.textContent).toContain('2025-06-15T15:06:40.000Z');
		expect(rows[0]?.textContent).toContain('2025-06-15T15:08:20.000Z');

		expect(rows[1]?.textContent).toContain('acme/models');
		expect(rows[1]?.textContent).toContain('2.1.0');
		expect(rows[1]?.textContent).toContain('Learned');
		expect(rows[1]?.textContent).toContain('Retired');
		expect(rows[1]?.textContent).toContain('did:key:zAgent');

		// Status is TEXT, never colour alone: the word is on the row itself.
		expect(screen.getAllByTestId('models-list-status').map((node) => node.textContent?.trim())).toEqual([
			'Candidate',
			'Retired'
		]);
		expect(screen.getAllByTestId('models-list-link').map((link) => link.getAttribute('href'))).toEqual([
			`/models?ref=${encodeURIComponent(DETAIL_REF)}`,
			'/models?ref=acme%2Fmodels%402.1.0'
		]);
	});

	test('models_list_shows_artifact_digest_as_presence_never_value', async () => {
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-list-row');

		const presence = screen.getAllByTestId('models-list-artifact');
		expect(presence[0]?.textContent?.trim()).toBe('Artifact present');
		expect(presence[1]?.textContent?.trim()).toBe('No artifact');

		// The presence cell NEVER carries a digest value — of either tier.
		for (const cell of presence) {
			expect(cell.textContent ?? '').not.toMatch(/[0-9a-f]{64}/);
		}
		// The single-row read's artifact value never leaks into the listing.
		expect(screen.getByTestId('models-list').textContent).not.toContain(ARTIFACT_DIGEST);
		expect(screen.getByTestId('models-list').textContent).not.toContain('a1a1a1a1');
	});

	test('models_list_renders_loading_empty_400_401_403_409_422_500_and_network_states', async () => {
		responder = async () => new Promise<Response>(() => undefined);
		await renderRoute();
		expect(screen.getByTestId('models-list-loading')).toBeTruthy();

		cleanup();
		responder = async () => jsonResponse({ rows: [], count: 0 });
		await renderRoute();
		await waitForTestId('models-list-empty');
		expect(screen.getByTestId('models-list-empty').textContent).toContain('No models are registered.');

		const refusals = [
			{ status: 400, text: 'The request was rejected as invalid.' },
			{ status: 401, text: 'Not authenticated.' },
			{ status: 403, text: 'Not authorized.' },
			{ status: 409, text: 'This row changed before your proposal was recorded.' },
			{ status: 422, text: 'The requested transition is not legal from this state.' },
			{ status: 500, text: 'The server could not complete the request.' }
		] as const;

		for (const refusal of refusals) {
			cleanup();
			responder = async () => jsonResponse({ error: 'refused' }, refusal.status);
			await renderRoute();
			await waitForTestId('models-list-error');
			expect(screen.getByTestId('models-list-error').textContent).toContain(refusal.text);

			// A refusal is NEVER an empty registry, and never a rendered row.
			expect(screen.queryByTestId('models-list-empty')).toBeNull();
			expect(screen.queryByTestId('models-list-row')).toBeNull();
			// A denial offers no retry: there is nothing to retry.
			if (refusal.status === 401 || refusal.status === 403) {
				expect(screen.queryByTestId('models-list-retry')).toBeNull();
			}
		}

		cleanup();
		responder = async () => {
			throw new TypeError('failed to fetch');
		};
		await renderRoute();
		await waitForTestId('models-list-network');
		expect(screen.getByTestId('models-list-network').textContent).toContain(
			'This is a host state, not an empty registry'
		);
		expect(screen.queryByTestId('models-list-empty')).toBeNull();
	});

	test('models_list_does_not_invent_unsupported_filters_or_pagination', async () => {
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-list-row');

		// No filter inputs, no page controls, no cursor vocabulary in the chrome.
		expect(document.querySelectorAll('input, select, textarea')).toHaveLength(0);
		expect(screen.queryByRole('navigation')).toBeNull();
		expect(screen.queryByRole('button', { name: /next|previous|prev|more|page|load/i })).toBeNull();
		expect(
			document.querySelectorAll(
				'[data-testid*="pagination"], [data-testid*="cursor"], [data-testid*="page"]'
			)
		).toHaveLength(0);
		for (const call of seen) {
			for (const key of new URL(call.url).searchParams.keys()) {
				expect(SUPPORTED_LIST_FILTERS).toContain(key as (typeof SUPPORTED_LIST_FILTERS)[number]);
			}
		}

		// An over-cap listing is REFUSED, never paginated into a tidy page.
		cleanup();
		const overflow = listBody();
		const template = (overflow['rows'] as Record<string, unknown>[])[0];
		const rows: Record<string, unknown>[] = [];
		for (let index = 0; index <= MODEL_REGISTRY_LIMITS.listRows; index += 1) {
			rows.push({ ...(template ?? {}), id: `overflow-${index}` });
		}
		overflow['rows'] = rows;
		overflow['count'] = rows.length;
		responder = async () => jsonResponse(overflow);
		await renderRoute();
		await waitForTestId('models-list-malformed');
		expect(screen.queryByTestId('models-list-row')).toBeNull();
		expect(screen.queryByTestId('models-list-empty')).toBeNull();
	});

	test('models_detail_decodes_ref_query_and_refuses_malformed_ref_without_wire', async () => {
		refUrl(DETAIL_REF);
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-detail-ready');

		const detailCalls = seen.filter((request) => pathOf(request.url).startsWith(DETAIL_PATH_PREFIX));
		expect(detailCalls).toHaveLength(1);
		// The ref travels as ONE captured segment, id@version.
		expect(decodeURIComponent(pathOf(detailCalls[0]?.url ?? ''))).toBe(`${DETAIL_PATH_PREFIX}${DETAIL_REF}`);
		expect(screen.getByTestId('models-detail-identity').textContent).toContain('rules-r32-e2e');
		expect(screen.getByTestId('models-detail-status').textContent).toContain('Candidate');

		// A malformed ref is refused BEFORE any wire is touched.
		for (const bad of ['not-a-ref', '@1.0.0', 'id@', 'a@b@c', ' ']) {
			cleanup();
			seen.length = 0;
			refUrl(bad);
			serveRegistry();
			await renderRoute();
			await waitForTestId('models-detail-ref-malformed');
			expect(screen.getByTestId('models-detail-ref-malformed').textContent).toContain(
				'Expected one id@version segment'
			);
			expect(seen.filter((request) => pathOf(request.url).startsWith(DETAIL_PATH_PREFIX))).toHaveLength(
				0
			);
			expect(seenPosts()).toHaveLength(0);
		}
	});

	test('models_detail_renders_identity_and_three_digest_tiers_separately', async () => {
		refUrl(DETAIL_REF);
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-detail-ready');

		const identity = screen.getByTestId('models-detail-identity');
		expect(identity.textContent).toContain('rules-r32-e2e');
		expect(identity.textContent).toContain('1.0.0');
		expect(identity.textContent).toContain('Deterministic rules');
		expect(identity.textContent).toContain('R32 e2e rules');
		expect(identity.textContent).toContain('did:key:zOperator');
		expect(identity.textContent).toContain('2025-06-15T15:06:40.000Z');
		expect(identity.textContent).toContain('2025-06-15T15:08:20.000Z');

		// Each digest tier is its own labelled cell, with its own value.
		const artifact = screen.getByTestId('models-detail-digest-artifact');
		const config = screen.getByTestId('models-detail-digest-config');
		const calibration = screen.getByTestId('models-detail-digest-calibration');
		const row = screen.getByTestId('models-detail-digest-row');
		expect(artifact.textContent).toContain(ARTIFACT_DIGEST);
		expect(config.textContent).toContain(CONFIG_DIGEST);
		expect(calibration.textContent?.trim()).toBe(ABSENT);
		expect(row.textContent).toContain(ROW_DIGEST);

		// No tier is rendered in place of another, and none is invented.
		for (const other of [config, calibration, row]) {
			expect(other.textContent).not.toContain(ARTIFACT_DIGEST);
		}
		for (const cell of [artifact, config, row]) {
			expect(cell.textContent).toMatch(/[0-9a-f]{64}/);
		}
		expect(calibration.textContent).not.toMatch(/[0-9a-f]{64}/);

		// Focus is managed: the surface opens on its own heading.
		await waitFor(() =>
			expect(document.activeElement).toBe(screen.getByRole('heading', { level: 1 }))
		);
	});

	test('models_detail_names_row_digest_as_a_proposal_pin_not_a_signature', async () => {
		refUrl(DETAIL_REF);
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-detail-ready');

		// In its own words, on the surface: a content pin, not a signature, and
		// not authentication.
		expect(screen.getByTestId('models-digest-pin-note').textContent?.trim()).toBe(
			'A content pin, not a signature and not authentication.'
		);
		const detail = screen.getByTestId('models-detail');
		expect(detail.textContent).toMatch(/content pin/i);
		expect(detail.textContent).toMatch(/not a signature/i);
		expect(detail.textContent).toMatch(/not authentication/i);
		expect(detail.textContent).not.toMatch(
			/(signature|authentication|authorisation) (proves|verifies|establishes|guarantees|attests)/i
		);
		expect(detail.textContent).not.toMatch(/\bverified by\b|\bsigned by\b|\bproof of\b/i);

		// The digest VALUE cell itself never claims to be a signature...
		const row = screen.getByTestId('models-detail-digest-row');
		expect(row.textContent).toMatch(/^[0-9a-f]{64}$/);
		expect(row.textContent ?? '').not.toMatch(/signature|authenticat|verif/i);
		// ...and it is pinned for a proposal, which is what it is for.
		expect(screen.getByTestId('models-proposal-remedy').textContent).toContain('digest');
	});

	test('models_detail_offers_only_legal_transitions_and_none_when_retired', async () => {
		refUrl(DETAIL_REF);
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-detail-ready');

		// candidate -> promote + retire, and nothing else.
		expect(offeredActions()).toHaveLength(2);
		expect(screen.getByTestId('models-action-promote').textContent).toBe('Propose promotion');
		expect(screen.getByTestId('models-action-retire').textContent).toBe('Propose retirement');
		expect(screen.queryByTestId('models-action-none')).toBeNull();

		// evaluated -> the same two; promoted -> retire only.
		cleanup();
		refUrl(DETAIL_REF);
		serveProposal(200, { id: 1, status: 'pending' }, withDetail((row) => (row['status'] = 'evaluated')));
		await renderRoute();
		await waitForTestId('models-detail-ready');
		expect(screen.getByTestId('models-detail-status').textContent).toContain('Evaluated');
		expect(offeredActions()).toHaveLength(2);

		// promoted -> retirement only: promotion is illegal from `promoted`.
		cleanup();
		refUrl(DETAIL_REF);
		serveProposal(200, { id: 1, status: 'pending' }, withDetail((row) => (row['status'] = 'promoted')));
		await renderRoute();
		await waitForTestId('models-detail-ready');
		expect(screen.getByTestId('models-detail-status').textContent).toContain('Promoted');
		expect(screen.queryByTestId('models-action-promote')).toBeNull();
		expect(screen.getByTestId('models-action-retire')).toBeTruthy();

		// retired -> NO transition at all, and the surface says why.
		cleanup();
		refUrl(DETAIL_REF);
		serveProposal(200, { id: 1, status: 'pending' }, withDetail((row) => (row['status'] = 'retired')));
		await renderRoute();
		await waitForTestId('models-detail-ready');
		expect(screen.getByTestId('models-detail-status').textContent).toContain('Retired');
		expect(offeredActions()).toHaveLength(0);
		expect(screen.queryByTestId('models-action-promote')).toBeNull();
		expect(screen.queryByTestId('models-action-retire')).toBeNull();
		expect(screen.getByTestId('models-action-none').textContent).toContain(
			'No transition is available from this state.'
		);
		expect(seenPosts()).toHaveLength(0);
	});

	test('models_detail_never_claims_evaluated_reachability', async () => {
		refUrl(DETAIL_REF);
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-detail-ready');

		// Two evaluation refs and the row is still `candidate`: nothing here
		// suggests a join can reach, produce, or imply `evaluated`.
		expect(screen.getByTestId('models-detail-status').textContent?.trim()).toBe('Candidate');
		const ready = screen.getByTestId('models-detail-ready');
		expect(ready.textContent).not.toMatch(/\bevaluated\b/i);
		expect(ready.textContent).not.toMatch(/reach/i);
		expect(ready.textContent).not.toMatch(/qualif|pass(ed)?|verdict|threshold|outcome/i);
		expect(ready.textContent).toContain('non-authoritative');

		// And the claim is not reachable by any control on the surface.
		expect(screen.queryByTestId('models-action-promote')).toBeTruthy();
		expect(screen.queryByRole('checkbox')).toBeNull();
		expect(screen.queryByRole('combobox')).toBeNull();
		expect(screen.queryByRole('switch')).toBeNull();
	});

	test('models_detail_evaluation_join_is_non_authoritative_and_never_a_status_signal', async () => {
		refUrl(DETAIL_REF);
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-detail-ready');

		// The join renders as a bounded, disclosed, non-authoritative list.
		const evaluation = screen.getByTestId('models-detail-evaluation');
		expect(evaluation.textContent).toContain('Evaluation references');
		expect(screen.getByTestId('models-detail-evaluation-note').textContent).toBe(
			'Evaluation references are non-authoritative and never change a status.'
		);
		expect(screen.queryByTestId('models-detail-evaluation-status')).toBeNull();

		// The disclosure is a real, keyboard-operable control: it is reachable
		// by Tab, and the keyboard alone opens it.
		const toggle = screen.getByTestId('models-detail-evaluation-toggle');
		expect(toggle.tagName).toBe('BUTTON');
		expect(toggle.getAttribute('aria-expanded')).toBe('false');
		expect(toggle.getAttribute('aria-controls')).toBe('models-evaluation-panel');
		expect(document.getElementById('models-evaluation-panel')?.hasAttribute('hidden')).toBe(true);
		const keyboard = userEvent.setup();
		for (let step = 0; step < 6 && document.activeElement !== toggle; step += 1) {
			await keyboard.tab();
		}
		expect(document.activeElement).toBe(toggle);
		await keyboard.keyboard('{Enter}');
		expect(toggle.getAttribute('aria-expanded')).toBe('true');
		expect(document.getElementById('models-evaluation-panel')?.hasAttribute('hidden')).toBe(false);
		expect(screen.getAllByTestId('models-detail-evaluation-item').map((item) => item.textContent)).toEqual([
			'eval-r30-001',
			'eval-r30-002'
		]);

		// The join does not gate the transitions: a row with NO refs offers the
		// identical set.
		const withRefs = offeredActions();
		cleanup();
		refUrl(DETAIL_REF);
		serveProposal(
			200,
			{ id: 1, status: 'pending' },
			withDetail((row) => (row['evaluation_refs'] = []))
		);
		await renderRoute();
		await waitForTestId('models-detail-ready');
		expect(offeredActions()).toEqual(withRefs);
		expect(screen.getByTestId('models-detail-status').textContent?.trim()).toBe('Candidate');
	});

	test('models_promotion_proposal_sends_exact_typed_lifecycle_payload', async () => {
		refUrl(DETAIL_REF);
		serveProposal(200, { id: 11, status: 'pending' });
		await renderRoute();
		await waitForTestId('models-detail-ready');

		const parsed = parseModelRegistryDetail(detailBody());
		expect(parsed.ok).toBe(true);
		if (!parsed.ok) return;

		await userEvent.setup().click(screen.getByTestId('models-action-promote'));
		await waitForTestId('models-proposal-pending');

		const posts = seenPosts();
		expect(posts).toHaveLength(1);
		expect(pathOf(posts[0]?.url ?? '')).toBe(PROPOSAL_PATH);

		const sent = postBody();
		// The typed lifecycle payload, and nothing else on the envelope.
		expect(Object.keys(sent).sort()).toEqual(['content', 'kind']);
		expect(sent['kind']).toBe('registry_lifecycle');
		// The content is the parser's VERBATIM serialization, byte for byte.
		expect(sent['content']).toBe(lifecycleProposalContent('promote', parsed.value));

		// The action and the row are carried INSIDE that content, which is the
		// exact document the kernel's own gate reads.
		const payload = JSON.parse(String(sent['content'])) as Record<string, unknown>;
		expect(Object.keys(payload).sort()).toEqual(['action', 'id', 'row', 'row_digest', 'version']);
		expect(payload['action']).toBe('promote');
		expect(payload['row_digest']).toBe(ROW_DIGEST);
		const row = payload['row'] as Record<string, unknown>;
		expect(row['id']).toBe('rules-r32-e2e');
		expect(row['version']).toBe('1.0.0');
		expect(row['status']).toBe('candidate');
		expect(row['evaluation_refs']).toEqual(['eval-r30-001', 'eval-r30-002']);

		// The row is EXACTLY the kernel's RegistryRow field set. The live wire
		// refused this payload twice before the E2E caught it: once for an
		// unknown field `row_digest` (RegistryRow is deny_unknown_fields and has
		// no such member) and once for a missing `approved_by` (no serde
		// default). The pin travels BESIDE the row, never inside it.
		expect(Object.keys(row).sort()).toEqual([...REGISTRY_ROW_FIELDS].sort());
		expect(row).not.toHaveProperty('row_digest');
		expect(Object.keys(row)).toContain('approved_by');
	});

	test('models_promotion_proposal_submits_verbatim_wire_values_never_display_sanitized_values', async () => {
		const rawName = `Acme${ZWSP}models`;
		refUrl(DETAIL_REF);
		serveProposal(200, { id: 12, status: 'pending' }, withDetail((row) => (row['name'] = rawName)));
		await renderRoute();
		await waitForTestId('models-detail-ready');

		// On the way to the DOM the invisible byte is removed.
		const shown = screen.getByTestId('models-detail-name');
		expect(shown.textContent).toBe('Acmemodels');
		expect(shown.textContent).not.toContain(ZWSP);

		await userEvent.setup().click(screen.getByTestId('models-action-promote'));
		await waitForTestId('models-proposal-pending');

		// On the way back to the kernel the ORIGINAL bytes ride, invisible byte
		// included: display sanitization never mutates outbound data.
		const sentContent = String(postBody()['content']);
		expect(sentContent).toContain(rawName);
		const sentPayload = JSON.parse(sentContent) as Record<string, unknown>;
		const sentRow = sentPayload['row'] as Record<string, unknown>;
		expect(sentRow['name']).toBe(rawName);
		expect(sentRow['name']).toContain(ZWSP);
		expect(sentRow['name']).not.toBe('Acmemodels');
		// And the payload still equals the parser's own bytes for that row.
		const parsed = parseModelRegistryDetail(withDetail((row) => (row['name'] = rawName)));
		expect(parsed.ok).toBe(true);
		if (!parsed.ok) return;
		expect(sentContent).toBe(lifecycleProposalContent('promote', parsed.value));
	});

	test('models_promotion_receipt_reports_pending_only', async () => {
		refUrl(DETAIL_REF);
		serveProposal(200, { id: 11, status: 'pending' });
		await renderRoute();
		await waitForTestId('models-detail-ready');
		await userEvent.setup().click(screen.getByTestId('models-action-promote'));
		await waitForTestId('models-proposal-pending');

		const receipt = screen.getByTestId('models-proposal-pending');
		expect(receipt.textContent).toContain('Pending human approval');
		expect(screen.getByTestId('models-proposal-id').textContent).toBe('11');
		// The receipt claims nothing about the row: a proposal is pending, and
		// the row's status is still whatever the kernel last reported.
		expect(receipt.textContent ?? '').not.toMatch(/promoted|retired|approved|applied|done/i);
		expect(screen.getByTestId('models-detail-status').textContent?.trim()).toBe('Candidate');
		expect(screen.getByTestId('models-detail-digest-row').textContent).toContain(ROW_DIGEST);

		// A receipt that is not `pending` is not shown as one.
		cleanup();
		refUrl(DETAIL_REF);
		serveProposal(200, { id: 12, status: 'approved' });
		await renderRoute();
		await waitForTestId('models-detail-ready');
		await userEvent.setup().click(screen.getByTestId('models-action-promote'));
		await waitForTestId('models-proposal-error');
		expect(screen.queryByTestId('models-proposal-pending')).toBeNull();
		expect(screen.getByTestId('models-detail-status').textContent?.trim()).toBe('Candidate');
	});

	test('models_promotion_refuses_named_errors_with_remedies', async () => {
		const refusals = [
			{ status: 400, text: 'The request was rejected as invalid.' },
			{ status: 401, text: 'Not authenticated.' },
			{ status: 403, text: 'Not authorized.' },
			{ status: 422, text: 'The requested transition is not legal from this state.' },
			{ status: 500, text: 'The server could not complete the request.' }
		] as const;

		for (const refusal of refusals) {
			cleanup();
			refUrl(DETAIL_REF);
			serveProposal(refusal.status, { error: 'registry_payload_invalid' });
			await renderRoute();
			await waitForTestId('models-detail-ready');
			await userEvent.setup().click(screen.getByTestId('models-action-promote'));
			await waitForTestId('models-proposal-error');

			// Each refusal is NAMED, and the remedy is on the surface.
			expect(screen.getByTestId('models-proposal-error').textContent).toContain(refusal.text);
			expect(screen.getByTestId('models-proposal-remedy').textContent).toContain(
				'A human approves it by digest'
			);
			expect(screen.queryByTestId('models-proposal-pending')).toBeNull();
			// A refused proposal changes nothing that is on screen.
			expect(screen.getByTestId('models-detail-status').textContent?.trim()).toBe('Candidate');
		}

		// An unreachable kernel is a host state, named as such.
		cleanup();
		refUrl(DETAIL_REF);
		responder = async (url, init, method) => {
			if (method === 'POST' && pathOf(url) === PROPOSAL_PATH) {
				throw new TypeError('failed to fetch');
			}
			return jsonResponse(detailBody());
		};
		await renderRoute();
		await waitForTestId('models-detail-ready');
		await userEvent.setup().click(screen.getByTestId('models-action-promote'));
		await waitForTestId('models-proposal-network');
		expect(screen.getByTestId('models-proposal-network').textContent).toContain(
			'This is a host state, not an empty registry'
		);
	});

	test('models_promotion_conflict_offers_reload_not_bare_retry', async () => {
		refUrl(DETAIL_REF);
		serveProposal(409, { error: 'registry_row_changed' });
		await renderRoute();
		await waitForTestId('models-detail-ready');
		await userEvent.setup().click(screen.getByTestId('models-action-promote'));
		await waitForTestId('models-proposal-conflict');

		expect(screen.getByTestId('models-proposal-conflict').textContent).toContain(
			'This row changed. Reload to read the current digest.'
		);
		expect(screen.getByTestId('models-detail-reload')).toBeTruthy();
		// No receipt is claimed, and the pinned bytes are not offered for a
		// re-send: the action is disabled until the row is re-read.
		expect(screen.queryByTestId('models-proposal-pending')).toBeNull();
		expect(screen.queryByTestId('models-proposal-retry')).toBeNull();
		expect(screen.getByTestId('models-action-promote').hasAttribute('disabled')).toBe(true);
		expect(screen.getByTestId('models-action-retire').hasAttribute('disabled')).toBe(true);

		// The reload re-reads the row; it does not resubmit the proposal.
		seen.length = 0;
		await userEvent.setup().click(screen.getByTestId('models-detail-reload'));
		await waitForTestId('models-detail-ready');
		expect(seenPosts()).toHaveLength(0);
		expect(seen.filter((request) => pathOf(request.url).startsWith(DETAIL_PATH_PREFIX)).length).toBeGreaterThan(
			0
		);
		expect(screen.queryByTestId('models-proposal-conflict')).toBeNull();
		expect(screen.getByTestId('models-action-promote').hasAttribute('disabled')).toBe(false);
	});

	test('models_never_offers_approve_reject_or_direct_status_write', async () => {
		refUrl(DETAIL_REF);
		serveProposal(200, { id: 11, status: 'pending' });
		await renderRoute();
		await waitForTestId('models-detail-ready');

		// No approval control, on either surface state, in any vocabulary.
		for (const name of [/approve/i, /reject/i, /dispose/i, /set status/i, /write status/i]) {
			expect(screen.queryByRole('button', { name })).toBeNull();
			expect(screen.queryByRole('link', { name })).toBeNull();
			expect(screen.queryByRole('menuitem', { name })).toBeNull();
		}
		// No free-text or enumerated field that could carry a status write.
		expect(screen.queryByRole('textbox')).toBeNull();
		expect(screen.queryByRole('combobox')).toBeNull();

		await userEvent.setup().click(screen.getByTestId('models-action-promote'));
		await waitForTestId('models-proposal-pending');

		// Exactly one write, on the proposal gate, carrying no status channel.
		const posts = seenPosts();
		expect(posts).toHaveLength(1);
		expect(pathOf(posts[0]?.url ?? '')).toBe(PROPOSAL_PATH);
		const sent = postBody();
		expect(Object.keys(sent).sort()).toEqual(['content', 'kind']);
		expect(sent).not.toHaveProperty('status');
		expect(sent).not.toHaveProperty('new_status');
		expect(seen.some((request) => request.url.includes('/approve'))).toBe(false);
		expect(seen.some((request) => request.url.includes('/reject'))).toBe(false);
	});

	test('models_surfaces_pass_axe_aa', async () => {
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-list-row');
		await expectAxeClean();

		// A denial is a first-class surface, not an afterthought.
		cleanup();
		responder = async () => jsonResponse({ error: 'forbidden' }, 403);
		await renderRoute();
		await waitForTestId('models-list-error');
		await expectAxeClean();

		// The single-row read, with its disclosure collapsed.
		cleanup();
		refUrl(DETAIL_REF);
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-detail-ready');
		await expectAxeClean();

		// The single-row read, with the evaluation join disclosed.
		await userEvent.setup().click(screen.getByTestId('models-detail-evaluation-toggle'));
		expect(toggleExpanded()).toBe(true);
		await expectAxeClean();

		// The proposal receipt and the conflict state.
		cleanup();
		refUrl(DETAIL_REF);
		serveProposal(200, { id: 11, status: 'pending' });
		await renderRoute();
		await waitForTestId('models-detail-ready');
		await userEvent.setup().click(screen.getByTestId('models-action-promote'));
		await waitForTestId('models-proposal-pending');
		await expectAxeClean();

		cleanup();
		refUrl(DETAIL_REF);
		serveProposal(409, { error: 'registry_row_changed' });
		await renderRoute();
		await waitForTestId('models-detail-ready');
		await userEvent.setup().click(screen.getByTestId('models-action-promote'));
		await waitForTestId('models-proposal-conflict');
		await expectAxeClean();

		// The malformed-ref refusal, which never reaches the wire.
		cleanup();
		refUrl('not-a-ref');
		serveRegistry();
		await renderRoute();
		await waitForTestId('models-detail-ref-malformed');
		await expectAxeClean();
	});

	test('models_read_403_does_not_blank_a_read_detail_ref', async () => {
		refUrl(DETAIL_REF);
		// The listing is Admin + DPO; the single-row read is Read. A principal
		// that can read the row is refused the listing — and the row still reads.
		responder = async (url) => {
			const path = pathOf(url);
			if (path === LIST_PATH) return jsonResponse({ error: 'forbidden' }, 403);
			if (path.startsWith(DETAIL_PATH_PREFIX)) return jsonResponse(detailBody());
			return notFound();
		};
		await renderRoute();
		await waitForTestId('models-detail-ready');

		// The forbidden listing is neither rendered nor requested in this state.
		expect(screen.queryByTestId('models-list-error')).toBeNull();
		expect(screen.queryByTestId('models-list-empty')).toBeNull();
		expect(screen.queryByTestId('models-list-row')).toBeNull();
		expect(seen.some((request) => pathOf(request.url) === LIST_PATH)).toBe(false);
		expect(seen.some((request) => pathOf(request.url).startsWith(DETAIL_PATH_PREFIX))).toBe(true);

		// The row is whole: identity, status, digests, and its own transitions.
		expect(screen.getByTestId('models-detail-identity').textContent).toContain('rules-r32-e2e');
		expect(screen.getByTestId('models-detail-status').textContent?.trim()).toBe('Candidate');
		expect(screen.getByTestId('models-detail-digest-artifact').textContent).toContain(ARTIFACT_DIGEST);
		expect(screen.getByTestId('models-detail-digest-row').textContent).toContain(ROW_DIGEST);
		expect(screen.getByTestId('models-action-promote')).toBeTruthy();
		expect(screen.getByTestId('models-action-retire')).toBeTruthy();
	});
});

function toggleExpanded(): boolean {
	return screen.getByTestId('models-detail-evaluation-toggle').getAttribute('aria-expanded') === 'true';
}
