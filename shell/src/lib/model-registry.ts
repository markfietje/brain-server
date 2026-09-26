/**
 * Model-registry wire projection for the M6-S2 shell.
 *
 * The generated client is intentionally weak for this surface: the kernel owns
 * the real contract. This module is the one runtime boundary between that wire
 * and the renderer. It accepts unknown values, proves the fields it displays,
 * bounds every collection/string, and never throws and never invents a missing
 * fact.
 *
 * Three laws this module exists to hold:
 *
 *  1. DISPLAY/SUBMISSION SEPARATION. Values are retained VERBATIM as they
 *     arrived. Nothing here calls `stripInvisible`: a renderer may sanitize on
 *     the way to the DOM, but an outbound lifecycle payload must carry the
 *     original wire bytes. Sanitizing a value that is later sent back mutates
 *     the operator's data and voids the evidence.
 *  2. A DIGEST IS A PIN, NOT A SIGNATURE. `row_digest` is carried, bounds-checked
 *     and forwarded. This module never computes, truncates, uppercases, repairs
 *     or re-derives one, and there is no function here that could.
 *  3. AN EVALUATION JOIN IS DATA. `evaluation_refs` is a bounded list of
 *     strings. It is not a status signal, and nothing in this module lets it
 *     become one.
 */
import { stripInvisible } from './sanitize';

export const MODEL_REGISTRY_LIMITS = {
	listRows: 50,
	evaluationRefs: 32,
	identifierChars: 256,
	versionChars: 64,
	nameChars: 512,
	vocabularyItems: 16,
	refChars: 320
} as const;

export type ModelKind = 'deterministic-rules' | 'learned' | 'reranker';
export type ModelStatus = 'candidate' | 'evaluated' | 'promoted' | 'retired';
export type OutputVocab = 'choice' | 'score' | 'noul';
export type LifecycleAction = 'promote' | 'retire';

export const MODEL_KINDS: readonly ModelKind[] = [
	'deterministic-rules',
	'learned',
	'reranker'
];
export const MODEL_STATUSES: readonly ModelStatus[] = [
	'candidate',
	'evaluated',
	'promoted',
	'retired'
];
export const OUTPUT_VOCABS: readonly OutputVocab[] = ['choice', 'score', 'noul'];

/** The closed transition table, mirroring the kernel's `transition_legal`. */
export const LIFECYCLE_TRANSITIONS: ReadonlyArray<{
	action: LifecycleAction;
	from: readonly ModelStatus[];
}> = [
	{ action: 'promote', from: ['candidate', 'evaluated'] },
	{ action: 'retire', from: ['candidate', 'evaluated', 'promoted'] }
];

export interface ModelSummary {
	id: string;
	version: string;
	kind: ModelKind;
	name: string;
	outputVocabulary: OutputVocab[];
	/** The listing carries artifact digest as a PRESENCE BOOLEAN only. */
	artifactDigestPresent: boolean;
	configDigest: string | null;
	status: ModelStatus;
	proposedBy: string;
	createdAt: number;
	updatedAt: number;
}

export interface ModelRegistryList {
	rows: ModelSummary[];
	count: number;
}

export interface ModelRegistryDetail {
	id: string;
	version: string;
	kind: ModelKind;
	name: string;
	outputVocabulary: OutputVocab[];
	artifactDigest: string | null;
	configDigest: string | null;
	calibrationRef: string | null;
	status: ModelStatus;
	evaluationRefs: string[];
	proposedBy: string;
	/** Nullable in the wire; REQUIRED as a key even when null. The kernel's
	 * RegistryRow is deny_unknown_fields with no serde(default), so omitting it
	 * is a 400 `registry_payload_invalid`, not a default. */
	approvedBy: string | null;
	createdAt: number;
	updatedAt: number;
	rowDigest: string;
}

/** The exact field set the kernel's `RegistryRow` accepts.
 *
 * MEASURED against `src/workflow/registry.rs` — `#[serde(deny_unknown_fields)]`,
 * fourteen fields, NO `row_digest`. `row_digest` is the CONTENT PIN and travels
 * beside the row, never inside it: the kernel's own contract test clones the
 * detail object and REMOVES `row_digest` before deserializing. Sending it
 * inside `row` is a live 400, which the E2E caught.
 */
export const REGISTRY_ROW_FIELDS = [
	'id',
	'version',
	'kind',
	'name',
	'output_vocabulary',
	'artifact_digest',
	'config_digest',
	'calibration_ref',
	'status',
	'evaluation_refs',
	'proposed_by',
	'approved_by',
	'created_at',
	'updated_at'
] as const;

