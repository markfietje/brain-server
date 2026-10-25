/**
 * The governance surface's three reads: domains, roles, retention.
 *
 * Domains and roles are Read-scoped; retention is Admin-scoped — so the
 * three can answer differently to one bearer, and the surface renders each
 * one's own answer rather than collapsing the page onto the weakest read.
 * Every read follows the same law as the audit module: a refusal is named
 * with its status, `unreachable` is offline, and a body this module cannot
 * prove is malformed — never a quiet empty list.
 */

import { client } from '#lib/api/client.js';
import { faultOf } from '#lib/api/wire-fault.js';
import type { WireFault } from '#lib/api/client.js';

export interface DomainRow {
	readonly name: string;
	readonly entries: number;
	readonly entities: number;
	readonly relations: number;
	readonly multiDb: boolean;
}

export interface RoleRow {
	readonly name: string;
	readonly description: string;
	readonly scopes: readonly string[];
	readonly can: readonly string[];
}

export interface RetentionPolicy {
	readonly enabled: boolean;
	/** memory kind → days kept, as the kernel states it */
	readonly policy: Readonly<Record<string, number>>;
}

export type GovernanceRead<T> =
	| { readonly kind: 'ok'; readonly value: T }
	| { readonly kind: 'offline'; readonly fault: WireFault }
	| { readonly kind: 'refused'; readonly status: number | null; readonly fault: WireFault }
	| { readonly kind: 'malformed' };

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function stringList(value: unknown): readonly string[] {
	return Array.isArray(value) ? value.filter((v): v is string => typeof v === 'string') : [];
}

function failed<T>(error: unknown): GovernanceRead<T> {
	const fault = faultOf(error);
	if (fault === 'unreachable') return { kind: 'offline', fault };
	return { kind: 'refused', status: null, fault };
}

function parseDomain(value: unknown): DomainRow | null {
	if (!isRecord(value) || typeof value['name'] !== 'string') return null;
	const n = (key: string): number => (typeof value[key] === 'number' ? (value[key] as number) : 0);
	return {
		name: value['name'],
		entries: n('entries'),
		entities: n('entities'),
		relations: n('relations'),
		multiDb: value['multi_db'] === true
	};
}

function parseRole(value: unknown): RoleRow | null {
	if (!isRecord(value) || typeof value['name'] !== 'string') return null;
	return {
		name: value['name'],
		description: typeof value['description'] === 'string' ? value['description'] : '',
		scopes: stringList(value['scopes']),
		can: stringList(value['can'])
	};
}

/** The registered domains with their counts. */
export async function fetchDomains(): Promise<GovernanceRead<readonly DomainRow[]>> {
	try {
		const result = await client.GET('/domains');
		if (result.response.status !== 200) {
			return { kind: 'refused', status: result.response.status, fault: 'unknown' };
		}
		if (result.data === undefined || !isRecord(result.data)) return { kind: 'malformed' };
		const raw = result.data['domains'];
		if (!Array.isArray(raw)) return { kind: 'malformed' };
		const rows: DomainRow[] = [];
		for (const row of raw) {
			const parsed = parseDomain(row);
			if (parsed === null) return { kind: 'malformed' };
			rows.push(parsed);
		}
		return { kind: 'ok', value: rows };
	} catch (error) {
		return failed(error);
	}
}

/** The role bundles the kernel enforces. */
export async function fetchRoles(): Promise<GovernanceRead<readonly RoleRow[]>> {
	try {
		const result = await client.GET('/roles');
		if (result.response.status !== 200) {
			return { kind: 'refused', status: result.response.status, fault: 'unknown' };
		}
		if (result.data === undefined || !isRecord(result.data)) return { kind: 'malformed' };
		const raw = result.data['roles'];
		if (!Array.isArray(raw)) return { kind: 'malformed' };
		const rows: RoleRow[] = [];
		for (const row of raw) {
			const parsed = parseRole(row);
			if (parsed === null) return { kind: 'malformed' };
			rows.push(parsed);
		}
		return { kind: 'ok', value: rows };
	} catch (error) {
		return failed(error);
	}
}

/** The retention policy — Admin only; a Read bearer sees a named refusal. */
export async function fetchRetention(): Promise<GovernanceRead<RetentionPolicy>> {
	try {
		const result = await client.GET('/retention');
		if (result.response.status !== 200) {
			return { kind: 'refused', status: result.response.status, fault: 'unknown' };
		}
		if (result.data === undefined || !isRecord(result.data)) return { kind: 'malformed' };
		const enabled = result.data['enabled'];
		if (typeof enabled !== 'boolean') return { kind: 'malformed' };
		const policy: Record<string, number> = {};
		const raw = result.data['policy'];
		if (isRecord(raw)) {
			for (const [kind, days] of Object.entries(raw)) {
				if (typeof days === 'number' && Number.isFinite(days)) policy[kind] = days;
			}
		}
		return { kind: 'ok', value: { enabled, policy } };
	} catch (error) {
		return failed(error);
	}
}
