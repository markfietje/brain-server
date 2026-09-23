import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
	plugins: [
		sveltekit(),
		{
			// Dev-only CSP relief: app.html carries the PRODUCTION
			// Content-Security-Policy meta (the PWA posture, D6). Vite's dev
			// server needs ws: for HMR, so the meta is stripped when serving
			// and strictly present in every build — the built artifact is
			// never looser than the pin.
			name: 'csp-meta-dev-relief',
			apply: 'serve',
			transformIndexHtml(html) {
				return html.replace(
					/<meta http-equiv="Content-Security-Policy"[^>]*>/,
					'<!-- CSP meta stripped by the dev server (the build keeps it strict) -->'
				);
			}
		}
	],
	test: {
		environment: 'jsdom',
		include: ['tests/**/*.test.ts'],
		setupFiles: ['tests/setup.ts']
	},
	// Under vitest, svelte must resolve to its CLIENT (browser) entry — the
	// jsdom environment alone does not flip the package conditions, and
	// mount() refuses the server entry (the @testing-library/svelte recipe).
	resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined
});
