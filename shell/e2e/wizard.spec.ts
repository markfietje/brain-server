import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';

/**
 * The e2e smoke: the BUILT shell (vite preview) against a REAL loopback
 * kernel booted by the global setup on a dedicated port — the support-
 * ticket pack renders END TO END over the live wire: the catalog read is
 * the kernel's validated data, the branch walk is client-side, and the
 * answers export is the typed evidence artifact.
 */
test('shell_renders_support_ticket_over_the_live_kernel_wire', async ({ page }) => {
	// The shipped artifact's CSP pin survives the harness's bypassCSP: the
	// BUILT page still carries the strict meta (default origin 8765 only).
	const built = readFileSync(join(process.cwd(), 'build/index.html'), 'utf8');
	expect(built).toContain('Content-Security-Policy');
	expect(built).toContain("connect-src 'self' http://127.0.0.1:8765");

	await page.goto('/');

	// The picker names the three ratified packs — served by the KERNEL, so
	// this assertion is already over the wire.
	await expect(page.getByRole('heading', { name: 'Choose a form' })).toBeVisible();
	for (const id of ['capture-pre-screen', 'support-ticket', 'tele-health']) {
		await expect(page.getByRole('button', { name: new RegExp(id) })).toBeVisible();
	}

	await page.getByRole('button', { name: /support-ticket/ }).click();

	// Q1: classification → billing.
	await expect(page.getByRole('heading', { name: 'Which class does this ticket belong to?' })).toBeVisible();
	await page.getByText('Billing', { exact: true }).click();
	await page.getByRole('button', { name: 'Continue' }).click();

	// Q2: urgency → low (branch 0) → the "already known" question.
	await expect(page.getByRole('heading', { name: 'How urgent is this ticket?' })).toBeVisible();
	await page.getByText('low', { exact: true }).click();
	await page.getByRole('button', { name: 'Continue' }).click();

	// Q3: known account → no → END.
	await expect(page.getByRole('heading', { name: 'The account is already known to support.' })).toBeVisible();
	await page.getByText('no, new account', { exact: true }).click();
	await page.getByRole('button', { name: 'Continue' }).click();

	await expect(page.getByText('All questions answered.')).toBeVisible();

	// The evidence export: one typed case, no free text, telemetry ids only.
	const download = page.waitForEvent('download');
	await page.getByTestId('export').click();
	const path = await (await download).path();
	const body = (await import('node:fs')).readFileSync(path!, 'utf8');
	const parsed = JSON.parse(body) as {
		case: {
			origin: string;
			pack: string;
			abstain: boolean;
			answers: Array<{ question: string; answer: string }>;
		};
	};
	expect(parsed.case).toMatchObject({ origin: 'wizard', pack: 'support-ticket', abstain: false });
	expect(parsed.case.answers).toEqual([
		{ question: 'q_class', answer: 'billing' },
		{ question: 'q_priority', answer: 'level 0: low' },
		{ question: 'q_known', answer: 'no, new account' }
	]);
});
