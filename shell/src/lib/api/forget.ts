/**
 * II-7c — the `/ump/forget` path: the core is the gate, the webview is the
 * wire, and the shell holds Write scope only.
 *
 * The order is `recall.ts`'s and is not interchangeable:
 *
 *   1. `core.authorize` — the Rust core bounds the id and REFUSES `hard`
 *      outright. Nothing downstream may add a field, and there is nothing to
 *      add: the authorized body is `{id}` alone.
 *   2. `transport` — the typed generated client POSTs it.
 *   3. `core.shape` — the Rust core classifies the answer. This file never
 *      decides whether a row was tombstoned, refused, or degraded.
 *
 * ## WHY THERE IS NO `hard` PARAMETER ANYWHERE IN THIS FILE
 *
 * `POST /ump/forget` accepts `hard`, and `hard: true` is the `/purge`
 * destructive act: the kernel gates it on Admin scope AND the `purge` role
 * (`cap_gate("admin")`, `Action::Admin`, `authorize_role(p, pool, "purge")`),
 * and a capability bearer can never reach it. This shell holds Write scope.
 *
 * A `hard?: boolean` on `ForgetRequest` would put that asymmetry one careless
 * default away from gone, so the flag does not exist here: the ONLY way to
 * express it is to build the request object with the field, hand it to the
 * core, and receive a `HardRefused` refusal that names why. There is no code
 * path in this module that could set it — which is a stronger statement than
 * "we remember not to", and `forget-wire.test.ts` proves it by construction.
 */

import { client } from './client';

/**
 * What the caller may ask for.
 *
 * `hard` is deliberately NOT a member. It is declared in the CORE's request
 * type solely to be refused, and the reason is that a boolean a caller can flip
 * is not a boundary. A caller who wants the erase path sends an object literal
 * with the field set and is told, by name, what it would have required.
 */
export interface ForgetRequest {
	id: string;
}

/** The body the core authorized — and the only body that may be sent. */
export interface ForgetDoc {
	id: string;
}

export interface AuthorizedForget {
	doc: ForgetDoc;
}

/** The kernel's `result`, verbatim. `erased` is unreachable through this path —
 * the core refuses the request that could produce it — and the projection
 * treats a reply carrying it as a contract violation rather than a success. */
export type ForgetResult = 'tombstoned' | 'erased';

/** What the kernel confirmed.
 *
 * `dsar_certificate` is present-and-null rather than absent: `/ump/forget`
 * returns exactly `{"result":"tombstoned"}` and emits no certificate.
 * Certificates belong to `POST /dsar` — the Admin-only, subject-addressed
 * Art 15/17 workflow that erases a whole subject rather than one citation. A
 * null here is a stated fact, so a receipt cannot be built from this outcome by
 * forgetting a field. */
export interface Tombstoned {
	state: 'tombstoned';
	id: string;
	result: ForgetResult;
	dsar_certificate: null;
}

/** A permission decision. A refusal is a RESULT: the row was NOT erased. */
export interface ForgetRefused {
	state: 'refused';
	status: number;
	scope: string;
}

/** The kernel would not serve this row — `openapi.yaml` states this 404 covers
 * a missing id AND a target row the caller's record gate does not admit, one
 * status for both. Carries no field from which existence could be read. */
export interface NoSuchRow {
	state: 'no_such_row';
	id: string;
	status: number;
}

/** Degraded, never silently not-done. A legal hold's 409 lands here: the
 * kernel's state machine declining, which is a different operator action from a
 * permission decision. */
export interface ForgetUnavailable {
	state: 'unavailable';
	status: number;
	detail: string;
}

/** A 2xx that is not the published contract — or that reports an ERASE. */
export interface ForgetContractViolation {
	state: 'contract_violation';
	status: number;
	detail: string;
}

export type ForgetOutcome =
	| Tombstoned
	| ForgetRefused
	| NoSuchRow
	| ForgetUnavailable
	| ForgetContractViolation;

/** The core's IPC surface. Injected so the path is pinnable without a Tauri
 * runtime, and so a test drives the REAL command rather than a copy. */
export interface ForgetCore {
	authorize(request: unknown): Promise<AuthorizedForget>;
	shape(exchange: { status: number; body: string }, doc: ForgetDoc): Promise<unknown>;
}

/** `unknown`, not `ForgetRequest`, and that asymmetry is deliberate: the core's
 * request type carries `hard` in order to REFUSE it, so a typed
 * `authorize(request: ForgetRequest)` here would quietly hide the very field
 * this module exists to make unexpressible. Taking `unknown` means a caller can
 * hand the core anything and learn what it says about it. */
export type ForgetTransport = (doc: ForgetDoc) => Promise<{ status: number; body: string }>;

export interface ForgetDeps {
	core: ForgetCore;
	transport: ForgetTransport;
}

/** The typed wire against the generated contract. This is the production
 * transport: no hand-written endpoint shape, and the body is the doc the core
 * minted. */
export const typedForgetTransport: ForgetTransport = async (doc) => {
	const { data, error, response } = await client.POST('/ump/forget', { body: doc });
	const payload = data !== undefined ? data : error;
	return {
		status: response.status,
		body: payload === undefined ? '' : JSON.stringify(payload)
	};
};