export type ModelRegistryParseFailure = 'not_an_object' | 'invalid_list' | 'invalid_detail';

export type ModelRegistryParseResult<T> =
	| { ok: true; value: T }
	| { ok: false; reason: ModelRegistryParseFailure };

export type ModelRegistryListQueryInput = {
	limit?: number;
	status?: ModelStatus;
	kind?: ModelKind;
};

export type ModelRegistryStatusKind =
	| 'ok'
	| 'unauthorized'
	| 'forbidden'
	| 'not_found'
	| 'conflict'
	| 'invalid'
	| 'server'
	| 'unknown';

export interface ModelRegistryStatus {
	kind: ModelRegistryStatusKind;
	status: number;
	retryable: boolean;
}

const DIGEST = /^[0-9a-f]{64}$/;
const EPOCH_MAX = 32_503_680_000; // 3000-01-01T00:00:00Z

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** A bounded, retained-verbatim operator string. */
function boundedString(
	value: unknown,
	max: number
): string | null {
	if (typeof value !== 'string') return null;
	if (value.length === 0 || value.length > max) return null;
	return value;
}

function closed<T extends string>(value: unknown, vocabulary: readonly T[]): T | null {
	if (typeof value !== 'string') return null;
	return (vocabulary as readonly string[]).includes(value) ? (value as T) : null;
}

/** Exact lowercase 64-hex. Never computed, never repaired. */
function exactDigest(value: unknown): string | null {
	return typeof value === 'string' && DIGEST.test(value) ? value : null;
}

function nullableDigest(value: unknown): string | null | undefined {
	if (value === null) return null;
	const digest = exactDigest(value);
	return digest === null ? undefined : digest;
}

/** Finite, safe, non-negative, epoch-guarded BEFORE any Date conversion. */
function epochSeconds(value: unknown): number | null {
	if (typeof value !== 'number') return null;
	if (!Number.isFinite(value)) return null;
	if (!Number.isSafeInteger(value)) return null;
	if (value < 0 || value > EPOCH_MAX) return null;
	return value;
}

function vocabulary(value: unknown): OutputVocab[] | null {
	if (!Array.isArray(value)) return null;
	if (value.length > MODEL_REGISTRY_LIMITS.vocabularyItems) return null;
	const out: OutputVocab[] = [];
	for (const item of value) {
		const word = closed(item, OUTPUT_VOCABS);
		if (word === null) return null;
		out.push(word);
	}
	return out;
}

function evaluationRefs(value: unknown): string[] | null {
	if (!Array.isArray(value)) return null;
	if (value.length > MODEL_REGISTRY_LIMITS.evaluationRefs) return null;
	const out: string[] = [];
	for (const item of value) {
		const ref = boundedString(item, MODEL_REGISTRY_LIMITS.refChars);
		if (ref === null) return null;
		out.push(ref);
	}
	return out;
}

function summary(value: unknown): ModelSummary | null {
	if (!isRecord(value)) return null;
	const id = boundedString(value['id'], MODEL_REGISTRY_LIMITS.identifierChars);
	const version = boundedString(value['version'], MODEL_REGISTRY_LIMITS.versionChars);
	const kind = closed(value['kind'], MODEL_KINDS);
	const name = boundedString(value['name'], MODEL_REGISTRY_LIMITS.nameChars);
	const status = closed(value['status'], MODEL_STATUSES);
	const proposedBy = boundedString(value['proposed_by'], MODEL_REGISTRY_LIMITS.refChars);
	const vocab = vocabulary(value['output_vocabulary']);
	const configDigest = nullableDigest(value['config_digest']);
	const createdAt = epochSeconds(value['created_at']);
	const updatedAt = epochSeconds(value['updated_at']);
	if (
		id === null ||
		version === null ||
		kind === null ||
		name === null ||
		status === null ||
		proposedBy === null ||
		vocab === null ||
		configDigest === undefined ||
		createdAt === null ||
		updatedAt === null
	) {
		return null;
	}
	if (typeof value['artifact_digest_present'] !== 'boolean') return null;
	return {
		id,
		version,
		kind,
		name,
		outputVocabulary: vocab,
		artifactDigestPresent: value['artifact_digest_present'],
		configDigest,
		status,
		proposedBy,
		createdAt,
		updatedAt
	};
}

