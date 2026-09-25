/**
 * Decision-run wire projection for the M6-S1 shell.
 *
 * The generated client is intentionally weak for this surface: detail and the
 * replay documents are free-form JSON, while the kernel owns the real contract.
 * This module is the one runtime boundary between that wire and the renderer.
 * It accepts unknown values, proves the fields it displays, bounds every
 * collection/string, and removes canonical invisible characters before any
 * value is retained. It never throws and never invents a missing fact.
 */
import { stripInvisible } from './sanitize';

export const DECISION_LIMITS = {
	listRows: 50,
	modelRefs: 8,
	contextRefs: 100,
	stages: 8,
	replayRows: 8,
	identifierChars: 256,
	labelChars: 4096,
	opaquePreviewChars: 65536,
	rawCaptureBytes: 1024 * 1024,
	opaqueDepth: 12,
	opaqueNodes: 512
} as const;

export type DecisionMode = 'deterministic' | 'exploratory';
export type DecisionAction = 'act' | 'approve' | 'reject' | 'escalate';
export type TrustTier = 'untrusted' | 'vetted' | 'governed';
export type RetrievalLeg = 'both' | 'vector' | 'fts' | 'graph';
export type StageName =
	| 'normalize'
	| 'retrieve_context'
	| 'candidate_generation'
	| 'decision_model'
	| 'rules_policy'
	| 'rerank'
	| 'threshold'
	| 'action_escalation';
export type EscalationReason =
	| 'out_of_vocabulary'
	| 'insufficient_evidence'
	| 'insufficient_evidence_tier'
	| 'invalid_input'
	| 'retrieval_unavailable'
	| 'misconfigured'
	| 'model_disabled'
	| 'policy_untrusted_only'
	| 'missing_decision'
	| 'fallback_escalate';

export interface DecisionRunSummary {
	id: number;
	runId: number;
	mode: DecisionMode;
	pipelineVersion: string;
	configHash: string;
	createdAt: number;
	stageCount: number;
}

export interface DecisionRunList {
	rows: DecisionRunSummary[];
	count: number;
}

export interface ModelReference {
	id: string;
	version: string;
	weightsDigest: string | null;
	registryId: string | null;
	registryVersion: string | null;
}

export interface ContextReference {
	evidenceId: string;
	contentDigest: string;
	tier: TrustTier;
	vectorRank: number | null;
	ftsRank: number | null;
	graphRank: number | null;
	fusedScore: number | null;
	flagged: boolean;
	untrusted: boolean;
}

export interface RetrievalParameters {
	rrfK: number;
	limit: number;
	leg: RetrievalLeg;
}

export interface EnvironmentFingerprint {
	kernelVersion: string;
	featureFlags: string[];
}

export interface DecisionStage {
	stage: StageName;
	algorithm: string;
	configHash: string;
	inputsDigest: string;
	outputsDigest: string;
	outputPreview: string | null;
	trustTiersSeen: TrustTier[];
	startedAt: number;
	durationNs: number;
}

export interface DecisionEscalation {
	stage: StageName;
	reason: EscalationReason;
	detail: string;
}

export interface DecisionOutcome {
	action: DecisionAction;
	escalation: DecisionEscalation | null;
	outputPreview: string | null;
}

export interface DecisionRunDetail {
	runId: number;
	pipelineVersion: string;
	mode: DecisionMode;
	configHash: string;
	modelRefs: ModelReference[];
	inputDigest: string;
	contextRefs: ContextReference[];
	retrievalParams: RetrievalParameters;
	envFingerprint: EnvironmentFingerprint;
	stages: DecisionStage[];
	outcome: DecisionOutcome;
}

export interface ReplayStageAgreement {
	stage: StageName;
	match: boolean;
	storedOutputsDigest: string;
	replayedOutputsDigest: string;
}

export interface ReplayAgreement {
	traceId: number;
	configHash: string;
	configHashMatch: boolean;
	replayInputDigest: string;
	inputDigestMatch: boolean;
	stages: ReplayStageAgreement[];
	allMatch: boolean;
}

