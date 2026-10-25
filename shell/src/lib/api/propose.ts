/**
 * "Add memory" — the ONE write the memory surface can make, and it is a
 * PROPOSAL, not a memory.
 *
 * `POST /ingest/proposal` stores a candidate that the review queue holds
 * until a human approves it; it creates no knowledge row and the kernel
 * never auto-promotes. That is the whole reason the surface's add button
 * goes here and not to `/add`: the human gate is the product's law, and a
 * memory typed into a webview enters the store the same way one captured
 * from a channel does — through Review.
 *
 * The request carries `content` and `kind` and nothing else. The outcome is
 * named: `filed` with the proposal's id, a `refused` with the kernel's
 * status, `offline` for the wire fault, or `malformed` for an answer this
 * module cannot prove — never a silent success.
 */

import { client } from '#lib/api/client.js';
import { faultOf } from '#lib/api/wire-fault.js';
import type { WireFault } from '#lib/api/client.js';

/** The two kinds a person files by hand; the kernel's other kinds are machine-born. */
export type ProposalKind = 'fact' | 'procedure';

export const PROPOSAL_CONTENT_MAX = 10000;

export type ProposeOutcome =
	| { readonly kind: 'filed'; readonly id: number }
	| { readonly kind: 'refused'; readonly status: number | null; readonly fault: WireFault }
	| { readonly kind: 'offline'; readonly fault: WireFault }
	| { readonly kind: 'malformed' };

export async function proposeMemory(content: string, kind: ProposalKind): Promise<ProposeOutcome> {
	try {
		const result = await client.POST('/ingest/proposal', {
			body: { content, kind }
		});
		if (result.response.status !== 200) {
			return { kind: 'refused', status: result.response.status, fault: 'unknown' };
		}
		const id = result.data?.['id'];
		if (typeof id !== 'number' || !Number.isSafeInteger(id)) return { kind: 'malformed' };
		return { kind: 'filed', id };
	} catch (error) {
		const fault = faultOf(error);
		if (fault === 'unreachable') return { kind: 'offline', fault };
		return { kind: 'refused', status: null, fault };
	}
}
