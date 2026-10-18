/**
 * P1 — the recall path: the core is the gate, the webview is the wire.
 *
 * The order of these three calls is the whole design, and it is not
 * interchangeable:
 *
 *   1. `core.authorize` — the Rust core applies the bounds (query length,
 *      result count, domain label) and mints the exact `QueryDoc` to send.
 *      Nothing downstream is allowed to add a field.
 *   2. `transport` — the typed generated client POSTs it. This is the only
 *      wire in the shell; `capabilities/main.json` states the webview reaches
 *      the kernel through the page's own fetch and never through Tauri, and
 *      no http plugin is installed.
 *   3. `core.shape` — the Rust core reads what came back and classifies it.
 *      The webview never decides whether a response is an answer, an
 *      abstention, an absence or a refusal. That decision is the whole
 *      confidentiality property, so it lives where a renderer cannot reach it.
 *
 * WHY THE CORE RECEIVES A RE-SERIALIZATION, not the kernel's raw bytes:
 * `openapi-fetch` consumes the body itself — for a chunked success it calls
 * `response.text()`, and for a failure `response.text()` again — so the
 * `response` it returns has already had its body used and cannot be re-read.
 * The one seam that does see raw bytes is `captureNextRawText`, and it is a
 * single module-level one-shot sink: two overlapping recalls would race for it
 * and could hand each other's bodies to the core, which would render one
 * query's rows as another query's answer. A data-integrity hazard is a worse
 * defect than a re-serialization, so this path uses the typed parse and the
 * core reads that. The honest consequence is recorded here rather than papered
 * over: the core validates the kernel's JSON *values*, not its byte framing.
 */

import { client } from './client';

/** What the caller asks for. The core owns every bound on these three fields. */
export interface RecallRequest {
	query: string;
	domain?: string;
	limit?: number;
}

/** The `QueryDoc` the core authorized — and the only body that may be sent. */
export interface AuthorizedRecall {
	doc: {
		v: number;
		query: string;
		domain?: string;
		limit: number;
		provenance: boolean;
	};
	query_bytes: number;
}

/** One passage as the core projects it. `valid_to` is a first-class field, not
 * something a renderer has to dig out of an evidence object — presenting an
 * expired fact as current is the failure this kernel exists to prevent. */
export interface Passage {
	id: number;
	title: string | null;
	text: string;
	snippet: string | null;
	domain: string | null;
	score: number;
	valid_from: string | null;
	valid_to: string | null;
	flagged: boolean;
	ingest_kind: string | null;
	memory_kind: string | null;
	authority: number | null;
}

export interface Searched {
	query: string;
	domain: string | null;
	limit: number;
	domains_searched: string[];
	included_global: boolean | null;
}

/**
 * The three answer-shaped states. `decision` is `ok` or `low_confidence` —
 * the kernel's published vocabulary, passed through verbatim. Plan 04 §2
 * writes `"answered" | "low_confidence"`; the published enum is
 * `[ok, low_confidence]` and this client does not invent a third spelling.
 */
export interface RecallAnswer {
	state: 'answered' | 'absent' | 'abstained';
	decision: 'ok' | 'low_confidence';
	passages: Passage[];
	confidence: number | null;
	searched: Searched;
}

/** A refusal. Note there is NO `passages` key and no `decision` key: the core
 * omits both, so a renderer branching on `passages.length` cannot reach a
 * refusal at all, and one branching on `decision` reads it as neither answered
 * nor abstained. */
export interface RecallRefused {
	state: 'refused';
	status: number;
	scope: string;
}

/**
 * A kernel that did not answer, and a 2xx that did not match the contract.
 *
 * These are TWO members rather than one interface with a two-literal `state`,
 * and the reason is a compiler fact worth recording: TypeScript narrows a
 * discriminated union only when each member's discriminant is a SINGLE literal
 * type. With `state: 'unreachable' | 'contract_violation'` on one member, the
 * union stops being discriminated, `outcome.state === 'unreachable'` no longer
 * excludes anything, and `recallPassages` cannot prove it is not looking at a
 * failure. Splitting them makes the narrowing work — so the compiler, not a
 * cast, is what guarantees the failure arms cannot fall through to
 * `return outcome.passages`. Neither may ever be drawn as "no results".
 */
export interface RecallUnreachable {
	state: 'unreachable';
	status: number;
	detail: string;
}

export interface RecallContractViolation {
	state: 'contract_violation';
	status: number;
	detail: string;
}

export type RecallOutcome =
	| RecallAnswer
	| RecallRefused
	| RecallUnreachable
	| RecallContractViolation;

/** The core's IPC surface. Injected so the path is pinnable without a Tauri
 * runtime, and so a test can drive the REAL `recallMemory` rather than a copy. */
export interface RecallCore {
	authorize(request: RecallRequest): Promise<AuthorizedRecall>;
	shape(
		exchange: { status: number; body: string },
		doc: AuthorizedRecall['doc']
	): Promise<unknown>;
}

/** The wire. Injected for the same reason, and so a pin can substitute a
 * refusal, a garbage body, or a throw without mocking the code under test. */
