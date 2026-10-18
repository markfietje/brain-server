/**
 * II-7c — the `/verify` path: the core is the gate, the webview is the wire.
 *
 * The order is `recall.ts`'s and is not interchangeable:
 *
 *   1. `core.authorize` — the Rust core applies the bounds (the claim's CHAR
 *      ceiling after trimming, the chunk id's floor) and mints the exact body.
 *   2. `transport` — the typed generated client POSTs it. This is the only wire
 *      in the shell; `capabilities/main.json` states the webview reaches the
 *      kernel through the page's own fetch and never through Tauri.
 *   3. `core.shape` — the Rust core reads what came back and classifies it.
 *      This file never decides whether a response is a verdict, a refusal or a
 *      degraded state. That decision is the confidentiality property, so it
 *      lives where a renderer cannot reach it.
 *
 * WHY THE SIX STATES AND NOT THREE: `unsupported` is the route WORKING — the
 * kernel read the chunk and the claim is not in it — and `no_such_chunk` is a
 * permission decision the kernel deliberately refuses to distinguish from an
 * absent row. A renderer that collapses either into the other is drawing
 * something the kernel declined to assert.
 */

import { client } from './client';

/** What the caller asks for. The core owns every bound on these two fields. */
export interface VerifyRequest {
	chunk_id: number;
	claim: string;
}

/** The body the core authorized — and the only body that may be sent. */
export interface VerifyDoc {
	chunk_id: number;
	claim: string;
}

export interface AuthorizedVerify {
	doc: VerifyDoc;
	claim_chars: number;
}

/**
 * The kernel's `decision`, verbatim from the published vocabulary. Named once
 * because two modules consume it and a union written out twice is two spellings
 * one edit apart.
 */
export type VerifyDecision = 'supported' | 'unsupported_claim';

/**
 * An answer, of either verdict.
 *
 * `match_ranges` is ABSENT on the unsupported arm rather than present-and-empty,
 * for the reason `recall.ts` omits `passages` on a refusal: an empty array and
 * an absent key are different facts, and only one of them may be drawn as
 * "found at …".
 */
export interface Verdict {
	chunk_id: number;
	decision: VerifyDecision;
	supported: boolean;
	match_ranges?: [number, number][];
	claim: string;
}

/**
 * A permission decision. Deliberately carries neither `decision` nor
 * `supported`: a renderer branching on `supported` cannot reach it, and one
 * reading it gets neither a verdict nor its absence.
 */
export interface VerifyRefused {
	state: 'refused';
	status: number;
	scope: string;
}

/**
 * The kernel would not serve this chunk — a miss AND a row the caller's record
 * gate does not admit, which `openapi.yaml` states as one 404. Carries no
 * field from which existence could be read; un-blurring the kernel's refusal to
 * distinguish would rebuild the enumeration channel it closes.
 */
export interface VerifyNoSuchChunk {
	state: 'no_such_chunk';
	chunk_id: number;
	status: number;
}

/**
 * Two members rather than one interface with a two-literal `state`, for the
 * TypeScript-narrowing reason `recall.ts` records: a union only discriminates
 * when each member's discriminant is a SINGLE literal type, and with two the
 * `switch` stops excluding anything.
 */
export interface VerifyUnreachable {
	state: 'unreachable';
	status: number;
	detail: string;
}

export interface VerifyContractViolation {
	state: 'contract_violation';
	status: number;
	detail: string;
}

export type VerifyOutcome =
	| ({ state: 'supported' } & Verdict)
	| ({ state: 'unsupported' } & Verdict)
	| VerifyRefused
	| VerifyNoSuchChunk
	| VerifyUnreachable
	| VerifyContractViolation;

/** The core's IPC surface. Injected so the path is pinnable without a Tauri
 * runtime, and so a test drives the REAL command rather than a copy. */
export interface VerifyCore {
	authorize(request: VerifyRequest): Promise<AuthorizedVerify>;
	shape(exchange: { status: number; body: string }, doc: VerifyDoc): Promise<unknown>;
}

/** The wire. Injected so a pin can substitute a refusal, garbage, or a throw
 * without mocking the code under test. */
export type VerifyTransport = (doc: VerifyDoc) => Promise<{ status: number; body: string }>;

export interface VerifyDeps {
	core: VerifyCore;
	transport: VerifyTransport;
}

/** The typed wire against the generated contract. This is the production
 * transport: no hand-written endpoint shape, and the body is the doc the core
 * minted. */
export const typedVerifyTransport: VerifyTransport = async (doc) => {
	const { data, error, response } = await client.POST('/verify', { body: doc });
	const payload = data !== undefined ? data : error;
	return {
		status: response.status,
		body: payload === undefined ? '' : JSON.stringify(payload)
	};
};

/**
 * Runs one verification through the core's bounds and back through the core's
 * classification.
 *
 * The failure arms are the point of the signature: a transport that throws, a
 * core that refuses, a core that returns a state this client does not know —
 * each becomes a NAMED state. None may become an unsupported verdict, because
 * "the kernel could not be reached" and "the claim is not in that chunk" are
 * different facts and conflating them tells the reader their citation was
 * checked when it was not.
 */
