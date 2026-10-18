import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { expect, test } from 'vitest';
import { shellRoot } from './fixtures';

/**
 * II-7d — the SHELL route census, and its parity verdicts.
 *
 * ## WHY THIS FILE EXISTS AT ALL, and what it is NOT
 *
 * Plan 09 §2's gates 1 and 3 name "the route-coverage table" and "the
 * route-authz guard" as their tests. Those are KERNEL tables —
 * `OPENAPI_ROUTES` and `AUTHZ_GATES` in `src/server/router/route_guards.rs` —
 * covering every path the composed *router* registers. They are already green
 * and they say NOTHING about the shell.
 *
 * Gate 1's actual text is different: "Every user-reachable **shell** route has
 * parity with its Dioxus counterpart". That is a check against a different
 * artifact, and it did not exist. Asserting the kernel tables again would
 * produce a passing gate that has never looked at the shell — which would look
 * exactly like the work being done. So this file walks the shell's own route
 * tree, and one pin below is dedicated to proving it did.
 *
 * ## THE ANTI-VACUITY FLOOR COMES FIRST, and it is a floor
 *
 * A route table that walks the tree and finds nothing is a green light for
 * nothing — the exact failure II-1's transport pin hit when `/chat` did not
 * exist yet. So the census asserts BOTH a floor and the exact route set. The
 * floor is the load-bearing half: deleting the walk, renaming the directory, or
 * breaking the path derivation all fail here rather than passing over an empty
 * list.
 */

/**
 * The floor. Not a count of what the walk happens to find today — a LOWER
 * bound, so adding routes cannot break this and removing them cannot pass it.
 *
 * 9 is the measured census as of II-7d. It is deliberately below the exact set
 * asserted in the next test so that the floor and the enumeration are two
 * independent statements: one says "the walk found something", the other says
 * "and it is exactly these".
 */
const ROUTE_FLOOR = 9;

/** Walks `shell/src/routes` and derives each user-reachable route path.
 *
 * SvelteKit's file-based convention, which is what makes this derivable rather
 * than hand-listed: `+page.svelte` is the page component, `[param]` is a dynamic
 * segment, and the route path is the directory chain with `routes/` and the
 * `+page.svelte` filename removed. `routes/+page.svelte` is `/`, which is the
 * one case where no directory name survives.
 */
function censusShellRoutes(routesDir: string): string[] {
	const found: string[] = [];
	const walk = (dir: string): void => {
		for (const entry of readdirSync(dir, { withFileTypes: true })) {
			const path = join(dir, entry.name);
			if (entry.isDirectory()) {
				walk(path);
			} else if (entry.name === '+page.svelte') {
				found.push(routePathFor(path, routesDir));
			}
		}
	};
	walk(routesDir);
	return found.sort();
}

function routePathFor(pageFile: string, routesDir: string): string {
	// Drop the filename, then the `routes/` root.
	const withoutFile = relative(routesDir, join(pageFile, '..'));
	const segments = withoutFile.split('/').filter((s) => s.length > 0);
	return segments.length === 0 ? '/' : `/${segments.join('/')}`;
}

/** The Dioxus counterpart's real route table, read from the app itself.
 *
 * Parsed out of `client/src/main.rs`'s `#[route("…")]` attributes rather than
 * restated from Plan 09 §1's prose table, and that choice is the finding: the
 * plan's table and the tree disagree, so a table written from the plan would be
 * confidently wrong. See `the_dioxus_counterpart_table_is_read_from_the_app`.
 *
 * It takes a PATH and reads the file, not contents-to-be-parsed. The first
 * draft took the file's contents, and the first draft of the parity test passed
 * that PATH — so the regex ran over a filesystem path, matched nothing, and the
 * table came back EMPTY. Every `no_counterpart` verdict was then vacuously
 * true and the `exact` rows failed on `expected [] to include '/'`. The same
 * class of silent-empty result the shell census guards against, reached through
 * the other file, so this one now FAILS CLOSED on an empty parse rather than
 * returning a list that quietly makes half the verdicts true.
 */
