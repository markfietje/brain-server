import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { kernelRoot } from './fixtures';
import { loadEvalFixture, parseEvalFixture } from '../src/lib/eval/fixture';

/**
 * Plan 08 §0 — the preregistration pin. A tolerance chosen after seeing the
 * numbers is a rationalization, not a criterion, and the ONLY defence is that
 * the criterion was committed first and cannot be loosened without editing a
 * PIN (which is a visible diff that owes a red-proof) rather than a data file.
 *
 * This is the red-first half of I1. Its red-proofs, both reverted below and in
 * the commit that landed it:
 *   - loosen T3 (delete the abstention floor) → the literal check below fires;
 *   - repoint T6 at a different fixture digest → the digest check fires.
 */

const PREREG_PATH = join(kernelRoot, 'shell', 'eval', 'baseline_preregistration.json');
const FIXTURE_PATH = join(kernelRoot, 'tests', 'fixtures', 'eval_queries.md');

interface Preregistration {
	record: string;
	version: number;
	preregistered_at: string;
	first_run_at: string | null;
	machine: Record<string, string>;
	dataset: {
		fixture: string;
		sha256: string;
		judged_queries: number;
		corpus_docs: number;
		frozen: boolean;
	};
	tolerances: Record<string, { gating: boolean; rule: string; decides_in_advance: string }>;
}

function readPreregistration(): Preregistration {
	return JSON.parse(readFileSync(PREREG_PATH, 'utf8')) as Preregistration;
}

test('the_baseline_tolerances_were_preregistered_before_the_first_run', () => {
	const prereg = readPreregistration();
	expect(prereg.record).toBe('shell-eval-baseline-preregistration');

	// A dated criterion. The ORDER is the whole discipline: the tolerance's
	// date may not be later than the run it judges, and until a run exists
	// `first_run_at` must be null — a run recorded in the same commit as the
	// tolerance is a tolerance chosen after the fact.
	expect(prereg.preregistered_at).toBe('2026-10-18');
	if (prereg.first_run_at !== null) {
		expect(prereg.first_run_at >= prereg.preregistered_at).toBe(true);
	}

	// Every tolerance must DECIDE, not merely measure: a criterion with no
	// stated consequence is the decoration Plan 08 §0 warns about.
	for (const [id, tolerance] of Object.entries(prereg.tolerances)) {
		expect(tolerance.rule.length, `${id} states no rule`).toBeGreaterThan(20);
		expect(tolerance.decides_in_advance.length, `${id} decides nothing in advance`).toBeGreaterThan(
			20
		);
	}
});

test('the_preregistered_tolerances_are_the_ones_committed', () => {
	const prereg = readPreregistration();
	const ids = Object.keys(prereg.tolerances).sort();

	// The regression lock: loosening a tolerance means editing this list, and
	// this file is a pin — its diff is the receipt that owes a red-proof.
	expect(ids).toEqual([
		'T1_wire_lossless',
		'T2_decision_complete',
		'T3_abstention_reachable',
		'T4_refusal_is_not_absence',
		'T5_recall_row',
		'T6_dataset_and_harness_drift'
	]);

	// T3 is the arm that changes a decision: zero abstentions is a finding
	// that a dedicated abstention fixture is owed, never a pass. If that
	// sentence is ever deleted, the tolerance has become decorative.
	expect(prereg.tolerances['T3_abstention_reachable']?.rule).toContain(
		'must return `decision: low_confidence`'
	);
	expect(prereg.tolerances['T3_abstention_reachable']?.decides_in_advance).toContain(
		'explicitly NOT a pass'
	);

	// T5 is recorded, not gated, and that asymmetry is deliberate: gating an
	// unmeasured retrieval number chosen after the fact is the failure §0
	// exists to prevent.
	expect(prereg.tolerances['T5_recall_row']?.gating).toBe(false);
	expect(prereg.tolerances['T5_recall_row']?.decides_in_advance).toContain('±3 points');
	for (const id of ['T1_wire_lossless', 'T2_decision_complete', 'T3_abstention_reachable', 'T4_refusal_is_not_absence', 'T6_dataset_and_harness_drift']) {
		expect(prereg.tolerances[id]?.gating, `${id} must gate`).toBe(true);
	}
});

test('the_preregistration_binds_the_frozen_dataset_it_will_judge', () => {
	const prereg = readPreregistration();
	const fixtureBytes = readFileSync(FIXTURE_PATH, 'utf8');
	const fixture = parseEvalFixture(fixtureBytes);

	// T6: the criterion names the exact bytes it judges.
	const onDisk = createHash('sha256').update(fixtureBytes, 'utf8').digest('hex');
	expect(prereg.dataset.fixture).toBe('tests/fixtures/eval_queries.md');
	expect(prereg.dataset.sha256).toBe(onDisk);
	expect(prereg.dataset.frozen).toBe(true);

	// The counts in the record are re-derived from the fixture, so the record
	// cannot claim a set size the file does not carry.
	expect(prereg.dataset.judged_queries).toBe(fixture.queries.length);
	expect(prereg.dataset.corpus_docs).toBe(fixture.docs.length);

	// The one asymmetry worth holding as a fact: this fixture carries exactly
	// ONE negation probe (tests/eval.rs carries two — `SnapSync` is absent
	// here). That is why T3's outcome had to be assigned before the run.
	expect(fixture.queries.length).toBe(106);
	expect(fixture.docs.length).toBe(25);
	expect(fixture.negationProbes.map((q) => q.query)).toEqual(['Kubernetes ingress']);
});

test('the_parsed_fixture_is_the_frozen_one_the_kernel_pins', () => {
	const fixture = loadEvalFixture(kernelRoot);
	expect(fixture.sha256).toBe(readPreregistration().dataset.sha256);
	// Every judged query except the negation probe names at least one gold doc:
	// a query that lost its gold set would otherwise score 1.0 for free.
	for (const q of fixture.queries) {
		if (q.relevant.length === 0) {
			expect(fixture.negationProbes.map((p) => p.id)).toContain(q.id);
			continue;
		}
		expect(q.relevant.length, `${q.id} has no gold doc`).toBeGreaterThan(0);
		expect(q.query.length, `${q.id} has an empty query text`).toBeGreaterThan(0);
	}
});
