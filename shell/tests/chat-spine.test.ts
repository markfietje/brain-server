import { describe, expect, test, vi } from 'vitest';
import type { SpineHandlers } from '../src/lib/chat/spine';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { shellRoot } from './fixtures';

/**
 * M3-d — the chat spine's consumer, pinned at the module boundary (the R141
 * pin pattern: the static host import captures the real `fetch`, and every
 * stream is a fixture `ReadableStream` — the kernel never runs here).
 *
 * The laws this file holds, and the red-proof each one owes (every drill is a
 * revert of the named law in `spine.ts`, watched red, then restored):
 *
 *  1. A frame SPLIT ACROSS two reads still parses (the parser's hardest case,
 *     plan 25 READ FIRST ③). Drill: re-assemble the buffer between pushes.
 *  2. axum `KeepAlive` COMMENT frames and blank dispatch lines are tolerated.
 *     Drill: treat a `:`-prefixed line as malformed.
 *  3. CRLF framing parses. Drill: cut on `\n` only.
 *  4. `replace: true` on a delta DISCARDS the accumulated text — appending
 *     blindly is the silent-corruption defect (READ FIRST ④). Drill: `+=`.
 *  5. The terminal is terminal: a second `final` is ignored and counted.
 *     Drill: emit both.
 *  6. Malformed frames are skipped + counted, never fatal below the cap; over
 *     the cap the stream ends as `contract_violation` (the taxonomy's name).
 *     Drill: raise the cap / delete the check.
 *  7. Stop (abort) — NO further handler calls. Drill: let the loop run on.
 *  8. An `error:` frame is a named failure ON THE TURN, never a transport
 *     fault; a mid-stream read failure is the opposite (the M3-b trap,
 *     restated for streams). Drill: swap the two dispatch arms.
 *  9. A stream that ends without a terminal frame is a contract violation.
 *     Drill: delete the end-of-stream check.
 * 10. The pre-stream refusals classify: 503 `chat_bridge_unavailable` and 400
 *     `locale_not_supported` are the TWO fallback postures (plan 25 D1); any
 *     other refusal carries NO posture. Drill: classify by status alone.
 * 11. The flush buffer coalesces: a 200-delta burst issues one flush per
 *     frame, never per delta. Drill: flush per accept.
 * 12. The envelope law: no new dependency, no EventSource, and the built JS
 *     stays within ±0.5 KiB gzipped of the recorded baseline (the M3-f
 *     refactor band; the 3 KiB growth law it replaced is in the register).
 */

// ── fixtures ─────────────────────────────────────────────────────────────

const GROUNDING_FRAME =
	'event: grounding\n' +
	'data: {"decision":"ok","hits":[{"id":7,"title":"Gate law","score":0.91,"domain":"global","flagged":false},{"id":9,"title":null,"score":0.42,"domain":null,"flagged":true}]}\n\n';

const DELTA_FRAME = (text: string, replace = false) =>
	`event: delta\ndata: ${JSON.stringify({ text, ...(replace ? { replace: true } : {}) })}\n\n`;

const FINAL_FRAME =
	'event: final\n' +
	'data: {"text":"The gate is read-first.","tier":"on_device","turn":3,"inputTokens":12,"outputTokens":340}\n\n';

const TYPED_FINAL_FRAME =
	'event: final\n' +
	'data: {"text":"Green tea is steamed Camellia sinensis.","tier":"on_device","turn":1,"inputTokens":40,"outputTokens":12,"abstain":false,"claims":[{"text":"Green tea is steamed Camellia sinensis.","cites":[0,2]}]}\n\n';

const ABSTAIN_FINAL_FRAME =
	'event: final\n' +
	'data: {"text":"Nothing in your memory answers this question, so I am not guessing an answer.","tier":"deterministic","turn":0,"inputTokens":0,"outputTokens":0,"abstain":true,"claims":[]}\n\n';

const SUPPORT_FRAME = (decision = 'partial') =>
	`event: support\ndata: ${JSON.stringify({ cited: 1, uncited: 1, unknown_refs: 0, decision })}\n\n`;

const ERROR_FRAME = (code: string, message?: string) =>
	`event: error\ndata: ${JSON.stringify(
		message === undefined ? { code } : { code, message }
	)}\n\n`;

/** A 200 `text/event-stream` response whose body emits the given chunks in
 * order and then closes. */
function sseResponse(chunks: string[]): Response {
	const encoder = new TextEncoder();
	let index = 0;
	const body = new ReadableStream<Uint8Array>({
		pull(controller) {
			if (index >= chunks.length) {
				controller.close();
				return;
			}
			controller.enqueue(encoder.encode(chunks[index]!));
			index += 1;
		}
	});
	return new Response(body, {
		status: 200,
		headers: { 'content-type': 'text/event-stream' }
	});
}

/** A 200 stream that emits `before`, then HANGS until the fetch signal
 * aborts — the Stop drill's posture. The stub honors the signal, which the
 * real fetch guarantees and a naive stub silently drops. */
function hangingResponse(before: string[], signal: AbortSignal | null | undefined): Response {
	const encoder = new TextEncoder();
	let index = 0;
	const body = new ReadableStream<Uint8Array>({
		pull(controller) {
			if (index < before.length) {
				controller.enqueue(encoder.encode(before[index]!));
				index += 1;
				return;
			}
			return new Promise<void>((resolve) => {
				if (signal === undefined || signal === null) return; // hangs forever
				signal.addEventListener(
					'abort',
					() => {
						controller.error(new DOMException('This operation was aborted', 'AbortError'));
						resolve();
					},
					{ once: true }
				);
			});
		}
	});
	return new Response(body, {
		status: 200,
		headers: { 'content-type': 'text/event-stream' }
	});
}

