/**
 * III-6, AS AMENDED — the review queue's ONE read, over the shell's typed
 * wire.
 *
 * ## WHAT THIS MODULE IS
 *
 * The single call that reaches the kernel's human review queue. It is a READ,
 * through the generated typed client, with the shell's own bearer — and the
 * kernel's own gates decide what comes back. The surface does not ask for a
 * weaker capability, does not invent one, and does not swallow a refusal.
 *
 * ## THE LAW THIS MODULE IS BUILT FOR
 *
 * **This module reads; it never decides.** The III-6 amendment (the
 * approve-action round, C2 G-6) moved ONE verb onto the surface — the
 * approve, digest-bound, audited, for the authorized — and it lives in its
 * own module (`#lib/api/approve.js`), scanned by the review surface's pins.
 * This module still holds exactly one verb, the READ: no reject, no edit,
 * and no second write path. The kernel's role gates decide both halves —
 * who sees the queue and who may approve on it — and a caller they refuse
 * sees the refusal, never a queue.
 *
 * ## THE THREE FAULTS THAT MATTER HERE
 *
 * - `unreachable` — the kernel could not be reached. This is the OFFLINE
 *   state: the surface shows the last-known METADATA with its age, and names
 *   the state. Never an empty queue.
 * - a refusal (401, 403, or any other non-success status) — the kernel said
 *   no. The surface renders the REFUSAL with its reason, and never an empty
 *   queue. An empty queue and a refused queue are different facts.
 * - anything else — the kernel answered with a body this surface cannot
 *   prove. Refused, named, never rendered as a queue.
 */

import { client, type WireFault } from '#lib/api/client.js';
import { faultOf } from '#lib/api/wire-fault.js';
import { parseQueueRows, QUEUE_LIMITS, type QueueRow } from './model.js';

export type QueueRead =
	| { readonly kind: 'ok'; readonly rows: readonly QueueRow[] }
	| { readonly kind: 'offline'; readonly fault: WireFault }
	| { readonly kind: 'refused'; readonly status: number | null; readonly fault: WireFault }
	| { readonly kind: 'malformed' };

/**
 * Read the PENDING queue.
 *
 * `limit` is the kernel's own default page size; the shell never asks for
 * more rows than the kernel's ceiling (`MAX_PROPOSALS`, 200) and does not
 * exceed the page the kernel serves by default. A caller that wants a
 * different page passes a number inside `QUEUE_LIMITS.rows`.
 */
export async function fetchPendingQueue(limit: number = QUEUE_LIMITS.defaultLimit): Promise<QueueRead> {
	const bounded = Number.isSafeInteger(limit) && limit > 0 && limit <= QUEUE_LIMITS.rows
		? limit
		: QUEUE_LIMITS.defaultLimit;
	try {
		const result = await client.GET('/proposals', {
			params: { query: { status: 'pending', limit: bounded } }
		});
		// A non-success status is a REFUSAL, and it is named. The kernel's
		// answer is not an empty queue, and a 403 here is the surface's most
		// important message: this bearer may not read the queue.
		if (result.response && result.response.status !== 200) {
			return { kind: 'refused', status: result.response.status, fault: 'unknown' };
		}
		if (result.error !== undefined || result.data === undefined) {
			return { kind: 'malformed' };
		}
		const parsed = parseQueueRows(result.data);
		if (!parsed.ok) {
			return { kind: 'malformed' };
		}
		return { kind: 'ok', rows: parsed.value };
	} catch (error) {
		// The fault is NAMED, never guessed: the browser engine reports a
		// blocked response, a rejected bearer and a dead port identically, and
		// the core asks the kernel which one it is.
		const fault = faultOf(error);
		if (fault === 'unreachable') {
			return { kind: 'offline', fault };
		}
		return { kind: 'refused', status: null, fault };
	}
}