export type RecallTransport = (doc: AuthorizedRecall['doc']) => Promise<{
	status: number;
	body: string;
}>;

export interface RecallDeps {
	core: RecallCore;
	transport: RecallTransport;
}

/** The typed wire against the generated contract. This is the production
 * transport: no hand-written endpoint shape, and the request body is the doc
 * the core minted. */
export const typedRecallTransport: RecallTransport = async (doc) => {
	const { data, error, response } = await client.POST('/recall', { body: doc });
	// A refusal arrives as `error` with a non-2xx status; both bodies are handed
	// on verbatim because the core — not this file — decides what they mean.
	const payload = data !== undefined ? data : error;
	return {
		status: response.status,
		body: payload === undefined ? '' : JSON.stringify(payload)
	};
};

/**
 * Runs one recall through the core's bounds and back through the core's
 * classification.
 *
 * The failure arms are the point of the signature: a transport that throws, a
 * core that refuses, a core that returns a state this client does not know —
 * each becomes a NAMED state. None of them may become an empty list, because
 * an empty list that means "you are not allowed" is indistinguishable from
 * "there is nothing here", and that is a confidentiality leak through the UI
 * (Plan 04 §5).
 */
export async function recallMemory(
	request: RecallRequest,
	deps: RecallDeps
): Promise<RecallOutcome> {
	const authorized = await deps.core.authorize(request);

	let exchange: { status: number; body: string };
	try {
		exchange = await deps.transport(authorized.doc);
	} catch (error) {
		// The kernel is unreachable. Degraded recall with a clear banner, never
		// a silent empty result — so this is `unreachable`, not `absent`.
		return {
			state: 'unreachable',
			status: 0,
			detail: `the recall transport failed: ${error instanceof Error ? error.message : String(error)}`
		};
	}

	const shaped = await deps.core.shape(exchange, authorized.doc);
	return assertOutcome(shaped);
}

/**
 * Validates that the core returned a state this client knows, and that an
 * answer-shaped state actually carries a decision.
 *
 * This exists because an unrecognised shape must not be defaulted. A `??`
 * fallback here would turn a core that grew or broke a state into a silently
 * empty answer — the exact leak the taxonomy exists to prevent.
 */
export function assertOutcome(value: unknown): RecallOutcome {
	if (typeof value !== 'object' || value === null) {
		throw new Error(`recall: the core returned ${typeof value}, not an outcome`);
	}
	const state = (value as { state?: unknown }).state;
	switch (state) {
		case 'answered':
		case 'absent':
		case 'abstained': {
			const answer = value as RecallAnswer;
			if (answer.decision !== 'ok' && answer.decision !== 'low_confidence') {
				throw new Error(`recall: ${state} carried decision ${String(answer.decision)}`);
			}
			if (!Array.isArray(answer.passages)) {
				throw new Error(`recall: ${state} carried no passages array`);
			}
			return answer;
		}
		case 'refused': {
			const refused = value as RecallRefused;
			if (typeof refused.status !== 'number') {
				throw new Error('recall: a refusal carried no status');
			}
			// The leak, closed at the type level: a refusal that also carried
			// passages could be drawn as an answer by a renderer that checks the
			// array before the state.
			if ('passages' in refused) {
				throw new Error('recall: a refusal must not carry passages');
			}
			return refused;
		}
		case 'unreachable':
		case 'contract_violation': {
			const failure = value as RecallUnreachable;
			if (typeof failure.status !== 'number' || typeof failure.detail !== 'string') {
				throw new Error(`recall: ${String(state)} was malformed`);
			}
			if ('passages' in failure) {
				throw new Error(`recall: ${String(state)} must not carry passages`);
			}
			return failure;
		}
		default:
			throw new Error(`recall: the core returned an unknown state ${String(state)}`);
	}
}

/** Did we get passages? Deliberately NOT "was it refused?" — an abstention also
 * has none, and the two must stay separable by the caller. */
export function hasPassages(outcome: RecallOutcome): boolean {
	return 'passages' in outcome && outcome.passages.length > 0;
}

/** True only for the state that must never be drawn as "no results". */
export function isRefusal(outcome: RecallOutcome): boolean {
	return outcome.state === 'refused';
}

/**
 * The passages — or a THROW when there are none because the kernel refused.
 *
 * This is the accessor a renderer is expected to use, and it exists to make
 * the confidentiality leak a crash rather than a blank list. A caller that
 * genuinely wants "nothing to show" branches on `outcome.state` and supplies
 * its own copy; a caller that reaches here on a refusal has a bug.
 */
export function recallPassages(outcome: RecallOutcome): Passage[] {
	if (outcome.state === 'refused') {
		throw new Error(
			`recall: refused with ${outcome.status} on ${outcome.scope} — this is not an absence`
		);
	}
	if (outcome.state === 'unreachable' || outcome.state === 'contract_violation') {
		throw new Error(`recall: ${outcome.state} (${outcome.status}) — this is not an absence`);
	}
	return outcome.passages;
}
