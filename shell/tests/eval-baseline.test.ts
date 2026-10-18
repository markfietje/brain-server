import { describe, expect, test } from 'vitest';
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { kernelRoot } from './fixtures';
import { loadEvalFixture } from '../src/lib/eval/fixture';
import {
	bootKernel,
	runBaseline,
	scoreRun,
	seedCorpus,
	unauthorizedProbe
} from '../src/lib/eval/baseline';

/**
 * I6 — the baseline run, against the PREREGISTERED tolerances.
 *
 * This suite is GATED on `BASELINE=1` and is not part of the ordinary shell
 * lane: it builds and boots a kernel, so running it on every commit would make
 * a measurement into a routine. The tolerances it asserts were committed in I1,
 * dated, with `first_run_at: null` — they were chosen before this code existed,
 * let alone before any number did.
 *
 * The committed record is `shell/eval/baseline_run.json`, and
 * `eval-preregistration.test.ts` re-checks it against the fixture digest and
 * the tolerance values WITHOUT booting anything. So the cheap lane verifies the
 * record, and this suite is what takes the measurement.
 *
 *     BASELINE=1 pnpm exec vitest run tests/eval-baseline.test.ts
 */

const PORT = Number(process.env['BASELINE_PORT'] ?? 8801);
/** Plan 08 §3's `N >= 3`. Retrieval is deterministic, so the repeats assert
 * determinism rather than averaging noise — see `baseline.ts`. */
const REPEATS = 3;
const OPERATOR_BASE = 'http://127.0.0.1:8765';
const RUN_PATH = join(kernelRoot, 'shell', 'eval', 'baseline_run.json');

interface Preregistration {
	preregistered_at: string;
	first_run_at: string | null;
	machine: { cpu: string; memory_bytes: number; macos: string; kernel_commit: string };
	dataset: { fixture: string; sha256: string; judged_queries: number; corpus_docs: number };
	tolerances: Record<string, unknown>;
}

function readPreregistration(): Preregistration {
	return JSON.parse(
		readFileSync(join(kernelRoot, 'shell', 'eval', 'baseline_preregistration.json'), 'utf8')
	) as Preregistration;
}

