import { describe, expect, test } from 'vitest';
import {
	assertVerifyOutcome,
	isRefusal,
	isSupported,
	matchRanges,
	verifyClaim,
	type AuthorizedVerify,
	type VerifyCore,
	type VerifyOutcome
} from '../src/lib/api/verify';

/**
 * II-7c — the `/verify` wire path: authorize → wire → shape, in that order.
 *
 * The fake stands in for the Tauri IPC, never for the subject: `authorize` and
 * `shape` are the Rust core's own commands, pinned in
 * `shell/src-tauri/src/verify_tests.rs`, and what is under test here is that
 * this path does not skip either of them or re-decide what they decided.
 */

/** A core that authorizes, sends and shapes, recording the ORDER it was asked
 * to work in — which is the whole claim of this file. */
function core(order: string[], shape: (exchange: { status: number; body: string }) => unknown) {
	return {
		authorize: async (): Promise<AuthorizedVerify> => {
			order.push('authorize');
			return { doc: { chunk_id: 7, claim: 'the claim' }, claim_chars: 9 };
		},
		shape: async (exchange: { status: number; body: string }) => {
			order.push('shape');
			return shape(exchange);
		}
	} satisfies VerifyCore;
}

const supportedOutcome: VerifyOutcome = {
	state: 'supported',
	chunk_id: 7,
	decision: 'supported',
	supported: true,
	match_ranges: [[0, 9]],
	claim: 'the claim'
};

