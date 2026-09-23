/**
 * The recall-trace defensive parser (M2-S3): the typed client types
 * GET /recall/{trace_id}/trace as a loose object (the openapi response is
 * free-form JSON), so the shell parses it CLOSED at runtime — the kernel
 * validates on the wire; the client refuses what it cannot prove. The
 * pinned policy (R25_PREREG §3): never throws, never invents — a field is
 * rendered from an exactly-typed value or it is `null` (the em-dash
 * absence of the Dioxus replay_str/replay_list convention, v1.20.20 M1).
 * Every accepted STRING crosses stripInvisible BEFORE it is stored — the
 * v1.20.3 render boundary — so stored metadata can neither spoof nor
 * smuggle. Unknown/extra fields are preserved by construction via the
 * verbatim-bytes export path (the raw wire text), never by this view.
 */
import { stripInvisible } from './sanitize';

export interface RecallTraceHitView {
	/** finite integer → decimal string; else null */
	id: string | null;
	/** finite number → decimal string; else null */
	score: string | null;
	source: string | null;
	relevance: string | null;
	assertion_kind: string | null;
	/** boolean only; absent/null → null (renders —, the Dioxus marker law) */
	decayed: boolean | null;
}

export interface RecallTraceView {
	decision: string | null;
	actor: string | null;
	/** the query's fingerprint — a hash, shown AS a hash, never reversed */
	query_hash: string | null;
	/** array of stripped strings; empty/null/absent/non-array → null */
	scope: string[] | null;
	domains_searched: string[] | null;
	/** one row per plain-object element; absent/non-array → null */
	hits: RecallTraceHitView[] | null;
}

function strOrNull(value: unknown): string | null {
	return typeof value === 'string' ? stripInvisible(value) : null;
}

function strListOrNull(value: unknown): string[] | null {
	if (!Array.isArray(value)) return null;
	const kept = value
		.filter((e): e is string => typeof e === 'string')
		.map(stripInvisible);
	return kept.length > 0 ? kept : null;
}

function finiteIntegerString(value: unknown): string | null {
	return typeof value === 'number' && Number.isInteger(value) && Number.isFinite(value)
		? String(value)
		: null;
}

function finiteNumberString(value: unknown): string | null {
	return typeof value === 'number' && Number.isFinite(value) ? String(value) : null;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function parseHit(value: unknown): RecallTraceHitView | null {
	if (!isPlainObject(value)) return null;
	return {
		id: finiteIntegerString(value['id']),
		score: finiteNumberString(value['score']),
		source: strOrNull(value['source']),
		relevance: strOrNull(value['relevance']),
		assertion_kind: strOrNull(value['assertion_kind']),
		decayed: typeof value['decayed'] === 'boolean' ? value['decayed'] : null
	};
}

/**
 * The closed parse of the trace wire shape. Hostile input (null, wrong
 * types, invisible chars) yields the honest null/stripped view — never an
 * exception, never an invented value.
 */
export function parseRecallTrace(value: unknown): RecallTraceView {
	const obj = isPlainObject(value) ? value : {};
	const rawHits = Array.isArray(obj['hits'])
		? (obj['hits'] as unknown[]).map(parseHit).filter((h): h is RecallTraceHitView => h !== null)
		: null;
	return {
		decision: strOrNull(obj['decision']),
		actor: strOrNull(obj['actor']),
		query_hash: strOrNull(obj['query_hash']),
		scope: strListOrNull(obj['scope']),
		domains_searched: strListOrNull(obj['domains_searched']),
		hits: rawHits
	};
}
