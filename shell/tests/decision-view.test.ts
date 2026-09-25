import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, test, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

const pageState: { params: Record<string, string> } = { params: {} };
vi.mock('$app/state', () => ({ page: pageState }));

type Responder = (url: string, init: RequestInit | undefined, method: string) => Promise<Response>;
const noWire: Responder = async () => new Response('no wire', { status: 599 });
let responder: Responder = noWire;
const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
	const url = input instanceof Request ? input.url : String(input);
	const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
	return responder(url, init, method);
});
vi.stubGlobal('fetch', fetchMock);

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function fixture(): Record<string, unknown> {
	const parsed: unknown = JSON.parse(
		readFileSync(join(process.cwd(), 'tests', 'fixtures', 'decision-run.json'), 'utf8')
	);
	if (!isRecord(parsed)) throw new Error('decision fixture must be an object');
	return parsed;
}

function listBody(): Record<string, unknown> {
	return {
		rows: [
			{
				id: 42,
				run_id: 7,
				mode: 'exploratory',
				pipeline_version: '1.32.11',
				config_hash: 'a'.repeat(64),
				created_at: 1750000000,
				stage_count: 2
			}
		],
		count: 1
	};
}

function replayBody(): Record<string, unknown> {
	return {
		trace_id: 42,
		config_hash: 'a'.repeat(64),
		config_hash_match: true,
		replay_input_digest: 'b'.repeat(64),
		input_digest_match: false,
		stages: [
			{
				stage: 'normalize',
				match: false,
				stored_outputs_digest: 'c'.repeat(64),
				replayed_outputs_digest: 'd'.repeat(64)
			}
		],
		all_match: false
	};
}

function serveJson(urlPart: string, body: unknown, status = 200): void {
	responder = async (url) =>
		url.includes(urlPart)
			? new Response(JSON.stringify(body), {
					status,
					headers: { 'content-type': 'application/json' }
				})
			: new Response('not found', { status: 404 });
}

async function renderRoute(routePath: string): Promise<void> {
	const module =
		routePath === '/decisions'
			? await import('../src/routes/decisions/+page.svelte')
			: await import('../src/routes/decisions/[id]/+page.svelte');
	render(module.default);
}

async function waitForTestId(testId: string): Promise<void> {
	await waitFor(() => expect(screen.queryByTestId(testId)).not.toBeNull(), { timeout: 5000 });
}

async function expectAxeClean(): Promise<void> {
	const axe = await import('axe-core');
	const results = await axe.default.run(document.body, {
		runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
	});
	expect(results.violations).toEqual([]);
}

async function setValue(testId: string, value: string): Promise<void> {
	await fireEvent.input(screen.getByTestId(testId), { target: { value } });
}

afterEach(() => {
	responder = noWire;
	pageState.params = {};
	vi.clearAllMocks();
});

