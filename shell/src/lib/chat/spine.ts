/**
 * M3-d / plan 25 §2 — the chat spine's consumer: the ONE new wire module.
 *
 * ## WHY THIS MODULE EXISTS, AND WHY IT IS NOT `client.ts`
 *
 * `POST /chat/turns` answers `text/event-stream`, and openapi-fetch cannot
 * stream a response — it consumes the body to classify it. So the spine rides
 * `fetch` DIRECTLY, with the SAME origin/bearer discipline as the typed
 * client: `apiBase()` (loopback-enforced, settings-overridable), the memoized
 * bootstrap bearer awaited BEFORE the first byte leaves (R130's
 * await-before-first-read law), and `authHeaders()`. There is deliberately no
 * new dependency and no `EventSource` — EventSource can neither POST nor
 * carry an Authorization header — so the SSE parser below is hand-rolled over
 * `TextDecoder` and pinned frame by frame in `tests/chat-spine.test.ts`.
 *
 * ## THE WIRE, EXACTLY AS THE KERNEL EMITS IT (measured, `src/handlers/chat.rs`)
 *
 *   event: grounding  data: {"decision":"ok","hits":[{id,title,score,domain,flagged}]}
 *   event: delta      data: {"text":"…","replace":false}      (zero or more)
 *   event: final      data: {"text":"…","tier":"on_device","turn":n,…}   (terminal)
 *   event: error      data: {"code":"…","message":"…","shortfallTokens":n}  (terminal)
 *
 * Between events, axum's `KeepAlive` emits comment-only frames (`: …`), which
 * the parser tolerates and never counts as malformed. Between reads, frames
 * arrive in arbitrary fragments — the parser is a state machine over chunk
 * strings, never a per-read regex.
 *
 * ## THE LAWS (each one a pin; the drills are named in the pin file)
 *
 * 1. **Malformed frames are skipped + counted**, never fatal below
 *    `MAX_MALFORMED_FRAMES`; over the cap the stream ends as
 *    `contract_violation` — the six-state taxonomy's own name, because a
 *    kernel that mostly cannot be read has broken the contract, not the
 *    network.
 * 2. **No artificial client timeout.** Keep-alive comments prove liveness;
 *    the user's Stop is the only early abort. A watchdog here would kill long
 *    generations on the slow decode tier.
 * 3. **`replace: true` is a wholesale revision** — the bridge's own semantic.
 *    The parser carries the flag; the delta buffer (below) does the discard.
 * 4. **The terminal is terminal.** After `final` or `error` the consumer
 *    cancels the body; a further complete frame is ignored and counted.
 * 5. **A refusal is an ANSWER, never a fault** (the M3-b trap, restated for
 *    streams): a non-2xx before the stream dispatches `onRefusal` with the
 *    named posture when it is one of the TWO the fallback law covers (plan 25
 *    D1: 503 `chat_bridge_unavailable`, 400 `locale_not_supported`). The
 *    kernel emits the 503 as code `recall_unavailable` with the marker on the
 *    MESSAGE prefix — measured off `src/handlers/chat.rs:137` — so the table
 *    accepts both spellings.
 * 6. **A stream that ends in `error:` is not a wire fault** — it renders ON
 *    THE TURN. Only a mid-stream transport failure (a rejected read) reaches
 *    the fault path, which is the banner's.
 */

import { apiBase } from '../api/client.js';
import { authHeaders } from '../api/token.js';
import { bootstrapToken } from '../api/bootstrap.js';

// ── the events (parsed, never trusted) ───────────────────────────────────

/** One grounding hit, as the kernel's grounding event carries it. */
export interface SpineHit {
	readonly id: number;
	readonly title: string | null;
	readonly score: number;
	readonly domain: string | null;
	readonly flagged: boolean;
}

export type SpineEvent =
	| { kind: 'grounding'; decision: string; hits: SpineHit[] }
	| { kind: 'delta'; text: string; replace: boolean }
	| {
			kind: 'final';
			text: string;
			tier: string | null;
			turn: number | null;
			inputTokens: number | null;
			outputTokens: number | null;
	  }
	| { kind: 'error'; code: string; message: string | null; shortfallTokens: number | null };

