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
 *
 * M3-a (plan `21_CHAT_FIRST_IA`): there is no nav bar to wait for — the
 * chrome is the brand mark, the ⌘K button and the settings gear, and `/` is
 * the conversation. The composer visible is the chrome-render proof; the
 * palette is driven to Recall, a route the six-route census kept.
 */
test('palette_opens_filters_and_lands_on_the_top_result', async ({ page }) => {
	const errors: string[] = [];
	page.on('pageerror', (e) => errors.push(e.message));

	await page.goto('/');
	// The home route renders the conversation, composer first (the chat-first
	// law) — and no nav landmark anywhere in the chrome.
	await page.getByTestId('chat-composer').waitFor();
	expect(await page.locator('nav').count()).toBe(0);

	const input = page.getByPlaceholder(/command or search/i);
	await expect(input).toHaveCount(0);

	await page.keyboard.press('Control+k');
	await expect(input).toBeVisible({ timeout: 5_000 });

	// Filtering narrows the list.
	await input.fill('recall');
	await page.waitForTimeout(300);
	await expect(page.getByRole('option')).toHaveCount(1);

	// The top result is already active, so Enter goes straight there.
	await expect(page.locator('[data-part="item"][data-highlighted]')).toHaveCount(1);
	await page.keyboard.press('Enter');

	await page.waitForURL(/\/recall/, { timeout: 10_000 });
	expect(page.url()).toContain('/recall');
	expect(errors).toEqual([]);
});
