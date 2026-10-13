import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

// SvelteKit 3 migration: `svelte.config.js` is no longer read. Configuration
// is passed to the `sveltekit()` plugin in the Vite config, flattened one
// level — `kit.adapter` became `adapter`, and `Config` extends
// VitePluginSvelteOptions, so `preprocess` sits beside it rather than under
// `kit`. The `sveltekit()` call is now async (`Promise<Plugin[]>`), which
// Vite resolves as part of the plugin array.
//
// Settings are carried over verbatim from the deleted `svelte.config.js`.
export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit({
			// The SPA posture (D4): one static build serves the PWA AND the
			// Tauri webview. No server runtime exists — the kernel is the only
			// backend, reached over the typed wire (D5).
			adapter: adapter({
				fallback: 'index.html',
				pages: 'build',
				assets: 'build',
				precompress: false,
				strict: true
			}),
			preprocess: vitePreprocess()
		})
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