/**
 * How many unreadable frames a stream may carry before the WHOLE stream is
 * treated as a contract violation. Below the cap, garbage is skipped so one
 * bad frame cannot eat a good answer; over it, the kernel is not speaking the
 * contract and the answer would be assembled from fragments.
 */
export const MAX_MALFORMED_FRAMES = 10;

/** A hit this module can prove is a hit — id and score are the load-bearing
 * fields (they become the citation); everything else is normalized, never
 * trusted. */
function parseHit(value: unknown): SpineHit | null {
	if (typeof value !== 'object' || value === null) return null;
	const record = value as Record<string, unknown>;
	const id = record['id'];
	const score = record['score'];
	if (typeof id !== 'number' || typeof score !== 'number') return null;
	const title = record['title'];
	const domain = record['domain'];
	return {
		id,
		score,
		title: typeof title === 'string' ? title : null,
		domain: typeof domain === 'string' ? domain : null,
		flagged: record['flagged'] === true
	};
}

/** Validate one frame's payload against its event name. Returns null for a
 * payload the event name cannot lawfully carry — the caller counts it
 * malformed and moves on. */
function parsePayload(eventName: string, data: string): SpineEvent | null {
	let body: unknown;
	try {
		body = JSON.parse(data);
	} catch {
		return null;
	}
	if (typeof body !== 'object' || body === null || Array.isArray(body)) return null;
	const record = body as Record<string, unknown>;
	switch (eventName) {
		case 'grounding': {
			const decision = record['decision'];
			const hits = record['hits'];
			if (typeof decision !== 'string' || !Array.isArray(hits)) return null;
			const parsed: SpineHit[] = [];
			for (const hit of hits) {
				const one = parseHit(hit);
				if (one === null) return null;
				parsed.push(one);
			}
			return { kind: 'grounding', decision, hits: parsed };
		}
		case 'delta': {
			const text = record['text'];
			if (typeof text !== 'string') return null;
			return { kind: 'delta', text, replace: record['replace'] === true };
		}
		case 'final': {
			const text = record['text'];
			if (typeof text !== 'string') return null;
			const tier = record['tier'];
			const turn = record['turn'];
			const inputTokens = record['inputTokens'];
			const outputTokens = record['outputTokens'];
			return {
				kind: 'final',
				text,
				tier: typeof tier === 'string' ? tier : null,
				turn: typeof turn === 'number' ? turn : null,
				inputTokens: typeof inputTokens === 'number' ? inputTokens : null,
				outputTokens: typeof outputTokens === 'number' ? outputTokens : null
			};
		}
		case 'error': {
			const code = record['code'];
			if (typeof code !== 'string' || code.length === 0) return null;
			const message = record['message'];
			const shortfallTokens = record['shortfallTokens'];
			return {
				kind: 'error',
				code,
				message: typeof message === 'string' ? message : null,
				shortfallTokens: typeof shortfallTokens === 'number' ? shortfallTokens : null
			};
		}
		default:
			return null;
	}
}

// ── the parser: a state machine over chunk strings ───────────────────────

export interface SpineParser {
	/** Feed one decoded chunk; returns every event COMPLETED by it, in order. */
	push(chunk: string): SpineEvent[];
	/** End-of-stream: completes the pending line and dispatches a frame the
	 * stream closed without a terminating blank line. */
	end(): SpineEvent[];
	/** Unreadable frames skipped (never emitted). */
	readonly malformed: number;
	/** Complete frames dropped after the terminal. */
	readonly terminalOverflow: number;
	/** True once the terminal event has been emitted. */
	readonly closed: boolean;
}

interface PendingFrame {
	eventName: string | null;
	dataLines: string[];
}