/** A 200 stream that emits `before`, then dies the way a TCP reset does. */
function resetResponse(before: string[]): Response {
	const encoder = new TextEncoder();
	let index = 0;
	const body = new ReadableStream<Uint8Array>({
		pull(controller) {
			if (index < before.length) {
				controller.enqueue(encoder.encode(before[index]!));
				index += 1;
				return;
			}
			controller.error(new TypeError('network error: connection reset'));
		}
	});
	return new Response(body, {
		status: 200,
		headers: { 'content-type': 'text/event-stream' }
	});
}

interface Recorder extends SpineHandlers {
	events: string[];
	finals: unknown[];
	supports: unknown[];
	errors: unknown[];
	refusals: unknown[];
	faults: string[];
}

function recorder(): Recorder {
	const record: Recorder = {
		events: [],
		finals: [],
		supports: [],
		errors: [],
		refusals: [],
		faults: [],
		onGrounding(hits, decision) {
			record.events.push(`grounding:${decision}:${hits.length}`);
		},
		onDelta(text, replace) {
			record.events.push(`delta:${replace ? 'replace' : 'append'}:${text}`);
		},
		onFinal(final) {
			record.finals.push(final);
		},
		onSupport(support) {
			record.supports.push(support);
		},
		onStreamError(error) {
			record.errors.push(error);
		},
		onRefusal(refusal) {
			record.refusals.push(refusal);
		},
		onTransportFault(detail) {
			record.faults.push(detail);
		}
	};
	return record;
}

/** Installs the fetch stub and imports the module FRESH — the spine reads
 * `apiBase()`/`authHeaders()` through its own imports, so a stub before the
 * import is the module-boundary posture (R141). */
async function withFetch(
	impl: (url: string, init: RequestInit | undefined) => Response | Promise<Response>
) {
	const fetchMock = vi.fn(impl);
	vi.stubGlobal('fetch', fetchMock);
	const mod = await import('../src/lib/chat/spine');
	return { mod, fetchMock };
}

/** `withFetch` answering every call with one prebuilt response. (The Response
 * is built eagerly; the stub is a FUNCTION — `vi.fn(response)` would mock a
 * non-function and return undefined.) */
function withFetchAnswering(response: () => Response) {
	return withFetch(async () => response());
}

const REQUEST = { message: 'what is the gate law?' };

// ── 1–6 · the parser, pure over chunk strings ────────────────────────────

