import { expect, test, vi, afterEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

afterEach(() => {
	vi.unstubAllGlobals();
});

/**
 * recall_renders_results_with_trace_links — the /recall surface renders
 * typed hit cards from POST /recall, renders the per-query trace deep-link
 * ONLY when the kernel returned a trace_id, and shows the honest
 * low_confidence abstention state (nothing invented) when the kernel
 * abstains.
 */
test('recall_renders_results_with_trace_links', async () => {
	const user = userEvent.setup();
	let abstainWire = false;
	let traceId: number | null = 4242;
	vi.stubGlobal(
		'fetch',
		vi.fn(async (input: RequestInfo | URL) => {
			// openapi-fetch passes a Request object — route by its .url.
			const url = input instanceof Request ? input.url : String(input);
			if (url.includes('/recall')) {
				const body = abstainWire
					? { hits: [], decision: 'low_confidence', domains_searched: [], trace_id: null }
					: {
							hits: [
								{
									id: 9,
									title: 'StewardOS wiring note',
									snippet: 'the recall bench seeds a support-ticket doc',
									score: 0.8234,
									source: 'vector',
									conflict: false
								}
							],
							decision: 'ok',
							domains_searched: ['global'],
							trace_id: traceId
						};
				return new Response(JSON.stringify(body), {
					status: 200,
					headers: { 'content-type': 'application/json' }
				});
			}
			return new Response('not found', { status: 404 });
		})
	);
	const mod = await import('../src/routes/recall/+page.svelte');
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	render(mod.default as any);

	// The typed happy path: hit card + the trace deep-link with the
	// kernel-provided trace id.
	const input = screen.getByTestId('recall-input');
	await user.type(input, 'wiring');
	await user.click(screen.getByRole('button', { name: 'Recall' }));
	const card = await screen.findByTestId('recall-hit');
	expect(card.textContent).toContain('StewardOS wiring note');
	expect(card.textContent).toContain('the recall bench seeds a support-ticket doc');
	expect(card.textContent).toContain('0.823');
	const link = await screen.findByTestId('trace-link');
	expect(link.getAttribute('href')).toBe('/recall/4242/trace');

	// The honest abstention: low_confidence + no hits → the abstain banner,
	// NO cards, NO trace link (nothing was invented).
	abstainWire = true;
	await user.type(input, '2');
	await user.click(screen.getByRole('button', { name: 'Recall' }));
	await waitFor(() =>
		expect(screen.getByRole('status').textContent).toContain('Low confidence')
	);
	expect(screen.queryByTestId('recall-hit')).toBeNull();
	expect(screen.queryByTestId('trace-link')).toBeNull();
});