function censusDioxusRoutes(clientMainPath: string): string[] {
	const source = readFileSync(clientMainPath, 'utf8');
	const found: string[] = [];
	const pattern = /#\[route\("([^"]*)"\)\]/g;
	let match: RegExpExecArray | null;
	while ((match = pattern.exec(source)) !== null) {
		found.push(match[1]!);
	}
	expect(
		found.length,
		`the Dioxus route parse of ${clientMainPath} found no \`#[route(…)]\` attributes. An empty ` +
			'parse is worse than a failing one: it makes every `no_counterpart` verdict vacuously true.'
	).toBeGreaterThan(0);
	return found.sort();
}

/**
 * A parity verdict, and the three kinds are not interchangeable.
 *
 * - `exact` — the same path exists on the Dioxus side.
 * - `renamed` — a counterpart exists at a DIFFERENT path. This is a real
 *   counterpart, not a gap: `/decisions` and Dioxus `/review` are the same
 *   surface under two names, and calling that "missing" would misdescribe the
 *   retirement work.
 * - `no_counterpart` — NO route on the Dioxus side serves this surface. There
 *   is no verdict that says "close enough", and there is no matching to invent:
 *   `nearest` records the closest Dioxus path as a FACT about the tree without
 *   promoting it to a counterpart.
 */
type ParityVerdict = 'exact' | 'renamed' | 'no_counterpart';

interface RouteParity {
	readonly route: string;
	readonly verdict: ParityVerdict;
	/** The Dioxus path that serves this surface, when one does. */
	readonly dioxus?: string;
	/** The nearest Dioxus path when there is NO counterpart. A fact about the
	 * tree, never a match — the field exists so a reader sees the near-miss
	 * without the table claiming parity on the strength of it. */
	readonly nearest?: string;
	readonly because: string;
}

/**
 * THE TABLE. One row per user-reachable shell route, every route carrying a
 * verdict, and `/chat` carrying `no_counterpart` — which is the finding Plan 09
 * §1 records as "missing on both" and which is STILL TRUE on the Dioxus side
 * even though the shell grew `/chat` in II-3.
 *
 * Two rows are `no_counterpart` where Plan 09 §1 claims a counterpart exists.
 * That is not this table being clever: `client/src/main.rs` has no `/overview`
 * and no `/models` route, and a table that reported parity for them would be
 * asserting a match that does not exist. Recorded in the register as a
 * plan-truth finding rather than quietly reconciled.
 */
const ROUTE_PARITY: readonly RouteParity[] = [
	{
		route: '/',
		verdict: 'exact',
		dioxus: '/',
		because: 'both sides serve a dashboard at the root'
	},
	{
		route: '/overview',
		verdict: 'no_counterpart',
		nearest: '/',
		because:
			'Plan 09 §1 marks overview as covered on both sides, but the Dioxus app has no /overview ' +
			'route — only a root dashboard. Promoting `/` to the counterpart would be inventing a ' +
			'match, so the verdict is that none exists.'
	},
	{
		route: '/search',
		verdict: 'renamed',
		dioxus: '/recall',
		because:
			'Plan 09 §1 records the gap itself ("folds into recall"). The Dioxus side has no /search ' +
			'route; the surface is the recall surface, so this is a rename rather than a gap.'
	},
	{
		route: '/recall',
		verdict: 'exact',
		dioxus: '/recall',
		because: 'the same path on both sides'
	},
	{
		route: '/recall/[trace_id]/trace',
		verdict: 'renamed',
		dioxus: '/recall/:trace_id',
		because:
			'the same surface at a different SHAPE: the shell nests a literal `/trace` segment under ' +
			'the id, Dioxus puts the trace at the leaf. Worth stating because a path-shape change is ' +
			'a routing decision, not a rename.'
	},
	{
		route: '/decisions',
		verdict: 'renamed',
		dioxus: '/review',
		because: 'the review queue is the decisions surface under its older name'
	},
	{
		route: '/decisions/[id]',
		verdict: 'renamed',
		dioxus: '/review/:proposal_id',
		because: 'the single-proposal view, renamed and with a different parameter name'
	},
	{
		route: '/models',
		verdict: 'no_counterpart',
		nearest: '/register',
		because:
			'Plan 09 §1 marks models as covered on both sides, but the Dioxus app has no /models ' +
			'route. `/register` registers CLIENTS and is the nearest path, not the same surface; ' +
			'recorded as a near-miss rather than promoted to a match.'
	},
	{
		route: '/chat',
		verdict: 'no_counterpart',
		because:
			'Plan 09 §1 records chat/conversation as MISSING ON BOTH SIDES. The shell grew /chat in ' +
			'II-3, so the row is now present in the shell and still absent from the Dioxus route ' +
			'table. There is no counterpart to compare against and this table does not invent one.'
	}
];

