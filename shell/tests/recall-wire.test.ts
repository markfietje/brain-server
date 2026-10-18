import { describe, expect, test } from 'vitest';
import {
	assertOutcome,
	hasPassages,
	isRefusal,
	recallMemory,
	recallPassages,
	type AuthorizedRecall,
	type RecallCore,
	type RecallOutcome,
	type RecallRequest,
	type RecallTransport
} from '../src/lib/api/recall';

/**
 * I3 — the refusal/absence distinction ON THE WIRE.
 *
 * Every arm drives the REAL `recallMemory` and the REAL `assertOutcome`. The
 * fake core here stands in for the Tauri IPC, not for the subject: the
 * classification under test is the client's plumbing and its refusal of
 * unrecognised shapes, and a copy of that logic inside the test would prove
 * nothing. The core's own six arms are pinned in `src-tauri/src/recall_tests.rs`
 * and the two suites meet at `assertOutcome`.
 */

/** A core that answers with whatever the test dictates. */
function coreReturning(outcome: unknown): RecallCore {
	return {
		authorize: async (): Promise<AuthorizedRecall> => ({
			doc: { v: 1, query: 'q', limit: 5, provenance: true },
			query_bytes: 1
		}),
		shape: async () => outcome
	};
}

/** A core that REFUSES, as the Rust gate does on a bound. */
function coreRefusing(message: string): RecallCore {
	return {
		authorize: async () => {
			throw new Error(message);
		},
		shape: async () => {
			throw new Error('shape must not be reached when the gate refuses');
		}
	};
}

function transportReturning(status: number, body: unknown): RecallTransport {
	return async () => ({ status, body: JSON.stringify(body) });
}

const answered: RecallOutcome = {
	state: 'answered',
	decision: 'ok',
	passages: [
		{
			id: 7,
			title: 't',
			text: 'the passage',
			snippet: null,
			domain: 'legal',
			score: 0.8,
			valid_from: '2024-01-01T00:00:00Z',
			valid_to: null,
			flagged: false,
			ingest_kind: 'markdown',
			memory_kind: 'fact',
			authority: 0.5
		}
	],
	confidence: 0.9,
	searched: {
		query: 'q',
		domain: null,
		limit: 5,
		domains_searched: ['legal'],
		included_global: null
	}
};

const absent: RecallOutcome = { ...answered, state: 'absent', passages: [], confidence: 0.2 };

const abstained: RecallOutcome = {
	...answered,
	state: 'abstained',
	decision: 'low_confidence',
	passages: [],
	confidence: 0.1
};

const refused: RecallOutcome = { state: 'refused', status: 403, scope: 'recall' };

const unreachable: RecallOutcome = { state: 'unreachable', status: 503, detail: 'kernel down' };

const contractViolation: RecallOutcome = {
	state: 'contract_violation',
	status: 200,
	detail: 'unreadable'
};

describe('the three states stay distinct', () => {
	test('answered, absent and abstained are three outcomes and not two', async () => {
		for (const [outcome, state] of [
			[answered, 'answered'],
			[absent, 'absent'],
			[abstained, 'abstained']
		] as const) {
			const got = await recallMemory(
				{ query: 'q' },
				{ core: coreReturning(outcome), transport: transportReturning(200, {}) }
			);
			expect(got.state).toBe(state);
			// Every answer-shaped outcome carries a decision — that is the arm.
			expect('decision' in got ? got.decision : null).not.toBeNull();
		}

		// Positional, not a count: the abstention says so and the absence does
		// not, even though both carry zero passages.
		expect(abstained.decision).toBe('low_confidence');
		expect(absent.decision).toBe('ok');
		expect(hasPassages(abstained)).toBe(false);
		expect(hasPassages(absent)).toBe(false);
		expect(hasPassages(answered)).toBe(true);
	});

	test('a refusal is distinguishable from an absence by every accessor', async () => {
		const got = await recallMemory(
			{ query: 'q' },
			{ core: coreReturning(refused), transport: transportReturning(403, { code: 'forbidden' }) }
		);

		expect(isRefusal(got)).toBe(true);
		expect(got.state).not.toBe(absent.state);

		// THE arm: asking for the passages of a refusal is a THROW, not an empty
		// array. A renderer that reaches here has a bug, and it fails loudly
		// instead of drawing "no results" for a row it was not allowed to read.
		expect(() => recallPassages(got)).toThrow(/not an absence/);

		// And the absence answers the same question without throwing, so the two
		// are not merely differently-labelled empties.
		const absence = await recallMemory(
			{ query: 'q' },
			{ core: coreReturning(absent), transport: transportReturning(200, {}) }
		);
		expect(isRefusal(absence)).toBe(false);
		expect(recallPassages(absence)).toEqual([]);
	});
});