export async function verifyClaim(
	request: VerifyRequest,
	deps: VerifyDeps
): Promise<VerifyOutcome> {
	const authorized = await deps.core.authorize(request);

	let exchange: { status: number; body: string };
	try {
		exchange = await deps.transport(authorized.doc);
	} catch (error) {
		return {
			state: 'unreachable',
			status: 0,
			detail: `the verify transport failed: ${error instanceof Error ? error.message : String(error)}`
		};
	}

	const shaped = await deps.core.shape(exchange, authorized.doc);
	return assertVerifyOutcome(shaped);
}

/**
 * Validates that the core returned a state this client knows.
 *
 * There is deliberately NO fallback. A `?? 'unreachable'` here would turn a core
 * that grew or broke a state into a silently wrong verdict, and the wrong
 * verdict on this route is a claim about whether the kernel's text supports a
 * sentence.
 */
export function assertVerifyOutcome(value: unknown): VerifyOutcome {
	if (typeof value !== 'object' || value === null) {
		throw new Error(`verify: the core returned ${typeof value}, not an outcome`);
	}
	const state = (value as { state?: unknown }).state;
	switch (state) {
		case 'supported':
		case 'unsupported': {
			const verdict = value as { state: 'supported' | 'unsupported' } & Verdict;
			const expected: VerifyDecision =
				state === 'supported' ? 'supported' : 'unsupported_claim';
			if (verdict.decision !== expected) {
				throw new Error(`verify: ${state} carried decision ${String(verdict.decision)}`);
			}
			if (typeof verdict.supported !== 'boolean' || verdict.supported !== (state === 'supported')) {
				throw new Error(`verify: ${state} carried supported=${String(verdict.supported)}`);
			}
			if (typeof verdict.chunk_id !== 'number') {
				throw new Error(`verify: ${state} carried no chunk_id`);
			}
			// The core's own invariant, re-checked on this side: `supported`
			// holds iff ranges were reported. A verdict claiming support with no
			// ranges is the one body a surface would render as "found at …".
			const ranges = verdict.match_ranges;
			if (state === 'supported') {
				if (!Array.isArray(ranges) || ranges.length === 0) {
					throw new Error('verify: a supported verdict carried no match ranges');
				}
			} else if (ranges !== undefined) {
				throw new Error(
					'verify: an unsupported verdict must carry no match ranges — the kernel reported none'
				);
			}
			return verdict as VerifyOutcome;
		}
		case 'refused': {
			const refused = value as VerifyRefused;
			if (typeof refused.status !== 'number') {
				throw new Error('verify: a refusal carried no status');
			}
			// Closed at the type level: a refusal that also carried `supported`
			// could be drawn as a verdict by a renderer that checks the flag.
			if ('supported' in refused) {
				throw new Error('verify: a refusal must not carry a verdict');
			}
			return refused;
		}
		case 'no_such_chunk': {
			const missing = value as VerifyNoSuchChunk;
			if (typeof missing.chunk_id !== 'number' || typeof missing.status !== 'number') {
				throw new Error('verify: a no_such_chunk was malformed');
			}
			if ('supported' in missing) {
				throw new Error('verify: a no_such_chunk must not carry a verdict');
			}
			return missing;
		}
		case 'unreachable':
		case 'contract_violation': {
			const failure = value as VerifyUnreachable;
			if (typeof failure.status !== 'number' || typeof failure.detail !== 'string') {
				throw new Error(`verify: ${String(state)} was malformed`);
			}
			if ('supported' in failure) {
				throw new Error(`verify: ${String(state)} must not carry a verdict`);
			}
			return failure;
		}
		default:
			throw new Error(`verify: the core returned an unknown state ${String(state)}`);
	}
}

/** Did the kernel FIND the claim? Deliberately NOT "was it refused?" — an
 * unsupported verdict and a refusal are the two states a confused renderer
 * collapses, and they must stay separable. */
export function isSupported(outcome: VerifyOutcome): boolean {
	return outcome.state === 'supported';
}

/** True for the states that must never be drawn as a verdict. */
export function isRefusal(outcome: VerifyOutcome): boolean {
	return outcome.state === 'refused' || outcome.state === 'no_such_chunk';
}

/**
 * The ranges — or a THROW.
 *
 * This is the accessor a renderer is expected to use, and it makes the
 * conflation a crash rather than a blank list. A caller that genuinely wants
 * "nothing to point at" branches on `outcome.state` and supplies its own copy.
 */
export function matchRanges(outcome: VerifyOutcome): [number, number][] {
	if (outcome.state === 'refused') {
		throw new Error(
			`verify: refused with ${outcome.status} on ${outcome.scope} — this is not an unsupported claim`
		);
	}
	if (
		outcome.state === 'no_such_chunk' ||
		outcome.state === 'unreachable' ||
		outcome.state === 'contract_violation'
	) {
		throw new Error(`verify: ${outcome.state} (${outcome.status}) — this is not an unsupported claim`);
	}
	if (outcome.state === 'unsupported') {
		throw new Error('verify: the kernel reported no support — there are no ranges to show');
	}
	return outcome.match_ranges ?? [];
}
