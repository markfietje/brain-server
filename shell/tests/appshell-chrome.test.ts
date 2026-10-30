import { readdirSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { goto } from '$app/navigation';
import { m } from '$paraglide/messages.js';
import { shellRoot } from './fixtures';
import { currentConversationEpoch, recordKernelFacts } from '../src/lib/shell/frame.svelte.js';
import type { SessionSummary } from '../src/lib/shell/sessions.svelte.js';

// The session store is imported DYNAMICALLY (the construction-order law,
// this file's own header): it runtime-imports the typed client, and the
// client captures `fetch` at creation — a static import would initialize it
// before the stub below is installed, and every store read would refuse.
type SessionsStore = typeof import('../src/lib/shell/sessions.svelte.js');
let sessions: SessionsStore;
async function store(): Promise<SessionsStore> {
	sessions ??= await import('../src/lib/shell/sessions.svelte.js');
	return sessions;
}

/**
 * The frame's STRUCTURAL pins — the sidebar chrome that replaced plan 21's
 * "almost no chrome" bar on 2026-10-25, shaped after the harness it
 * borrows from: a sidebar listing the surfaces beside a main column with a
 * one-line status bar. The law moved as a NAMED act, and this file is the
 * new law.
 *
 * Five claims, five kinds:
 *
 * 1. **The frame.** AppShell renders EXACTLY ONE `<nav>` landmark carrying
 *    the surface list in order — Chat, Memory, Review, Audit, Governance as
 *    links, then Settings as a dialog button — ONE `<main>`, and a status
 *    bar that reads healthy at rest. The conversation is the FIRST entry
 *    and the current one at `/`.
 * 2. **The count by exception.** The review count rides the Review entry
 *    ONLY while the queue read returned rows, carries a state token and an
 *    accessible name, and is ABSENT — no node — when the queue is empty or
 *    the read failed. Chrome-by-exception survived the redesign: an empty
 *    queue still costs the UI nothing.
 * 3. **The facts are read, never guessed.** The sidebar's integrity line
 *    says "verified" only when `/audit/verify` answered `ok: true`; a
 *    refused verify says "not verified", a false verdict says "BROKEN". The
 *    row count appears only when `/stats` answered; the kernel version only
 *    when `/health` did.
 * 4. **New chat.** The sidebar's primary act bumps the conversation epoch
 *    (the chat surface drops its transcript on it) and navigates home.
 * 5. **The retirement (M3-a).** No file under `shell/src` imports
 *    `lib/wizard`, and no dropped route directory survives.
 */

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));
vi.mock('$app/state', () => ({ page: { url: new URL('http://127.0.0.1/'), params: {} } }));

// The frame reads the wire at mount (the queue, the facts, the chain), so
// the construction-order law (chat-surface.test.ts) applies: the typed
// client holds the `fetch` it was created with, and the host import below
// is DYNAMIC — the stub must be installed at module top, before any
// component module loads. One identity-stable mock; per-test responders
// swap underneath it.
type Responder = (
	path: string,
	init?: RequestInit
) => Promise<Response>;
const json = (body: unknown, status = 200): Response =>
	new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const quiet: Responder = async (path) =>
	path === '/proposals'
		? json([])
		: path === '/chat/sessions'
			? json({ sessions: [], total: 0 })
			: new Response('not found', { status: 404 });
let responder: Responder = quiet;
vi.stubGlobal(
	'fetch',
	vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
		// A Request object's String() is "[object Request]" — the URL is `.url`.
		const url = input instanceof Request ? input.url : String(input);
		// openapi-fetch 0.17 builds a Request for EVERY call, so the method
		// lives on the REQUEST, never on `init` — reading init?.method was the
		// R141 defect class in a second site (the no-write pin was blind to
		// every write until the drill's own plant proved it).
		const method = input instanceof Request ? input.method : (init?.method ?? 'GET');
		return responder(new URL(url).pathname, { method });
	})
);

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