export type DecisionParseFailure =
	| 'not_an_object'
	| 'invalid_list'
	| 'invalid_detail'
	| 'invalid_replay';

export type DecisionParseResult<T> =
	| { ok: true; value: T }
	| { ok: false; reason: DecisionParseFailure };

export type DecisionListQuery = { limit: number; run_id?: number };

export type DecisionStatusKind =
	| 'unauthorized'
	| 'forbidden'
	| 'not_found'
	| 'conflict'
	| 'bad_request'
	| 'server'
	| 'unknown';

export interface DecisionStatus {
	kind: DecisionStatusKind;
	status: number;
	retryable: boolean;
}

const DIGEST = /^[0-9a-f]{64}$/;
const DECIMAL = /^(0|[1-9][0-9]*)$/;
const STAGE_ORDER: readonly StageName[] = [
	'normalize',
	'retrieve_context',
	'candidate_generation',
	'decision_model',
	'rules_policy',
	'rerank',
	'threshold',
	'action_escalation'
];

function isPlainObject(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function cleanString(value: unknown, max: number): string | undefined {
	if (typeof value !== 'string' || value.length > max) return undefined;
	return stripInvisible(value);
}

function requiredString(value: unknown, max: number): string | null {
	const cleaned = cleanString(value, max);
	return cleaned !== undefined && cleaned.length > 0 ? cleaned : null;
}


function digest(value: unknown, optional = false): string | null | undefined {
	if (value === null && optional) return null;
	if (typeof value !== 'string' || !DIGEST.test(value)) return undefined;
	return value;
}

function requiredDigest(value: unknown): string | null {
	const parsed = digest(value);
	return parsed === undefined ? null : parsed;
}

function optionalDigest(value: unknown): string | null | undefined {
	return digest(value, true);
}

function safeInteger(value: unknown, min = 0): number | null {
	if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < min) return null;
	return value;
}

function finiteInteger(value: unknown, min = 0): number | null {
	if (typeof value !== 'number' || !Number.isFinite(value) || !Number.isInteger(value) || value < min) {
		return null;
	}
	return value;
}

