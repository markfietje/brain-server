import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { kernelRoot, shellRoot } from './fixtures';

/**
 * A1 / A3 — the abstention state, and whether it can exist.
 *
 * P1's T3 found zero abstentions over 106 queries and preregistered the
 * consequence: a dedicated abstention fixture is OWED. This file is the other
 * half of that debt, and it reaches a different answer than the prompt
 * anticipated — the prompt called the fixture "a prerequisite, not a chore"
 * and it is a prerequisite that CANNOT BE MET.
 *
 * WHY THE ARITHMETIC, because "we tried and could not find one" is not a
 * finding. `/recall` abstains only on `Recommendation::ClarifyQuery` AND an
 * empty final hit list (`abstention_decision`, src/handlers/recall.rs).
 * `ClarifyQuery` is one arm of `HeuristicEstimator::decide`
 * (src/search/quality.rs), whose gates are, in order:
 *
 *     overlap < 0.2            -> IncreaseTopK      (hard)
 *     gap    < 0.023           -> RunPrf            (hard)
 *     score >= 0.85            -> RunReranker
 *     score >= 0.60            -> RunPrf
 *     score >= 0.35            -> IncreaseTopK
 *     otherwise                -> ClarifyQuery
 *
 * and `score = 0.4*overlap + 0.3*gap + 0.2*rr + 0.1*lex`. For the top row
 * `rr` is 1.0 — `reciprocal_rank` reads `provenance.vector_rank`, which the
 * kernel reports 0-BASED, so `1/(1+0) = 1`. That constant 0.2 is what makes
 * the arm unreachable, and the enumeration below is over the CLOSED space of
 * reachable rank assignments (every leg membership, ranks handed out in row
 * order), so a zero here is a property of the estimator rather than of the
 * corpus we happened to try.
 *
 * THE CONVENTION TRAP, recorded because this pin's first draft got it wrong
 * and produced 54 false `clarify_query` cases: the FUSED score is RRF over
 * 1-based ranks (`1/(60+rank)`) while `reciprocal_rank` is 0-based. Mixing
 * them makes the top row score `rr = 0.5`, which is 0.1 less and lands 54
 * assignments under the 0.35 gate that the kernel cannot produce. The live
 * corroboration below is what caught it: the model must reproduce a real
 * measurement to four decimals.
 */

const PREREG = join(shellRoot, 'eval', 'abstention_preregistration.json');

/**
 * The estimator's constants are READ FROM THE KERNEL, not transcribed.
 *
 * The first draft hard-coded them, and the drill found the consequence
 * immediately: lowering the transcribed `CLARIFY_CEILING` from 0.35 to 0.44
 * or 0.50 turned the finding red, but 0.30 left it GREEN — so a pin whose
 * constants are a private copy tracks nothing. It would keep reporting
 * "unreachable" after someone retuned the kernel in `src/config.rs`, which is
 * precisely the change that should make abstention reachable.
 *
 * So the numbers below are parsed out of `src/config.rs`'s
 * `QualityConfig::default()`, and the 0.35 ClarifyQuery ceiling is parsed out
 * of `src/search/quality.rs`'s `decide`. A pin that reads its subject's own
 * constants fails when the subject moves; one that transcribes them does not.
 */
const CONFIG_SRC = readFileSync(join(kernelRoot, 'src', 'config.rs'), 'utf8');
const QUALITY_SRC = readFileSync(join(kernelRoot, 'src', 'search', 'quality.rs'), 'utf8');

/**
 * Pulls `field: <value>,` out of `QualityConfig`'s `Default` impl.
 *
 * The slice is anchored on `impl Default for QualityConfig` and runs to the
 * NEXT `impl`, because the field DECLARATIONS above it carry types rather than
 * values and the `from_env` block below it reads the environment -- so an
 * unscoped search finds `overlap_weight: f32` first and matches nothing.
 */