test('the_shell_route_census_finds_every_page_and_asserts_its_own_floor', () => {
	const routesDir = join(shellRoot, 'src', 'routes');

	// ---- ANTI-VACUITY FIRST: the walk found something ----
	//
	// Deliberately the FIRST assertion in the file. A census that reports an
	// empty array would satisfy every "every route has a verdict" claim below by
	// having no routes, so the floor is asserted before anything reads the list.
	const routes = censusShellRoutes(routesDir);
	expect(
		routes.length,
		'the census walked shell/src/routes and found no +page.svelte — deleting the walk, ' +
			'renaming the directory, or breaking the path derivation must FAIL here, not pass empty'
	).toBeGreaterThanOrEqual(ROUTE_FLOOR);

	// ---- AND the exact set, so the floor is not the whole claim ----
	//
	// The floor says "something was found"; this says "and it is exactly these".
	// A new route fails here until it is given a verdict, which is the moment a
	// parity question gets asked rather than skipped.
	expect(routes).toEqual([
		'/',
		'/chat',
		'/decisions',
		'/decisions/[id]',
		'/models',
		'/overview',
		'/recall',
		'/recall/[trace_id]/trace',
		'/search'
	]);
	expect(routes.length).toBe(ROUTE_FLOOR);

	// ---- THE DERIVATION IS PROVEN, not just its output ----
	//
	// A hardcoded list would pass the two assertions above. These arms pin the
	// RULES the list came from, so a route added under a new convention (a
	// group directory, a layout, a `(group)`) cannot be silently mis-pathed.
	const page = (rel: string): string => join(routesDir, rel, '+page.svelte');
	expect(existsSync(page('')), 'routes/+page.svelte must exist — it is the root route').toBe(true);
	expect(censusShellRoutes(routesDir)).toContain('/');
	// Dynamic segments keep their bracket form, which is what a reader needs to
	// see: `/decisions/[id]` is not `/decisions/id`.
	expect(routes).toContain('/decisions/[id]');
	expect(routes).not.toContain('/decisions/id');
	// A nested path keeps every segment.
	expect(routes).toContain('/recall/[trace_id]/trace');

	// Anti-vacuity on the walk itself: a directory with no `+page.svelte`
	// contributes NOTHING rather than a phantom route. `decisions/[id]` has one;
	// if a future layout-only directory appears, this is what stops it being
	// reported as a user-reachable page.
	const withLayoutOnly = censusShellRoutes(routesDir);
	expect(new Set(withLayoutOnly).size, 'the census must not emit duplicates').toBe(
		withLayoutOnly.length
	);
});

