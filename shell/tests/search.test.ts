import { expect, test, vi, afterEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

afterEach(() => {
	vi.unstubAllGlobals();
});

/**
 * search_returns_typed_results — the /search surface renders TYPED result
 * cards from GET /search (title, source badge, similarity, snippet) and
 * shows the honest empty + error states on the other wire outcomes.
 */
test('search_returns_typed_results', async () => {
	const user = userEvent.setup();
	let failWire = false;
	let emptyWire = false;
	vi.stubGlobal(
		'fetch',
		vi.fn(async () => {
			if (failWire) return new Response('boom', { status: 500 });
			const body = emptyWire
				? { success: true, results: [] }
				: {
						success: true,
						results: [
							{
								id: 3,
								similarity: 0.9137,
								title: 'Lead-safe wiring note',
								snippet: 'the bench wiring is lead-safe and labelled',
								source: 'vector'
							}
						]
					};
			return new Response(JSON.stringify(body), {
				status: 200,
				headers: { 'content-type': 'application/json' }
			});
		})
	);
	const mod = await import('../src/routes/search/+page.svelte');
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	render(mod.default as any);

	// The typed happy path: one card with the typed fields rendered.
	const input = screen.getByTestId('search-input');
	await user.type(input, 'wiring');
	await user.click(screen.getByRole('button', { name: 'Search' }));
	const card = await screen.findByTestId('search-result');
	expect(card.textContent).toContain('Lead-safe wiring note');
	expect(card.textContent).toContain('the bench wiring is lead-safe and labelled');
	expect(card.textContent).toContain('vector');
	expect(card.textContent).toContain('0.914');

	// The honest empty state.
	emptyWire = true;
	await user.type(input, '2');
	await user.click(screen.getByRole('button', { name: 'Search' }));
	await waitFor(() => expect(screen.getByRole('status').textContent).toContain('No results.'));

	// The honest error state (a 5xx wire, never a fake result).
	emptyWire = false;
	failWire = true;
	await user.click(screen.getByRole('button', { name: 'Search' }));
	await waitFor(() =>
		expect(screen.getByRole('alert').textContent).toContain('The search failed.')
	);
});