function finiteNumber(value: unknown): number | null {
	return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function optionalNumber(value: unknown): number | null | undefined {
	if (value === undefined || value === null) return null;
	return finiteNumber(value);
}

function mode(value: unknown): DecisionMode | null {
	if (value === 'deterministic' || value === 'exploratory') return value;
	return null;
}

function action(value: unknown): DecisionAction | null {
	if (value === 'act' || value === 'approve' || value === 'reject' || value === 'escalate') {
		return value;
	}
	return null;
}

function tier(value: unknown): TrustTier | null {
	if (value === 'untrusted' || value === 'vetted' || value === 'governed') return value;
	return null;
}

function leg(value: unknown): RetrievalLeg | null {
	if (value === 'both' || value === 'vector' || value === 'fts' || value === 'graph') return value;
	return null;
}

function stageName(value: unknown): StageName | null {
	switch (value) {
		case 'normalize':
		case 'retrieve_context':
		case 'candidate_generation':
		case 'decision_model':
		case 'rules_policy':
		case 'rerank':
		case 'threshold':
		case 'action_escalation':
			return value;
		default:
			return null;
	}
}

function escalationReason(value: unknown): EscalationReason | null {
	switch (value) {
		case 'out_of_vocabulary':
		case 'insufficient_evidence':
		case 'insufficient_evidence_tier':
		case 'invalid_input':
		case 'retrieval_unavailable':
		case 'misconfigured':
		case 'model_disabled':
		case 'policy_untrusted_only':
		case 'missing_decision':
		case 'fallback_escalate':
			return value;
		default:
			return null;
	}
}

function boundedStrings(
	value: unknown,
	maxItems: number,
	maxChars: number
): string[] | null {
	if (!Array.isArray(value) || value.length > maxItems) return null;
	const result: string[] = [];
	for (const item of value) {
		const cleaned = cleanString(item, maxChars);
		if (cleaned === undefined) return null;
		result.push(cleaned);
	}
	return result;
}

function parseSummary(value: unknown): DecisionRunSummary | null {
	if (!isPlainObject(value)) return null;
	const id = safeInteger(value['id']);
	const runId = safeInteger(value['run_id']);
	const parsedMode = mode(value['mode']);
	const pipelineVersion = requiredString(
		value['pipeline_version'],
		DECISION_LIMITS.identifierChars
	);
	const configHash = requiredDigest(value['config_hash']);
	const createdAt = safeInteger(value['created_at']);
	const stageCount = safeInteger(value['stage_count']);
	if (
		id === null ||
		runId === null ||
		parsedMode === null ||
		pipelineVersion === null ||
		configHash === null ||
		createdAt === null ||
		stageCount === null ||
		stageCount > DECISION_LIMITS.stages
	) {
		return null;
	}
	return {
		id,
		runId,
		mode: parsedMode,
		pipelineVersion,
		configHash,
		createdAt,
		stageCount
	};
}

/** Parse the bounded newest-first summary page without trusting its shape. */
export function parseDecisionRunList(value: unknown): DecisionParseResult<DecisionRunList> {
	if (!isPlainObject(value)) return { ok: false, reason: 'not_an_object' };
	const rawRows = value['rows'];
	const rawCount = value['count'];
	if (!Array.isArray(rawRows) || rawRows.length > DECISION_LIMITS.listRows) {
		return { ok: false, reason: 'invalid_list' };
	}
	const count = safeInteger(rawCount);
	if (count === null || count !== rawRows.length) return { ok: false, reason: 'invalid_list' };
	const rows: DecisionRunSummary[] = [];
	for (const row of rawRows) {
		const parsed = parseSummary(row);
		if (parsed === null) return { ok: false, reason: 'invalid_list' };
		rows.push(parsed);
	}
	return { ok: true, value: { rows, count } };
}

/** Parse the free-form stored trace into a bounded, display-safe projection. */
export function parseDecisionRunDetail(value: unknown): DecisionParseResult<DecisionRunDetail> {
	if (!isPlainObject(value)) return { ok: false, reason: 'not_an_object' };

	const runId = safeInteger(value['run_id']);
	const parsedMode = mode(value['mode']);
	const pipelineVersion = requiredString(
		value['pipeline_version'],
		DECISION_LIMITS.identifierChars
	);
	const configHash = requiredDigest(value['config_hash']);
	const inputDigest = requiredDigest(value['input_digest']);
	const rawModelRefs = value['model_refs'];
	const rawContextRefs = value['context_refs'];
	const rawStages = value['stages'];
	if (
		runId === null ||
		parsedMode === null ||
		pipelineVersion === null ||
		configHash === null ||
		inputDigest === null ||
		!Array.isArray(rawModelRefs) ||
		rawModelRefs.length > DECISION_LIMITS.modelRefs ||
		!Array.isArray(rawContextRefs) ||
		rawContextRefs.length > DECISION_LIMITS.contextRefs ||
		!Array.isArray(rawStages) ||
		rawStages.length > DECISION_LIMITS.stages
	) {
		return { ok: false, reason: 'invalid_detail' };
	}

	const modelRefs: ModelReference[] = [];
	for (const raw of rawModelRefs) {
		const parsed = parseModelReference(raw);
		if (parsed === null) return { ok: false, reason: 'invalid_detail' };
		modelRefs.push(parsed);
	}

	const contextRefs: ContextReference[] = [];
	for (const raw of rawContextRefs) {
		const parsed = parseContextReference(raw);
		if (parsed === null) return { ok: false, reason: 'invalid_detail' };
		contextRefs.push(parsed);
	}

	const retrievalParams = parseRetrievalParameters(value['retrieval_params']);
	const envFingerprint = parseEnvironmentFingerprint(value['env_fingerprint']);
	const stages: DecisionStage[] = [];
	let previousStageIndex = -1;
	for (const raw of rawStages) {
		const parsed = parseStage(raw);
		if (parsed === null) return { ok: false, reason: 'invalid_detail' };
		const stageIndex = STAGE_ORDER.indexOf(parsed.stage);
		if (stageIndex <= previousStageIndex) return { ok: false, reason: 'invalid_detail' };
		previousStageIndex = stageIndex;
		stages.push(parsed);
	}
	const outcome = parseOutcome(value['outcome']);
	if (
		retrievalParams === null ||
		envFingerprint === null ||
		outcome === null
	) {
		return { ok: false, reason: 'invalid_detail' };
	}

	return {
		ok: true,
		value: {
			runId,
			pipelineVersion,
			mode: parsedMode,
			configHash,
			modelRefs,
			inputDigest,
			contextRefs,
			retrievalParams,
			envFingerprint,
			stages,
			outcome
		}
	};
}

function parseModelReference(value: unknown): ModelReference | null {
	if (!isPlainObject(value)) return null;
	const id = requiredString(value['id'], DECISION_LIMITS.identifierChars);
	const version = requiredString(value['version'], DECISION_LIMITS.identifierChars);
	const weightsDigest = optionalDigest(value['weights_digest']);
	if (id === null || version === null || weightsDigest === undefined) return null;
	let registryId: string | null = null;
	let registryVersion: string | null = null;
	const registry = value['registry_ref'];
	if (registry !== undefined && registry !== null) {
		if (!isPlainObject(registry)) return null;
		registryId = requiredString(registry['registry_id'], DECISION_LIMITS.identifierChars);
		registryVersion = requiredString(
			registry['registry_version'],
			DECISION_LIMITS.identifierChars
		);
		if (registryId === null || registryVersion === null) return null;
	}
	return { id, version, weightsDigest, registryId, registryVersion };
}

function parseContextReference(value: unknown): ContextReference | null {
	if (!isPlainObject(value)) return null;
	const evidenceId = requiredString(value['evidence_id'], DECISION_LIMITS.identifierChars);
	const contentDigest = requiredDigest(value['content_digest']);
	const parsedTier = tier(value['tier']);
	const vectorRank = optionalNumber(value['vector_rank']);
	const ftsRank = optionalNumber(value['fts_rank']);
	const graphRank = optionalNumber(value['graph_rank']);
	const fusedScore = optionalNumber(value['fused_score']);
	if (
		evidenceId === null ||
		contentDigest === null ||
		parsedTier === null ||
		vectorRank === undefined ||
		ftsRank === undefined ||
		graphRank === undefined ||
		fusedScore === undefined ||
		typeof value['flagged'] !== 'boolean' ||
		typeof value['untrusted'] !== 'boolean'
	) {
		return null;
	}
	return {
		evidenceId,
		contentDigest,
		tier: parsedTier,
		vectorRank,
		ftsRank,
		graphRank,
		fusedScore,
		flagged: value['flagged'],
		untrusted: value['untrusted']
	};
}

function parseRetrievalParameters(value: unknown): RetrievalParameters | null {
	if (!isPlainObject(value)) return null;
	const rrfK = safeInteger(value['rrf_k'], 1);
	const limit = safeInteger(value['limit'], 1);
	const parsedLeg = leg(value['leg']);
	if (rrfK === null || rrfK > 1000 || limit === null || limit > 100 || parsedLeg === null) {
		return null;
	}
	return { rrfK, limit, leg: parsedLeg };
}

function parseEnvironmentFingerprint(value: unknown): EnvironmentFingerprint | null {
	if (!isPlainObject(value)) return null;
	const kernelVersion = requiredString(
		value['kernel_version'],
		DECISION_LIMITS.identifierChars
	);
	const featureFlags = boundedStrings(
		value['feature_flags'],
		DECISION_LIMITS.opaqueNodes,
		DECISION_LIMITS.labelChars
	);
	if (kernelVersion === null || featureFlags === null) return null;
	return { kernelVersion, featureFlags };
}

function parseStage(value: unknown): DecisionStage | null {
	if (!isPlainObject(value)) return null;
	const parsedStage = stageName(value['stage']);
	const algorithm = requiredString(value['algorithm'], DECISION_LIMITS.labelChars);
	const configHash = requiredDigest(value['config_hash']);
	const inputsDigest = requiredDigest(value['inputs_digest']);
	const outputsDigest = requiredDigest(value['outputs_digest']);
	const startedAt = finiteInteger(value['started_at']);
	const durationNs = finiteInteger(value['duration_ns']);
	const tiers = boundedTierList(value['trust_tiers_seen']);
	if (
		parsedStage === null ||
		!isPlainObject(value['outputs']) ||
		algorithm === null ||
		configHash === null ||
		inputsDigest === null ||
		outputsDigest === null ||
		startedAt === null ||
		durationNs === null ||
		tiers === null
	) {
		return null;
	}
	const outputPreview = opaquePreview(value['outputs']);
	return {
		stage: parsedStage,
		algorithm,
		configHash,
		inputsDigest,
		outputsDigest,
		outputPreview,
		trustTiersSeen: tiers,
		startedAt,
		durationNs
	};
}

function boundedTierList(value: unknown): TrustTier[] | null {
	if (!Array.isArray(value) || value.length > DECISION_LIMITS.stages) return null;
	const result: TrustTier[] = [];
	for (const item of value) {
		const parsed = tier(item);
		if (parsed === null) return null;
		result.push(parsed);
	}
	return result;
}

function parseOutcome(value: unknown): DecisionOutcome | null {
	if (!isPlainObject(value)) return null;
	const parsedAction = action(value['action']);
	if (parsedAction === null) return null;
	const rawEscalation = value['escalation'];
	let escalation: DecisionEscalation | null = null;
	if (rawEscalation !== undefined && rawEscalation !== null) {
		if (!isPlainObject(rawEscalation)) return null;
		const parsedStage = stageName(rawEscalation['stage']);
		const reason = escalationReason(rawEscalation['reason']);
		const detail = requiredString(rawEscalation['detail'], DECISION_LIMITS.labelChars);
		if (parsedStage === null || reason === null || detail === null) return null;
		escalation = { stage: parsedStage, reason, detail };
	}
	const rawOutput = value['output'];
	if (rawOutput !== undefined && rawOutput !== null && !isPlainObject(rawOutput)) return null;
	return {
		action: parsedAction,
		escalation,
		outputPreview: rawOutput === undefined || rawOutput === null ? null : opaquePreview(rawOutput)
	};
}


/** Parse a replay report; agreement fields remain data, not authority. */
export function parseReplayAgreement(value: unknown): DecisionParseResult<ReplayAgreement> {
	if (!isPlainObject(value)) return { ok: false, reason: 'not_an_object' };
	const traceId = safeInteger(value['trace_id']);
	const configHash = requiredDigest(value['config_hash']);
	const replayInputDigest = requiredDigest(value['replay_input_digest']);
	const rawStages = value['stages'];
	if (
		traceId === null ||
		configHash === null ||
		replayInputDigest === null ||
		typeof value['config_hash_match'] !== 'boolean' ||
		typeof value['input_digest_match'] !== 'boolean' ||
		typeof value['all_match'] !== 'boolean' ||
		!Array.isArray(rawStages) ||
		rawStages.length > DECISION_LIMITS.replayRows
	) {
		return { ok: false, reason: 'invalid_replay' };
	}
	const stages: ReplayStageAgreement[] = [];
	let previousStageIndex = -1;
	for (const raw of rawStages) {
		if (!isPlainObject(raw)) return { ok: false, reason: 'invalid_replay' };
		const parsedStage = stageName(raw['stage']);
		const storedOutputsDigest = requiredDigest(raw['stored_outputs_digest']);
		const replayedOutputsDigest = requiredDigest(raw['replayed_outputs_digest']);
		if (
			parsedStage === null ||
			storedOutputsDigest === null ||
			replayedOutputsDigest === null ||
			typeof raw['match'] !== 'boolean'
		) {
			return { ok: false, reason: 'invalid_replay' };
		}
		const stageIndex = STAGE_ORDER.indexOf(parsedStage);
		if (stageIndex <= previousStageIndex) return { ok: false, reason: 'invalid_replay' };
		previousStageIndex = stageIndex;
		stages.push({
			stage: parsedStage,
			match: raw['match'],
			storedOutputsDigest,
			replayedOutputsDigest
		});
	}
	return {
		ok: true,
		value: {
			traceId,
			configHash,
			configHashMatch: value['config_hash_match'],
			replayInputDigest,
			inputDigestMatch: value['input_digest_match'],
			stages,
			allMatch: value['all_match']
		}
	};
}

/** Build only the two query parameters the kernel actually supports. */
export function decisionRunListQuery(
	limit: number,
	runId?: number | null
): DecisionListQuery | null {
	if (!Number.isSafeInteger(limit) || limit < 1 || limit > DECISION_LIMITS.listRows) return null;
	if (runId === undefined || runId === null) return { limit };
	if (!Number.isSafeInteger(runId) || runId < 0) return null;
	return { limit, run_id: runId };
}

export function parseDecisionRouteId(raw: string | undefined): number | null {
	if (raw === undefined || !DECIMAL.test(raw)) return null;
	const id = Number(raw);
	return Number.isSafeInteger(id) && id >= 0 ? id : null;
}

export function classifyDecisionStatus(status: number): DecisionStatus {
	if (status === 401) return { kind: 'unauthorized', status, retryable: false };
	if (status === 403) return { kind: 'forbidden', status, retryable: false };
	if (status === 404) return { kind: 'not_found', status, retryable: false };
	if (status === 409) return { kind: 'conflict', status, retryable: false };
	if ([400, 413, 415, 422].includes(status)) {
		return { kind: 'bad_request', status, retryable: false };
	}
	if (status >= 500) return { kind: 'server', status, retryable: true };
	return { kind: 'unknown', status, retryable: false };
}

export function createDecisionRequestGuard(): {
	begin: () => number;
	isCurrent: (generation: number) => boolean;
} {
	let current = 0;
	return {
		begin: () => {
			current += 1;
			return current;
		},
		isCurrent: (generation) => generation === current
	};
}

export function rawCaptureWithinLimit(text: string): boolean {
	return new TextEncoder().encode(text).byteLength <= DECISION_LIMITS.rawCaptureBytes;
}

interface OpaqueState {
	nodes: number;
}

function opaquePreview(value: unknown): string | null {
	const state: OpaqueState = { nodes: 0 };
	try {
		const serialized = serializeOpaque(value, 0, state);
		if (serialized === null) return null;
		const cleaned = stripInvisible(serialized);
		return cleaned.length <= DECISION_LIMITS.opaquePreviewChars ? cleaned : null;
	} catch {
		return null;
	}
}

function serializeOpaque(value: unknown, depth: number, state: OpaqueState): string | null {
	state.nodes += 1;
	if (state.nodes > DECISION_LIMITS.opaqueNodes || depth > DECISION_LIMITS.opaqueDepth) return null;
	if (value === null) return 'null';
	if (typeof value === 'string') {
		if (value.length > DECISION_LIMITS.opaquePreviewChars) return null;
		const encoded = JSON.stringify(value);
		return encoded === undefined ? null : encoded;
	}
	if (typeof value === 'boolean') return value ? 'true' : 'false';
	if (typeof value === 'number') return Number.isFinite(value) ? JSON.stringify(value) : null;
	if (Array.isArray(value)) {
		const parts: string[] = [];
		for (const item of value) {
			const part = serializeOpaque(item, depth + 1, state);
			if (part === null) return null;
			parts.push(part);
		}
		return `[${parts.join(',')}]`;
	}
	if (!isPlainObject(value)) return null;
	const keys = Object.keys(value);
	const parts: string[] = [];
	for (const key of keys) {
		if (key.length > DECISION_LIMITS.labelChars) return null;
		const encodedKey = JSON.stringify(key);
		const part = serializeOpaque(value[key], depth + 1, state);
		if (encodedKey === undefined || part === null) return null;
		parts.push(`${encodedKey}:${part}`);
	}
	return `{${parts.join(',')}}`;
}