export function parseModelRegistryList(value: unknown): ModelRegistryParseResult<ModelRegistryList> {
	if (!isRecord(value)) return { ok: false, reason: 'not_an_object' };
	const rawRows = value['rows'];
	if (!Array.isArray(rawRows)) return { ok: false, reason: 'invalid_list' };
	// A bounded listing that overflows its cap is a REFUSAL, not a truncation:
	// a silently truncated list would render as authoritative.
	if (rawRows.length > MODEL_REGISTRY_LIMITS.listRows) {
		return { ok: false, reason: 'invalid_list' };
	}
	const rows: ModelSummary[] = [];
	for (const raw of rawRows) {
		const row = summary(raw);
		if (row === null) return { ok: false, reason: 'invalid_list' };
		rows.push(row);
	}
	if (typeof value['count'] !== 'number' || !Number.isSafeInteger(value['count'])) {
		return { ok: false, reason: 'invalid_list' };
	}
	return { ok: true, value: { rows, count: value['count'] } };
}

export function parseModelRegistryDetail(
	value: unknown
): ModelRegistryParseResult<ModelRegistryDetail> {
	if (!isRecord(value)) return { ok: false, reason: 'not_an_object' };
	const id = boundedString(value['id'], MODEL_REGISTRY_LIMITS.identifierChars);
	const version = boundedString(value['version'], MODEL_REGISTRY_LIMITS.versionChars);
	const kind = closed(value['kind'], MODEL_KINDS);
	const name = boundedString(value['name'], MODEL_REGISTRY_LIMITS.nameChars);
	const status = closed(value['status'], MODEL_STATUSES);
	const proposedBy = boundedString(value['proposed_by'], MODEL_REGISTRY_LIMITS.refChars);
	const vocab = vocabulary(value['output_vocabulary']);
	const artifactDigest = nullableDigest(value['artifact_digest']);
	const configDigest = nullableDigest(value['config_digest']);
	const calibrationRef =
		value['calibration_ref'] === null
			? null
			: boundedString(value['calibration_ref'], MODEL_REGISTRY_LIMITS.refChars);
	const refs = evaluationRefs(value['evaluation_refs']);
	const proposedByCheck = boundedString(value['proposed_by'], MODEL_REGISTRY_LIMITS.refChars);
	const approvedBy =
		value['approved_by'] === null
			? null
			: boundedString(value['approved_by'], MODEL_REGISTRY_LIMITS.refChars);
	const createdAt = epochSeconds(value['created_at']);
	const updatedAt = epochSeconds(value['updated_at']);
	const rowDigest = exactDigest(value['row_digest']);
	if (
		id === null ||
		version === null ||
		kind === null ||
		name === null ||
		status === null ||
		proposedBy === null ||
		vocab === null ||
		artifactDigest === undefined ||
		configDigest === undefined ||
		calibrationRef === undefined ||
		refs === null ||
		proposedByCheck === null ||
		approvedBy === undefined ||
		createdAt === null ||
		updatedAt === null ||
		rowDigest === null
	) {
		return { ok: false, reason: 'invalid_detail' };
	}
	return {
		ok: true,
		value: {
			id,
			version,
			kind,
			name,
			outputVocabulary: vocab,
			artifactDigest,
			configDigest,
			calibrationRef,
			status,
			evaluationRefs: refs,
			proposedBy,
			approvedBy,
			createdAt,
			updatedAt,
			rowDigest
		}
	};
}

/**
 * The typed filter OBJECT the generated client actually consumes.
 *
 * WHY THIS EXISTS, AND WHY IT IS NOT OPTIONAL: openapi-fetch 0.17's
 * `createQuerySerializer` iterates its argument only when
 * `typeof queryParams === 'object'`. Handed a STRING it returns `''`, and
 * `createFinalURL` then appends nothing. Verified against the shipped 0.17.0
 * dist:
 *
 *     createQuerySerializer({limit:20, status:'candidate'}) -> "limit=20&status=candidate"
 *     createQuerySerializer('?limit=20&status=candidate')  -> ""      (silently dropped)
 *
 * There is no error and no warning. A string filter therefore does not fail
 * loudly -- it sends an UNFILTERED request, which for an audited registry
 * listing means the operator's filter is silently discarded. This function is
 * the object form, so the correct call is the easy one.
 */
export function modelRegistryListQueryParams(
	input: ModelRegistryListQueryInput
): Record<string, string | number> | null {
	const params: Record<string, string | number> = {};
	if (input.limit !== undefined) {
		const { limit } = input;
		if (typeof limit !== 'number' || !Number.isInteger(limit) || limit < 1 || limit > 50) {
			return null;
		}
		params['limit'] = limit;
	}
	if (input.status !== undefined) {
		if (closed(input.status, MODEL_STATUSES) === null) return null;
		params['status'] = input.status;
	}
	if (input.kind !== undefined) {
		if (closed(input.kind, MODEL_KINDS) === null) return null;
		params['kind'] = input.kind;
	}
	return params;
}

