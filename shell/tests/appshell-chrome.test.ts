import { readdirSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, vi } from 'vitest';
import { render } from '@testing-library/svelte';
import { shellRoot } from './fixtures';

/**
 * M3-a — the chat-first IA's STRUCTURAL pins (plan `21_CHAT_FIRST_IA` §1):
 * the chrome earns its place by exception, and the retirement is a fact
 * about the tree.
 *
 * Two claims, two kinds:
 *
 * 1. **The chrome.** AppShell renders NO `<nav>` landmark and EXACTLY two
 *    permanent header controls — the ⌘K palette button and the settings
 *    gear. The conversation is the window; every other surface is the
 *    palette, a dialog, or a URL. A permanent Review link would violate
 *    chrome-by-exception on day one (the badge lands in M3-b with the
 *    queue-count wire), so the count is TWO, and a third permanent control
 *    fails here by name.
 * 2. **The retirement.** No file under `shell/src` imports `lib/wizard`,
 *    and no dropped route directory survives. The build already refuses a
 *    wizard import (the module is gone); this pin makes the retirement
 *    VISIBLE in the gate lane — a source scan in the II-6 pattern — instead
 *    of leaving it to be rediscovered by a resolver error.
 */

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));

test('the_app_shell_carries_no_nav_landmark_and_exactly_two_permanent_controls', async () => {
	const host = (await import('./AppShellHost.svelte')).default;
	render(host);

	// NO nav — not an empty one, not a hidden one: none. A `<nav>` that
	// renders nothing would still be a landmark a screen reader announces.
	expect(
		document.querySelector('nav'),
		'the shell renders a <nav> landmark — chat-first IA (plan 21 §1) removed the nav bar; ' +
			'chrome appears by exception, never by default'
	).toBeNull();

	// The permanent header controls: EXACTLY TWO, and both NAMED. A third
	// button (a Review link, a status chip, anything) reds here with its own
	// name; an unnamed control reds with the emptiness it would hand a
	// screen reader. What the two DO is command-palette.test.ts's subject —
	// this pin is the chrome's SHAPE.
	const controls = [...document.querySelectorAll('header button')].map(
		(button) => button.getAttribute('aria-label') ?? ''
	);
	expect(
		controls,
		`the header carries ${controls.length} permanent controls (${controls.join(', ')}) — the whole ` +
			'permanent chrome is the ⌘K palette button and the settings gear (plan 21 §1)'
	).toHaveLength(2);
	for (const label of controls) {
		expect(label.length, `a permanent header control carries no accessible name`).toBeGreaterThan(0);
	}
});

test('the_shell_tree_holds_no_wizard_import_and_no_dropped_route_directory', () => {
	// ---- no `lib/wizard` import anywhere in src ----
	const offenders: string[] = [];
	const walk = (dir: string): void => {
		for (const entry of readdirSync(dir, { withFileTypes: true })) {
			const path = join(dir, entry.name);
			if (entry.isDirectory()) walk(path);
			else if (/\.(ts|svelte)$/.test(entry.name)) {
				if (/from\s+'#lib\/wizard|from\s+'\.\.\/wizard|from\s+'\.\.\/\.\.\/wizard/.test(readFileSync(path, 'utf8'))) {
					offenders.push(path);
				}
			}
		}
	};
	walk(join(shellRoot, 'src'));
	expect(
		offenders,
		`shell/src still imports the retired wizard lib (${offenders.join(', ')}) — the wizard was ` +
			'dropped in M3-a (plan 21 §2); the module is deleted, so this import cannot build either'
	).toEqual([]);

	// ---- and the dropped route directories are GONE ----
	const routesDir = join(shellRoot, 'src', 'routes');
	const dropped = ['chat', 'search', 'overview', 'models'].filter((dir) =>
		existsSync(join(routesDir, dir))
	);
	expect(
		dropped,
		`dropped route directories still present under src/routes (${dropped.join(', ')}) — the ` +
			'census is the six-route set, and a surviving directory is a route the walk would find'
	).toEqual([]);
});