describe('the SSE parser', () => {
	test('a_frame_split_across_two_reads_still_parses', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const parser = mod.createSpineParser();
		const whole = GROUNDING_FRAME;
		// Split INSIDE the JSON payload — the hardest fragment boundary.
		const cut = whole.indexOf('"score"');
		const first = parser.push(whole.slice(0, cut));
		expect(first, 'nothing complete until the frame closes').toEqual([]);
		const second = parser.push(whole.slice(cut));
		expect(second).toHaveLength(1);
		const event = second[0]!;
		expect(event.kind).toBe('grounding');
		if (event.kind === 'grounding') {
			expect(event.decision).toBe('ok');
			expect(event.hits).toHaveLength(2);
			expect(event.hits[0]).toMatchObject({ id: 7, score: 0.91, domain: 'global' });
			expect(event.hits[1]).toMatchObject({ id: 9, flagged: true, title: null });
		}
		expect(parser.malformed).toBe(0);
	});

	test('keepalive_comment_frames_and_blank_dispatch_lines_are_tolerated', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const parser = mod.createSpineParser();
		// axum's KeepAlive emits comment-only frames between events.
		const events = parser.push(`: keepalive\n\n\n${DELTA_FRAME('hello')}`);
		expect(events.map((e) => e.kind)).toEqual(['delta']);
		expect(events[0]).toMatchObject({ kind: 'delta', text: 'hello', replace: false });
		expect(parser.malformed, 'a comment frame is not a malformed frame').toBe(0);
	});

	test('crlf_framing_parses', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const parser = mod.createSpineParser();
		const crlf = FINAL_FRAME.replaceAll('\n', '\r\n');
		// ...and split BETWEEN the \r and the \n of the closing blank line.
		const cut = crlf.lastIndexOf('\r\n') + 1;
		const first = parser.push(crlf.slice(0, cut));
		const second = parser.push(crlf.slice(cut));
		expect([...first, ...second]).toHaveLength(1);
		expect(parser.closed).toBe(true);
		expect(parser.malformed).toBe(0);
	});

	test('field_values_parse_with_and_without_the_space_after_the_colon', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const parser = mod.createSpineParser();
		const events = parser.push('event:delta\ndata:{"text":"tight","replace":true}\n\n');
		expect(events).toEqual([{ kind: 'delta', text: 'tight', replace: true }]);
	});

	test('replace_deltas_arrive_with_their_flag', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const parser = mod.createSpineParser();
		const events = parser.push(
			DELTA_FRAME('first draft') + DELTA_FRAME('wholesale revision', true) + DELTA_FRAME(' more')
		);
		expect(events).toEqual([
			{ kind: 'delta', text: 'first draft', replace: false },
			{ kind: 'delta', text: 'wholesale revision', replace: true },
			{ kind: 'delta', text: ' more', replace: false }
		]);
	});

	test('a_second_terminal_is_ignored_and_counted', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const parser = mod.createSpineParser();
		const first = parser.push(FINAL_FRAME);
		expect(first).toHaveLength(1);
		expect(parser.closed).toBe(true);
		const after = parser.push(FINAL_FRAME);
		expect(after, 'the terminal is terminal — nothing is emitted after it').toEqual([]);
		expect(parser.terminalOverflow).toBe(1);
	});

	test('malformed_frames_are_skipped_and_counted_not_fatal', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const parser = mod.createSpineParser();
		const garbage = [
			'data: {"text":"no event name"}\n\n', // no event field
			'event: delta\ndata: not-json\n\n', // broken payload
			'event: mystery\ndata: {}\n\n', // unknown event name
			'event: delta\ndata: {"replace":true}\n\n', // delta with no text
			'event: final\ndata: {}\n\n' // final with no text
		].join('');
		const events = parser.push(garbage + DELTA_FRAME('still alive'));
		expect(events).toEqual([{ kind: 'delta', text: 'still alive', replace: false }]);
		expect(parser.malformed).toBe(5);
		expect(parser.closed).toBe(false);
	});

	test('the_grounding_payload_is_validated_not_trusted', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const parser = mod.createSpineParser();
		// A hit without id/score makes its WHOLE frame malformed; a hit with
		// extraneous fields and null-able strings parses with them normalized.
		const events = parser.push(
			'event: grounding\ndata: {"decision":"ok","hits":[{"title":"no id or score"}]}\n\n' +
				'event: grounding\ndata: {"decision":"degraded","hits":[],"extra":"ignored"}\n\n'
		);
		expect(events, 'the malformed frame is skipped; the valid one parses').toEqual([
			{ kind: 'grounding', decision: 'degraded', hits: [] }
		]);
		expect(parser.malformed).toBe(1);
	});

	// ── LL-04 — the typed final and the trailing support verdict ─────────

	test('the_typed_final_parses_abstain_and_claims_verbatim', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([TYPED_FINAL_FRAME]));
		const parser = mod.createSpineParser();
		const events = parser.push(TYPED_FINAL_FRAME);
		expect(events).toEqual([
			{
				kind: 'final',
				text: 'Green tea is steamed Camellia sinensis.',
				tier: 'on_device',
				turn: 1,
				inputTokens: 40,
				outputTokens: 12,
				abstain: false,
				claims: [{ text: 'Green tea is steamed Camellia sinensis.', cites: [0, 2] }]
			}
		]);
	});

	test('the_support_verdict_dispatches_after_the_final_and_is_not_overflow', async () => {
		const { mod } = await withFetchAnswering(() =>
			sseResponse([GROUNDING_FRAME, DELTA_FRAME('An answer.'), TYPED_FINAL_FRAME, SUPPORT_FRAME('unsupported')])
		);
		const parser = mod.createSpineParser();
		const events = parser.push(GROUNDING_FRAME + DELTA_FRAME('An answer.') + TYPED_FINAL_FRAME + SUPPORT_FRAME('unsupported'));
		expect(events.map((e) => e.kind)).toEqual(['grounding', 'delta', 'final', 'support']);
		expect(events[3]).toEqual({ kind: 'support', cited: 1, uncited: 1, unknownRefs: 0, decision: 'unsupported' });
		expect(parser.terminalOverflow, 'the contract trailing frame is NOT overflow').toBe(0);
		expect(parser.closed).toBe(true);
		expect(parser.supportDelivered).toBe(true);
	});

	test('a_second_frame_after_the_support_is_still_overflow', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([TYPED_FINAL_FRAME]));
		const parser = mod.createSpineParser();
		parser.push(TYPED_FINAL_FRAME + SUPPORT_FRAME('supported'));
		const events = parser.push(SUPPORT_FRAME('supported'));
		expect(events).toEqual([]);
		expect(parser.terminalOverflow, 'support follows final EXACTLY once').toBe(1);
	});

	test('a_support_before_any_terminal_is_malformed', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([SUPPORT_FRAME('supported')]));
		const parser = mod.createSpineParser();
		const events = parser.push(SUPPORT_FRAME('supported'));
		expect(events).toEqual([]);
		expect(parser.malformed, 'the contract shape is support-after-terminal').toBe(1);
	});

	test('a_drifted_support_vocabulary_is_malformed_not_a_sixth_decision', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([TYPED_FINAL_FRAME]));
		const parser = mod.createSpineParser();
		const events = parser.push(
			TYPED_FINAL_FRAME +
				'event: support\ndata: {"cited":1,"uncited":0,"unknown_refs":0,"decision":"hallucinated"}\n\n'
		);
		expect(events.filter((e) => e.kind === 'support')).toEqual([]);
		expect(parser.malformed).toBe(1);
	});

	test('a_drifted_claim_shape_makes_the_final_malformed', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([TYPED_FINAL_FRAME]));
		const parser = mod.createSpineParser();
		const events = parser.push(
			'event: final\ndata: {"text":"x","abstain":false,"claims":[{"text":42,"cites":[0]}]}\n\n'
		);
		expect(events).toEqual([]);
		expect(parser.malformed).toBe(1);
	});

	test('an_abstaining_final_parses_with_the_typed_keys_and_no_claims', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([ABSTAIN_FINAL_FRAME, SUPPORT_FRAME('abstained')]));
		const parser = mod.createSpineParser();
		const abstainedSupport =
			'event: support\ndata: {"cited":0,"uncited":0,"unknown_refs":0,"decision":"abstained"}\n\n';
		const events = parser.push(ABSTAIN_FINAL_FRAME + abstainedSupport);
		expect(events[0]).toMatchObject({ kind: 'final', abstain: true, claims: [] });
		expect(events[1]).toEqual({ kind: 'support', cited: 0, uncited: 0, unknownRefs: 0, decision: 'abstained' });
	});
});

// ── 11 · the flush buffer ────────────────────────────────────────────────