describe('Decision Explorer surfaces', () => {
	test('decision_list_uses_only_limit_and_run_id_server_queries', async () => {
		const urls: string[] = [];
		responder = async (url) => {
			urls.push(url);
			return new Response(JSON.stringify(listBody()), {
				status: 200,
				headers: { 'content-type': 'application/json' }
			});
		};
		await renderRoute('/decisions');
		await waitForTestId('decision-list-row');
		const requestUrl = new URL(urls[0] ?? 'http://127.0.0.1/');
		expect([...requestUrl.searchParams.keys()].sort()).toEqual(['limit']);
		expect(requestUrl.searchParams.get('limit')).toBe('20');
		expect(requestUrl.searchParams.has('mode')).toBe(false);
		expect(requestUrl.searchParams.has('outcome')).toBe(false);
		expect(requestUrl.searchParams.has('date')).toBe(false);
	});

	test('decision_explorer_renders_summary_rows_and_deep_links', async () => {
		serveJson('/workflow/decision-runs', listBody());
		await renderRoute('/decisions');
		await waitForTestId('decision-list-row');
		expect(screen.getByTestId('decision-list-row').textContent).toContain('42');
		expect(screen.getByTestId('decision-list-row').textContent).toContain('1.32.11');
		expect(screen.getByTestId('decision-list-link').getAttribute('href')).toBe('/decisions/42');
	});

	test('decision_explorer_renders_mode_and_stage_provenance', async () => {
		pageState.params = { id: '42' };
		serveJson('/workflow/decision-runs/42', fixture());
		await renderRoute('/decisions/42');
		await waitForTestId('decision-detail-ready');
		expect(screen.getByTestId('decision-detail-ready').textContent).toContain('exploratory');
		expect(screen.getByTestId('decision-context-table').textContent).toContain('untrusted');
		expect(screen.getByTestId('decision-model-table').textContent).toContain('rules-r32-fixture');
		await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('heading', { level: 1 })));
		const stageButton = screen.getByRole('button', { name: /normalize/ });
		expect(stageButton.getAttribute('aria-expanded')).toBe('false');
		await userEvent.setup().click(stageButton);
		expect(stageButton.getAttribute('aria-expanded')).toBe('true');
		expect(screen.getByTestId('decision-stage-output').textContent).toContain('query_digest');
		expect(screen.getByTestId('decision-stage-timing').textContent).toContain('2025-06-15T15:06:40.000Z');
	});

	test('decision_explorer_marks_exploratory_output_non_promotable', async () => {
		pageState.params = { id: '42' };
		serveJson('/workflow/decision-runs/42', fixture());
		await renderRoute('/decisions/42');
		await waitForTestId('decision-detail-nonpromotable');
		expect(screen.getByTestId('decision-detail-nonpromotable').textContent).toContain(
			'cannot promote'
		);
		expect(screen.queryByRole('button', { name: /promot/i })).toBeNull();
	});

	test('decision_detail_renders_loading_empty_403_404_and_invalid_states', async () => {
		responder = async () => new Promise<Response>(() => undefined);
		await renderRoute('/decisions');
		expect(screen.getByTestId('decision-list-loading')).toBeTruthy();

		responder = async () =>
			new Response(JSON.stringify({ rows: [], count: 0 }), {
				status: 200,
				headers: { 'content-type': 'application/json' }
			});
		const { cleanup } = await import('@testing-library/svelte');
		cleanup();
		await renderRoute('/decisions');
		await waitForTestId('decision-list-empty');

		cleanup();
		responder = async () => new Response(JSON.stringify({ error: 'not_found' }), { status: 404 });
		await renderRoute('/decisions');
		await waitForTestId('decision-list-error');
		expect(screen.getByTestId('decision-list-error').textContent).toContain('not found');

		cleanup();
		responder = async () => new Response(JSON.stringify({ error: 'conflict' }), { status: 409 });
		await renderRoute('/decisions');
		await waitForTestId('decision-list-error');
		expect(screen.getByTestId('decision-list-error').textContent).toContain('conflicts');

		cleanup();
		pageState.params = { id: '42' };
		responder = async () => new Response(JSON.stringify({ error: 'forbidden' }), { status: 403 });
		await renderRoute('/decisions/42');
		await waitForTestId('decision-detail-error');

		cleanup();
		responder = async () => new Response(JSON.stringify({ error: 'conflict' }), { status: 409 });
		await renderRoute('/decisions/42');
		await waitForTestId('decision-detail-error');
		expect(screen.getByTestId('decision-detail-error').textContent).toContain('conflicts');

		cleanup();
		pageState.params = { id: '999' };
		responder = async () => new Response(JSON.stringify({ error: 'not_found' }), { status: 404 });
		await renderRoute('/decisions/42');
		await waitForTestId('decision-detail-not-found');

		cleanup();
		pageState.params = { id: 'not-an-id' };
		await renderRoute('/decisions/42');
		await waitForTestId('decision-detail-invalid');
	});

	test('decision_replay_requires_operator_supplied_inputs', async () => {
		pageState.params = { id: '42' };
		serveJson('/workflow/decision-runs/42', fixture());
		await renderRoute('/decisions/42');
		await waitForTestId('decision-replay');
		const submit = screen.getByTestId('replay-submit');
		expect(submit.hasAttribute('disabled')).toBe(true);
		await userEvent.setup().click(submit);
		expect(
			fetchMock.mock.calls.some(([input]) => {
				const url = input instanceof Request ? input.url : String(input);
				return url.includes('/replay-diff');
			})
		).toBe(false);
	});

	test('decision_replay_never_persists_or_logs_raw_inputs', async () => {
		pageState.params = { id: '42' };
		serveJson('/workflow/decision-runs/42', fixture());
		const original = responder;
		responder = async (url, init, method) => {
			if (url.includes('/replay-diff') && method === 'POST') {
				return new Response(JSON.stringify(replayBody()), {
					status: 200,
					headers: { 'content-type': 'application/json' }
				});
			}
			return original(url, init, method);
		};
		const storage = vi.spyOn(Storage.prototype, 'setItem');
		const log = vi.spyOn(console, 'log').mockImplementation(() => undefined);
		const errorLog = vi.spyOn(console, 'error').mockImplementation(() => undefined);
		const user = userEvent.setup();
		await renderRoute('/decisions/42');
		await waitForTestId('decision-replay');
		await setValue('replay-config', '{"pipeline_id":"synthetic"}');
		await setValue('replay-rules', '{"model_id":"synthetic"}');
		await setValue('replay-request-id', 'synthetic-request');
		await setValue('replay-question-ids', 'needs_human');
		await setValue('replay-query', 'synthetic operator query');
		await user.click(screen.getByTestId('replay-submit'));
		await waitForTestId('decision-replay-report');
		expect(storage).not.toHaveBeenCalled();
		expect(log).not.toHaveBeenCalled();
		expect(errorLog).not.toHaveBeenCalled();
		const queryInput = screen.getByTestId('replay-query');
		expect(queryInput instanceof HTMLTextAreaElement ? queryInput.value : '').toBe('');
		storage.mockRestore();
		log.mockRestore();
		errorLog.mockRestore();
	});

	test('decision_replay_renders_agreement_as_data_not_a_gate', async () => {
		pageState.params = { id: '42' };
		serveJson('/workflow/decision-runs/42', fixture());
		const original = responder;
		responder = async (url, init, method) => {
			if (url.includes('/replay-diff') && method === 'POST') {
				return new Response(JSON.stringify(replayBody()), {
					status: 200,
					headers: { 'content-type': 'application/json' }
				});
			}
			return original(url, init, method);
		};
		const user = userEvent.setup();
		await renderRoute('/decisions/42');
		await waitForTestId('decision-replay');
		await setValue('replay-config', '{"pipeline_id":"synthetic"}');
		await setValue('replay-rules', '{"model_id":"synthetic"}');
		await setValue('replay-request-id', 'synthetic-request');
		await setValue('replay-question-ids', 'needs_human');
		await setValue('replay-query', 'synthetic operator query');
		await user.click(screen.getByTestId('replay-submit'));
		await waitForTestId('decision-replay-report');
		const report = screen.getByTestId('decision-replay-report');
		expect(report.textContent).toContain('false');
		expect(report.textContent).toContain('normalize');
		expect(report.textContent).not.toMatch(/promot|approve|pass\/fail/i);
	});

	test('decision_replay_does_not_change_the_trace', async () => {
		pageState.params = { id: '42' };
		serveJson('/workflow/decision-runs/42', fixture());
		const original = responder;
		let detailReads = 0;
		responder = async (url, init, method) => {
			if (url.includes('/replay-diff') && method === 'POST') {
				return new Response(JSON.stringify(replayBody()), {
					status: 200,
					headers: { 'content-type': 'application/json' }
				});
			}
			if (url.includes('/workflow/decision-runs/42') && method !== 'POST') detailReads += 1;
			return original(url, init, method);
		};
		const user = userEvent.setup();
		await renderRoute('/decisions/42');
		await waitForTestId('decision-replay');
		const before = detailReads;
		await setValue('replay-config', '{"pipeline_id":"synthetic"}');
		await setValue('replay-rules', '{"model_id":"synthetic"}');
		await setValue('replay-request-id', 'synthetic-request');
		await setValue('replay-question-ids', 'needs_human');
		await setValue('replay-query', 'synthetic operator query');
		await user.click(screen.getByTestId('replay-submit'));
		await waitForTestId('decision-replay-report');
		expect(detailReads).toBe(before);
		expect(screen.getByTestId('decision-outcome').textContent).toBe('escalate');
	});

	test('decision_replay_refuses_bad_gate_statuses_without_authority', async () => {
		const cases = [
			{ status: 400, expected: 'invalid' },
			{ status: 401, expected: 'Authentication' },
			{ status: 403, expected: 'cannot' },
			{ status: 404, expected: 'no longer' },
			{ status: 409, expected: 'Config hash mismatch' }
		] as const;
		for (const item of cases) {
			pageState.params = { id: '42' };
			serveJson('/workflow/decision-runs/42', fixture());
			const original = responder;
			responder = async (url, init, method) => {
				if (url.includes('/replay-diff') && method === 'POST') {
					return new Response(JSON.stringify({ error: 'synthetic refusal' }), {
						status: item.status,
						headers: { 'content-type': 'application/json' }
					});
				}
				return original(url, init, method);
			};
			const user = userEvent.setup();
			await renderRoute('/decisions/42');
			await waitForTestId('decision-replay');
			await setValue('replay-config', '{"pipeline_id":"synthetic"}');
			await setValue('replay-rules', '{"model_id":"synthetic"}');
			await setValue('replay-request-id', 'synthetic-request');
			await setValue('replay-question-ids', 'needs_human');
			await setValue('replay-query', 'synthetic operator query');
			await user.click(screen.getByTestId('replay-submit'));
			await waitForTestId('replay-error');
			expect(screen.getByTestId('replay-error').textContent).toContain(item.expected);
			const { cleanup } = await import('@testing-library/svelte');
			cleanup();
		}
	});

	test('decision_surfaces_pass_axe_aa', async () => {
		const { cleanup } = await import('@testing-library/svelte');

		serveJson('/workflow/decision-runs', listBody());
		await renderRoute('/decisions');
		await waitForTestId('decision-list-table');
		await expectAxeClean();

		cleanup();
		responder = async () => new Response(JSON.stringify({ error: 'conflict' }), { status: 409 });
		await renderRoute('/decisions');
		await waitForTestId('decision-list-error');
		await expectAxeClean();

		cleanup();
		pageState.params = { id: '42' };
		serveJson('/workflow/decision-runs/42', fixture());
		await renderRoute('/decisions/42');
		await waitForTestId('decision-detail-ready');
		await expectAxeClean();

		cleanup();
		responder = async () => new Response(JSON.stringify({ error: 'conflict' }), { status: 409 });
		await renderRoute('/decisions/42');
		await waitForTestId('decision-detail-error');
		await expectAxeClean();
	});
});