/** A responder serving a body per path; anything else is a 404. */
function serve(bodies: Readonly<Record<string, unknown | (() => Response)>>): void {
	responder = async (path) => {
		if (!Object.prototype.hasOwnProperty.call(bodies, path)) {
			return new Response('not found', { status: 404 });
		}
		const body = bodies[path];
		return typeof body === 'function' ? (body as () => Response)() : json(body);
	};
}

async function renderHost(): Promise<void> {
	const host = (await import('./AppShellHost.svelte')).default;
	render(host);
	await waitFor(() => expect(screen.getByTestId('host-content')).toBeTruthy());
}

/** The nav's entries in DOM order: testid → accessible name. */
function navEntries(): Array<{ testid: string; name: string; tag: string }> {
	return [...document.querySelectorAll('nav li > a, nav li > button')].map((el) => ({
		testid: el.getAttribute('data-testid') ?? '',
		name: (el.getAttribute('aria-label') ?? el.textContent ?? '').trim(),
		tag: el.tagName.toLowerCase()
	}));
}

test('the_frame_renders_one_nav_with_the_surface_list_in_order_one_main_and_a_healthy_status_bar', async () => {
	responder = quiet;
	// THE PROOF SURFACES REVEALED (the operator's ask, 2026-10-30): the
	// first GUI shows ONLY the conversation; Memory/Review/Audit/Governance
	// are the proof surfaces (proof of memory, of retrieval, of deletion),
	// revealed via the sidebar's right-click (the NATIVE context menu on
	// desktop) and persisted. This pin witnesses the FULL revealed order —
	// the persisted posture the store restores — and the default-hidden
	// posture has its own arm below.
	window.localStorage.setItem(
		'stewardmem.sidebar.surfaces',
		JSON.stringify({ memory: true, review: true, audit: true, governance: true })
	);
	await renderHost();

	// EXACTLY ONE nav landmark — not zero (the old law), not two.
	const navs = document.querySelectorAll('nav');
	expect(navs.length, 'the frame carries exactly one <nav> landmark: the sidebar\'s surface list').toBe(1);
	expect(navs[0]!.getAttribute('aria-label'), 'the nav is named for a screen reader').toBe(m['shell.nav']());

	// The surface list, in order. The conversation is FIRST; the rest are
	// LINKS with hrefs. Settings LEFT the nav (the 2026-10-30 dsh-UX round:
	// it is the app's preferences, not a surface) — it lives in the sidebar
	// FOOT below the facts (still a button, same testid, asserted after the
		// nav list) and in the native StewardMem → Settings… ⌘, item.
	const entries = navEntries();
	expect(entries.map((e) => e.testid)).toEqual([
		'nav-chat',
		'nav-memory',
		'nav-review',
		'nav-audit',
		'nav-governance'
	]);
	expect(entries.map((e) => e.tag)).toEqual(['a', 'a', 'a', 'a', 'a']);
	for (const entry of entries) {
		expect(entry.name.length, `${entry.testid} carries no accessible name`).toBeGreaterThan(0);
	}
	expect(screen.getByTestId('nav-chat').getAttribute('href')).toBe('/');
	expect(screen.getByTestId('nav-memory').getAttribute('href')).toBe('/recall');
	expect(screen.getByTestId('nav-review').getAttribute('href')).toBe('/review');
	expect(screen.getByTestId('nav-audit').getAttribute('href')).toBe('/audit');
	expect(screen.getByTestId('nav-governance').getAttribute('href')).toBe('/governance');

	// At `/` the conversation is the CURRENT entry, and nothing else is.
	const current = [...document.querySelectorAll('nav [aria-current="page"]')];
	expect(current.map((el) => el.getAttribute('data-testid'))).toEqual(['nav-chat']);

	// ONE main landmark, holding the surface.
	expect(document.querySelectorAll('main').length).toBe(1);
	expect(document.querySelector('main')?.contains(screen.getByTestId('host-content'))).toBe(true);

	// The status bar reads healthy at rest: no wire fault was classified.
	const status = screen.getByTestId('shell-status');
	expect(status.getAttribute('data-wire')).toBe('healthy');
	expect(status.textContent).toContain(m['shell.status.healthy']());

	// The brand and the ⌘K control are present. The brand is the RAIL'S
	// S-MARK now (zero chrome text — the Codex shape): the mark's presence
	// is the assertion, and the accessible brand name lives on the aside.
	expect(screen.getByTestId('shell-brand').textContent).toBe('S');
	expect(screen.getByTestId('shell-palette').getAttribute('aria-label')).toBe(m['palette.open']());
	expect(
		[...document.querySelectorAll('[aria-label="' + m['nav.settings']() + '"]')].length,
		'exactly one control carries the Settings name'
	).toBe(1);
	// The DEFAULT posture (ChatGPT-minimal first GUI) has its own FILE-level
	// pin: appshell-default-nav.test.ts — a fresh module registry is needed
	// (the visibility store reads localStorage at construction), and a
	// same-file re-render cannot give one.
});

