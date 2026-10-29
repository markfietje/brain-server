import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, expect, test, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import { shellRoot } from './fixtures';
import { CENSUS_ROUTES, type CensusRoute } from './route-census';
import { APP_CSS, forcedDarkCss, schemeBlocks } from './theme-css';

/**
 * III-0c — axe AA BREADTH, census-driven: one sweep per census route.
 *
 * ## Why this file exists, and what it is NOT
 *
 * Gate 5's claim ("keyboard-complete, axe AA clean") had five per-view arms —
 * renderer, chat-surface, chat-affordance-a11y, models-view (six states),
 * decision-view (four states) — and four census routes with NO arm at all.
 * This file is the BREADTH artifact: every census route, one sweep each. It
 * does not replace the per-view arms — those carry state DEPTH (decisions:
 * list/error/detail/error; the chat: every rendering kind) that a single
 * default render does not. Breadth and depth are different claims.
 *
 * M3-a (plan `21_CHAT_FIRST_IA`): the census is six routes — the chat was
 * promoted to `/`, and the wizard, `/search`, `/overview` and `/models`
 * retired, recipes and all. The home recipe waits for the CHAT render proof
 * (the composer), never a wizard render. The sidebar frame (2026-10-25)
 * added `/audit` and `/governance`, each with its own recipe and proof.
 *
 * ## The anti-vacuity law (TRAP 5)
 *
 * Every route waits for ITS OWN render proof BEFORE axe runs. A route that
 * renders nothing must FAIL, never sweep clean: `axe.run` over an empty body
 * reports `violations: []`, which is the exact shape of a green gate that
 * measured nothing. The proof tokens are per route and named in the table
 * below.
 *
 * ## The label walk (TRAP 2, the II-7g finding)
 *
 * axe-core 4.14 passes a placeholder-only input — measured directly in
 * `chat-affordance-a11y.test.ts` drill 1. So the sweep carries the label
 * association walk per route: every `input:not([type=hidden])`, `select` and
 * `textarea` (excluding `submit`/`reset`/`button`/`image` inputs) must resolve
 * an accessible name through one of exactly four mechanisms:
 *
 *   1. `label[for=id]` with non-empty text
 *   2. a wrapping `<label>` with non-empty text
 *   3. a non-empty `aria-label`
 *   4. `aria-labelledby` resolving to non-empty text
 *
 * A placeholder alone is NOT a name.
 *
 * ## The wire and the page state (TRAPs 3 and 4)
 *
 * openapi-fetch binds `fetch` ONCE, at `createClient` time, so the stubbed
 * global's IDENTITY never changes: one `fetchMock` for the whole file, and
 * each test swaps the responder UNDERNEATH it. `$app/state` is mocked once
 * with a mutable holder carrying both `params` (decisions/[id], trace) and
 * `url` — per test, mutate the holder, never re-register the mock.
 *
 * ## The axe profile (TRAP 6)
 *
 * `axe.run(document.body, { runOnly: { type: 'tag', values: ['wcag2a',
 * 'wcag2aa'] } })` and `violations` `toEqual([])` — the exact profile the
 * per-view arms use. Under jsdom `color-contrast` reports `incomplete` (no
 * layout), never a violation, so a non-empty violations list is a REAL
 * structural finding: fix the surface or record a named exception inline.
 *
 * ## The census (TRAP 1, fail-closed on growth)
 *
 * The route set is imported from `route-census.ts` — the SAME literal
 * `route-parity.test.ts` asserts against the filesystem walk. A census route
 * with no recipe fails with "no sweep arm for <route>"; an orphan recipe (a
 * route not in the census) fails the coverage test. Growth is fail-closed on
 * both artifacts.
 */

// The chat route's IPC seam, stubbed at the module boundary exactly as
// `chat-surface.test.ts` does. Nothing in this sweep SENDS a turn, so the mock
// answers only the bootstrap probe and refuses everything else loudly: an
// accidental IPC call is a finding, not a silent `undefined`.
vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string) => {
		if (command === 'kernel_token') return null;
		throw new Error(`unexpected command ${command}`);
	})
}));

/** The mutable `$app/state` holder: one mock for the file, per-test values. */
const pageState: { params: Record<string, string>; url: URL } = {
	params: {},
	url: new URL('http://127.0.0.1/')
};
vi.mock('$app/state', () => ({ page: pageState }));

