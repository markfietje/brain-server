import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';

/**
 * The WebKit NO-bypass leg (the R24 M2 Phase-0 regression guard): the BUILT
 * page must render its chrome under the STRICT injected CSP meta with
 * bypassCSP:false on WebKit — the engine class where the R23 blank-page bug
 * (inline bootstrap scripts blocked → an empty shell) actually appeared.
 * The wire call to the e2e kernel origin may be CSP-blocked here (the built
 * page pins ONLY the default 8765 origin; the harness kernel runs on the
 * dedicated E2E_KERNEL_PORT) — that accommodation cuts the other way on
 * this leg, so this spec asserts CHROME rendering + the CSP pin, never
 * wire data.
 */
test('webkit_no_bypass_renders_the_built_pwa', async ({ page }) => {
	// The shipped artifact's CSP pin is present on the bytes being served.
	const built = readFileSync(join(process.cwd(), 'build/index.html'), 'utf8');
	expect(built).toContain('Content-Security-Policy');
	expect(built).toContain("script-src 'self' 'sha256-");
	expect(built).toContain("connect-src 'self' http://127.0.0.1:8765");

	await page.goto('/');

	// The app chrome RENDERS under the strict CSP on WebKit (no blank page):
	// the app title comes from the i18n bundle inside the hashed external
	// scripts — if the inline bootstrap were blocked, nothing would render.
	await expect(page.getByRole('heading', { level: 1 })).toBeVisible({ timeout: 15_000 });
});
