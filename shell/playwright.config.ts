import { defineConfig } from '@playwright/test';

export default defineConfig({
	testDir: 'e2e',
	timeout: 120_000,
	globalSetup: './e2e/global-setup.ts',
	use: {
		baseURL: 'http://127.0.0.1:4173',
		headless: true,
		// The harness accommodation (documented in e2e/global-setup.ts): the
		// e2e kernel runs on a DEDICATED port (never the operator's live
		// 8765), while the built page's strict CSP meta pins the default
		// origin only. The shipped artifact keeps the pin — the spec asserts
		// it against build/index.html; the TEST context crosses its own
		// origin.
		bypassCSP: true
	},
	webServer: {
		// The rebuild rides the SAME command so preview never serves a build
		// replaced underneath it: bake the DEDICATED e2e kernel origin as
		// the API base (loopback-enforced; see e2e/global-setup.ts), then
		// preview the fresh output.
		command: 'pnpm build && pnpm exec vite preview --host 127.0.0.1 --port 4173 --strictPort',
		port: 4173,
		reuseExistingServer: false,
		timeout: 120_000,
		env: { VITE_BRAIN_API_BASE: 'http://127.0.0.1:8799' }
	}
});