/**
 * Runs one forget through the core's bounds and back through its
 * classification.
 *
 * A core refusal THROWS here rather than becoming an outcome, and that is a
 * choice worth naming: the refusal taxonomy here is the AUTHORITY boundary, not
 * a transport state. A caller that asked for `hard: true` has a bug, and a
 * silent `unavailable` would hide it behind a banner. Every other failure is a
 * named state.
 */
export async function forgetRecord(request: unknown, deps: ForgetDeps): Promise<ForgetOutcome> {
	const authorized = await deps.core.authorize(request);

	let exchange: { status: number; body: string };
	try {
		exchange = await deps.transport(authorized.doc);
	} catch (error) {
		return {
			state: 'unavailable',
			status: 0,
			detail: `the forget transport failed: ${error instanceof Error ? error.message : String(error)}`
		};
	}

	const shaped = await deps.core.shape(exchange, authorized.doc);
	return assertForgetOutcome(shaped);
}

/**
 * Validates that the core returned a state this client knows, with no
 * fallback. A defaulted unknown state would be the softest possible version of
 * the failure this module exists to prevent: a receipt claiming a tombstone
 * nobody performed.
 */
export function assertForgetOutcome(value: unknown): ForgetOutcome {
	if (typeof value !== 'object' || value === null) {
		throw new Error(`forget: the core returned ${typeof value}, not an outcome`);
	}
	const state = (value as { state?: unknown }).state;
	switch (state) {
		case 'tombstoned': {
			const done = value as Tombstoned;
			if (typeof done.id !== 'string') {
				throw new Error('forget: a tombstone carried no id');
			}
			if (done.result !== 'tombstoned' && done.result !== 'erased') {
				throw new Error(`forget: a tombstone carried result ${String(done.result)}`);
			}
			// The certificate is asserted PRESENT and null. It cannot be a
			// string: `/ump/forget` emits none, and a string here is the
			// fabricated reference `chat/forget.ts` exists to refuse.
			if (!('dsar_certificate' in done) || done.dsar_certificate !== null) {
				throw new Error(
					`forget: /ump/forget emits no DSAR certificate; the core reported ${String(done.dsar_certificate)}`
				);
			}
			return done;
		}
		case 'refused': {
			const refused = value as ForgetRefused;
			if (typeof refused.status !== 'number') {
				throw new Error('forget: a refusal carried no status');
			}
			if ('result' in refused) {
				throw new Error('forget: a refusal must not carry a result');
			}
			return refused;
		}
		case 'no_such_row': {
			const missing = value as NoSuchRow;
			if (typeof missing.id !== 'string' || typeof missing.status !== 'number') {
				throw new Error('forget: a no_such_row was malformed');
			}
			if ('result' in missing) {
				throw new Error('forget: a no_such_row must not carry a result');
			}
			return missing;
		}
		case 'unavailable':
		case 'contract_violation': {
			const failure = value as ForgetUnavailable;
			if (typeof failure.status !== 'number' || typeof failure.detail !== 'string') {
				throw new Error(`forget: ${String(state)} was malformed`);
			}
			if ('result' in failure) {
				throw new Error(`forget: ${String(state)} must not carry a result`);
			}
			return failure;
		}
		default:
			throw new Error(`forget: the core returned an unknown state ${String(state)}`);
	}
}

/** Did a row get tombstoned? Deliberately NOT "was it refused?" — a refusal
 * and a completed tombstone are different facts about the operator's data. */
export function isTombstoned(outcome: ForgetOutcome): boolean {
	return outcome.state === 'tombstoned';
}

/** True for the states that must never be drawn as "forgotten". */
export function isRefusal(outcome: ForgetOutcome): boolean {
	return outcome.state === 'refused' || outcome.state === 'no_such_row';
}

/**
 * DID THE BYTES GO? — and the answer is the RESULT, never the state.
 *
 * This accessor exists because the distinction is the whole hazard on this
 * route and it is invisible in the outcome's NAME: `tombstoned` means flagged
 * and still retrievable, `erased` means gone. A predicate reading the state
 * would report every tombstone as an erasure, telling an operator their memory
 * was destroyed when a flag was set — so it reads `result`, and it is the one
 * place in the shell where `erased` is asked about at all.
 */
export function didErase(outcome: ForgetOutcome): boolean {
	return outcome.state === 'tombstoned' && outcome.result === 'erased';
}

/**
 * The id a receipt may name — or a THROW.
 *
 * The accessor a renderer is expected to use, so reaching it on a refusal is a
 * crash rather than a receipt about a row that was never touched.
 */
export function forgottenId(outcome: ForgetOutcome): string {
	if (outcome.state === 'refused') {
		throw new Error(
			`forget: refused with ${outcome.status} — nothing was erased, so there is no receipt`
		);
	}
	if (
		outcome.state === 'no_such_row' ||
		outcome.state === 'unavailable' ||
		outcome.state === 'contract_violation'
	) {
		throw new Error(`forget: ${outcome.state} (${outcome.status}) — there is no receipt`);
	}
	return outcome.id;
}
