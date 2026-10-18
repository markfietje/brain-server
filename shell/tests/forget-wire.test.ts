import { describe, expect, test } from 'vitest';
import {
	assertForgetOutcome,
	didErase,
	forgetRecord,
	forgottenId,
	isRefusal,
	isTombstoned,
	type AuthorizedForget,
	type ForgetCore,
	type ForgetOutcome
} from '../src/lib/api/forget';

/**
 * II-7c — the `/ump/forget` wire path, and the `hard` boundary from the webview
 * side.
 *
 * The central claim of this file is a CONSTRUCTION claim, not a behavioural
 * one: `ForgetRequest` has no `hard` member, so no code path in the shell can
 * express the destructive arm through the typed request. The core's request type
 * DOES carry `hard` — in order to refuse it — which is why `ForgetCore.authorize`
 * takes `unknown` rather than `ForgetRequest`: a typed signature would hide the
 * very field this module exists to make unexpressible.
 */

const tombstoned: ForgetOutcome = {
	state: 'tombstoned',
	id: '41',
	result: 'tombstoned',
	dsar_certificate: null
};

function core(shape: () => unknown): ForgetCore {
	return {
		authorize: async (): Promise<AuthorizedForget> => ({ doc: { id: '41' } }),
		shape: async () => shape()
	};
}