describe('no failure arm becomes an empty result', () => {
	test('a transport that throws is unreachable, never absent', async () => {
		const got = await recallMemory(
			{ query: 'q' },
			{
				core: coreReturning(answered),
				transport: async () => {
					throw new Error('ECONNREFUSED');
				}
			}
		);
		expect(got.state).toBe('unreachable');
		expect(hasPassages(got)).toBe(false);
		expect(() => recallPassages(got)).toThrow(/not an absence/);
		// The detail survives, so the banner can say what happened.
		expect((got as { detail: string }).detail).toContain('ECONNREFUSED');
	});

	test('a core that refuses on a bound propagates the refusal loudly', async () => {
		// The gate refusing is NOT an absence either — it is an exception the
		// caller must handle, and the transport is never reached.
		await expect(
			recallMemory(
				{ query: 'q'.repeat(2001) },
				{ core: coreRefusing('query_too_long'), transport: transportReturning(200, {}) }
			)
		).rejects.toThrow(/query_too_long/);
	});

	test('an unreadable 2xx is a contract violation, not an absence', async () => {
		const got = await recallMemory(
			{ query: 'q' },
			{ core: coreReturning(contractViolation), transport: transportReturning(200, 'garbage') }
		);
		expect(got.state).toBe('contract_violation');
		expect(hasPassages(got)).toBe(false);
	});

	test('a refusal never carries a passages array', () => {
		// The core omits the key; the client refuses it if it ever appears.
		expect('passages' in refused).toBe(false);
		expect(() =>
			assertOutcome({ state: 'refused', status: 403, scope: 'recall', passages: [] })
		).toThrow(/must not carry passages/);
		expect(() =>
			assertOutcome({ state: 'unreachable', status: 503, detail: 'd', passages: [] })
		).toThrow(/must not carry passages/);
	});
});

describe('an unrecognised core shape is refused, never defaulted', () => {
	test('an unknown state throws instead of becoming an empty answer', () => {
		// The `?? []` this replaces would have turned a core that grew or broke
		// a state into a silently empty answer — the leak the taxonomy exists
		// to prevent.
		expect(() => assertOutcome({ state: 'maybe' })).toThrow(/unknown state maybe/);
		expect(() => assertOutcome({ passages: [] })).toThrow(/unknown state undefined/);
		expect(() => assertOutcome(null)).toThrow(/not an outcome/);
		expect(() => assertOutcome('refused')).toThrow(/not an outcome/);
	});

	test('an answer state without a decision or passages throws', () => {
		expect(() => assertOutcome({ state: 'answered', passages: [] })).toThrow(
			/decision undefined/
		);
		expect(() => assertOutcome({ state: 'answered', decision: 'ok' })).toThrow(
			/no passages array/
		);
		// A third decision value is a contract lie, not a new state.
		expect(() =>
			assertOutcome({ state: 'answered', decision: 'answered', passages: [] })
		).toThrow(/decision answered/);
		expect(() => assertOutcome({ state: 'refused' })).toThrow(/carried no status/);
	});
});

describe('the round trip through the real module', () => {
	test('the request reaches the wire as the doc the core minted', async () => {
		// Anti-vacuity for the whole suite: if `recallMemory` ignored the
		// authorized doc, every arm above would still pass while sending
		// something the core never bounded.
		const minted: AuthorizedRecall = {
			doc: { v: 1, query: 'bounded query', domain: 'legal', limit: 5, provenance: true },
			query_bytes: 13
		};
		let sent: unknown = null;
		const request: RecallRequest = { query: 'bounded query', domain: 'legal' };

		await recallMemory(request, {
			core: {
				authorize: async () => minted,
				shape: async (_exchange, doc) => ({ ...answered, searched: { ...answered.searched, query: doc.query } })
			},
			transport: async (doc) => {
				sent = doc;
				return { status: 200, body: '{}' };
			}
		});

		// Byte-for-byte the doc the core authorized — not the caller's request,
		// and not a doc with the core's fields widened.
		expect(sent).toEqual(minted.doc);
		expect(sent).not.toHaveProperty('authorization');
		expect(sent).not.toHaveProperty('token');
	});
});