describe('the delta flush buffer', () => {
	/** A manual frame scheduler: `tick()` runs one animation frame. */
	function manualScheduler(): { schedule: (cb: () => void) => void; tick: () => number } {
		const queue: Array<() => void> = [];
		return {
			schedule: (cb) => queue.push(cb),
			tick: () => {
				const frame = queue.splice(0);
				for (const cb of frame) cb();
				return frame.length;
			}
		};
	}

	test('a_200_delta_burst_flushes_at_most_once_per_frame_never_per_delta', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const frames = manualScheduler();
		let flushes = 0;
		let last = '';
		const buffer = mod.createDeltaBuffer(
			(committed) => {
				flushes += 1;
				last = committed;
			},
			frames.schedule
		);
		// 200 deltas spread across 10 frames — the token-rate > frame-rate case.
		for (let frame = 0; frame < 10; frame += 1) {
			for (let d = 0; d < 20; d += 1) buffer.accept(`t${frame}-${d} `, false);
			frames.tick();
		}
		expect(flushes, 'one flush per frame — 20 deltas coalesce into it').toBe(10);
		expect(flushes).toBeLessThanOrEqual(60);
		expect(last.split(' ').filter(Boolean)).toHaveLength(200);
	});

	test('replace_resets_the_accumulated_text_committed_and_buffered', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const frames = manualScheduler();
		let last = '';
		const buffer = mod.createDeltaBuffer((committed) => (last = committed), frames.schedule);
		buffer.accept('first draft. ', false);
		buffer.accept('more of it ', false);
		buffer.accept('WHOLESALE REVISION', true); // discards BOTH committed and buffered
		frames.tick();
		expect(last, 'the revision is the whole text, not an append').toBe('WHOLESALE REVISION');
		// And the accumulation continues from the revision.
		buffer.accept(' plus', false);
		frames.tick();
		expect(last).toBe('WHOLESALE REVISION plus');
	});

	test('flushNow_coalesces_without_waiting_for_a_frame', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const frames = manualScheduler();
		let last = '';
		const buffer = mod.createDeltaBuffer((committed) => (last = committed), frames.schedule);
		buffer.accept('terminal incoming ', false);
		buffer.flushNow();
		expect(last).toBe('terminal incoming ');
		// A final must not leave text stranded in the buffer.
		frames.tick();
		expect(last).toBe('terminal incoming ');
	});

	test('discard_stops_the_scheduled_flush_and_the_accumulation', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const frames = manualScheduler();
		let flushes = 0;
		const buffer = mod.createDeltaBuffer(() => (flushes += 1), frames.schedule);
		buffer.accept('cancelled mid-stream', false);
		buffer.discard();
		frames.tick();
		expect(flushes, 'no flush fires after the abort').toBe(0);
		// And a NEW buffer start is clean (the same buffer object is reused by
		// the page only until the turn ends; the assert pins the discard).
		buffer.accept('x', false);
		expect(buffer.committed).toBe('');
	});
});

// ── 7–10 · the consumer over fetch ───────────────────────────────────────