/** One identity-stable fetch mock; per-test responders swap underneath it. */
type Responder = (url: string, init: RequestInit | undefined, method: string) => Promise<Response>;
const noWire: Responder = async () => new Response('no wire', { status: 599 });
let responder: Responder = noWire;
const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
	const url = input instanceof Request ? input.url : String(input);
	const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
	return responder(url, init, method);
});
vi.stubGlobal('fetch', fetchMock);

function jsonResponse(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

function serveJsonByPath(bodies: Readonly<Record<string, unknown>>): void {
	responder = async (url) => {
		const path = new URL(url).pathname;
		if (!Object.prototype.hasOwnProperty.call(bodies, path)) {
			return new Response('not found', { status: 404 });
		}
		return jsonResponse(bodies[path]);
	};
}

function fixture(name: string): Record<string, unknown> {
	const parsed: unknown = JSON.parse(
		readFileSync(join(shellRoot, 'tests', 'fixtures', name), 'utf8')
	);
	if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
		throw new Error(`${name} must be a JSON object`);
	}
	return parsed as Record<string, unknown>;
}

function decisionListBody(): Record<string, unknown> {
	return {
		rows: [
			{
				id: 42,
				run_id: 7,
				mode: 'exploratory',
				pipeline_version: '1.32.11',
				config_hash: 'a'.repeat(64),
				created_at: 1750000000,
				stage_count: 2
			}
		],
		count: 1
	};
}

/** The render proof: a token only the RENDERED route can carry. */
type Proof = { readonly kind: 'testid' | 'text'; readonly value: string };

interface RouteRecipe {
	/** Points the page state and the wire at this route's default render. */
	readonly prepare: () => void;
	readonly load: () => Promise<{ default: unknown }>;
	readonly proof: Proof;
}

/**
 * THE RECIPE MAP — one arm per census route. The keys are checked EQUAL to
 * `CENSUS_ROUTES` by the coverage test below, and each generated test fails
 * with "no sweep arm for <route>" when its key is missing.
 *
 * Typed `Partial` on purpose: the runtime failure is the fail-closed message,
 * not a TypeScript error that would stop the suite compiling. A route added to
 * the census must fail here, visibly, with its own name.
 */
const RECIPES: Partial<Record<CensusRoute, RouteRecipe>> = {
	// The PROMOTED home route: the conversation. No wire call at rest — the
	// page renders composer + empty transcript and waits; the IPC seam is
	// stubbed loud (an accidental call is a finding), exactly as the old
	// `/chat` recipe did before M3-a moved the file.
	'/': {
		prepare: () => {
			responder = async () => jsonResponse({ hits: [], decision: 'ok' });
		},
		load: () => import('../src/routes/+page.svelte'),
		proof: { kind: 'testid', value: 'chat-composer' }
	},
	'/recall': {
		prepare: () => {
			serveJsonByPath({
				'/recall': {
					hits: [
						{
							id: 9,
							title: 'StewardMem wiring note',
							snippet: 'the recall bench seeds a support-ticket doc',
							score: 0.8234,
							source: 'vector',
							conflict: false
						}
					],
					decision: 'ok',
					domains_searched: ['global'],
					trace_id: 4242
				}
			});
		},
		load: () => import('../src/routes/recall/+page.svelte'),
		proof: { kind: 'testid', value: 'recall-input' }
	},
	'/recall/[trace_id]/trace': {
		prepare: () => {
			pageState.params = { trace_id: '4242' };
			responder = async (url) => {
				const path = new URL(url).pathname;
				if (path === '/recall/4242/trace') {
					return new Response(
						readFileSync(join(shellRoot, 'tests', 'fixtures', 'recall-trace.json'), 'utf8'),
						{ status: 200, headers: { 'content-type': 'application/json' } }
					);
				}
				return new Response('not found', { status: 404 });
			};
		},
		load: () => import('../src/routes/recall/[trace_id]/trace/+page.svelte'),
		proof: { kind: 'testid', value: 'trace-view' }
	},
	'/decisions': {
		prepare: () => {
			serveJsonByPath({ '/workflow/decision-runs': decisionListBody() });
		},
		load: () => import('../src/routes/decisions/+page.svelte'),
		proof: { kind: 'testid', value: 'decision-list-table' }
	},
	'/decisions/[id]': {
		prepare: () => {
			pageState.params = { id: '42' };
			serveJsonByPath({ '/workflow/decision-runs/42': fixture('decision-run.json') });
		},
		load: () => import('../src/routes/decisions/[id]/+page.svelte'),
		proof: { kind: 'testid', value: 'decision-detail-ready' }
	},
	'/review': {
		prepare: () => {
			responder = async (url) => {
				const path = new URL(url).pathname;
				if (path === '/proposals') {
					return jsonResponse([
						{
							id: 42,
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
						}
					]);
				}
				return new Response('not found', { status: 404 });
			};
		},
		load: () => import('../src/routes/review/+page.svelte'),
		proof: { kind: 'testid', value: 'review-list-row' }
	},
	// The audit surface: one page of events and the chain verdict, both
	// served, so the sweep sees the table and the verified badge.
	'/audit': {
		prepare: () => {
			serveJsonByPath({
				'/audit': {
					events: [
						{
							id: 7,
							ts: '2026-10-25T13:41:07Z',
							kind: 'recall',
							actor: 'operator',
							target_hash: 'a91f'.padEnd(64, '0'),
							status: 'ok',
							detail_hash: 'b'.repeat(64),
							tenant_id: 'global',
							domain: 'global'
						}
					]
				},
				'/audit/verify': { ok: true, domains: { global: true } }
			});
		},
		load: () => import('../src/routes/audit/+page.svelte'),
		proof: { kind: 'testid', value: 'audit-row' }
	},
	// The governance surface: domains, roles and retention all served, so
	// every section renders its live arm.
	'/governance': {
		prepare: () => {
			serveJsonByPath({
				'/domains': {
					domains: [{ name: 'global', entries: 8745, entities: 120, relations: 48, multi_db: false }]
				},
				'/roles': {
					roles: [
						{
							name: 'operator',
							description: 'the one who runs this kernel',
							scopes: ['private', 'domain'],
							owner_filter: 'all',
							can: ['read', 'write', 'approve']
						}
					]
				},
				'/retention': { enabled: true, policy: { chunk: 30, procedure: 180 } }
			});
		},
		load: () => import('../src/routes/governance/+page.svelte'),
		proof: { kind: 'testid', value: 'governance-domain-row' }
	}
};

