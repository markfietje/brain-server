/**
 * The audit surface's two reads: the chain's verdict and its events.
 *
 * Both are Admin-scoped in the kernel's own gate table, and both go through
 * the typed wire with the shell's own bearer. The kernel decides what comes
 * back; this module names what it got and never fills a gap:
 *
 * - a refusal (401/403/anything non-200) is a REFUSAL with its status — the
 *   kernel spoke, and "this credential cannot read the audit log" is the
 *   surface's most important sentence;
 * - `unreachable` is OFFLINE, the wire-level fault, never an empty log;
 * - a body this module cannot prove is MALFORMED, never rendered as rows.
 *
 * Rows carry hashes only (the kernel's own contract): a target is a SHA-256
 * of the affected id, so the surface shows prefixes, never content.
 */

import { client } from '#lib/api/client.js';
import { faultOf } from '#lib/api/wire-fault.js';
import type { WireFault } from '#lib/api/client.js';

export interface AuditEvent {
	readonly id: number;
	readonly ts: string;
	readonly kind: string;
	readonly actor: string;
	readonly status: string;
	readonly targetHash: string;
	readonly domain: string;
}

export type AuditRead =
	| { readonly kind: 'ok'; readonly events: readonly AuditEvent[] }
	| { readonly kind: 'offline'; readonly fault: WireFault }
	| { readonly kind: 'refused'; readonly status: number | null; readonly fault: WireFault }
	| { readonly kind: 'malformed' };

export type ChainRead =
	| { readonly kind: 'ok'; readonly verified: boolean; readonly domains: Readonly<Record<string, boolean>> }
	| { readonly kind: 'offline'; readonly fault: WireFault }
	| { readonly kind: 'refused'; readonly status: number | null; readonly fault: WireFault }
	| { readonly kind: 'malformed' };

/** The page the surface reads at a time; the kernel caps a list at its own ceiling. */
export const AUDIT_PAGE = 100;

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function parseEvent(value: unknown): AuditEvent | null {
	if (!isRecord(value)) return null;
	const { id, ts, kind, actor, status, target_hash, domain } = value;
	if (typeof id !== 'number' || !Number.isSafeInteger(id)) return null;
	if (typeof ts !== 'string' || typeof kind !== 'string' || typeof actor !== 'string') return null;
	if (typeof status !== 'string' || typeof target_hash !== 'string') return null;
	return {
		id,
		ts,
		kind,
		actor,
		status,
		targetHash: target_hash,
		domain: typeof domain === 'string' ? domain : 'global'
	};
}

function refusalOf(error: unknown): AuditRead & ChainRead {
	const fault = faultOf(error);
	if (fault === 'unreachable') return { kind: 'offline', fault };
	return { kind: 'refused', status: null, fault };
}

/** Read one page of events, newest first. `offset` pages further back. */
export async function fetchAuditEvents(offset = 0): Promise<AuditRead> {
	const from = Number.isSafeInteger(offset) && offset >= 0 ? offset : 0;
	try {
		const result = await client.GET('/audit', {
			params: { query: { limit: AUDIT_PAGE, offset: from } }
		});
		if (result.response.status !== 200) {
			return { kind: 'refused', status: result.response.status, fault: 'unknown' };
		}
		if (result.data === undefined || !isRecord(result.data)) return { kind: 'malformed' };
		const raw = result.data['events'];
		if (!Array.isArray(raw)) return { kind: 'malformed' };
		const events: AuditEvent[] = [];
		for (const row of raw) {
			const parsed = parseEvent(row);
			if (parsed === null) return { kind: 'malformed' };
			events.push(parsed);
		}
		return { kind: 'ok', events };
	} catch (error) {
		return refusalOf(error);
	}
}

/** Ask the kernel to recompute every domain's chain and report the verdict. */
export async function verifyAuditChain(): Promise<ChainRead> {
	try {
		const result = await client.GET('/audit/verify');
		if (result.response.status !== 200) {
			return { kind: 'refused', status: result.response.status, fault: 'unknown' };
		}
		if (result.data === undefined || !isRecord(result.data)) return { kind: 'malformed' };
		const ok = result.data['ok'];
		if (typeof ok !== 'boolean') return { kind: 'malformed' };
		const domains: Record<string, boolean> = {};
		const raw = result.data['domains'];
		if (isRecord(raw)) {
			for (const [name, verdict] of Object.entries(raw)) {
				if (typeof verdict === 'boolean') domains[name] = verdict;
			}
		}
		return { kind: 'ok', verified: ok, domains };
	} catch (error) {
		return refusalOf(error);
	}
}