test('every_user_reachable_shell_route_carries_a_parity_verdict', () => {
	const routes = censusShellRoutes(join(shellRoot, 'src', 'routes'));
	const clientMain = join(shellRoot, '..', 'client', 'src', 'main.rs');
	const dioxus = censusDioxusRoutes(clientMain);

	// ---- EXHAUSTIVE over the census: one row per route, no route unjudged ----
	const judged = new Set(ROUTE_PARITY.map((row) => row.route));
	for (const route of routes) {
		expect(
			judged.has(route),
			`${route} has no parity verdict. Every user-reachable shell route needs one, and the ` +
				'verdict may legitimately be "no counterpart".'
		).toBe(true);
	}
	// And no row for a route that does not exist — a table that invents a route
	// is the same defect as one that misses one.
	for (const row of ROUTE_PARITY) {
		expect(
			routes,
			`the parity table names ${row.route}, which the census did not find. A row for a route ` +
				'that does not exist would make the table agree with itself.'
		).toContain(row.route);
	}
	expect(ROUTE_PARITY.length).toBe(routes.length);

	// ---- EVERY ROW'S VERDICT IS TRUE OF THE REAL DIOXUS TABLE ----
	//
	// This is the arm that makes the table a measurement rather than an essay.
	// `exact` and `renamed` both assert the named path EXISTS on the Dioxus
	// side, so a Dioxus route removed or renamed fails here and the table must
	// be corrected rather than left to rot.
	for (const row of ROUTE_PARITY) {
		switch (row.verdict) {
			case 'exact':
				expect(
					dioxus,
					`${row.route} claims EXACT parity with Dioxus ${row.dioxus}, which is not in the ` +
						'route table any more'
				).toContain(row.dioxus);
				break;
			case 'renamed':
				expect(
					dioxus,
					`${row.route} claims a counterpart at Dioxus ${row.dioxus}, which is not in the ` +
						'route table. A renamed route whose counterpart is gone is a no_counterpart.'
				).toContain(row.dioxus);
				break;
			case 'no_counterpart': {
				// The load-bearing assertion: NO Dioxus route serves this surface,
				// and the table must NOT name one as a counterpart.
				expect(
					row.dioxus,
					`${row.route} is recorded as having no counterpart but names Dioxus ${row.dioxus}. ` +
						'A no_counterpart row may carry `nearest`, never `dioxus` — a match must not be ' +
						'invented to fill the cell.'
				).toBeUndefined();
				// And `nearest`, if present, must be a REAL Dioxus path — it is a
				// claim about the tree too, just not about parity.
				if (row.nearest !== undefined) {
					expect(
						dioxus,
						`${row.route} names ${row.nearest} as its nearest Dioxus path, which is not in ` +
							'the route table'
					).toContain(row.nearest);
				}
				break;
			}
		}
		// Every verdict carries a reason. A row with an empty `because` is the
		// decoration Plan 08 §0 warns about.
		expect(
			row.because.length,
			`${row.route} carries a verdict with no reason`
		).toBeGreaterThan(20);
	}

	// ---- THE FINDING THE PROMPT NAMED: /chat HAS NO COUNTERPART ----
	//
	// Stated as its own assertion rather than left implicit in the table, because
	// it is the row a reader is most likely to assume was quietly filled in.
	const chat = ROUTE_PARITY.find((row) => row.route === '/chat');
	expect(chat, '/chat must carry a verdict — it is the shell\'s newest route').toBeDefined();
	expect(chat!.verdict).toBe('no_counterpart');
	expect(chat!.dioxus).toBeUndefined();
	expect(chat!.because).toContain('MISSING ON BOTH SIDES');
	// And it is absent from the real Dioxus table, so the verdict is a
	// measurement rather than a stance.
	expect(dioxus.some((path) => path.includes('chat') || path.includes('conversation'))).toBe(
		false
	);

	// ---- AND THE VERDICT MIX IS ITSELF A FINDING ----
	//
	// Gate 1 says "every shell route has parity with its Dioxus counterpart".
	// Three of nine do not, and two of those contradict Plan 09 §1's own table.
	// Asserting the mix means a future route that arrives `exact` by default,
	// or a table edited to make gate 1 look reachable, fails here.
	const mix = ROUTE_PARITY.reduce<Record<ParityVerdict, number>>(
		(acc, row) => ({ ...acc, [row.verdict]: (acc[row.verdict] ?? 0) + 1 }),
		{} as Record<ParityVerdict, number>
	);
	expect(mix).toEqual({ exact: 2, renamed: 4, no_counterpart: 3 });
	expect(
		mix.no_counterpart,
		'gate 1 cannot go green while any route has no counterpart — recorded, not reconciled'
	).toBe(3);
});