describe.skipIf(process.env['BASELINE'] !== '1')('the frozen-fixture baseline', () => {
	test('T6 the fixture on disk is the fixture the tolerances were written against', () => {
		const fixture = loadEvalFixture(kernelRoot);
		const prereg = readPreregistration();
		expect(fixture.sha256).toBe(prereg.dataset.sha256);
		expect(fixture.queries.length).toBe(prereg.dataset.judged_queries);
		expect(fixture.docs.length).toBe(prereg.dataset.corpus_docs);
	});

	test(
		'T1-T6: the run, the raw rows, and every gating tolerance',
		async () => {
			const fixture = loadEvalFixture(kernelRoot);
			const prereg = readPreregistration();

			// THE DISCIPLINE, asserted rather than promised. A `first_run_at ===
			// null` check only holds on a FIRST run and silently stops meaning
			// anything on the second, so the property that actually matters is
			// used instead: the tolerance block must be byte-identical to the one
			// in the commit that introduced it. Loosening a tolerance after
			// seeing the numbers changes this file and fails here — on every
			// re-run, not just the first.
			const preregCommit = spawnSync(
				'git',
				['log', '-1', '--format=%H', '--', 'shell/eval/baseline_preregistration.json'],
				{ cwd: kernelRoot, encoding: 'utf8' }
			).stdout.trim();
			expect(preregCommit).toMatch(/^[0-9a-f]{40}$/);
			const committed = JSON.parse(
				spawnSync('git', ['show', `${preregCommit}:shell/eval/baseline_preregistration.json`], {
					cwd: kernelRoot,
					encoding: 'utf8'
				}).stdout
			) as { tolerances: Preregistration['tolerances']; dataset: Preregistration['dataset'] };
			expect(
				prereg.tolerances,
				'the tolerance block must match the commit that introduced it, verbatim'
			).toEqual(committed.tolerances);
			expect(prereg.dataset.sha256).toBe(committed.dataset.sha256);

			const kernel = await bootKernel(kernelRoot, PORT);
			try {
				await seedCorpus(kernel.base, fixture.docs);
				const { rows, unmapped } = await runBaseline(kernel.base, fixture, REPEATS);

				// ---- T1: wire losslessness, compared POSITIONALLY ------------
				const divergent = rows.filter((row) => row.ids.join(',') !== row.typedIds.join(','));
				expect(
					divergent.map((row) => `${row.id}: control [${row.ids}] vs typed [${row.typedIds}]`),
					'T1: the typed wire must return the kernel\'s ORDERED ids, position for position'
				).toEqual([]);

				// An unmappable hit is recorded as -1 and never dropped, but a run
				// full of them means the mapping is broken and every recall number
				// below is meaningless — so it is a hard failure, not a warning.
				expect(unmapped, 'every hit must map back to a corpus index').toBe(0);

				// ---- T2: every response carries a decision from the enum ------
				const missing = rows.filter((row) => row.decision === '');
				expect(missing.map((row) => row.id), 'T2: no response may lack a decision').toEqual(
					[]
				);
				const offEnum = rows.filter(
					(row) => row.decision !== 'ok' && row.decision !== 'low_confidence'
				);
				expect(
					offEnum.map((row) => `${row.id}: ${row.decision}`),
					'T2: decision must be inside the published enum [ok, low_confidence]'
				).toEqual([]);

				// ---- T3: the abstention arm -----------------------------------
				// The arm that was assigned BEFORE the numbers existed, so it is
				// reported as the finding it is either way: an abstention, or a
				// corpus that cannot support the abstention eval.
				const abstentions = rows.filter(
					(row) => row.decision === 'low_confidence' && row.ids.length === 0
				);

				// ---- T5: the recorded row (recorded, NOT gated) ----------------
				const { recall5, zeroHit } = scoreRun(fixture, rows);

				// ---- T4: the refusal, observed where auth is enforced ----------
				const refusal = await unauthorizedProbe(OPERATOR_BASE);
				expect(
					refusal.refused,
					`T4: the operator kernel answered the unauthenticated probe with ${refusal.status}`
				).toBe(true);

				const record = {
					record: 'shell-eval-baseline-run',
					version: 1,
					tolerances_ref: 'shell/eval/baseline_preregistration.json',
					preregistered_at: prereg.preregistered_at,
					preregistration_commit: preregCommit,
					first_run_at: new Date().toISOString().slice(0, 10),
					machine: {
						cpu: 'Apple M1 Pro',
						memory_bytes: 17179869184,
						macos: '27.0.1',
						kernel_commit: prereg.machine.kernel_commit
					},
					dataset: {
						fixture: prereg.dataset.fixture,
						sha256: fixture.sha256,
						judged_queries: fixture.queries.length,
						corpus_docs: fixture.docs.length,
						repeats: REPEATS
					},
					tolerance_results: {
						T1_wire_lossless: {
							pass: divergent.length === 0,
							divergent: divergent.length
						},
						T2_decision_complete: {
							pass: missing.length === 0 && offEnum.length === 0,
							missing_decision: missing.length,
							off_enum: offEnum.length
						},
						T3_abstention_reachable: {
							pass: abstentions.length > 0,
							abstentions: abstentions.length,
							abstaining_queries: abstentions.map((row) => row.id),
							finding:
								abstentions.length > 0
									? 'the abstention state was exercised by the frozen fixture'
									: 'THIS CORPUS CANNOT SUPPORT THE ABSTENTION EVAL (Plan 08 2.3) and a dedicated abstention fixture is OWED. It is explicitly NOT a pass, exactly as T3 preregistered before this run existed.'
						},
						T6_dataset_and_harness_drift: {
							pass: fixture.sha256 === prereg.dataset.sha256,
							fixture_sha256: fixture.sha256,
							preregistered_sha256: prereg.dataset.sha256
						},
						T4_refusal_is_not_absence: {
							pass: refusal.refused,
							status: refusal.status,
							structurally_distinct: true
						},
						T5_recall_row: {
							gating: false,
							recall_at_5: recall5,
							zero_hit_queries: zeroHit
						}
					},
					rows: rows.map((row) => ({
						id: row.id,
						vertical: row.vertical,
						query: row.query,
						relevant: row.relevant,
						ids: row.ids,
						decision: row.decision,
						confidence: row.confidence
					}))
				};

				writeFileSync(RUN_PATH, `${JSON.stringify(record, null, '\t')}\n`, 'utf8');

				// The gating tolerances are asserted, so a run that violates one
				// is red and no record is trusted.
				expect(divergent, 'T1').toEqual([]);
				expect({ missing: missing.length, offEnum: offEnum.length }, 'T2').toEqual({
					missing: 0,
					offEnum: 0
				});
				expect(refusal.refused, 'T4').toBe(true);
				expect(fixture.sha256, 'T6').toBe(prereg.dataset.sha256);

				// T3 is reported rather than asserted, because the preregistered
				// decision for a zero is "record it and owe a fixture" — not "fail
				// the build", and not "pass". The finding is printed either way so
				// it cannot be missed.
				console.log(
					`[baseline] recall@5 = ${recall5.toFixed(4)} over ${rows.length} queries; ` +
						`abstentions = ${abstentions.length}; zero-hit = ${zeroHit.length}; ` +
						`refusal probe = ${refusal.status}`
				);
				if (abstentions.length === 0) {
					console.warn(
						'[baseline] T3 FINDING: zero abstentions. The preregistered outcome ' +
							'for this is "a dedicated abstention fixture is owed" — NOT a pass.'
					);
				}
			} finally {
				kernel.stop();
			}
		},
		600_000
	);
});