export function createSpineParser(): SpineParser {
	let buffer = '';
	let malformed = 0;
	let terminalOverflow = 0;
	let closed = false;
	let pending: PendingFrame = { eventName: null, dataLines: [] };
	// A chunk may end with `\r` whose `\n` is in the NEXT chunk; remembering
	// the CR is what makes a CRLF split across reads parse.
	let sawCarriageReturn = false;

	const dispatch = (out: SpineEvent[]): void => {
		const frame = pending;
		pending = { eventName: null, dataLines: [] };
		// A comment-only or entirely empty frame (the keep-alive shape) is
		// not malformed — it is liveness.
		if (frame.eventName === null && frame.dataLines.length === 0) return;
		if (closed) {
			terminalOverflow += 1;
			return;
		}
		if (frame.eventName === null || frame.dataLines.length === 0) {
			malformed += 1;
			return;
		}
		const event = parsePayload(frame.eventName, frame.dataLines.join('\n'));
		if (event === null) {
			malformed += 1;
			return;
		}
		if (event.kind === 'final' || event.kind === 'error') closed = true;
		out.push(event);
	};

	const processLine = (line: string, out: SpineEvent[]): void => {
		if (line.length === 0) {
			dispatch(out);
			return;
		}
		if (line.startsWith(':')) return; // comment — liveness, never data
		const colon = line.indexOf(':');
		const field = colon === -1 ? line : line.slice(0, colon);
		let value = colon === -1 ? '' : line.slice(colon + 1);
		if (value.startsWith(' ')) value = value.slice(1);
		if (field === 'event') pending.eventName = value;
		else if (field === 'data') pending.dataLines.push(value);
		// `id`/`retry`/unknown fields are legal SSE and irrelevant here.
	};

	return {
		push(chunk: string): SpineEvent[] {
			const out: SpineEvent[] = [];
			if (sawCarriageReturn && chunk.startsWith('\n')) {
				// The `\n` half of a `\r\n` split across reads: consumed silently.
				chunk = chunk.slice(1);
			}
			sawCarriageReturn = false;
			buffer += chunk;
			let index = 0;
			while (index < buffer.length) {
				const char = buffer[index];
				if (char === '\r') {
					if (index + 1 === buffer.length) {
						// Possibly half of a CRLF: cut the line here, remember the CR,
						// and let the next push resolve the ambiguity.
						processLine(buffer.slice(0, index), out);
						buffer = '';
						sawCarriageReturn = true;
						return out;
					}
					processLine(buffer.slice(0, index), out);
					const advance = buffer[index + 1] === '\n' ? 2 : 1;
					buffer = buffer.slice(index + advance);
					index = 0;
					continue;
				}
				if (char === '\n') {
					processLine(buffer.slice(0, index), out);
					buffer = buffer.slice(index + 1);
					index = 0;
					continue;
				}
				index += 1;
			}
			return out;
		},
		end(): SpineEvent[] {
			const out: SpineEvent[] = [];
			if (buffer.length > 0) {
				// A stream that closes mid-line still owes that line its dispatch.
				processLine(buffer, out);
				buffer = '';
			}
			dispatch(out);
			return out;
		},
		get malformed() {
			return malformed;
		},
		get terminalOverflow() {
			return terminalOverflow;
		},
		get closed() {
			return closed;
		}
	};
}

// ── the D1 fallback table (pure; the page's decision, pinned) ────────────

/** The TWO named postures the fallback law covers (plan 25 §4, D1). Every
 * other refusal carries NO posture and renders as the refusal it is. */
export type SpinePosture = 'bridge_unavailable' | 'locale_not_supported';

export function postureOfRefusal(
	status: number,
	code: string,
	message: string | null
): SpinePosture | null {
	if (
		status === 503 &&
		(code === 'chat_bridge_unavailable' ||
			(message !== null && message.startsWith('chat_bridge_unavailable')))
	) {
		return 'bridge_unavailable';
	}
	if (code === 'locale_not_supported') return 'locale_not_supported';
	return null;
}

// ── the delta buffer: rAF-gated accumulation (plan 25 §3) ────────────────

export type FrameScheduler = (callback: () => void) => void;

/** The production scheduler: one flush per animation frame. */
export const rafScheduler: FrameScheduler = (callback) => {
	requestAnimationFrame(() => callback());
};

