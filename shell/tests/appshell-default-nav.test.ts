/**
 * The default sidebar posture — its own FILE because the visibility store
 * reads localStorage at construction (a fresh module registry per test file
 * is the only clean way to witness a fresh machine).
 *
 * THE ChatGPT-MINIMAL FIRST GUI (the operator's call, 2026-10-30): the
 * first screen shows ONLY the conversation. Memory / Review / Audit /
 * Governance are the PROOF surfaces (proof of memory, of retrieval, of
 * deletion) — revealed via the sidebar's right-click (the NATIVE context
 * menu on desktop, a minimal popup in the browser) and persisted. They are
 * never absent from the wire — the routes exist and the authz table still
 * gates them — only from the first GUI.
 */
import { expect, test } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';

test('a fresh machine renders only the conversation in the sidebar', async () => {
	window.localStorage.removeItem('stewardmem.sidebar.surfaces');
	const { default: AppShellHost } = await import('./AppShellHost.svelte');
	render(AppShellHost);
	await waitFor(() => expect(screen.getByTestId('host-content')).toBeTruthy());

	const testids = [...document.querySelectorAll('nav li > a')].map((el) =>
		el.getAttribute('data-testid')
	);
	expect(testids, 'the first GUI is the conversation alone').toEqual(['nav-chat']);
});
