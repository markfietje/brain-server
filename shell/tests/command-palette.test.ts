import { expect, test, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

// The SvelteKit routing modules are mocked at the boundary: the palette's
// navigation law (goto over resolve) is what this test pins.
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));
vi.mock('$app/state', () => ({
	page: { url: new URL('http://localhost/'), params: {} }
}));

import { goto } from '$app/navigation';
import AppShellHost from './AppShellHost.svelte';

const PALETTE_PLACEHOLDER = 'Type a command or search…';

beforeEach(() => {
	vi.mocked(goto).mockClear();
});

afterEach(() => {
	vi.unstubAllGlobals();
});

/**
 * command_palette_opens_and_navigates — ⌘K opens the palette (the in-page
 * keyboard law; NO OS global shortcut), the items navigate via goto over
 * resolve, and the settings entry opens the settings panel instead of
 * navigating.
 */
test('command_palette_opens_and_navigates', async () => {
	const user = userEvent.setup();
	render(AppShellHost);

	// ⌘K opens the palette.
	await user.keyboard('{Meta>}{k}{/Meta}');
	const dialog = await screen.findByRole('dialog');
	expect(dialog).toBeTruthy();
	const input = screen.getByPlaceholderText(PALETTE_PLACEHOLDER);
	expect(input).toBeTruthy();

	// Selecting the Overview item navigates (goto over resolve) and closes
	// the palette. The nav bar ALSO says "Overview" — query inside the
	// palette dialog.
	await user.type(input, 'Overview');
	await user.click(within(dialog as HTMLElement).getByText('Overview'));
	await waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalledWith('/overview'));
	await waitFor(() =>
		expect(screen.queryByPlaceholderText(PALETTE_PLACEHOLDER)).toBeNull()
	);

	// ⌘K reopens the palette; the Settings entry opens the settings panel
	// (no navigation). The top bar's settings BUTTON shares the accessible
	// name — the palette item is the one inside the dialog.
	await user.keyboard('{Meta>}{k}{/Meta}');
	const reopened = await screen.findByRole('dialog');
	await user.click(within(reopened as HTMLElement).getByText('Settings'));
	await waitFor(() => expect(screen.getByText('Kernel origin')).toBeTruthy());
	await user.keyboard('{Escape}');
	await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
	expect(vi.mocked(goto)).toHaveBeenCalledTimes(1);
});
