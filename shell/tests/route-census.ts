/**
 * The SHELL route census — ONE home for the set every census-keyed artifact
 * reads.
 *
 * `route-parity.test.ts` (II-7d/III-0b) derived this set from the filesystem
 * walk and asserted it as a literal; `route-a11y.test.ts` (III-0c) sweeps the
 * same set with a per-route recipe. Two literals would be two censuses, and a
 * route added to one would age the other silently — so the literal MOVED here
 * and both import it.
 *
 * This is a plain module, NOT a `.test.ts`: importing a test file would
 * re-register its `test()` calls in every importer.
 *
 * `ROUTE_FLOOR` is deliberately a separate literal from `CENSUS_ROUTES`. The
 * floor says "the walk found something"; the list says "and it is exactly
 * these". Deriving one from the other would collapse two independent
 * statements into one. A route added to the tree fails the parity census's
 * exact-set assertion and the sweep's coverage assertion until both artifacts
 * are given the route — growth is fail-closed on both.
 */
export const ROUTE_FLOOR = 6;

/**
 * The measured census, re-measured by M3-a (2026-10-22, plan
 * `21_CHAT_FIRST_IA`): the chat is the app. `/chat` is PROMOTED to `/` and
 * the wizard that served `/` is dropped with its lib; `/search`, `/overview`
 * and `/models` retire as routes (a chat turn IS the recall; overview folds
 * into the chat home state; models is an ops concern behind Settings).
 * Six routes: the conversation and the surfaces a conversation cites.
 */
export const CENSUS_ROUTES = [
	'/',
	'/decisions',
	'/decisions/[id]',
	'/recall',
	'/recall/[trace_id]/trace',
	'/review'
] as const;

export type CensusRoute = (typeof CENSUS_ROUTES)[number];
