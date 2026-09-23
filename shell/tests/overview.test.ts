import { expect, test, vi, afterEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';

afterEach(() => {
	vi.unstubAllGlobals();
});

/**
 * overview_renders_kernel_truth — the Overview's numbers come from the
 * WIRE (/health, /version, /stats over the typed client), never from
 * assumptions. The transport is mocked at the fetch boundary and routed
 * by URL; /version is text/plain (the kernel's actual contract).
 */
test('overview_renders_kernel_truth', async () => {
	vi.stubGlobal(
		'fetch',
		vi.fn(async (input: RequestInfo | URL) => {
			// openapi-fetch passes a Request object — route by its .url.
			const url = input instanceof Request ? input.url : String(input);
			const json = (body: unknown) =>
				new Response(JSON.stringify(body), {
					status: 200,
					headers: { 'content-type': 'application/json' }
				});
			if (url.includes('/health')) {
				return json({ status: 'ok', version: '1.28.92' });
			}
			if (url.includes('/version')) {
				return new Response('1.28.92', {
					status: 200,
					headers: { 'content-type': 'text/plain' }
				});
			}
			if (url.includes('/stats')) {
				return json({
					count: 42,
					embeddings: 42,
					entities: 7,
					relationships: 11,
					model: 'test-embed-model',
					version: '1.28.92'
				});
			}
			return new Response('not found', { status: 404 });
		})
	);
	const mod = await import('../src/routes/overview/+page.svelte');
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	render(mod.default as any);

	await waitFor(() =>
		expect(screen.getByTestId('stat-health')?.textContent).toContain('ok')
	);
	expect(screen.getByTestId('stat-version')?.textContent).toContain('1.28.92');
	expect(screen.getByTestId('stat-count')?.textContent).toContain('42');
	expect(screen.getByTestId('stat-entities')?.textContent).toContain('7');
	expect(screen.getByTestId('stat-relationships')?.textContent).toContain('11');
	expect(screen.getByTestId('stat-model')?.textContent).toContain('test-embed-model');
});