test('the_review_count_rides_the_entry_only_while_the_queue_is_nonempty', async () => {
	// ── present: the queue read returned rows ──
	serve({ '/proposals': [proposalRow(42)] });
	await renderHost();

	const badge = await screen.findByTestId('review-badge');
	expect(badge.getAttribute('data-queue')).toBe('nonempty');
	expect(
		badge.getAttribute('aria-label'),
		'the count is named for a screen reader, carrying the count the read returned'
	).toBe(m['review.badge']({ count: 1 }));
	// It lives INSIDE the Review entry — the entry is the control, the count
	// is its decoration — so the keyboard path is the link itself.
	expect(screen.getByTestId('nav-review').contains(badge)).toBe(true);

	cleanup();

	// ── absent: the queue read answered EMPTY (a fact, not a fault) ──
	responder = quiet;
	await renderHost();
	await waitFor(() => {
		expect(
			document.querySelector('[data-testid="review-badge"]'),
			'an empty queue must cost the UI nothing — no count node'
		).toBeNull();
	});

	cleanup();

	// ── absent: the queue read itself FAILED (offline/refused) ──
	// An unreadable queue is not a non-empty queue: the count is chrome, not
	// a fault surface, and a failed read renders NOTHING rather than a guess.
	responder = async () => new Response('no wire', { status: 599 });
	await renderHost();
	await waitFor(() => {
		expect(
			document.querySelector('[data-testid="review-badge"]'),
			'a failed queue read must not render the count on a guess'
		).toBeNull();
	});
});

test('the_sidebar_facts_are_read_from_the_kernel_and_never_guessed', async () => {
	// ── everything answered: version, rows, and a verified chain ──
	serve({
		'/proposals': [],
		'/health': { status: 'ok', version: '1.29.32' },
		'/stats': { count: 8745, embeddings: 8745, entities: 1, relationships: 0, model: 'test-model', version: '1.29.32' },
		'/audit/verify': { ok: true, domains: { global: true } }
	});
	await renderHost();
	await waitFor(() => {
		expect(screen.getByTestId('shell-facts').getAttribute('data-integrity')).toBe('verified');
		expect(screen.getByTestId('shell-facts').textContent).toContain(
			m['shell.facts.rows']({ count: 8745 })
		);
	});
	const facts = screen.getByTestId('shell-facts');
	expect(facts.textContent).toContain(m['shell.facts.integrity.verified']());
	expect(screen.getByTestId('shell-kernel').textContent).toContain(
		m['shell.kernel']({ version: '1.29.32' })
	);
	expect(screen.getByTestId('shell-status-model').textContent).toContain(
		m['shell.status.model']({ model: 'test-model' })
	);

	cleanup();
	// The frame store is module-level: the next scenario must not inherit
	// this one's reading.
	recordKernelFacts(null);

	// ── the chain read REFUSED (a Read bearer): "not verified", never "verified" ──
	serve({
		'/proposals': [],
		'/health': { status: 'ok', version: '1.29.32' },
		'/stats': { count: 8745, model: 'test-model' },
		'/audit/verify': () => json({ error: 'forbidden' }, 403)
	});
	await renderHost();
	// Both reads land BEFORE the teardown: an in-flight read that resolves
	// after cleanup would write its answer into the module store and leak
	// into the next scenario.
	await waitFor(() => {
		expect(screen.getByTestId('shell-facts').getAttribute('data-integrity')).toBe('unverified');
		expect(screen.getByTestId('shell-facts').textContent).toContain(
			m['shell.facts.rows']({ count: 8745 })
		);
	});
	expect(screen.getByTestId('shell-facts').textContent).toContain(
		m['shell.facts.integrity.unverified']()
	);
	expect(screen.getByTestId('shell-facts').textContent).not.toContain(
		m['shell.facts.integrity.verified']()
	);

	cleanup();
	recordKernelFacts(null);

	// ── the chain read answered FALSE: "BROKEN", named as such ──
	serve({
		'/proposals': [],
		'/health': { status: 'ok', version: '1.29.32' },
		'/stats': () => json({ error: 'forbidden' }, 403),
		'/audit/verify': { ok: false, domains: { global: false } }
	});
	await renderHost();
	await waitFor(() => {
		expect(screen.getByTestId('shell-facts').getAttribute('data-integrity')).toBe('broken');
		// The facts read landed too: the version is on screen.
		expect(screen.getByTestId('shell-kernel').textContent).toContain('1.29.32');
	});
	expect(screen.getByTestId('shell-facts').textContent).toContain(
		m['shell.facts.integrity.broken']()
	);
	// A refused /stats leaves the count OUT — never "0 rows" over a refusal —
	// and the model line is absent, not blank.
	expect(screen.getByTestId('shell-facts').textContent).not.toMatch(/\b0 rows\b/);
	expect(screen.queryByTestId('shell-status-model')).toBeNull();
});