/** A control's identifying shape, for a failure message a reader can act on. */
function describeControl(el: HTMLElement): string {
	const testid = el.getAttribute('data-testid');
	return `${el.tagName.toLowerCase()}${el.id ? `#${el.id}` : ''}${
		testid ? `[data-testid="${testid}"]` : ''
	}`;
}

interface LabelFinding {
	readonly control: string;
	readonly reason: string;
}

/**
 * TRAP 2's walk: every text-entry control must resolve an accessible name
 * through one of the four accepted mechanisms. Placeholder alone is not a name
 * — that is the II-7g finding, measured, and it is why this exists beside axe.
 */
function labelFindings(): LabelFinding[] {
	const findings: LabelFinding[] = [];
	const labels = [...document.querySelectorAll('label[for]')];
	for (const el of document.querySelectorAll<HTMLElement>('input, select, textarea')) {
		if (el instanceof HTMLInputElement) {
			const type = el.type.toLowerCase();
			if (['hidden', 'submit', 'reset', 'button', 'image'].includes(type)) continue;
		}
		const control = describeControl(el);

		const ariaLabel = (el.getAttribute('aria-label') ?? '').trim();
		if (ariaLabel.length > 0) continue;

		const labelledBy = (el.getAttribute('aria-labelledby') ?? '').trim();
		if (labelledBy.length > 0) {
			const text = labelledBy
				.split(/\s+/)
				.map((id) => document.getElementById(id)?.textContent?.trim() ?? '')
				.filter((part) => part.length > 0)
				.join(' ')
				.trim();
			if (text.length > 0) continue;
			findings.push({
				control,
				reason: `aria-labelledby="${labelledBy}" resolves to no non-empty text`
			});
			continue;
		}

		if (el.id.length > 0) {
			const labelled = labels.find((label) => label.getAttribute('for') === el.id);
			if (labelled && (labelled.textContent ?? '').trim().length > 0) continue;
		}

		const wrapping = el.closest('label');
		if (wrapping && (wrapping.textContent ?? '').trim().length > 0) continue;

		findings.push({
			control,
			reason:
				'no label[for], no wrapping <label>, no aria-label and no resolving aria-labelledby — a placeholder is not a name'
		});
	}
	return findings;
}

afterEach(() => {
	responder = noWire;
	pageState.params = {};
	pageState.url = new URL('http://127.0.0.1/');
	vi.clearAllMocks();
});

/**
 * What the label walk actually SAW, per route — filled by the sweep below and
 * read by `the_label_walk_measured_a_real_population_not_an_empty_one`, which
 * runs after it in this file. A walk that finds no controls passes on every
 * route and proves nothing, so the population is its own pin.
 */
const measuredControls = new Map<string, number>();

for (const route of CENSUS_ROUTES) {
	test(
		`every_census_route_renders_passes_axe_aa_and_associates_every_input — ${route}`,
		// The test bound sits OUTSIDE the proof arm's 4 s so the proof's own
		// message is what a reader sees. (Vitest 4 takes options as the SECOND
		// argument; the third-argument form is removed, not deprecated.)
		{ timeout: 15000 },
		async () => {
			const recipe = RECIPES[route];
			expect(recipe, `no sweep arm for ${route}`).toBeDefined();
			if (!recipe) return;

			recipe.prepare();
			const module = await recipe.load();
			// eslint-disable-next-line @typescript-eslint/no-explicit-any
			render(module.default as any);

			// TRAP 5 — the anti-vacuity arm. The proof token is the route's OWN;
			// an axe run over a body that never rendered reports `[]` and would be
			// the exact shape of a green gate that measured nothing.
			//
			// The waitFor bound (4 s) sits INSIDE the test bound (15 s) on
			// purpose: the first cut used the default 5 s for both, and drill B
			// failed as `Test timed out` instead of with the message below —
			// the arm reddened, but the reader learned nothing about WHY. A
			// proof arm whose failure message is a stopwatch has not spoken.
			await waitFor(
				() => {
					const seen =
						recipe.proof.kind === 'testid'
							? screen.queryAllByTestId(recipe.proof.value).length > 0
							: screen.queryByText(recipe.proof.value) !== null;
					expect(
						seen,
						`${route}: the render proof "${recipe.proof.value}" never appeared — a route that renders nothing must FAIL here, never sweep clean`
					).toBe(true);
				},
				{ timeout: 4000 }
			);

			// TRAP 6 — the exact profile the five existing arms use. Do not
			// widen, narrow, or configure it.
			const { default: axe } = await import('axe-core');
			const results = await axe.run(document.body, {
				runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
			});
			const violations = results.violations.map(
				(violation) => `${violation.id} (${violation.nodes.length} node(s))`
			);
			const labels = labelFindings();
			const controls = document.querySelectorAll(
				'input:not([type=hidden]):not([type=submit]):not([type=reset]):not([type=button]):not([type=image]), select, textarea'
			).length;
			measuredControls.set(route, controls);

			expect(
				violations,
				`${route}: axe AA violations — fix the surface or record a named exception (route + id + reason)`
			).toEqual([]);
			expect(
				labels,
				`${route}: every input/select/textarea must resolve an accessible name through label[for], a wrapping label, aria-label, or aria-labelledby`
			).toEqual([]);
		}
	);
}

/**
 * ## The forced-DARK arm (M3-e, plan 24) — the same census, the dark scheme
 *
 * The light sweep above proves the census routes' a11y structure ONCE; the
 * DOM is scheme-independent (theming is CSS-only), so a second structural
 * axe pass would alone prove little. What a dark sweep CAN prove — and what
 * this arm does — is sweep with the dark scheme ACTUALLY IN EFFECT, so any
 * violation that a dark-only DOM state could introduce is caught, and the
 * sheet's dark block is proven to parse and apply:
 *
 *   1. The OS preference is FORCED to dark — `matchMedia` (undefined under
 *      jsdom) is installed and answers `(prefers-color-scheme: dark)` with
 *      `matches: true`, so any scheme-aware JS on any route takes the dark
 *      path.
 *   2. The dark block's declarations are applied UNCONDITIONALLY as one
 *      unlayered `:root` rule (`forcedDarkCss()` over the parsed CSSOM) —
 *      the media gate pried open, since jsdom never enters
 *      `prefers-color-scheme` media.
 *   3. The anti-vacuity assertion reads the computed `--bg` and requires the
 *      DARK block's value: a sweep over the light tokens is not a dark
 *      sweep, and this arm fails with its own words if that is what it did.
 *
 * The axe profile, the proof arm and the label walk are byte-identical to
 * the light sweep. The AA *numbers* for both schemes (4.5 text / 3.0 UI)
 * are carried by the WCAG arithmetic over the token pairs in
 * `theme-retune.test.ts` — under jsdom axe's `color-contrast` reports
 * `incomplete`, never a violation, and this file's header says why that is
 * the exact shape of the gate that must not be trusted for colour math.
 */
for (const route of CENSUS_ROUTES) {
	test(
		`every_census_route_passes_axe_aa_under_a_forced_dark_scheme — ${route}`,
		{ timeout: 15000 },
		async () => {
			const recipe = RECIPES[route];
			expect(recipe, `no sweep arm for ${route}`).toBeDefined();
			if (!recipe) return;

			// (1) the forced preference; restored by hand — unstubAllGlobals()
			// would also tear out the module-level fetch stub.
			const originalMatchMedia = window.matchMedia;
			window.matchMedia = ((query: string) => ({
				matches: /prefers-color-scheme:\s*dark/.test(query),
				media: query,
				onchange: null,
				addListener: () => {},
				removeListener: () => {},
				addEventListener: () => {},
				removeEventListener: () => {},
				dispatchEvent: () => false
			})) as unknown as typeof window.matchMedia;
			// (2) the dark declarations, unconditionally in effect
			const forced = document.createElement('style');
			forced.textContent = `${APP_CSS}\n${forcedDarkCss()}`;
			document.head.append(forced);

			try {
				recipe.prepare();
				const module = await recipe.load();
				// eslint-disable-next-line @typescript-eslint/no-explicit-any
				render(module.default as any);

				await waitFor(
					() => {
						const seen =
							recipe.proof.kind === 'testid'
								? screen.queryAllByTestId(recipe.proof.value).length > 0
								: screen.queryByText(recipe.proof.value) !== null;
						expect(
							seen,
							`${route} (dark): the render proof "${recipe.proof.value}" never appeared — a route that renders nothing must FAIL here, never sweep clean`
						).toBe(true);
					},
					{ timeout: 4000 }
				);

				// (3) the force took: the dark tokens are what the document computes.
				const { dark } = schemeBlocks();
				const computedBg = window
					.getComputedStyle(document.documentElement)
					.getPropertyValue('--bg')
					.trim();
				expect(
					computedBg,
					`${route} (dark): the forced-dark arm computed --bg = "${computedBg}", not the dark block's "${dark.get('--bg')}" — this sweep measured the LIGHT scheme and is not a dark arm`
				).toBe(dark.get('--bg'));

				const { default: axe } = await import('axe-core');
				const results = await axe.run(document.body, {
					runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
				});
				const violations = results.violations.map(
					(violation) => `${violation.id} (${violation.nodes.length} node(s))`
				);
				expect(
					violations,
					`${route} (dark): axe AA violations under the forced dark scheme — fix the surface or record a named exception (route + id + reason)`
				).toEqual([]);
				expect(
					labelFindings(),
					`${route} (dark): every input/select/textarea must resolve an accessible name`
				).toEqual([]);
			} finally {
				forced.remove();
				if (originalMatchMedia) {
					window.matchMedia = originalMatchMedia;
				} else {
					delete (window as { matchMedia?: unknown }).matchMedia;
				}
			}
		}
	);
}