test('this_census_reads_the_SHELL_not_the_kernel_route_tables', () => {
	// THE WRONG-ARTIFACT PIN, and the one this whole file is at risk of being.
	//
	// `OPENAPI_ROUTES` and `AUTHZ_GATES` in `src/server/router/route_guards.rs`
	// are what Plan 09 §2's gates name, they are already green, and they cover
	// the composed SERVER router. Substituting them for this census would pass a
	// gate that has never looked at the shell — and would look exactly like the
	// work being done. So the difference is asserted, not asserted-about.
	const kernelGuards = readFileSync(
		join(shellRoot, '..', 'src', 'server', 'router', 'route_guards.rs'),
		'utf8'
	);
	const kernelRoutes = new Set(
		[...kernelGuards.matchAll(/^\s{4}"(\/[^"]*)",$/gm)].map((m) => m[1]!)
	);

	const shellRoutes = censusShellRoutes(join(shellRoot, 'src', 'routes'));

	// Every shell route the kernel does NOT serve. These are precisely the routes
	// a kernel-table census would MISS, so if someone swapped the walk for the
	// kernel table, this set would collapse and the equality below would fail.
	const shellOnly = ['/overview', '/decisions', '/decisions/[id]', '/models', '/chat'];
	for (const route of shellOnly) {
		expect(shellRoutes, `the census must find ${route}`).toContain(route);
		expect(
			[...kernelRoutes],
			`${route} is asserted to be absent from the kernel's route table; if it now appears ` +
				'there, this pin has stopped proving that the two artifacts differ'
		).not.toContain(route);
	}
	expect(shellOnly.length).toBeGreaterThan(0);

	// And the difference runs BOTH ways, so neither artifact can be a superset
	// standing in for the other: the kernel serves paths that are not user-facing
	// shell pages at all.
	const kernelOnly = [...kernelRoutes].filter((route) => !shellRoutes.includes(route));
	expect(
		kernelOnly.length,
		'the kernel route table must contain paths the shell has no page for, or the two artifacts ' +
			'are not actually different and this pin is measuring nothing'
	).toBeGreaterThan(0);

	// Finally: the census is derived from the shell's route DIRECTORY, so it is
	// asserted to be looking there and not somewhere else.
	expect(existsSync(join(shellRoot, 'src', 'routes', '+page.svelte'))).toBe(true);
	expect(existsSync(join(shellRoot, '..', 'src', 'server', 'router', 'route_guards.rs'))).toBe(true);
});

test('the_dioxus_counterpart_table_is_read_from_the_app_not_from_the_plan', () => {
	const clientMain = join(shellRoot, '..', 'client', 'src', 'main.rs');
	const dioxus = censusDioxusRoutes(clientMain);

	// Anti-vacuity: the parse found the route table. An empty parse would make
	// every `no_counterpart` verdict vacuously true — which is the failure this
	// whole file could have. (`censusDioxusRoutes` now fails closed on this
	// itself; the assertion stays because it also pins the COUNT.)
	expect(dioxus.length, 'the Dioxus route parse found fewer routes than expected').toBeGreaterThan(
		10
	);
	expect(dioxus).toContain('/');
	expect(dioxus).toContain('/recall');

	// THE PLAN-TRUTH FINDING, asserted so it cannot be quietly reconciled.
	//
	// Plan 09 §1's table marks overview, search, decisions and models as covered
	// on the Dioxus side. Against the real route table: `/overview` and `/models`
	// do not exist there at all, and `/search` and `/decisions` exist under other
	// names. Writing the parity table from the plan's prose would have produced a
	// confidently wrong file; reading it from the app is what found this.
	for (const absentInDioxus of ['/overview', '/models', '/search', '/decisions']) {
		expect(
			dioxus,
			`${absentInDioxus} is asserted absent from the Dioxus route table. Plan 09 §1 lists it as ` +
				'covered; if it now exists there, the plan-truth finding in the register must be updated.'
		).not.toContain(absentInDioxus);
	}

	// And the routes the plan DID get right, so the finding is a difference and
	// not a wholesale disagreement: the root and /recall match exactly, and the
	// decisions surface really is /review.
	expect(dioxus).toContain('/review');
	expect(dioxus).toContain('/review/:proposal_id');
	expect(dioxus).toContain('/recall/:trace_id');

	// The catch-all is present and is NOT a counterpart for anything: a route
	// that matches everything cannot be cited as a specific surface existing.
	expect(dioxus).toContain('/:..segments');
});