test('new_chat_bumps_the_conversation_epoch_and_navigates_home', async () => {
	responder = quiet;
	await renderHost();
	const before = currentConversationEpoch();
	await userEvent.click(screen.getByTestId('shell-new-chat'));
	expect(currentConversationEpoch()).toBe(before + 1);
	await waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalledWith('/'));
});

test('the_collapsed_rail_keeps_every_entry_named', async () => {
	responder = quiet;
	await renderHost();
	const shell = document.querySelector('[data-slot="app-shell"]');
	expect(shell?.getAttribute('data-threads')).toBe('shown');

	const toggle = screen.getByTestId('shell-sidebar-toggle');
	expect(toggle.getAttribute('aria-expanded')).toBe('true');
	await userEvent.click(toggle);
	expect(shell?.getAttribute('data-threads')).toBe('hidden');
	expect(toggle.getAttribute('aria-expanded')).toBe('false');

	// Collapsed hides the labels VISUALLY only: every entry still resolves an
	// accessible name, so the rail is navigable by a screen reader.
	for (const entry of navEntries()) {
		expect(entry.name.length, `${entry.testid} lost its name when collapsed`).toBeGreaterThan(0);
	}
	expect(screen.getByTestId('nav-chat').textContent?.trim()).toBe(m['nav.chat']());
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
			'census is the eight-route set, and a surviving directory is a route the walk would find'
	).toEqual([]);
});

// ── M3-c · the session region (plan 23 §2 D5) ────────────────────────────

/** A kernel `/chat/sessions` row, as openapi.yaml states it. */
function sessionRow(
	key: string,
	overrides: Partial<SessionSummary> = {}
): SessionSummary {
	return {
		session_key: key,
		label: '',
		turn_count: 2,
		created_at: 1_760_000_000,
		last_turn_at: 1_760_000_000,
		...overrides
	};
}

/** The module store is LEVEL ACROSS SCENARIOS (the M3 lesson — every
 * scenario waits for its reads to LAND before teardown), and the session
 * store is a second module store beside the frame's. Reset it by waiting
 * for a landed empty read — and clear the SESSION-LOCAL state the frame
 * persists (the sidebar's collapsed flag lives in sessionStorage and would
 * otherwise leak the rail posture into the next scenario's DOM). */
async function resetSessionStore(): Promise<void> {
	try {
		sessionStorage.clear();
	} catch {
		// private mode: nothing to clear
	}
	const s = await store();
	responder = quiet;
	await s.refresh();
	await waitFor(() => expect(s.currentReadState()).toBe('ok'));
	await waitFor(() => expect(s.currentSessions().length).toBe(0));
}

