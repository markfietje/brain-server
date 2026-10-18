import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import { shellRoot } from './fixtures';
import {
	chainTransitionObserved,
	didErase,
	forgetReceipt,
	refusedForget,
	tombstonedReceipt,
	unavailableForget,
	type ChainEvidence
} from '../src/lib/chat/forget';

/**
 * II-6 — `forget`, and the receipt it may honestly claim.
 *
 * Plan 06 §4 says the DSAR certificate reference IS the receipt and that "the
 * kernel already emits one". It does not, on this route. `POST /ump/forget`
 * returns exactly `{"result": "tombstoned"}`, and a certificate belongs to
 * `POST /dsar` — a different, Admin-only, subject-addressed workflow.
 *
 * So the pin's honest reading is the prompt's `or fails` arm: the receipt
 * names the absence and does NOT synthesise a reference. These arms are what
 * stops a future edit from "helpfully" adding `dsar-${date}-${id}`.
 */

const EVIDENCE: ChainEvidence = {
	auditId: 900,
	ts: '2026-10-18T14:31:00Z',
	kind: 'ingest',
	targetHash: 'sha256:abcdef0123456789',
	status: 'ok',
	domain: 'global'
};

describe('II-6 forget', () => {
	test('forget_shows_a_receipt_and_never_invents_a_certificate', () => {
		const receipt = tombstonedReceipt('tombstoned', 41, EVIDENCE);
		// The certificate is ABSENT and SAYS WHY. A fabricated reference would
		// survive to an auditor as a lie.
		expect(receipt.dsarCertificate).toBeNull();
		expect(receipt.certificateAbsentBecause).toContain('/ump/forget');
		expect(receipt.certificateAbsentBecause).toContain('/dsar');

		const text = forgetReceipt(receipt).text;
		expect(text).toContain('forgot brain-server #41');
		expect(text).toContain('tombstoned');
		expect(text).toContain('DSAR certificate: none');
		// And nothing that LOOKS like a certificate id appears anywhere.
		expect(text).not.toMatch(/dsar-\d{4}-\d{2}-\d{2}-\d+/);
	});

	test('a_chain_transition_is_claimed_only_when_it_was_observed', () => {
		const observed = tombstonedReceipt('tombstoned', 41, EVIDENCE);
		expect(chainTransitionObserved(observed)).toBe(true);
		expect(forgetReceipt(observed).text).toContain('chain row 900');

		// The kernel confirmed the tombstone but the evidence read returned
		// nothing. A tombstone still renders; the chain claim does not.
		const unobserved = tombstonedReceipt('tombstoned', 41, null);
		// II-7c CORRECTION: this used to expect `true`, which agreed with the
		// old name-based `didErase` and so passed on the defect it was meant to
		// police. A tombstone is a FLAG — the row is still retrievable — so it is
		// not an erasure, and the result field is what says so.
		expect(didErase(unobserved)).toBe(false);
		expect(chainTransitionObserved(unobserved)).toBe(false);
		expect(forgetReceipt(unobserved).text).toContain('no chain transition is claimed');
	});

	test('didErase_reads_the_result_and_not_the_outcomes_name', () => {
		// The distinction is invisible in the outcome's NAME and total in the
		// kernel's `result`. Both cases carry outcome 'tombstoned'.
		expect(didErase(tombstonedReceipt('tombstoned', 41, null))).toBe(false);
		expect(didErase(tombstonedReceipt('erased', 41, null))).toBe(true);
		// And the receipt for a soft forget must not read as a destruction.
		const soft = forgetReceipt(tombstonedReceipt('tombstoned', 41, null));
		expect(soft.text).not.toContain('erased');
	});

	test('a_refusal_is_a_result_and_says_nothing_was_erased', () => {
		const refused = refusedForget(41, 409, 'legal_hold_active');
		expect(didErase(refused)).toBe(false);
		expect(refused.dsarCertificate).toBeNull();
		const text = forgetReceipt(refused).text;
		expect(text).toContain('forget refused');
		expect(text).toContain('nothing was erased');
		// A refusal must never read as a success.
		expect(text).not.toContain('forgot brain-server');
	});

	test('an_unavailable_kernel_is_degraded_not_empty', () => {
		const unavailable = unavailableForget(41, 0, 'the transport failed');
		expect(didErase(unavailable)).toBe(false);
		const text = forgetReceipt(unavailable).text;
		expect(text).toContain('forget unavailable');
		expect(text).toContain('degraded, not empty');
		expect(text).not.toContain('forgot brain-server');
	});

	test('no_auto_approve_path_exists_in_the_shell', () => {
		// The receipt carries `humanGated: true` and `autoApprovable: false` as
		// CONSTANTS, not parameters, so a caller cannot pass `false`. The kernel
		// removed `brain_memory_proposal_decide` in 1.29.18 and role-gates both
		// proposal verbs to humans; a shell that could approve its own proposal
		// makes that gate decorative.
		for (const outcome of [
			tombstonedReceipt('tombstoned', 1, null),
			refusedForget(1, 403, 'no'),
			unavailableForget(1, 0, 'no')
		]) {
			const receipt = forgetReceipt(outcome);
			expect(receipt.humanGated).toBe(true);
			expect(receipt.autoApprovable).toBe(false);
		}

		// THE ARM THAT HAD NO TEETH, and it is the one worth keeping.
		//
		// The first version of this pin passed against a `forgetReceipt` whose
		// `humanGated` was a PARAMETER (`forgetReceipt(outcome, humanGated =
		// true)`). Every arm above still passed: the tests all call it with one
		// argument, so the default held and the assertions were true. A caller
		// passing `false` would have got `autoApprovable: true` and a green
		// suite -- which is exactly the path the kernel's 1.29.18 removal of
		// `brain_memory_proposal_decide` exists to close.
		//
		// Asserting behaviour cannot catch it, because the behaviour under test
		// is the DEFAULT. So this arm asserts the SHAPE: the signature must not
		// accept a second parameter, and the constants must be literal types. A
		// guard on a promise has to constrain the promise, not just observe it.
		const source = readFileSync(join(shellRoot, 'src/lib/chat/forget.ts'), 'utf8');
		const signature = /export function forgetReceipt\(([^)]*)\)/.exec(source);
		expect(signature, 'forgetReceipt has no signature').not.toBeNull();
		expect(
			signature![1]!.split(',').length,
			`forgetReceipt takes ${signature![1]} — a caller can pass humanGated`
		).toBe(1);
		expect(source).toContain('readonly humanGated: true');
		expect(source).toContain('readonly autoApprovable: false');

		// And no shell module reaches for a proposal-decision verb at all.
		const chatDir = join(shellRoot, 'src/lib/chat');
		for (const file of ['forget.ts', 'turn.ts', 'provenance.ts']) {
			const code = readFileSync(join(chatDir, file), 'utf8')
				.split('\n')
				.filter((line) => {
					const trimmed = line.trim();
					return !trimmed.startsWith('//') && !trimmed.startsWith('*') && !trimmed.startsWith('/*');
				})
				.join('\n');
			expect(code, `${file} calls a proposal-decision verb`).not.toMatch(
				/proposal_decide|proposalDecide|auto_?approve/
			);
		}
	});

	test('the_certificate_claim_matches_the_published_contract', () => {
		// The claim "the kernel issues no certificate here" is only honest while
		// openapi says so. This arm reads the contract and asserts the response
		// schema carries `result` and nothing else -- so if the kernel later
		// starts emitting one, this pin goes red and the module must be taught
		// to show it rather than continuing to deny it exists.
		const spec = readFileSync(join(shellRoot, '..', 'openapi.yaml'), 'utf8');
		const start = spec.indexOf('\n  /ump/forget:');
		expect(start, 'openapi.yaml has no /ump/forget').toBeGreaterThan(-1);
		const block = spec.slice(start, spec.indexOf('\n  /ump/', start + 10));
		expect(block, '/ump/forget documents a certificate').not.toMatch(/certificate/i);
		expect(block).toContain('tombstoned');
	});
});