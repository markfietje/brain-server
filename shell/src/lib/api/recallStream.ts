/**
 * P1 — the relay. Streaming without interpretation.
 *
 * Two rules, both from Plan 04 §4, and the second is the one people get
 * wrong:
 *
 * 1. **The relay RELAYS; it does not interpret.** Each event is forwarded the
 *    moment it arrives, in order, without being read. Buffering a whole turn
 *    to "tidy it up" is what turns a 200 ms first token into 4 s — the cost is
 *    invisible in the code and enormous on screen.
 * 2. **Tool output is not prose and not JSON.** It arrives as a TYPED EVENT
 *    and renders as the citation block. Mixing raw tool JSON into the token
 *    stream is how provenance gets lost, because a renderer that has to parse
 *    a blob mid-sentence cannot know which part is the answer.
 *
 * There is no chat surface to attach this to yet — that is P2, and it depends
 * on this phase. What lands here is the relay itself and the pins that hold
 * it to rule 1, because a relay that is written correctly and never exercised
 * is a relay that is written correctly once.
 */

import type { Passage, RecallDecision, Searched } from './recall';

/** One typed event. The discriminant is what a renderer branches on; nothing
 * else is ever inspected on the streaming path. */
export type RecallEvent =
	| { kind: 'passage'; index: number; passage: Passage }
	| { kind: 'decision'; decision: RecallDecision; confidence: number | null }
	| { kind: 'refused'; status: number; scope: string }
	| { kind: 'complete'; searched: Searched };

/**
 * Forwards each event as it arrives.
 *
 * `yield*` over the source is the whole implementation, and that is the point:
 * there is no array, no accumulator and no `await` between an event arriving
 * and the consumer seeing it. The function is deliberately incapable of
 * buffering, so a future edit that reintroduces it has to be written
 * deliberately rather than reached by accident.
 */
export async function* relayRecall(
	source: AsyncIterable<RecallEvent>
): AsyncGenerator<RecallEvent, void, undefined> {
	for await (const event of source) {
		yield event;
	}
}

/**
 * The whole-turn-buffering control, kept so the pin can prove it would CATCH
 * this shape.
 *
 * This is not dead code and not an example: it is the negative arm. A pin that
 * only ever sees the real relay has not shown it can see the failure — and
 * "the guard would fire if" is a claim, not a proof. It is private to the pin
 * file for the same reason: nothing in the app may call it.
 */
export async function* bufferWholeTurn(
	source: AsyncIterable<RecallEvent>
): AsyncGenerator<RecallEvent, void, undefined> {
	const collected: RecallEvent[] = [];
	for await (const event of source) {
		collected.push(event);
	}
	for (const event of collected) {
		yield event;
	}
}

/**
 * Renders one event as a fragment of the citation block.
 *
 * Per-event rather than per-response, so the block fills in as the rows
 * arrive. It returns a STRING and never an object: this is the boundary where
 * a typed event becomes prose the transcript can hold, and returning structured
 * data here is how a future caller ends up `JSON.stringify`-ing it into the
 * token stream.
 */
export function renderRecallEvent(event: RecallEvent): string {
	switch (event.kind) {
		case 'passage': {
			const { index, passage } = event;
			const domain = passage.domain ? ` · domain ${passage.domain}` : '';
			// An expired fact says so where the reader sees it, not in a
			// footnote: presenting a `valid_to` that has passed as current is the
			// failure this kernel exists to prevent.
			//
			// SNAKE_CASE, because that is what `Passage` carries off the core's
			// wire. The first version of this read `passage.validTo` and
			// `validFrom`, which are `undefined` on a snake_case object -- so
			// EVERY passage rendered "validity not stated" and the pin caught it.
			// A renderer that silently loses the validity of every row is worse
			// than one that never had the field.
			const validity = passage.valid_to
				? `valid until ${passage.valid_to}`
				: (passage.valid_from ? `valid from ${passage.valid_from}` : 'validity not stated');
			return `[${index}] id ${passage.id}${domain}\n    ${passage.text}\n    ${validity}`;
		}
		case 'decision': {
			const confidence =
				event.confidence === null ? '' : `, confidence ${event.confidence.toFixed(2)}`;
			return `decision: ${event.decision}${confidence}`;
		}
		case 'refused':
			// The refusal renders as a refusal on the STREAMING path too. An
			// empty block arriving because a row was withheld is the leak.
			return `access refused — HTTP ${event.status} on ${event.scope}; this is not an empty result`;
		case 'complete':
			return `searched: ${event.searched.query} (limit ${event.searched.limit})`;
	}
}

/**
 * Consumes the relay and returns each rendered fragment as it arrives.
 *
 * The consumer shape matters as much as the producer: the returned array is
 * built by APPENDING, so a caller can measure when the first fragment appeared
 * without the measurement itself being the thing that forces buffering.
 */
export async function collectRendered(
	source: AsyncIterable<RecallEvent>
): Promise<{ fragments: string[]; firstFragmentAtMs: number; totalMs: number }> {
	const started = Date.now();
	const fragments: string[] = [];
	let firstFragmentAtMs = -1;
	for await (const event of relayRecall(source)) {
		fragments.push(renderRecallEvent(event));
		if (firstFragmentAtMs < 0) firstFragmentAtMs = Date.now() - started;
	}
	return { fragments, firstFragmentAtMs, totalMs: Date.now() - started };
}