/**
 * The same filter as a query STRING, for assertions and for any caller that
 * needs the textual form. Derived from {@link modelRegistryListQueryParams} so
 * the two can never disagree.
 *
 * NOTE: this string is for COMPARISON, not for handing to the client -- see
 * {@link modelRegistryListQueryParams} for why passing it to `params.query`
 * silently drops every filter.
 */
export function modelRegistryListQuery(input: ModelRegistryListQueryInput): string | null {
	const params = modelRegistryListQueryParams(input);
	if (params === null) return null;
	const search = Object.entries(params)
		.map(([key, value]) => `${key}=${value}`)
		.join('&');
	return search.length === 0 ? '' : `?${search}`;
}

/** The closed transition table, read from the kernel's law and not invented. */
export function isLegalLifecycleAction(action: string, from: string): boolean {
	const row = LIFECYCLE_TRANSITIONS.find((entry) => entry.action === action);
	if (!row) return false;
	return (row.from as readonly string[]).includes(from);
}

/** Every action legal from `status`; empty when none are. */
export function legalLifecycleActions(status: string): LifecycleAction[] {
	return LIFECYCLE_TRANSITIONS.filter((row) =>
		(row.from as readonly string[]).includes(status)
	).map((row) => row.action);
}

/** Decode one whole `{id}@{version}` segment, mirroring `parse_model_ref`. */
export function parseModelRef(raw: string): { id: string; version: string } | null {
	if (typeof raw !== 'string') return null;
	if (raw.length === 0 || raw.length > MODEL_REGISTRY_LIMITS.refChars) return null;
	const at = raw.indexOf('@');
	if (at <= 0 || at === raw.length - 1) return null;
	const id = raw.slice(0, at);
	const version = raw.slice(at + 1);
	if (id.includes('@') || version.includes('@')) return null;
	if (id.length > MODEL_REGISTRY_LIMITS.identifierChars) return null;
	if (version.length > MODEL_REGISTRY_LIMITS.versionChars) return null;
	return { id, version };
}

/**
 * Build the VERBATIM wire content for a lifecycle proposal.
 *
 * The `row_digest` is the content pin captured from the detail read. It is
 * forwarded exactly as received — never recomputed, never repaired, never
 * display-sanitized.
 *
 * The `row` is EXACTLY the kernel's `RegistryRow` field set: the detail object
 * MINUS `row_digest`, with `approved_by` present even when null. Two live 400s
 * were found by the E2E and are pinned by the tests: sending `row_digest`
 * INSIDE `row` (`registry_payload_invalid`, unknown field) and omitting
 * `approved_by` (`registry_payload_invalid`, missing field). The pin travels
 * BESIDE the row, never inside it.
 *
 * There is deliberately no `status` channel here: a lifecycle change is a
 * PROPOSAL a human approves, not a mutation the client performs.
 */
export function lifecycleProposalContent(
	action: LifecycleAction,
	detail: ModelRegistryDetail
): string {
	return JSON.stringify({
		action,
		id: detail.id,
		version: detail.version,
		row_digest: detail.rowDigest,
		row: {
			id: detail.id,
			version: detail.version,
			kind: detail.kind,
			name: detail.name,
			output_vocabulary: detail.outputVocabulary,
			artifact_digest: detail.artifactDigest,
			config_digest: detail.configDigest,
			calibration_ref: detail.calibrationRef,
			status: detail.status,
			evaluation_refs: detail.evaluationRefs,
			proposed_by: detail.proposedBy,
			approved_by: detail.approvedBy,
			created_at: detail.createdAt,
			updated_at: detail.updatedAt
		}
	});
}

/**
 * Classify a response status WITHOUT consulting any client-side role.
 *
 * Kernel authorization is the only authority: a 403 is a `forbidden` state that
 * renders as forbidden, and is never collapsed into an empty list. `unknown`
 * exists so an unrecognized status can never be read as success.
 */
export function classifyModelRegistryStatus(status: number): ModelRegistryStatus {
	const kind: ModelRegistryStatusKind =
		status === 200
			? 'ok'
			: status === 401
				? 'unauthorized'
				: status === 403
					? 'forbidden'
					: status === 404
						? 'not_found'
						: status === 409
							? 'conflict'
							: status === 400 || status === 422
								? 'invalid'
								: status >= 500
									? 'server'
									: 'unknown';
	return {
		kind,
		status,
		retryable: kind === 'server' || kind === 'conflict'
	};
}

/**
 * The DISPLAY projection: the only sanctioned place invisible characters are
 * removed. A renderer uses this on its way to the DOM and MUST NOT use it on
 * anything that is later sent to the kernel.
 */
export function displayText(value: string): string {
	return stripInvisible(value);
}