/**
 * The label walk's own anti-vacuity arm. The floor is a TOTAL, not per route:
 * several census routes legitimately carry no text-entry control at rest (the
 * trace view, the review queue) — a per-route floor would have to invent a
 * control on surfaces that have none. The number is a MEASUREMENT, re-measured
 * whenever the census or a recipe moves.
 */
test('the_label_walk_measured_a_real_population_not_an_empty_one', () => {
	// Every census route must have RECORDED a count — including those that
	// legitimately recorded zero. A missing key means a sweep did not run, and
	// a total over fewer routes is not the measurement this floor is set on.
	expect(
		[...measuredControls.keys()].sort(),
		'every census route must record what its label walk saw, zero included — a missing key is a sweep that did not run'
	).toEqual([...CENSUS_ROUTES].sort());
	const total = [...measuredControls.values()].reduce((sum, count) => sum + count, 0);
	expect(
		total,
		`the label walk saw ${total} control(s) across ${measuredControls.size} route(s). It measured ` +
			'12 on 2026-10-19 and 11 on 2026-10-24 (M3-a: the search input and the replay-form fields ' +
			'retire with their routes; the chat composer arrives with the home route); a walk that finds ' +
			'nothing passes every route vacuously, which is the failure this floor exists for'
	).toBeGreaterThanOrEqual(11);
});

test('the_sweep_route_set_equals_the_census_route_set_in_both_directions', () => {
	const swept = Object.keys(RECIPES).sort();
	expect(
		swept,
		'the sweep and the census must be the SAME set: a census route with no recipe fails closed (growth cannot sweep clean)'
	).toEqual([...CENSUS_ROUTES].sort());
});
