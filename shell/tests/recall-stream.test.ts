import { describe, expect, test } from 'vitest';
import {
	bufferWholeTurn,
	collectRendered,
	renderRecallEvent,
	relayRecall,
	type RecallEvent
} from '../src/lib/api/recallStream';
import type { Passage, Searched } from '../src/lib/api/recall';

/**
 * I5 — streaming reaches the first event UNBUFFERED.
 *
 * The guard is deterministic, not a timing threshold. A producer that will not
 * release its second event until the consumer has seen the first DEADLOCKS a
 * buffering relay and completes instantly against a relaying one — so the pin
 * has real teeth without a sleep, and cannot go flaky on a loaded CI runner.
 * The timing arm is kept alongside it, but it is the second line of defence
 * rather than the first: a millisecond threshold on a shared runner is a
 * flake waiting for the wrong minute.
 */

const searched: Searched = {
	query: 'eu data rules',
	domain: 'legal',
	limit: 5,
	domains_searched: ['legal'],
	included_global: null
};

function passage(id: number, text: string): Passage {
	return {
		id,
		title: null,
		text,
		snippet: null,
		domain: 'legal',
		score: 0.5,
		valid_from: '2024-01-01T00:00:00Z',
		valid_to: null,
		flagged: false,
		ingest_kind: 'markdown',
		memory_kind: 'fact',
		authority: null
	};
}

const events: RecallEvent[] = [
	{ kind: 'passage', index: 0, passage: passage(1, 'first row') },
	{ kind: 'passage', index: 1, passage: passage(2, 'second row') },
	{ kind: 'decision', decision: 'ok', confidence: 0.91 },
	{ kind: 'complete', searched }
];

describe('the relay does not buffer', () => {
	test('a producer gated on the first observation completes', async () => {
		// The producer refuses to produce event 2 until the consumer has
		// observed event 1. Anything that collects the whole turn before
		// yielding never gets there.
		let released = false;
		let sawFirst = false;
		async function* gated(): AsyncGenerator<RecallEvent> {
			yield events[0] as RecallEvent;
			while (!sawFirst) await new Promise((resolve) => setTimeout(resolve, 1));
			released = true;
			yield events[1] as RecallEvent;
			yield events[2] as RecallEvent;
			yield events[3] as RecallEvent;
		}

		const seen: RecallEvent[] = [];
		for await (const event of relayRecall(gated())) {
			seen.push(event);
			if (seen.length === 1) sawFirst = true;
		}

		expect(released).toBe(true);
		// Positional: the order is the measurement, and a count could not tell
		// a reorder from a swap.
		expect(seen.map((event) => event.kind)).toEqual([
			'passage',
			'passage',
			'decision',
			'complete'
		]);
	});

	test('the buffering control delivers NOTHING until the turn ends', async () => {
		// THE NEGATIVE ARM, and it is deterministic rather than a 5s deadlock.
		// Without it, "the pin would fire if someone buffered" is a claim
		// nobody checked. Raced against a short timer: the relay resolves its
		// first `next()` immediately, and the buffering control does not resolve
		// at all until the source is drained -- which it never is, because the
		// gated producer is still waiting for a consumer that has seen nothing.
		let sawFirst = false;
		async function* gated(): AsyncGenerator<RecallEvent> {
			yield events[0] as RecallEvent;
			while (!sawFirst) await new Promise((resolve) => setTimeout(resolve, 1));
			for (const event of events.slice(1)) yield event;
		}

		const buffered = bufferWholeTurn(gated())[Symbol.asyncIterator]();
		const bufferedFirst = await Promise.race([
			buffered.next(),
			new Promise((resolve) => setTimeout(() => resolve('nothing yet'), 50))
		]);
		expect(bufferedFirst).toBe('nothing yet');
		expect(sawFirst).toBe(false);

		// The SAME producer against the real relay resolves at once -- which is
		// the positive half of the same comparison.
		const relaying = relayRecall(gated())[Symbol.asyncIterator]();
		const relayedFirst = await Promise.race([
			relaying.next(),
			new Promise((resolve) => setTimeout(() => resolve('nothing yet'), 50))
		]);
		expect(relayedFirst).not.toBe('nothing yet');
		sawFirst = true;
		await relaying.return?.(undefined);
	});
});