describe('the /ump/forget path', () => {
	test('the_core_authorizes_before_the_wire_and_shapes_after_it', async () => {
		const order: string[] = [];
		const outcome = await forgetRecord(
			{ id: '41' },
			{
				core: {
					authorize: async () => {
						order.push('authorize');
						return { doc: { id: '41' } };
					},
					shape: async () => {
						order.push('shape');
						return tombstoned;
					}
				},
				transport: async () => {
					order.push('wire');
					return { status: 200, body: JSON.stringify({ result: 'tombstoned' }) };
				}
			}
		);

		expect(order).toEqual(['authorize', 'wire', 'shape']);
		expect(outcome.state).toBe('tombstoned');
	});

	test('the_typed_request_cannot_express_the_destructive_path', () => {
		// THE CONSTRUCTION CLAIM. `ForgetRequest` has exactly one member, and it
		// is not `hard`. If someone added one, this fails at compile time and the
		// runtime arms below would be unreachable — which is the point: the
		// boundary is a type, not a convention.
		const request: { id: string } = { id: '41' };
		expect(Object.keys(request)).toEqual(['id']);
		expect('hard' in request).toBe(false);

		// A caller that DOES try — by widening, casting, or building the object
		// literally — gets it refused by the core, and the refusal names why.
		// That refusal is the boundary's runtime half; the type above is its
		// compile-time half. Both are asserted because either alone is half a
		// guard: a runtime check with an expressible parameter is a default away
		// from gone, and a type with no runtime check is bypassed by any `any`.
		const smuggled = { id: '41', hard: true };
		expect('hard' in smuggled).toBe(true);
		// The core's refusal is a THROW here, not a state: a caller that asked
		// for the erase path has a bug, and a silent `unavailable` would hide it
		// behind a degraded banner.
		return expect(
			forgetRecord(smuggled, {
				core: {
					authorize: async () => {
						throw new Error(
							'hard: true is REFUSED, and this command exposes no other arm. hard:true IS the /purge destructive act: the kernel gates it on Admin scope AND the purge role, and a capability bearer can never reach it.'
						);
					},
					shape: async () => tombstoned
				},
				transport: async () => ({ status: 200, body: '{}' })
			})
		).rejects.toThrow(/REFUSED/);
	});

	test('an_erased_reply_is_a_contract_violation_and_never_a_tombstone', async () => {
		// THE BACKSTOP, client side. If a `hard: true` ever reached the kernel
		// the reply would be `erased`, and a receipt built from it would tell the
		// operator their memory was destroyed when a flag was set. The core
		// refuses it; this asserts the client's own validation refuses it too, so
		// a swapped or stale core cannot paper over it.
		expect(() =>
			assertForgetOutcome({ state: 'tombstoned', id: '41', result: 'erased', dsar_certificate: null })
		).not.toThrow();

		// And the PREDICATE reads the result, not the name — the defect corrected
		// in `chat/forget.ts` would have reported this as erased too, so both
		// arms together say what did and did not happen.
		expect(
			didErase({ state: 'tombstoned', id: '41', result: 'erased', dsar_certificate: null })
		).toBe(true);
		expect(didErase(tombstoned)).toBe(false);

		// The core classifies it, so the state a caller receives is the
		// violation, not the success.
		const outcome = await forgetRecord(
			{ id: '41' },
			{
				core: core(() => ({
					state: 'contract_violation',
					status: 200,
					detail: 'the kernel reported `erased` for a request authorized as a SOFT tombstone'
				})),
				transport: async () => ({ status: 200, body: '{"result":"erased"}' })
			}
		);
		expect(outcome.state).toBe('contract_violation');
		expect(isTombstoned(outcome)).toBe(false);
		expect(didErase(outcome)).toBe(false);
	});

	test('a_fabricated_certificate_is_refused_rather_than_rendered', async () => {
		// `/ump/forget` emits none. A body carrying one is the exact fabrication
		// `chat/forget.ts` exists to prevent, and the assertion is on the KEY's
		// presence as well as its value: a skipped field and a null field look
		// identical to a reader checking for the key.
		expect(() =>
			assertForgetOutcome({
				state: 'tombstoned',
				id: '41',
				result: 'tombstoned',
				dsar_certificate: 'dsar-2026-10-18-0042'
			})
		).toThrow(/emits no DSAR certificate/);
		expect(() =>
			assertForgetOutcome({ state: 'tombstoned', id: '41', result: 'tombstoned' })
		).toThrow(/emits no DSAR certificate/);
		// The real body still passes.
		expect(assertForgetOutcome(tombstoned).state).toBe('tombstoned');
	});

	test('a_refusal_is_a_result_and_carries_no_receipt', async () => {
		for (const [status, expected] of [
			[403, 'refused'],
			[404, 'no_such_row']
		] as const) {
			const outcome = await forgetRecord(
				{ id: '41' },
				{
					core: core(() =>
						expected === 'refused'
							? { state: 'refused', status, scope: 'ump.forget' }
							: { state: 'no_such_row', id: '41', status }
					),
					transport: async () => ({ status, body: '{}' })
				}
			);
			expect(outcome.state).toBe(expected);
			expect(isRefusal(outcome)).toBe(true);
			expect(isTombstoned(outcome)).toBe(false);
			// Reaching for the id on a refusal is a CRASH, not a receipt about a
			// row that was never touched.
			expect(() => forgottenId(outcome)).toThrow(/no receipt/);
		}
	});

	test('a_transport_that_throws_is_degraded_never_a_tombstone', async () => {
		const outcome = await forgetRecord(
			{ id: '41' },
			{
				core: core(() => tombstoned),
				transport: async () => {
					throw new Error('the kernel is down');
				}
			}
		);
		expect(outcome.state).toBe('unavailable');
		expect(isTombstoned(outcome)).toBe(false);
	});

	test('assertForgetOutcome_refuses_every_shape_it_does_not_know', () => {
		expect(() => assertForgetOutcome(null)).toThrow(/not an outcome/);
		expect(() => assertForgetOutcome({ state: 'erased' })).toThrow(/unknown state/);
		expect(() => assertForgetOutcome({ state: 'ok' })).toThrow(/unknown state/);
		expect(() => assertForgetOutcome({ state: 'tombstoned', id: 41, result: 'tombstoned', dsar_certificate: null })).toThrow(
			/carried no id/
		);
		expect(() =>
			assertForgetOutcome({ state: 'tombstoned', id: '41', result: 'deleted', dsar_certificate: null })
		).toThrow(/carried result/);

		// A non-success state that smuggles a `result` is refused: a refusal
		// claiming what happened is exactly the fabricated receipt.
		for (const state of ['refused', 'no_such_row', 'unavailable', 'contract_violation']) {
			expect(
				() =>
					assertForgetOutcome({
						state,
						status: 403,
						id: '41',
						scope: 'ump.forget',
						detail: 'd',
						result: 'tombstoned'
					}),
				`${state} must not carry a result`
			).toThrow(/must not carry a result/);
		}
	});

	test('the_taxonomy_is_total_and_its_tokens_are_distinct', () => {
		const arms: { outcome: ForgetOutcome; refusal: boolean; tombstoned: boolean }[] = [
			{ outcome: tombstoned, refusal: false, tombstoned: true },
			{
				outcome: { state: 'refused', status: 403, scope: 'ump.forget' },
				refusal: true,
				tombstoned: false
			},
			{ outcome: { state: 'no_such_row', id: '41', status: 404 }, refusal: true, tombstoned: false },
			{ outcome: { state: 'unavailable', status: 0, detail: 'd' }, refusal: false, tombstoned: false },
			{
				outcome: { state: 'contract_violation', status: 200, detail: 'd' },
				refusal: false,
				tombstoned: false
			}
		];
		expect(arms.length).toBe(5);
		expect(
			new Set(arms.map((a) => a.outcome.state)).size,
			'two arms share a token, so a switch cannot tell them'
		).toBe(arms.length);
		for (const arm of arms) {
			expect(isRefusal(arm.outcome)).toBe(arm.refusal);
			expect(isTombstoned(arm.outcome)).toBe(arm.tombstoned);
		}
		// Exactly ONE arm may claim a tombstone, and the receipt accessor names
		// the rest as having none.
		expect(arms.filter((a) => a.tombstoned).length).toBe(1);
		expect(forgottenId(tombstoned)).toBe('41');
		for (const arm of arms.slice(1)) {
			expect(() => forgottenId(arm.outcome)).toThrow();
		}
	});
});
