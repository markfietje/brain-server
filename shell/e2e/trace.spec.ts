import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';

/**
 * The R25 trace replay e2e — the live-wire byte-parity gate over the REAL
 * kernel booted by the global setup. The bootstrap (global-setup.ts) seeded
 * one document and ran a TRACED recall (raw fetch; the typed client cannot
 * send trace:true — R24 FINDING 2), exposing the trace_id via the
 * environment. A missing id fails LOUDLY here — the gate never silently
 * skips.
 */
const traceId = Number(process.env['E2E_TRACE_ID']);
const kernelUrl = `http://127.0.0.1:${process.env['E2E_KERNEL_PORT'] ?? 8799}`;

test('trace_replay_renders_and_exports_verbatim', async ({ page }) => {
	expect(Number.isInteger(traceId) && traceId >= 0, 'E2E_TRACE_ID from the bootstrap').toBe(true);

	// The byte-parity reference: this spec's OWN raw GET of the trace —
	// the exact bytes the kernel serves for the id the view will render.
	const wireRes = await fetch(`${kernelUrl}/recall/${traceId}/trace`);
	expect(wireRes.ok, `the trace GET over the live wire (${wireRes.status})`).toBe(true);
	const wireBytes = Buffer.from(await wireRes.arrayBuffer());

	await page.goto(`/recall/${traceId}/trace`);

	// The structured parity view renders over the live wire: the decision
	// path is visible and the seeded document appears as the injected hit.
	const view = page.getByTestId('trace-view');
	await expect(view).toBeVisible();
	const decision = page.getByTestId('trace-field-decision');
	await expect(decision).toBeVisible();
	expect((await decision.textContent())!.length).toBeGreaterThan(0);
	expect(await page.getByTestId('trace-hit-row').count()).toBeGreaterThan(0);

	// The verbatim export (the Art. 22/ADMT evidence door): the downloaded
	// file's bytes are the EXACT response bytes, named per the parity law.
	const downloadPromise = page.waitForEvent('download');
	await page.getByTestId('trace-export').click();
	const download = await downloadPromise;
	expect(download.suggestedFilename()).toBe(`trace-${traceId}.json`);
	const exported = readFileSync((await download.path())!);
	expect(exported.equals(wireBytes), 'export bytes === GET response bytes').toBe(true);

	// The raw form stays alongside the structured view (both evidence
	// forms, one surface).
	await expect(page.getByTestId('trace-json')).toBeVisible();

	// The honest 404: a bogus id renders the not-found STATE — not an
	// error, not an invented empty view.
	await page.goto('/recall/999999/trace');
	await expect(page.getByTestId('trace-not-found')).toBeVisible();
});