export interface DeltaBuffer {
	accept(text: string, replace: boolean): void;
	/** Coalesces everything buffered NOW — a final/error must not wait for a
	 * frame to show the text it supersedes. */
	flushNow(): void;
	/** Drops the buffer and disarms the scheduled flush (the abort path). */
	discard(): void;
	/** Everything committed so far (what a Stop keeps on the turn). */
	readonly committed: string;
}

export function createDeltaBuffer(
	onFlush: (committed: string) => void,
	schedule: FrameScheduler = rafScheduler
): DeltaBuffer {
	let committed = '';
	let buffered = '';
	let scheduled = false;
	// A discarded buffer must not fire a flush already handed to the
	// scheduler — the generation counter disarms stale callbacks without
	// needing a cancellable handle (rAF has none).
	let generation = 0;

	const run = (): void => {
		if (!scheduled) return;
		scheduled = false;
		if (buffered.length === 0) return;
		committed += buffered;
		buffered = '';
		onFlush(committed);
	};

	return {
		accept(text: string, replace: boolean): void {
			if (replace) {
				// THE REVISION SEMANTIC: a `replace` delta discards the accumulated
				// text — BOTH what is committed and what is still buffered.
				// Appending blindly is the silent-corruption defect (plan 25
				// READ FIRST ④).
				committed = '';
				buffered = text;
			} else {
				buffered += text;
			}
			if (!scheduled) {
				scheduled = true;
				const armed = generation;
				schedule(() => {
					if (armed !== generation) return;
					run();
				});
			}
		},
		flushNow(): void {
			committed += buffered;
			buffered = '';
			onFlush(committed);
		},
		discard(): void {
			generation += 1;
			scheduled = false;
			buffered = '';
		},
		get committed() {
			return committed;
		}
	};
}

// ── the consumer: fetch + the handler contract ───────────────────────────

export interface SpineRequest {
	readonly message: string;
	/** The active session key (plan 23). Absent = the kernel's "default". */
	readonly session?: string;
	/** Primary BCP-47 tag; the kernel fences generation against the model's
	 * MEASURED roster and refuses by name (`locale_not_supported`). */
	readonly locale?: string;
	/** 16..=1024 on the wire (`max_response_tokens`); the kernel clamps again. */
	readonly maxResponseTokens?: number;
}

export interface SpineFinal {
	readonly text: string;
	readonly tier: string | null;
	readonly turn: number | null;
	readonly inputTokens: number | null;
	readonly outputTokens: number | null;
}

export interface SpineErrorEvent {
	readonly code: string;
	readonly message: string | null;
	readonly shortfallTokens: number | null;
}

export interface SpineRefusal {
	readonly status: number;
	/** The envelope's own code, verbatim (may be '' on a non-JSON body). */
	readonly code: string;
	/** The named posture when this refusal is one of the TWO the fallback law
	 * covers — else null. The page renders `null` as the refusal it is. */
	readonly posture: SpinePosture | null;
	readonly message: string | null;
}

export interface SpineHandlers {
	onGrounding(hits: readonly SpineHit[], decision: string): void;
	onDelta(text: string, replace: boolean): void;
	onFinal(final: SpineFinal): void;
	/** A NAMED failure on the stream — renders ON THE TURN, never the banner. */
	onStreamError(error: SpineErrorEvent): void;
	/** A pre-stream refusal — the kernel SPOKE and said no. */
	onRefusal(refusal: SpineRefusal): void;
	/** A mid-stream transport failure — the wire died. The banner's path. */
	onTransportFault(detail: string): void;
}

export interface SpineTurn {
	abort(): void;
	/** Resolves when the turn is over for ANY reason — final, error, refusal,
	 * fault, or Stop. Handlers are silent from the resolution onward. */
	done: Promise<void>;
}

function isAbort(error: unknown): boolean {
	return (
		typeof error === 'object' &&
		error !== null &&
		'name' in error &&
		(error as { name?: unknown }).name === 'AbortError'
	);
}