describe('the spine consumer', () => {
	test('the_happy_stream_grounds_deltas_and_finals_in_order', async () => {
		const { mod } = await withFetchAnswering(() =>
			sseResponse([GROUNDING_FRAME, DELTA_FRAME('The gate '), DELTA_FRAME('is read-first.'), FINAL_FRAME])
		);
		const record = recorder();
		const turn = mod.turnSpine({ ...REQUEST, session: 's-abcd1234', locale: 'en' }, record);
		await turn.done;

		expect(record.events).toEqual(['grounding:ok:2', 'delta:append:The gate ', 'delta:append:is read-first.']);
		expect(record.finals).toEqual([
			{
				text: 'The gate is read-first.',
				tier: 'on_device',
				turn: 3,
				inputTokens: 12,
				outputTokens: 340,
				// The pre-LL-04 fixture carries neither typed key: both read
				// null — the additive law, on the consumer side.
				abstain: null,
				claims: null
			}
		]);
		expect(record.errors).toEqual([]);
		expect(record.faults).toEqual([]);
		expect(record.refusals).toEqual([]);
	});

	test('stop_aborts_and_no_further_handler_calls_land', async () => {
		// A holder object, not a bare `let`: TypeScript narrows a captured let
		// to its initializer across the closure assignment, and the final
		// assertion would read `never`.
		const captured: { signal: AbortSignal | null } = { signal: null };
		const { mod } = await withFetch((_url, init) => {
			captured.signal = init?.signal ?? null;
			return hangingResponse([GROUNDING_FRAME, DELTA_FRAME('partial ')], captured.signal);
		});
		const record = recorder();
		const turn = mod.turnSpine(REQUEST, record);
		// Let the first chunks land, then Stop.
		await vi.waitFor(() => expect(record.events.length).toBeGreaterThanOrEqual(1));
		turn.abort();
		await turn.done;
		expect(record.faults, 'an abort is not a transport fault').toEqual([]);
		expect(record.errors).toEqual([]);
		expect(record.finals, 'no terminal arrives after Stop').toEqual([]);
		expect(record.events.every((e) => !e.startsWith('delta:append:after'))).toBe(true);
		expect(captured.signal?.aborted, 'the fetch signal was aborted').toBe(true);
	});

	test('the_verdict_fires_after_the_final_and_before_done', async () => {
		// LL-04 on the LIVE consumer path: final lands the answer of record,
		// the trailing support patches the kernel's label, THEN the turn
		// resolves — and the body is cancelled, the wire over.
		const order: string[] = [];
		const { mod } = await withFetchAnswering(() =>
			sseResponse([GROUNDING_FRAME, TYPED_FINAL_FRAME, SUPPORT_FRAME('partial')])
		);
		const record = recorder();
		record.onFinal = (final) => {
			order.push(`final:${final.abstain === null ? 'untyped' : final.abstain ? 'abstain' : 'claim'}`);
			record.finals.push(final);
		};
		record.onSupport = (support) => {
			order.push(`support:${support.decision}`);
			record.supports.push(support);
		};
		const turn = mod.turnSpine(REQUEST, record);
		await turn.done;
		expect(order).toEqual(['final:claim', 'support:partial']);
		expect(record.faults).toEqual([]);
		expect(record.errors).toEqual([]);
	});

	test('a_stream_without_support_still_completes_cleanly', async () => {
		// An old kernel (pre-LL-04) never sends the verdict: the turn ends at
		// the final, no contract violation, no support.
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME, FINAL_FRAME]));
		const record = recorder();
		const turn = mod.turnSpine(REQUEST, record);
		await turn.done;
		expect(record.finals).toHaveLength(1);
		expect(record.supports).toEqual([]);
		expect(record.errors, 'no contract violation for a missing verdict').toEqual([]);
	});

	test('a_503_bridge_refusal_classifies_as_the_bridge_posture', async () => {
		const { mod: _unused } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		vi.stubGlobal(
			'fetch',
			vi.fn(async () =>
				new Response(
					JSON.stringify({
						error: {
							code: 'recall_unavailable',
							message: 'chat_bridge_unavailable — no brain-afm binary beside the kernel'
						}
					}),
					{ status: 503 }
				)
			)
		);
		const fresh = await import('../src/lib/chat/spine');
		const record = recorder();
		const turn = fresh.turnSpine(REQUEST, record);
		await turn.done;
		expect(record.refusals).toEqual([
			{
				status: 503,
				code: 'recall_unavailable',
				posture: 'bridge_unavailable',
				message: 'chat_bridge_unavailable — no brain-afm binary beside the kernel'
			}
		]);
		expect(record.faults).toEqual([]);
		expect(record.events).toEqual([]);
	});

	test('locale_not_supported_classifies_as_the_locale_posture', async () => {
		await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		vi.stubGlobal(
			'fetch',
			vi.fn(async () =>
				new Response(
					JSON.stringify({
						error: {
							code: 'locale_not_supported',
							message: 'the measured model cannot generate in de; it reports 24 supported languages'
						}
					}),
					{ status: 400 }
				)
			)
		);
		const fresh = await import('../src/lib/chat/spine');
		const record = recorder();
		const turn = fresh.turnSpine(REQUEST, record);
		await turn.done;
		expect(record.refusals[0]).toMatchObject({ status: 400, posture: 'locale_not_supported' });
	});

	test('an_unnamed_refusal_carries_no_posture_and_is_never_the_banner', async () => {
		await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		vi.stubGlobal(
			'fetch',
			vi.fn(async () =>
				new Response(
					JSON.stringify({ error: { code: 'chat_message_too_long', message: 'too long' } }),
					{ status: 400 }
				)
			)
		);
		const fresh = await import('../src/lib/chat/spine');
		const record = recorder();
		const turn = fresh.turnSpine(REQUEST, record);
		await turn.done;
		expect(record.refusals[0]).toMatchObject({ status: 400, posture: null, code: 'chat_message_too_long' });
		expect(record.faults, 'a refusal is an ANSWER, not a fault (the M3-b trap)').toEqual([]);
	});

	test('an_error_event_on_the_stream_is_not_a_transport_fault', async () => {
		const { mod } = await withFetchAnswering(() =>
			sseResponse([
				GROUNDING_FRAME,
				DELTA_FRAME('half an answer'),
				ERROR_FRAME('bridge_exited', 'the AFM bridge closed mid-turn')
			])
		);
		const record = recorder();
		const turn = mod.turnSpine(REQUEST, record);
		await turn.done;
		expect(record.errors).toEqual([
			{ code: 'bridge_exited', message: 'the AFM bridge closed mid-turn', shortfallTokens: null }
		]);
		expect(record.faults, 'a named stream error renders ON THE TURN').toEqual([]);
		expect(record.finals).toEqual([]);
	});

	test('the_shortfall_rides_the_conversation_does_not_fit_error', async () => {
		await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		vi.stubGlobal(
			'fetch',
			vi.fn(async () =>
				new Response(
					new ReadableStream<Uint8Array>({
						start(controller) {
							const encoder = new TextEncoder();
							controller.enqueue(
								encoder.encode(
									'event: error\ndata: {"code":"conversation_does_not_fit","message":"the conversation no longer fits","shortfallTokens":1234}\n\n'
								)
							);
						}
					}),
					{ status: 200, headers: { 'content-type': 'text/event-stream' } }
				)
			)
		);
		const fresh = await import('../src/lib/chat/spine');
		const record = recorder();
		const turn = fresh.turnSpine(REQUEST, record);
		await turn.done;
		expect(record.errors).toEqual([
			{
				code: 'conversation_does_not_fit',
				message: 'the conversation no longer fits',
				shortfallTokens: 1234
			}
		]);
	});

	test('a_mid_stream_transport_failure_reaches_the_fault_path', async () => {
		await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => resetResponse([GROUNDING_FRAME, DELTA_FRAME('half ')]))
		);
		const fresh = await import('../src/lib/chat/spine');
		const record = recorder();
		const turn = fresh.turnSpine(REQUEST, record);
		await turn.done;
		expect(record.faults.length).toBe(1);
		// Everything that ARRIVED before the wire died was already dispatched —
		// the handlers never lie about what the user saw.
		expect(record.events).toEqual(['grounding:ok:2', 'delta:append:half ']);
		expect(record.errors, 'a wire death is not a named stream error').toEqual([]);
	});

	test('a_stream_ending_without_a_terminal_is_a_contract_violation', async () => {
		await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => sseResponse([GROUNDING_FRAME, DELTA_FRAME('no terminal follows')]))
		);
		const fresh = await import('../src/lib/chat/spine');
		const record = recorder();
		const turn = fresh.turnSpine(REQUEST, record);
		await turn.done;
		expect(record.errors).toEqual([
			{ code: 'contract_violation', message: expect.stringContaining('without a terminal'), shortfallTokens: null }
		]);
		expect(record.faults).toEqual([]);
	});

	test('more_than_ten_malformed_frames_abort_as_contract_violation', async () => {
		await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		const garbage = Array.from({ length: 11 }, () => 'event: delta\ndata: nope\n\n').join('');
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => sseResponse([garbage, DELTA_FRAME('never seen')]))
		);
		const fresh = await import('../src/lib/chat/spine');
		const record = recorder();
		const turn = fresh.turnSpine(REQUEST, record);
		await turn.done;
		expect(record.errors).toEqual([
			{ code: 'contract_violation', message: expect.stringContaining('unreadable'), shortfallTokens: null }
		]);
		expect(record.events, 'the stream does not continue past the cap').toEqual([]);
	});

	test('the_request_rides_the_kernel_origin_the_bearer_and_the_named_fields', async () => {
		const { mod } = await withFetch(async (url, init) => {
			// The URL is the kernel origin's /chat/turns; the body names exactly
			// the wire's fields — session only when a key is active.
			expect(url).toBe('http://127.0.0.1:8765/chat/turns');
			expect(init?.method).toBe('POST');
			const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
			expect(body).toEqual({ message: 'what is the gate law?', session: 's-abcd1234', locale: 'en' });
			return sseResponse([FINAL_FRAME]);
		});
		const record = recorder();
		const turn = mod.turnSpine({ ...REQUEST, session: 's-abcd1234', locale: 'en' }, record);
		await turn.done;
		expect(record.finals).toHaveLength(1);
	});
});