describe('the /verify path', () => {
	test('the_core_authorizes_before_the_wire_and_shapes_after_it', async () => {
		const order: string[] = [];
		const outcome = await verifyClaim(
			{ chunk_id: 7, claim: 'the claim' },
			{
				core: core(order, () => supportedOutcome),
				transport: async () => {
					order.push('wire');
					return { status: 200, body: JSON.stringify({ supported: true }) };
				}
			}
		);

		expect(order, 'the core authorizes BEFORE the wire, and classifies after it').toEqual([
			'authorize',
			'wire',
			'shape'
		]);
		expect(outcome.state).toBe('supported');

		// The negative: a path that skipped authorize could not produce a
		// classification at all, because the core is what produces one.
		const skipped: string[] = [];
		await verifyClaim(
			{ chunk_id: 7, claim: 'the claim' },
			{
				core: {
					authorize: async () => {
						skipped.push('authorize');
						return { doc: { chunk_id: 7, claim: 'the claim' }, claim_chars: 9 };
					},
					shape: async () => {
						skipped.push('shape');
						return supportedOutcome;
					}
				},
				transport: async () => {
					skipped.push('wire');
					return { status: 200, body: '{}' };
				}
			}
		);
		expect(skipped[0], 'nothing reaches the wire before authorize').toBe('authorize');
	});

	test('an_unsupported_verdict_is_an_answer_and_not_an_error', async () => {
		const outcome = await verifyClaim(
			{ chunk_id: 7, claim: 'absent from the chunk' },
			{
				core: core([], () => ({
					state: 'unsupported',
					chunk_id: 7,
					decision: 'unsupported_claim',
					supported: false,
					claim: 'absent from the chunk'
				})),
				transport: async () => ({ status: 200, body: '{}' })
			}
		);

		// The route WORKING. Not an error, and not an absence of an answer — the
		// whole product claim of `[correct]` is that this state is reachable.
		expect(outcome.state).toBe('unsupported');
		expect(isSupported(outcome)).toBe(false);
		// THE distinction the route turns on, asserted on the client side too:
		// unsupported is NOT a refusal.
		expect(isRefusal(outcome)).toBe(false);
		// And it carries NO ranges, so nothing can render "found at …" beside it.
		expect('match_ranges' in outcome).toBe(false);
		expect(() => matchRanges(outcome)).toThrow(/no support/);
	});

	test('a_refusal_and_a_missing_chunk_are_never_drawn_as_a_verdict', async () => {
		for (const [status, expected] of [
			[403, 'refused'],
			[404, 'no_such_chunk']
		] as const) {
			const outcome = await verifyClaim(
				{ chunk_id: 7, claim: 'the claim' },
				{
					core: core([], () =>
						expected === 'refused'
							? { state: 'refused', status, scope: 'verify' }
							: { state: 'no_such_chunk', chunk_id: 7, status }
					),
					transport: async () => ({ status, body: '{}' })
				}
			);

			expect(outcome.state).toBe(expected);
			expect(isRefusal(outcome)).toBe(true);
			expect(isSupported(outcome)).toBe(false);
			// The confidentiality law, on the accessor a renderer uses: reaching
			// for the ranges on a refusal is a CRASH, not an empty list. An empty
			// list means "the kernel looked and found nothing", which is a lie
			// about a chunk the reader may not read.
			expect(() => matchRanges(outcome)).toThrow(/not an unsupported claim/);
		}
	});

	test('a_transport_that_throws_is_unreachable_never_unsupported', async () => {
		// This is the arm that would catch the worst plausible bug: a
		// `catch` that returned an unsupported verdict instead of a degraded
		// state, which would tell the reader their citation was checked when the
		// kernel was never reached.
		const outcome = await verifyClaim(
			{ chunk_id: 7, claim: 'the claim' },
			{
				core: core([], () => supportedOutcome),
				transport: async () => {
					throw new Error('the kernel is down');
				}
			}
		);
		expect(outcome.state).toBe('unreachable');
		expect(outcome).toMatchObject({ status: 0 });
		expect(isRefusal(outcome)).toBe(false);
		expect(isSupported(outcome)).toBe(false);
	});

	test('assertVerifyOutcome_refuses_every_shape_it_does_not_know', () => {
		// No fallback anywhere: a defaulted unknown state would be a claim
		// about whether the kernel's text supports a sentence.
		expect(() => assertVerifyOutcome(null)).toThrow(/not an outcome/);
		expect(() => assertVerifyOutcome('supported')).toThrow(/not an outcome/);
		expect(() => assertVerifyOutcome({ state: 'answered' })).toThrow(/unknown state/);
		expect(() => assertVerifyOutcome({ state: 'absent', chunk_id: 7 })).toThrow(/unknown state/);

		// A verdict whose `decision` and `state` disagree is a contradiction.
		expect(() =>
			assertVerifyOutcome({
				state: 'supported',
				chunk_id: 7,
				decision: 'unsupported_claim',
				supported: true,
				match_ranges: [[0, 1]]
			})
		).toThrow(/carried decision/);

		// A supported verdict with NO ranges is the body a surface would render
		// as "found at …", so it is refused rather than rendered.
		expect(() =>
			assertVerifyOutcome({
				state: 'supported',
				chunk_id: 7,
				decision: 'supported',
				supported: true,
				claim: 'k'
			})
		).toThrow(/no match ranges/);

		// An unsupported verdict CARRYING ranges claims support it does not have.
		expect(() =>
			assertVerifyOutcome({
				state: 'unsupported',
				chunk_id: 7,
				decision: 'unsupported_claim',
				supported: false,
				match_ranges: [[0, 1]],
				claim: 'k'
			})
		).toThrow(/must carry no match ranges/);

		// A failure state that smuggles a verdict is refused: this is the leak
		// closed at the type level.
		for (const state of ['refused', 'no_such_chunk', 'unreachable', 'contract_violation']) {
			expect(
				() =>
					assertVerifyOutcome({
						state,
						status: 403,
						detail: 'd',
						chunk_id: 7,
						scope: 'verify',
						supported: false
					}),
				`${state} must not carry a verdict`
			).toThrow(/must not carry a verdict|must not carry/);
		}

		// Anti-vacuity: the real states still pass.
		expect(assertVerifyOutcome(supportedOutcome).state).toBe('supported');
		expect(
			assertVerifyOutcome({
				state: 'unsupported',
				chunk_id: 7,
				decision: 'unsupported_claim',
				supported: false,
				claim: 'k'
			}).state
		).toBe('unsupported');
	});

	test('the_taxonomy_is_total_and_its_tokens_are_distinct', () => {
		// The wrong-artifact guard: this table is the CLIENT's, over states the
		// client can receive. Six arms, six distinct tokens, and each one's
		// `isRefusal`/`isSupported` answers stated — because a taxonomy with two
		// arms that answer identically is a taxonomy that has collapsed already.
		const arms: {
			state: VerifyOutcome['state'];
			outcome: VerifyOutcome;
			refusal: boolean;
			supported: boolean;
		}[] = [
			{ state: 'supported', outcome: supportedOutcome, refusal: false, supported: true },
			{
				state: 'unsupported',
				outcome: {
					state: 'unsupported',
					chunk_id: 7,
					decision: 'unsupported_claim',
					supported: false,
					claim: 'k'
				},
				refusal: false,
				supported: false
			},
			{
				state: 'refused',
				outcome: { state: 'refused', status: 403, scope: 'verify' },
				refusal: true,
				supported: false
			},
			{
				state: 'no_such_chunk',
				outcome: { state: 'no_such_chunk', chunk_id: 7, status: 404 },
				refusal: true,
				supported: false
			},
			{
				state: 'unreachable',
				outcome: { state: 'unreachable', status: 0, detail: 'd' },
				refusal: false,
				supported: false
			},
			{
				state: 'contract_violation',
				outcome: { state: 'contract_violation', status: 200, detail: 'd' },
				refusal: false,
				supported: false
			}
		];

		expect(arms.length).toBe(6);
		expect(
			new Set(arms.map((a) => a.state)).size,
			'two arms share a token, so a switch cannot tell them'
		).toBe(arms.length);
		for (const arm of arms) {
			expect(isRefusal(arm.outcome), `${arm.state} isRefusal`).toBe(arm.refusal);
			expect(isSupported(arm.outcome), `${arm.state} isSupported`).toBe(arm.supported);
		}
		// The load-bearing pair, stated separately because it is the whole
		// confidentiality property of this route and the two answers differ.
		// Indexed with `!`: the table's length is asserted immediately above, so
		// a non-null assertion here is a narrowing of a fact the test just proved
		// rather than a suppression of a possible undefined.
		const unsupported = arms[1]!.outcome;
		const refused = arms[2]!.outcome;
		expect(isRefusal(unsupported)).toBe(false);
		expect(isRefusal(refused)).toBe(true);
	});
});
