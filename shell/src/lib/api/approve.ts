/**
 * THE APPROVE ACTION — the review queue's ONE write, and the III-6
 * amendment made real: display for everyone; approve for the authorized,
 * digest-bound, audited.
 *
 * ## THE LAW THIS MODULE IS BUILT FOR
 *
 * **A human approves the bytes they were shown, or nothing.** The call
 * carries the `content_digest` the queue rendered on the row — the same
 * value the reviewer read beside the content. The generated contract types
 * `digest` as REQUIRED on the approve operation, so the module cannot even
 * express a digest-less call: the kernel's `400 digest_required` answers a
 * caller that bypassed the type, never this one. A `409` is DRIFT — the
 * row changed since it was displayed — and the surface must name it as a
 * reload-and-re-decide, never as a success.
 *
 * **One verb, and only one.** The module holds exactly the approve: no
 * reject, no edit — the amendment's scope is the approve alone, and the
 * review surface's pins scan for the others. The kernel's role gates stay
 * where they are: a caller the kernel refuses sees the refusal NAMED
 * (`refused` with the status), never an empty success.
 *
 * ## THE FOUR FAULTS THAT MATTER HERE
 *
 * - `drift` — the kernel said the bytes moved (409). The displayed digest
 *   no longer matches; reload and decide again.
 * - a refusal (401, 403, or any other non-success status) — the kernel
 *   said no. Named, never swallowed.
 * - `offline` — the kernel could not be reached; nothing was approved.
 * - anything else — an answer this module cannot prove. `malformed`, and
 *   nothing was approved.
 */

import { client } from '#lib/api/client.js';
import { faultOf } from '#lib/api/wire-fault.js';
import type { WireFault } from '#lib/api/client.js';

export type ApproveOutcome =
	| { readonly kind: 'approved'; readonly chunkId: number | null }
	| { readonly kind: 'drift' }
	| { readonly kind: 'refused'; readonly status: number | null; readonly fault: WireFault }
	| { readonly kind: 'offline'; readonly fault: WireFault }
	| { readonly kind: 'malformed' };

/**
 * Approve the proposal `id` with the digest the queue DISPLAYED for it.
 * The digest parameter is the transport's whole point: the caller passes
 * what the reviewer saw, and the kernel re-derives it from the stored row
 * before the decision lands.
 */
export async function approveProposal(id: number, digest: string): Promise<ApproveOutcome> {
	try {
		const result = await client.POST('/proposals/{id}/approve', {
			params: { path: { id }, query: { digest } }
		});
		if (result.response.status === 409) return { kind: 'drift' };
		if (result.response.status !== 200) {
			return { kind: 'refused', status: result.response.status, fault: 'unknown' };
		}
		const data = result.data as Record<string, unknown> | undefined;
		const chunkId = data?.['chunk_id'];
		if (chunkId !== undefined && chunkId !== null && typeof chunkId !== 'number') {
			return { kind: 'malformed' };
		}
		return { kind: 'approved', chunkId: typeof chunkId === 'number' ? chunkId : null };
	} catch (error) {
		const fault = faultOf(error);
		if (fault === 'unreachable') return { kind: 'offline', fault };
		return { kind: 'refused', status: null, fault };
	}
}