// ── 10 · the posture table, pure ─────────────────────────────────────────

	describe('postureOfRefusal — the D1 fallback table', () => {
	test('the_two_named_postures_and_nothing_else', async () => {
		const { mod } = await withFetchAnswering(() => sseResponse([GROUNDING_FRAME]));
		// The kernel's real 503 envelope: code `recall_unavailable`, the marker
		// on the MESSAGE (measured off src/handlers/chat.rs — the message
		// prefix IS the name; a future code fix still classifies).
		expect(
			mod.postureOfRefusal(503, 'recall_unavailable', 'chat_bridge_unavailable — no binary')
		).toBe('bridge_unavailable');
		expect(mod.postureOfRefusal(503, 'chat_bridge_unavailable', null)).toBe('bridge_unavailable');
		expect(mod.postureOfRefusal(400, 'locale_not_supported', '…')).toBe('locale_not_supported');
		// Every other refusal carries NO posture: the page renders it as the
		// refusal it is — never as the fallback, never as the banner.
		expect(mod.postureOfRefusal(400, 'chat_message_too_long', 'too long')).toBeNull();
		expect(mod.postureOfRefusal(503, 'recall_unavailable', 'some other 503')).toBeNull();
		expect(mod.postureOfRefusal(401, 'unauthorized', null)).toBeNull();
	});
});

// ── 13 · the web-posture fallback core (D1's browser half) ───────────

	describe('webRecallCore — the browser posture of the D1 fallback', () => {
	const GOOD_HIT = {
		id: 7,
		title: 'Retention policy',
		content: 'Audit rows are retained for 400 days by default.',
		snippet: null,
		domain: 'infra',
		score: 0.82,
		evidence: { valid_from: '2026-03-02', valid_to: null },
		flagged: false
	};

	test('the_mapping_mirrors_the_rust_classifier_arm_for_arm', async () => {
		const { recallMemory, webRecallCore } = await import('../src/lib/api/recall');
		const drive = (status: number, body: string) =>
			recallMemory({ query: 'retention' }, {
				core: webRecallCore,
				transport: async () => ({ status, body })
			});

		// 200 + hits + ok → answered (the Rust arm: Decision::Ok → Answered).
		const answered = await drive(
			200,
			JSON.stringify({ decision: 'ok', hits: [GOOD_HIT], telemetry: { confidence: 0.82 } })
		);
		expect(answered.state).toBe('answered');
		if (answered.state === 'answered') {
			expect(answered.passages[0]).toMatchObject({ id: 7, text: GOOD_HIT.content, valid_from: '2026-03-02' });
			expect(answered.confidence).toBe(0.82);
		}

		// 200 + ok + zero hits → absent (the Rust arm: Ok + empty → Absent).
		expect((await drive(200, JSON.stringify({ decision: 'ok', hits: [] }))).state).toBe('absent');
		// 200 + low_confidence → abstained (Rust: LowConfidence → Abstained).
		expect((await drive(200, JSON.stringify({ decision: 'low_confidence', hits: [GOOD_HIT] }))).state).toBe('abstained');
		// 401/403 → refused with the scope the Rust core names.
		expect(await drive(403, '')).toEqual({ state: 'refused', status: 403, scope: 'recall' });
		// Other non-2xx → unreachable with the Rust sentence.
		expect(await drive(500, '')).toMatchObject({ state: 'unreachable', detail: 'the kernel answered with a non-success status' });
		// An unreadable 200 → contract_violation, never a best-effort answer.
		expect(await drive(200, 'not json')).toMatchObject({ state: 'contract_violation' });
		expect(await drive(200, JSON.stringify({ hits: [] }))).toMatchObject({ state: 'contract_violation' });
		// A hit that fails the shape → the whole body is a contract violation
		// (the Rust serde would refuse it too).
		expect(await drive(200, JSON.stringify({ decision: 'ok', hits: [{ title: 'no id' }] }))).toMatchObject({
			state: 'contract_violation'
		});
	});

	test('the_bounds_mirror_the_rust_gate_and_the_doc_is_what_the_wire_gets', async () => {
		const { recallMemory, webRecallCore } = await import('../src/lib/api/recall');
		let sent: unknown = null;
		const outcome = await recallMemory({ query: 'retention' }, {
			core: webRecallCore,
			transport: async (doc) => {
				sent = doc;
				return { status: 200, body: JSON.stringify({ decision: 'ok', hits: [] }) };
			}
		});
		// The doc is the core's own shape: v1, provenance on, the default limit 5.
		expect(sent).toEqual({ v: 1, query: 'retention', limit: 5, provenance: true });
		expect(outcome.state).toBe('absent');

		// The bounds: an empty query and an over-ceiling query are REFUSED
		// before the wire — the Rust authorize's own two first arms.
		await expect(
			recallMemory({ query: '' }, { core: webRecallCore, transport: async () => ({ status: 200, body: '{}' }) })
		).rejects.toThrow(/empty/);
		await expect(
			recallMemory(
				{ query: 'x'.repeat(2001) },
				{ core: webRecallCore, transport: async () => ({ status: 200, body: '{}' }) }
			)
		).rejects.toThrow(/ceiling/);
	});
});

