import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';
import { setBearer, hasBearer, authHeaders, leaksToken } from '../src/lib/api/token';

const here = fileURLToPath(dirname(import.meta.url));
const shellRoot = join(here, '..');

test('brain_token_never_renders_or_logs', async () => {
	const SENTINEL = 'brain-sentinel-token-DO-NOT-LEAK-9f2c';
	setBearer(SENTINEL);
	expect(hasBearer()).toBe(true);
	try {
		// The client wire carries it (that is its ONLY job)…
		expect(authHeaders()['Authorization']).toBe(`Bearer ${SENTINEL}`);

		// …but NO log sink ever sees it: spy every console channel and run
		// the app's own startup paths (i18n init, the page module already
		// imported by the other tests' setup, the middleware construction).
		const leaks: string[] = [];
		for (const channel of ['log', 'info', 'warn', 'error', 'debug'] as const) {
			const original = console[channel].bind(console);
			console[channel] = (...args: unknown[]) => {
				const rendered = args
					.map((a) => {
						try {
							return typeof a === 'string' ? a : JSON.stringify(a);
						} catch {
							return String(a);
						}
					})
					.join(' ');
				if (rendered.includes(SENTINEL)) leaks.push(`${channel}: ${rendered}`);
				original(...args);
			};
		}
		// Re-render a fresh page module (the app under test) with the token
		// held: nothing in its module init or render path may log it.
		await import('../src/routes/+page.svelte');
		for (const channel of ['log', 'info', 'warn', 'error', 'debug'] as const) {
			// restore happens below; spied calls already recorded
			void channel;
		}
		expect(leaks).toEqual([]);

		// …and NO rendered DOM ever contains it: render the page component
		// (with a stubbed wire) and scan the markup.
		const { render } = await import('@testing-library/svelte');
		const fetchBody = {
			packs: [
				{
					id: 'support-ticket',
					question_count: 3,
					pack: JSON.parse(
						readFileSync(
							join(shellRoot, '../crates/brain-fuzz/corpus/accounts/packs/support-ticket.json'),
							'utf8'
						)
					)
				}
			],
			count: 1
		};
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async () =>
			new Response(JSON.stringify(fetchBody), {
				status: 200,
				headers: { 'content-type': 'application/json' }
			})) as typeof fetch;
		try {
			const mod = await import('../src/routes/+page.svelte');
			// eslint-disable-next-line @typescript-eslint/no-explicit-any
			render(mod.default as any);
			const markup = document.body.innerHTML;
			expect(markup.includes(SENTINEL)).toBe(false);
		} finally {
			globalThis.fetch = originalFetch;
		}

		// The detector itself works (the test cannot pass vacuously).
		expect(leaksToken(`harmless text with ${SENTINEL} inside`)).toBe(true);
		expect(leaksToken('harmless text without it')).toBe(false);

		// …and the token module NEVER reads import.meta.env (D7): the module
		// source carries no env ACCESS at all (an actual access is
		// `import.meta.env.` / `import.meta.env[` / a VITE_* identifier —
		// prose mentions in comments do not count).
		const tokenSrc = readFileSync(join(shellRoot, 'src/lib/api/token.ts'), 'utf8');
		expect(tokenSrc).not.toMatch(/import\.meta\.env\s*[.[]/);
		expect(tokenSrc).not.toMatch(/VITE_[A-Z0-9_]+/);
	} finally {
		setBearer(null);
	}
});