test('the_session_region_is_absent_at_zero_sessions_and_never_guesses_a_state', async () => {
	const s = await store();
	// ── zero rows (a fact, not a fault): the region renders NOTHING ──
	await resetSessionStore();
	await renderHost();
	await waitFor(() => expect(screen.getByTestId('session-region')).toBeTruthy());
	expect(
		document.querySelectorAll('[data-testid="session-row"]').length,
		'zero sessions must cost the UI nothing — no rows, no heading (chrome-by-exception)'
	).toBe(0);
	// The heading is INSIDE the guard too — an always-on guard that renders
	// only the heading is the exact near-miss this pin's message names (the
	// drill proved the rows-only assertion let it through).
	expect(
		screen.queryByText(m['chat.sessions.heading']()),
		'zero sessions must not render the History heading either'
	).toBeNull();
	cleanup();

	// ── rows: the region renders one row per session, named ──
	serve({
		'/proposals': [],
		'/chat/sessions': {
			sessions: [sessionRow('s-alpha', { label: 'Gate notes', last_turn_at: 1_760_000_000 })],
			total: 1
		}
	});
	await renderHost();
	const rows = await screen.findAllByTestId('session-row');
	expect(rows.length).toBe(1);
	expect(rows[0]!.textContent).toContain('Gate notes');
	cleanup();

	// ── the read REFUSED: the region renders nothing — a refusal is not an
	//    empty list, and the status bar (not the region) carries faults ──
	await resetSessionStore();
	serve({
		'/proposals': [],
		'/chat/sessions': () => json({ error: 'forbidden' }, 403)
	});
	await renderHost();
	await waitFor(() => expect(s.currentReadState()).toBe('refused'));
	expect(
		document.querySelectorAll('[data-testid="session-row"]').length,
		'a refused read must not render rows on a guess'
	).toBe(0);
	cleanup();

	// ── the read is OFFLINE: same posture — nothing, and the fault is named ──
	// Offline is a NETWORK failure (fetch rejects), not a status: the kernel
	// that ANSWERS 5xx has SPOKEN, and a refusal is its own state.
	await resetSessionStore();
	responder = async () => {
		throw new TypeError('the network is down');
	};
	await renderHost();
	await waitFor(() => expect(s.currentReadState()).toBe('offline'));
	expect(document.querySelectorAll('[data-testid="session-row"]').length).toBe(0);
});

test('session_rows_carry_a_relative_time_in_the_rendered_locale', async () => {
	await resetSessionStore();
	// One minute ago — every locale renders a MINUTE-granularity relative.
	const now = Math.floor(Date.now() / 1000);
	serve({
		'/proposals': [],
		'/chat/sessions': {
			sessions: [sessionRow('s-t', { label: 'fresh', last_turn_at: now - 60 })],
			total: 1
		}
	});
	await renderHost();
	const time = await screen.findByTestId('session-relative-time');
	// Locale-aware, never hand-rolled English: the text is whatever
	// Intl.RelativeTimeFormat produces for the active locale — asserted
	// against the SAME instrument, so the pin reds if the component ever
	// hardcodes a string.
	const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });
	expect(time.textContent!.trim()).toBe(rtf.format(-1, 'minute'));
});

test('the_glance_triggers_refresh_and_no_interval_exists_in_the_region', async () => {
	const s = await store();
	// STRUCTURAL: the region's module carries NO polling loop — no
	// setInterval, no setTimeout loop, no watchdog. The glance moments
	// (mount, focus, palette open, after-final) are the whole refresh law.
	const source = readFileSync(
		join(shellRoot, 'src', 'lib', 'shell', 'sessions.svelte.ts'),
		'utf8'
	);
	expect(source, 'the session store must not poll (setInterval)').not.toContain('setInterval');
	expect(source, 'the session store must not poll (setTimeout)').not.toContain('setTimeout');
	// And the frame mounts it at the glance moments: mount (the effect),
	// focus (the window handler), palette open.
	const shell = readFileSync(join(shellRoot, 'src', 'lib', 'components', 'AppShell.svelte'), 'utf8');
	expect(shell).toContain('refreshSessions()');
	expect(shell).toContain('onfocus=');

	// BEHAVIORAL: a refresh is a READ of the wire and nothing else — the
	// glance never writes (the lazy law's store half: nothing persists until
	// a turn is said).
	await resetSessionStore();
	const methods: string[] = [];
	responder = async (path) => {
		methods.push(path);
		return path === '/chat/sessions' ? json({ sessions: [], total: 0 }) : json([]);
	};
	await s.refresh();
	await waitFor(() => expect(s.currentReadState()).toBe('ok'));
	expect(methods, 'a glance refresh issues no write').toEqual(['/chat/sessions']);
});