// ── 12 · the envelope law ────────────────────────────────────────────────

describe('the envelope law', () => {
	test('the_spine_adds_no_dependency_and_reaches_for_no_eventsource', () => {
		const pkg = JSON.parse(
			readFileSync(join(shellRoot, 'package.json'), 'utf8')
		) as { dependencies: Record<string, string>; devDependencies: Record<string, string> };
		for (const deps of [pkg.dependencies, pkg.devDependencies]) {
			for (const name of Object.keys(deps)) {
				expect(
					name.toLowerCase(),
					`no SSE polyfill may enter the dependency set (found ${name})`
				).not.toMatch(/event-?source/);
			}
		}
		// Comment prose is stripped first: the module's doc comments NAME
		// EventSource precisely to say why it is not used (a comment does not
		// execute — the same rule chat-turn.test.ts records).
		const source = readFileSync(join(shellRoot, 'src/lib/chat/spine.ts'), 'utf8');
		const code = source
			.split('\n')
			.filter((line) => {
				const trimmed = line.trim();
				return !trimmed.startsWith('//') && !trimmed.startsWith('*') && !trimmed.startsWith('/*');
			})
			.join('\n');
		expect(code, 'the spine parses SSE by hand — EventSource cannot POST with a bearer').not.toContain(
			'EventSource'
		);
		expect(code, 'the spine adds no streaming library').not.toMatch(
			/from '(eventsource|event-source-polyfill|sse\.js)'/
		);
	});

	test('the_built_js_stays_within_half_a_kib_gzipped_of_the_r0_baseline', { timeout: 180_000 }, async () => {
		// THE REFACTOR BAND (plan 25 §3's envelope law, renamed and re-based
		// as a named act in M3-f), and the pin measures rather than trusts: it
		// builds the client and gzips every emitted JS file with the SAME
		// routine that measured the baseline below. The band is a NAMED act to
		// move — the same shape as the kernel's SQL census.
		const { execFileSync } = await import('node:child_process');
		const { gzipSync } = await import('node:zlib');
		const { readdirSync, statSync, rmSync } = await import('node:fs');
		// THE CLEAR IS THE WHOLE GENERATED TREE (R0a, 2026-10-27): `.svelte-kit`
		// and the adapter's `build/` are shared state, and the measured defect
		// class is a SECOND build on the same checkout (an operator's
		// `pnpm build`, a second suite run) clearing `.svelte-kit/output`
		// mid-build — the loser surfaces `ENOENT: scandir
		// '.svelte-kit/output/server'` inside `vite build` (or the ENOTEMPTY
		// twin on `build`), both reproduced here by racing two builds. The
		// output-only clear this pin carried before the round provably covers
		// nothing: the kit re-runs `sync.all` and its own rm of `output`
		// inside EVERY build (drilled: with the clear neutered, a deleted
		// `output/server` still builds green). Every byte under both roots is
		// generated intermediate state, never tracked.
		const clearGenerated = (): void => {
			rmSync(join(shellRoot, '.svelte-kit'), { recursive: true, force: true });
			rmSync(join(shellRoot, 'build'), { recursive: true, force: true });
		};
		// THE BUILD MUST SEE THE LANE'S OWN ENVIRONMENT: inherited `VITEST` /
		// `NODE_ENV=test` flip vite's resolve conditions and produce a SMALLER,
		// DIFFERENT bundle (39 files / 226 KB vs 45 files / 203 KB — measured),
		// which would measure the wrong artifact. The env is stripped to what a
		// plain `pnpm build` on CI would carry.
		const cleanEnv: Record<string, string> = {};
		for (const [key, value] of Object.entries(process.env)) {
			if (value === undefined) continue;
			if (key === 'VITEST' || key === 'NODE_ENV' || key === 'VITEST_POOL_ID') continue;
			cleanEnv[key] = value;
		}
		// And run it at LOW scheduler priority: a six-second build inside a
		// vitest worker starves the other workers' quick shell-outs (one
		// pre-existing 5 s-bounded pin timed out beside it once — the R135
		// class), and `nice` is the portable way to take the edge off.
		const build = (): void => {
			execFileSync('nice', ['-n', '10', 'pnpm', 'exec', 'vite', 'build'], {
				cwd: shellRoot,
				stdio: 'pipe',
				env: cleanEnv
			});
		};
		// ONE bounded retry, and only on a thrown build: a raced sibling's rm
		// can land between this build's phases no matter what this pin clears
		// (the race is the OTHER process's buildStart). A raced build always
		// THROWS — it never completes into a corrupt tree — so the retry can
		// never measure a wrong artifact: it re-clears and rebuilds alone.
		try {
			clearGenerated();
			build();
		} catch {
			clearGenerated();
			build();
		}
		const dist = join(shellRoot, 'build', '_app');
		const jsFiles: string[] = [];
		const walk = (dir: string): void => {
			for (const entry of readdirSync(dir, { withFileTypes: true })) {
				const path = join(dir, entry.name);
				if (entry.isDirectory()) walk(path);
				else if (entry.name.endsWith('.js')) jsFiles.push(path);
			}
		};
		walk(dist);
		expect(jsFiles.length, 'the build emitted JS').toBeGreaterThan(3);
		let total = 0;
		for (const file of jsFiles) {
			if (!statSync(file).isFile()) continue;
			total += gzipSync(readFileSync(file)).length;
		}
		// Baseline measured 2026-10-26 at the pre-round tip (9f5c5be0), 45 JS
		// files, with this pin's own gzip routine (node zlib, default level):
		// 203,185 bytes gzipped. Plan 25 §3 budgeted +3 KiB; the round's
		// deliverable measured +5,067 bytes (the streaming states, the five-
		// locale failure/posture copy, the web-posture fallback core, the
		// spine), so the ceiling became the MEASUREMENT — the budget moved as
		// a NAMED ACT (the SQL-census pattern), recorded in the register.
		//
		// THE LAW IS NOW THE REFACTOR BAND (M3-f, plan 26 §5, named act): the
		// Tier A registry round is a REFACTOR — surface literals move from
		// AppShell/the chat page into `lib/extensions/**` — so the envelope
		// may move ±0.5 KiB around the R0 baseline and NOT ONE BYTE MORE, in
		// BOTH directions (a shrink is a surface silently lost, the same
		// failure a growth is). R0 baseline measured 2026-10-27 at 1effc85d
		// (the repaired pin, R0a), 45 JS files, this pin's own routine:
		// 207,772 bytes gzipped. Moving the band is a named act; the register
		// row carries it.
		//
		// NAMED ACT (M3-g, plan 27 G-3, measured 2026-10-27): the audit
		// principal filter's shell surface — the audit page's ?principal=
		// read + filter chip + clear link, the governance role-row footprint
		// link, the audit.ts wire param, and three message keys × five
		// locales — grew the built JS to 208,726 bytes gzipped (+954 over the
		// R0 midpoint; +626 over the refactor's landed 208,100). The band's
		// midpoint MOVES to the measurement, the same named act the plan-25
		// round made; the register row carries it.
		//
		// NAMED ACT (A-4, the approve-action round, C2 G-6 — the III-6
		// amendment, measured 2026-10-28): the review surface's ONE write —
		// the digest-carrying approve transport (`lib/api/approve.ts`), the
		// affordance + the rendered receipt on the live listing, the parity
		// row's named mutate, and seven message keys × five locales — grew
		// the built JS to 210,568 bytes gzipped (+1,842 over the M3-g
		// midpoint). The amendment is a BUILD round, not a refactor: the
		// surface grew by a named act, the band's midpoint moves to the
		// measurement, and the register row carries it.
		//
		// NAMED ACT (F-2, the audit-filter round, measured 2026-10-28): the
		// audit surface's `since` arm — the URL-read bound + its chip and
		// clear link, the labeled datetime input whose Apply writes the URL
		// and re-reads, the audit.ts wire param, and four message keys ×
		// five locales — grew the built JS to 211,244 bytes gzipped (+676
		// over the A-4 midpoint). The bound is a named act, the band's
		// midpoint moves to the measurement, and the register row carries it.
		//
		// NAMED ACT (LL-04 §1.4, the cite-or-abstain render, measured
		// 2026-10-28): the answer's support surface — the pure claims module
		// (`lib/chat/claims.ts`: the sentence↔claim mapping, index-resolved
		// chips), the spine's typed-final + trailing-support parse, the
		// generated arm's abstained card / provenance chips / dimmed
		// unsupported sentences / verdict line, the replayed verdict, and
		// seven message keys × five locales — grew the built JS to 213,910
		// bytes gzipped (+2,666 over the F-2 midpoint). §1.4 is a BUILD
		// round, not a refactor: the surface grew by a named act, the
		// band's midpoint moves to the measurement, and the register row
		// carries it. AND AGAIN (2026-10-30, the S-4 onboarding round):
		// the retrieval model's download surface (settings section + poll,
		// the Advanced fold, the drag-region fix, 30 message keys × five
		// locales) → 215,696 gzipped (+1,786) — the same law: a named
		// surface act moves the midpoint, the register row carries it.
		// AND AGAIN (2026-10-30, the dsh-harness UX round): the resizable
		// sidebar (the layout store, the divider gesture, the persisted
		// width preference, the settings foot) → 216,921 gzipped (+1,225)
		// — the same law: named act, midpoint moves, register carries it.
		const R0_BASELINE = 216_921;
		const BAND = 512;
		expect(
			Math.abs(total - R0_BASELINE),
			`the built JS is ${total} bytes gzipped; the refactor band holds it to ` +
				`${R0_BASELINE} ± ${BAND} — a growth or a shrink beyond the band is a surface change, not a refactor`
		).toBeLessThanOrEqual(BAND);
	});
});
