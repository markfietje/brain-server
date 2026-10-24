import { readdirSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { goto } from '$app/navigation';
import { m } from '$paraglide/messages.js';
import { shellRoot } from './fixtures';

/**
 * M3-a / M3-b — the chat-first IA's STRUCTURAL pins (plan `21_CHAT_FIRST_IA`
 * §1): the chrome earns its place by exception, and the retirement is a fact
 * about the tree.
 *
 * Three claims, three kinds:
 *
 * 1. **The chrome (amended in M3-b).** AppShell renders NO `<nav>` landmark,
 *    EXACTLY two PERMANENT header controls — the ⌘K palette button and the
 *    settings gear — and at most ONE exception control: the review badge,
 *    which must be ABSENT when the queue read is empty. The amendment is the
 *    badge's law, not its permission: without the absent-when-empty half, a
 *    permanent third button could wear the "exception" name forever.
 * 2. **The badge (M3-b).** The review badge renders ONLY while the pending
 *    queue holds rows, carries a state token and an accessible name, and
 *    navigates to `/review`. Its wire is the queue read itself
 *    (`fetchPendingQueue` — no polling, no SSE; plan 22 R4's corrected
 *    finding: the proposal path publishes NO event).
 * 3. **The retirement (M3-a).** No file under `shell/src` imports
 *    `lib/wizard`, and no dropped route directory survives. The build
 *    already refuses a wizard import (the module is gone); this pin makes
 *    the retirement VISIBLE in the gate lane — a source scan in the II-6
 *    pattern — instead of leaving it to be rediscovered by a resolver error.
 */

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));

// The badge reads the queue on mount, so AppShell now touches the wire. The
// construction-order law (chat-surface.test.ts) applies: the typed client
// holds the `fetch` it was created with, and the host import below is DYNAMIC
// — the stub must be installed at module top, before any component module
// loads. One identity-stable mock; per-test responders swap underneath it.
type Responder = (url: string) => Promise<Response>;
const emptyQueue: Responder = async () => new Response('[]', { status: 200 });
let responder: Responder = emptyQueue;
vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
	// A Request object's String() is "[object Request]" — the URL is `.url`.
	const url = input instanceof Request ? input.url : String(input);
	return responder(new URL(url).pathname);
}));

/** The kernel's `/proposals` row shape, as `parseQueueRows` validates it. */
function proposalRow(id: number): Record<string, unknown> {
	return {
		id,
		kind: 'capture',
		content: 'the bench wiring is lead-safe and labelled',
		content_digest: 'a'.repeat(64),
		source: 'vector',
		authority: 0.8,
		novelty: 0.9137,
		conflict_with: null,
		salience: 0.512,
		created_at: 1750000000,
		screen_verdict: 'clean',
		edited_at: null,
		decided_at: null,
		expires_at: 1750003600,
		warn_secs: 600,
		critical_secs: 300,
		owner: 'operator',
		qa_note: null,
		qa_score: 90,
		domain: 'global',
		title: 'Lead-safe wiring note'
	};
}

function serveQueue(rows: Record<string, unknown>[] | 'fail'): void {
	responder =
		rows === 'fail'
			? async () => new Response('no wire', { status: 599 })
			: async (path) =>
					path === '/proposals'
						? new Response(JSON.stringify(rows), { status: 200 })
						: new Response('not found', { status: 404 });
}

async function renderHost(): Promise<void> {
	responder = emptyQueue;
	const host = (await import('./AppShellHost.svelte')).default;
	render(host);
}

/** The header's control labels, the badge EXCLUDED — the permanent chrome. */
function permanentControls(): string[] {
	return [...document.querySelectorAll('header button')]
		.filter((button) => button.getAttribute('data-testid') !== 'review-badge')
		.map((button) => button.getAttribute('aria-label') ?? '');
}

test('the_app_shell_carries_no_nav_landmark_two_permanent_controls_and_at_most_one_exception_control', async () => {
	await renderHost();

	// NO nav — not an empty one, not a hidden one: none. A `<nav>` that
	// renders nothing would still be a landmark a screen reader announces.
	expect(
		document.querySelector('nav'),
		'the shell renders a <nav> landmark — chat-first IA (plan 21 §1) removed the nav bar; ' +
			'chrome appears by exception, never by default'
	).toBeNull();

	// The permanent header controls: EXACTLY TWO, and both NAMED. The review
	// badge is the ONE allowed exception control (M3-b), and it is filtered
	// out by its own testid — a badge that renamed its testid to hide here
	// would fail the badge pin's absent-when-empty half instead. A third
	// PERMANENT control reds here with its own name; an unnamed control reds
	// with the emptiness it would hand a screen reader. What the two DO is
	// command-palette.test.ts's subject — this pin is the chrome's SHAPE.
	const controls = permanentControls();
	expect(
		controls,
		`the header carries ${controls.length} permanent controls (${controls.join(', ')}) — the whole ` +
			'permanent chrome is the ⌘K palette button and the settings gear (plan 21 §1)'
	).toHaveLength(2);
	for (const label of controls) {
		expect(label.length, `a permanent header control carries no accessible name`).toBeGreaterThan(0);
	}

	// The exception half's TEETH: with the queue read empty there is NO
	// badge node at all — not a hidden one, an absent one. This is the arm
	// that keeps "exception" from decaying into "permanent".
	expect(
		document.querySelector('[data-testid="review-badge"]'),
		'the review badge rendered while the queue read is empty — chrome-by-exception means ' +
			'absent when there is nothing to review (plan 21 §1)'
	).toBeNull();
	expect(
		document.querySelectorAll('header button').length,
		'the header holds more than the two permanent controls while the queue is empty'
	).toBe(2);
});

test('the_review_badge_renders_only_while_the_queue_is_nonempty', async () => {
	// ── present: the queue read returned rows ──
	serveQueue([proposalRow(42)]);
	const host = (await import('./AppShellHost.svelte')).default;
	render(host);

	const badge = await screen.findByTestId('review-badge');
	expect(badge.getAttribute('data-queue')).toBe('nonempty');
	expect(
		badge.getAttribute('aria-label'),
		'the badge is named for a screen reader, carrying the count the read returned'
	).toBe(m['review.badge']({ count: 1 }));

	// The header's WHOLE population with the exception live: two permanent
	// + the badge, and no fourth control smuggled in beside it.
	expect(document.querySelectorAll('header button').length).toBe(3);

	// Keyboard-reachable, and it navigates to the operator's surface.
	badge.focus();
	expect(document.activeElement).toBe(badge);
	await userEvent.click(badge);
	await waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalledWith('/review'));

	cleanup();

	// ── absent: the queue read answered EMPTY (a fact, not a fault) ──
	await renderHost();
	await waitFor(() => expect(screen.getByTestId('host-content')).toBeTruthy());
	await waitFor(() => {
		expect(
			document.querySelector('[data-testid="review-badge"]'),
			'an empty queue must cost the UI nothing — no badge node'
		).toBeNull();
	});

	cleanup();

	// ── absent: the queue read itself FAILED (offline/refused) ──
	// An unreadable queue is not a non-empty queue: the badge is chrome, not
	// a fault surface, and a failed read renders NOTHING rather than a guess.
	responder = async () => new Response('no wire', { status: 599 });
	render(host);
	await waitFor(() => expect(screen.getByTestId('host-content')).toBeTruthy());
	await waitFor(() => {
		expect(
			document.querySelector('[data-testid="review-badge"]'),
			'a failed queue read must not render the badge on a guess'
		).toBeNull();
	});
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