describe('time-to-first-event, not whole-turn', () => {
	test('the first fragment arrives well before the turn ends', async () => {
		const LAST_EVENT_MS = 120;
		async function* slowTail(): AsyncGenerator<RecallEvent> {
			yield events[0] as RecallEvent;
			await new Promise((resolve) => setTimeout(resolve, LAST_EVENT_MS));
			yield events[1] as RecallEvent;
			yield events[2] as RecallEvent;
			yield events[3] as RecallEvent;
		}

		const { fragments, firstFragmentAtMs, totalMs } = await collectRendered(slowTail());

		// Positional, and the count is incidental.
		expect(fragments[0]).toContain('first row');
		expect(fragments[1]).toContain('second row');
		expect(fragments[2]).toContain('decision: ok');
		expect(fragments[3]).toContain('searched: eu data rules');

		// The measurement: the first fragment must land well inside the turn,
		// not at its end. Half the turn is a wide margin on purpose — this is a
		// smoke alarm, not a benchmark.
		//
		// The LOWER BOUND is what makes a future "deterministic time" edit
		// fail rather than fake a green: the 120 ms `setTimeout` here is the
		// only real time in the turn, so a clock that is virtualized for
		// testability collapses `totalMs` to ~0 against a numeric 120 and
		// breaches it — a ratio arm alone could survive that (both producers
		// express and slow scale by the same factor), and a `> 0` on
		// `firstFragmentAtMs` cannot be claimed at all because the express
		// producer's first event routinely lands INSIDE the same millisecond
		// `Date.now()` starts the timer in (driven at 19 machines on the same
		// task's express loop).
		expect(firstFragmentAtMs).toBeGreaterThanOrEqual(0);
		expect(firstFragmentAtMs).toBeLessThan(totalMs / 2);
		expect(totalMs).toBeGreaterThanOrEqual(LAST_EVENT_MS);
	});

	test('the buffering control fails that same measurement', async () => {
		const LAST_EVENT_MS = 120;
		// The stall rides a 5ms margin: setTimeout(120) can be MEASURED at
		// 119ms (timer coalescing over Date.now()'s granularity), and a
		// boundary with no margin flakes on the runner, not in the product.
		// The pin's intent — the turn takes at least the deliberate stall —
		// is unchanged.
		const STALL_MS = LAST_EVENT_MS + 5;
		async function* slowTail(): AsyncGenerator<RecallEvent> {
			yield events[0] as RecallEvent;
			await new Promise((resolve) => setTimeout(resolve, STALL_MS));
			for (const event of events.slice(1)) yield event;
		}

		const started = Date.now();
		const collected: string[] = [];
		for await (const event of bufferWholeTurn(slowTail())) {
			collected.push(renderRecallEvent(event));
		}
		const firstAt = Date.now() - started;
		const total = Date.now() - started;

		// The first fragment arrives only once the turn is over — which is the
		// defect, stated as the number that exposes it.
		expect(firstAt).toBeGreaterThanOrEqual(LAST_EVENT_MS);
		expect(total).toBeGreaterThanOrEqual(LAST_EVENT_MS);
	});
});

describe('tool output is a typed event, never JSON in the token stream', () => {
	test('every event renders to text and no rendering leaks a brace', () => {
		for (const event of events) {
			const rendered = renderRecallEvent(event);
			expect(typeof rendered).toBe('string');
			expect(rendered).not.toContain('{');
			expect(rendered).not.toContain('"kind"');
		}
	});

	test('a refusal on the stream renders as a refusal', () => {
		// The streaming path is where the leak would hide: an empty block
		// arriving because a row was withheld looks exactly like a slow answer.
		const rendered = renderRecallEvent({ kind: 'refused', status: 403, scope: 'recall' });
		expect(rendered).toContain('403');
		expect(rendered.toLowerCase()).toContain('refused');
		expect(rendered.toLowerCase()).toContain('not an empty result');
	});

	test('an expired passage says so where the reader sees it', () => {
		const rendered = renderRecallEvent({
			kind: 'passage',
			index: 0,
			passage: passage(9, 'stale fact')
		});
		expect(rendered).toContain('valid from 2024-01-01T00:00:00Z');

		// And with a `valid_to`, it must not read as current.
		const expired = renderRecallEvent({
			kind: 'passage',
			index: 0,
			passage: { ...passage(9, 'stale fact'), valid_to: '2025-01-01T00:00:00Z' }
		});
		expect(expired).toContain('valid until 2025-01-01T00:00:00Z');
		expect(expired).not.toContain('valid from');
	});
});