test('new_chat_mints_a_fresh_key_and_the_store_performs_no_write', async () => {
	const s = await store();
	await resetSessionStore();
	await renderHost();
	const beforeKey = s.currentActiveKey();
	const beforeEpoch = currentConversationEpoch();
	const writes: string[] = [];
	responder = async (path, init) => {
		const method = (init?.method ?? 'GET').toUpperCase();
		if (method !== 'GET') writes.push(`${method} ${path}`);
		return path === '/proposals'
			? json([])
			: path === '/chat/sessions'
				? json({ sessions: [], total: 0 })
				: new Response('not found', { status: 404 });
	};
	await userEvent.click(screen.getByTestId('shell-new-chat'));
	expect(currentConversationEpoch()).toBe(beforeEpoch + 1);
	expect(s.currentActiveKey(), 'a fresh chat activates a minted key').not.toBe(beforeKey);
	expect(s.currentActiveKey(), 'the minted key carries the s- prefix').toMatch(/^s-[0-9a-f]{8}$/);
	// Give the queued effects a tick, then hold the law: NO write left the
	// store. (A PATCH/DELETE/POST here would be the create-route defect.)
	await new Promise((resolve) => setTimeout(resolve, 50));
	expect(writes, 'new chat must mint and clear WITHOUT a write').toEqual([]);
});

test('the_collapsed_rail_keeps_every_session_name_accessible', async () => {
	await resetSessionStore();
	serve({
		'/proposals': [],
		'/chat/sessions': {
			sessions: [sessionRow('s-rail', { label: 'rail visible', last_turn_at: 1_760_000_000 })],
			total: 1
		}
	});
	await renderHost();
	await screen.findAllByTestId('session-row');
	await userEvent.click(screen.getByTestId('shell-sidebar-toggle'));
	expect(document.querySelector('[data-slot="app-shell"]')?.getAttribute('data-threads')).toBe(
		'hidden'
	);
	// THE CODEX LAW (re-lawed 2026-10-30): hiding the threads column
	// REMOVES it (unmount) — the old R141 sr-only-while-collapsed law
	// belonged to the old rail-collapse shape. The sessions return fully
		// named when the column is shown again.
	expect(screen.queryByTestId('session-row')).toBeNull();
});

test('session_delete_confirms_then_refreshes_the_list', async () => {
	const s = await store();
	await resetSessionStore();
	serve({
		'/proposals': [],
		'/chat/sessions': {
			sessions: [sessionRow('s-doomed', { label: 'the doomed', last_turn_at: 1_760_000_000 })],
			total: 1
		}
	});
	await renderHost();
	await screen.findAllByTestId('session-row');
	// The destructive act asks first: the dialog names the conversation and
	// carries two named buttons.
	await userEvent.click(screen.getByTestId('session-delete'));
	const dialog = await screen.findByRole('dialog');
	expect(dialog.textContent).toContain('the doomed');
	let deleted = false;
	responder = async (path) => {
		if (path === '/chat/sessions/s-doomed') {
			deleted = true;
			return json({ ok: true, turns_deleted: 2 });
		}
		return path === '/proposals' ? json([]) : json({ sessions: [], total: 0 });
	};
	await userEvent.click(screen.getByTestId('session-delete-confirm'));
	await waitFor(() => expect(deleted).toBe(true));
	// The confirmed delete IS a glance: the list re-reads and the row leaves.
	await waitFor(() => expect(s.currentSessions().length).toBe(0));
	await waitFor(() => expect(s.currentTotal()).toBe(0));
});
