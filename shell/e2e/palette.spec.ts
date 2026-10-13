import { expect, test } from '@playwright/test';

/**
 * The ⌘K palette, over the BUILT page: open by the in-page keyboard law
 * (NOT an OS global shortcut), filter, and land on the top result with a
 * single Enter — no arrow press first.
 *
 * This is the only coverage the palette has in a real browser. The unit test
 * mocks `$app/navigation`, so it proves the wiring but never the machine;
 * before this spec the command list had NO browser coverage at all, which is
 * how a rewrite can pass 85 unit tests and still ship a palette where Enter
 * does nothing.
 */
test('palette_opens_filters_and_lands_on_the_top_result', async ({ page }) => {
	const errors: string[] = [];
	page.on('pageerror', (e) => errors.push(e.message));

	await page.goto('/');
	await page.getByRole('link', { name: 'Overview' }).waitFor();

	const input = page.getByPlaceholder(/command or search/i);
	await expect(input).toHaveCount(0);

	await page.keyboard.press('Control+k');
	await expect(input).toBeVisible({ timeout: 5_000 });

	// Filtering narrows the list.
	await input.fill('models');
	await page.waitForTimeout(300);
	await expect(page.getByRole('option')).toHaveCount(1);

	// The top result is already active, so Enter goes straight there.
	await expect(page.locator('[data-part="item"][data-highlighted]')).toHaveCount(1);
	await page.keyboard.press('Enter');

	await page.waitForURL(/\/models/, { timeout: 10_000 });
	expect(page.url()).toContain('/models');
	expect(errors).toEqual([]);
});