function configDefault(field: string): number {
	const start = CONFIG_SRC.indexOf('impl Default for QualityConfig');
	expect(start, 'src/config.rs has no `impl Default for QualityConfig`').toBeGreaterThan(-1);
	const rest = CONFIG_SRC.slice(start);
	const end = rest.indexOf('\nimpl ', 1);
	const block = end > 0 ? rest.slice(0, end) : rest;
	// The pattern must accept an INTEGER too: `agreement_min: 2` carries no
	// decimal point, and a decimal-only pattern would fail on exactly the one
	// field the first gate is built from.
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

/** Fused RRF score — 1-BASED ranks, one term per leg the row appears in. */
function rrf(rank: Rank): number {
	let s = 0;
	if (rank.vector !== null) s += 1 / (RRF_K + rank.vector);
	if (rank.fts !== null) s += 1 / (RRF_K + rank.fts);
	return s;
}

/** src/search/quality.rs `compute_signals`. */
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

	// 0-BASED, matching `provenance.vector_rank`: the top row is rank 0, so
	// `1/(1+0) = 1`. See the convention note above.
	const reciprocalRank =
		ranks.length > 0
			? 1 /
				(1 + Math.min(...ranks.map((r) => Math.min(r.vector ?? Infinity, r.fts ?? Infinity) - 1)))
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

/**
 * Every reachable rank assignment for `n` rows. Leg membership is one of four
 * (absent / vector / fts / both) and ranks are handed out in row order, so each
 * leg's ranks are a permutation over the rows present in it — which is what
 * the fuser produces. `4^n` is therefore CLOSED for a given n, not a sample.
 */
function enumerate(n: number): Rank[][] {
	const out: Rank[][] = [];
	for (let mask = 0; mask < 4 ** n; mask++) {
		const rows: Rank[] = [];
		let vNext = 1;
		let fNext = 1;
		for (let i = 0; i < n; i++) {
			const m = Math.floor(mask / 4 ** i) % 4;
			rows.push({
				vector: m === 1 || m === 3 ? vNext++ : null,
				fts: m === 2 || m === 3 ? fNext++ : null
			});
		}
		out.push(rows);
	}
	return out;
}

const LEX_VALUES = [0, 0.25, 0.5, 0.75, 1];

function sweep(): {
	total: number;
	clearingBothGates: number;
	clarify: number;
	best: { n: number; c: Signal; ranks: Rank[]; lex: number } | null;
} {
	let total = 0;
	let clearingBothGates = 0;
	let clarify = 0;
	let best: { n: number; c: Signal; ranks: Rank[]; lex: number } | null = null;
	for (let n = 1; n <= 7; n++) {
		for (const ranks of enumerate(n)) {
			for (const lex of LEX_VALUES) {
				const c = computeSignals(ranks, lex);
				total += 1;
				if (decide(c) === 'clarify_query') clarify += 1;
				if (c.overlap >= AGREEMENT_MIN / 10 && c.gap >= GAP_THRESHOLD) {
					clearingBothGates += 1;
					if (best === null || c.score < best.c.score) best = { n, c, ranks, lex };
				}
			}
		}
	}
	return { total, clearingBothGates, clarify, best };
}

test('A1_the_abstention_state_is_unreachable_in_the_recall_pipeline', () => {
	const { total, clearingBothGates, clarify, best } = sweep();

	// Anti-vacuity: the enumeration must actually have run, and it must have
	// reached the region where the arm is even possible. A zero over an empty
	// sweep proves nothing.
	expect(total, 'the enumeration ran no cases').toBeGreaterThan(100_000);
	expect(
		clearingBothGates,
		'no assignment cleared both hard gates, so the zero below is vacuous'
	).toBeGreaterThan(1000);

	// The finding. If this ever goes red the fixture becomes buildable and the
	// preregistration's A1 consequence fires — which is why it is a pin and not
	// a comment.
	expect(
		clarify,
		`the estimator reached ClarifyQuery ${clarify} times — abstention IS reachable and the abstention fixture is OWED (see shell/eval/abstention_preregistration.json)`
	).toBe(0);

	// And the bound, so the zero is legible rather than mysterious.
	expect(best, 'no case cleared both hard gates').not.toBeNull();
	expect(
		best!.c.score,
		`closest case cleared both gates at score ${best!.c.score?.toFixed(4)}; ClarifyQuery needs < ${CLARIFY_CEILING}`
	).toBeGreaterThanOrEqual(CLARIFY_CEILING);
});

test('the_estimator_model_reproduces_a_measured_exchange', () => {
	// The bound above is worthless if the transcription is wrong, so it is
	// held to a REAL measurement taken from a live kernel: the query `passes`
	// against a five-row compaction corpus returned these signals, and the
	// model must reproduce them.
	//
	// Measured 2026-10-18 over HTTP against `target/debug/brain-server`, read
	// from `hits[0].provenance.quality_assessment`.
	const MEASURED = { overlap: 0.2, gap: 0.5082, lexical_density: 0.0, score: 0.4325 };

	// r0 in both legs at rank 1, r1 in the vector leg at rank 2, the rest absent.
	const ranks: Rank[] = [
		{ vector: 1, fts: 1 },
		{ vector: 2, fts: null },
		{ vector: null, fts: null },
		{ vector: null, fts: null },
		{ vector: null, fts: null }
	];
	const c = computeSignals(ranks, MEASURED.lexical_density);

	expect(c.overlap).toBeCloseTo(MEASURED.overlap, 3);
	expect(c.gap).toBeCloseTo(MEASURED.gap, 3);
	expect(c.lexicalDensity).toBeCloseTo(MEASURED.lexical_density, 3);
	expect(c.score).toBeCloseTo(MEASURED.score, 3);
	// And it is NOT clarify_query, which is the finding restated on the very
	// configuration that came closest.
	expect(decide(c)).not.toBe('clarify_query');
});

test('A2_no_artifact_claims_an_abstention_corpus_that_was_never_built', () => {
	const prereg = JSON.parse(readFileSync(PREREG, 'utf8')) as {
		first_run_at: string | null;
		dataset: { fixture: string; sha256: string | null; state: string; judged_queries: number };
		tolerances: Record<string, { rule: string; decides_in_advance: string }>;
	};

	// The honest state: named, dated, with no digest — because no corpus was
	// built. A digest here would assert a fixture that does not exist.
	expect(prereg.dataset.fixture).toBe('shell/eval/abstention_queries.md');
	expect(prereg.dataset.sha256, 'a digest here would claim a corpus that was never built').toBeNull();
	expect(prereg.dataset.judged_queries).toBe(0);
	expect(prereg.dataset.state).toBe('NOT BUILT');

	// The criteria were committed before any run could observe them.
	expect(prereg.first_run_at).toBeNull();

	// Every tolerance decides rather than measures.
	for (const [id, t] of Object.entries(prereg.tolerances)) {
		expect(t.rule.length, `${id} states no rule`).toBeGreaterThan(20);
		expect(t.decides_in_advance.length, `${id} decides nothing in advance`).toBeGreaterThan(20);
	}

	// And the frozen fixture is untouched: P1 pinned its SHA-256, so an
	// "abstention corpus" smuggled into that file would be caught here.
	const frozen = readFileSync(
		join(kernelRoot, 'tests', 'fixtures', 'eval_queries.md'),
		'utf8'
	);
	const baseline = JSON.parse(
		readFileSync(join(shellRoot, 'eval', 'baseline_preregistration.json'), 'utf8')
	) as { dataset: { sha256: string } };
	const { createHash } = require('node:crypto') as typeof import('node:crypto');
	expect(createHash('sha256').update(frozen, 'utf8').digest('hex')).toBe(
		baseline.dataset.sha256
	);
});