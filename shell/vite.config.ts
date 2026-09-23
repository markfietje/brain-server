import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
	plugins: [sveltekit()],
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