function detailOf(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function parseRefusalBody(text: string): { code: string; message: string | null } {
	try {
		const parsed: unknown = JSON.parse(text);
		if (typeof parsed === 'object' && parsed !== null && 'error' in parsed) {
			const error = (parsed as { error: unknown }).error;
			if (typeof error === 'object' && error !== null) {
				const code = (error as { code?: unknown }).code;
				const message = (error as { message?: unknown }).message;
				return {
					code: typeof code === 'string' ? code : '',
					message: typeof message === 'string' ? message : null
				};
			}
		}
	} catch {
		// A non-JSON refusal body: the status is the fact.
	}
	return { code: '', message: null };
}

/**
 * One chat turn over the spine. The handlers fire in wire order; after
 * `abort()` none of them fire again, and `done` resolves without an error —
 * a Stop is a user act, not a failure.
 */
export function turnSpine(request: SpineRequest, handlers: SpineHandlers): SpineTurn {
	const controller = new AbortController();
	let aborted = false;

	const done = (async (): Promise<void> => {
		let response: Response;
		try {
			// The credential is a PRECONDITION of the first byte, not a race
			// with it (R130).
			await bootstrapToken();
			response = await fetch(`${apiBase()}/chat/turns`, {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...authHeaders() },
				body: JSON.stringify({
					message: request.message,
					...(request.session !== undefined ? { session: request.session } : {}),
					...(request.locale !== undefined && request.locale.length > 0
						? { locale: request.locale }
						: {}),
					...(request.maxResponseTokens !== undefined
						? { max_response_tokens: request.maxResponseTokens }
						: {})
				}),
				signal: controller.signal
			});
		} catch (error) {
			if (aborted || isAbort(error)) return; // Stop is not a fault
			handlers.onTransportFault(detailOf(error));
			return;
		}

		if (!response.ok) {
			// THE REFUSAL IS AN ANSWER. The kernel spoke; classify by the table.
			const body = await response.text().catch(() => '');
			const { code, message } = parseRefusalBody(body);
			handlers.onRefusal({
				status: response.status,
				code,
				posture: postureOfRefusal(response.status, code, message),
				message
			});
			return;
		}

		const body = response.body;
		if (body === null) {
			handlers.onTransportFault('the chat stream has no body');
			return;
		}

		const parser = createSpineParser();
		const reader = body.getReader();
		const decoder = new TextDecoder();

		const dispatch = (event: SpineEvent): boolean => {
			// Returns false when the turn is over — the terminal is terminal. The
			// handlers receive the DTO, not the parsed frame: the page branches on
			// WHICH handler ran, never on a second discriminant.
			switch (event.kind) {
				case 'grounding':
					handlers.onGrounding(event.hits, event.decision);
					return true;
				case 'delta':
					handlers.onDelta(event.text, event.replace);
					return true;
				case 'final':
					handlers.onFinal({
						text: event.text,
						tier: event.tier,
						turn: event.turn,
						inputTokens: event.inputTokens,
						outputTokens: event.outputTokens
					});
					return false;
				case 'error':
					handlers.onStreamError({
						code: event.code,
						message: event.message,
						shortfallTokens: event.shortfallTokens
					});
					return false;
			}
		};

		try {
			while (true) {
				const { value, done: finished } = await reader.read();
				if (finished) break;
				for (const event of parser.push(decoder.decode(value, { stream: true }))) {
					if (!dispatch(event)) {
						void reader.cancel().catch(() => {});
						return;
					}
				}
				if (parser.malformed > MAX_MALFORMED_FRAMES) {
					void reader.cancel().catch(() => {});
					handlers.onStreamError({
						code: 'contract_violation',
						message: `the stream carried more than ${MAX_MALFORMED_FRAMES} unreadable frames`,
						shortfallTokens: null
					});
					return;
				}
			}
			// End of stream: dispatch what the close completed, then hold the
			// kernel to the contract's "exactly one terminal" clause.
			for (const event of parser.end()) {
				if (!dispatch(event)) return;
			}
			if (!parser.closed) {
				handlers.onStreamError({
					code: 'contract_violation',
					message: 'the stream ended without a terminal frame',
					shortfallTokens: null
				});
			}
		} catch (error) {
			if (aborted || isAbort(error)) return; // Stop is not a fault
			handlers.onTransportFault(detailOf(error));
		}
	})();

	return {
		abort(): void {
			aborted = true;
			controller.abort();
		},
		done
	};
}
