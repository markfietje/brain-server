import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import {
	classifyDecisionStatus,
	createDecisionRequestGuard,
	decisionRunListQuery,
	DECISION_LIMITS,
	parseDecisionRouteId,
	parseDecisionRunDetail,
	parseDecisionRunList,
	parseReplayAgreement,
	rawCaptureWithinLimit
} from '../src/lib/decision-run';

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function fixture(): Record<string, unknown> {
	const parsed: unknown = JSON.parse(
		readFileSync(join(process.cwd(), 'tests', 'fixtures', 'decision-run.json'), 'utf8')
	);
	if (!isRecord(parsed)) throw new Error('decision fixture must be an object');
	return parsed;
}

function summary(index: number): Record<string, unknown> {
	return {
		id: index + 1,
		run_id: 40 + index,
		mode: index % 2 === 0 ? 'deterministic' : 'exploratory',
		pipeline_version: '1.32.11',
		config_hash: 'a'.repeat(64),
		created_at: 1750000000 + index,
		stage_count: 2
	};
}

function replay(): Record<string, unknown> {
	return {
		trace_id: 9,
		config_hash: 'a'.repeat(64),
		config_hash_match: true,
		replay_input_digest: 'b'.repeat(64),
		input_digest_match: true,
		stages: [
			{
				stage: 'normalize',
				match: true,
				stored_outputs_digest: 'c'.repeat(64),
				replayed_outputs_digest: 'c'.repeat(64)
			}
		],
		all_match: true
	};
}

describe('decision-run runtime projection', () => {
	test('decision_run_parser_accepts_the_shipped_trace_projection', () => {
		const result = parseDecisionRunDetail(fixture());
		expect(result.ok).toBe(true);
		if (!result.ok) return;
		expect(result.value.mode).toBe('exploratory');
		expect(result.value.runId).toBe(42);
		expect(result.value.contextRefs[0]?.tier).toBe('untrusted');
		expect(result.value.stages.map((stage) => stage.stage)).toEqual([
			'normalize',
			'retrieve_context'
		]);
		expect(result.value.outcome.action).toBe('escalate');
		expect(result.value.outcome.escalation?.reason).toBe('policy_untrusted_only');
		expect(result.value.stages[0]?.outputPreview).toContain('query_digest');
	});

	test('decision_run_parser_refuses_malformed_and_unbounded_values', () => {
		const rows = Array.from({ length: DECISION_LIMITS.listRows + 1 }, (_, index) => summary(index));
		expect(parseDecisionRunList({ rows, count: rows.length }).ok).toBe(false);
		expect(parseDecisionRunList({ rows: [summary(0)], count: 2 }).ok).toBe(false);
		expect(parseDecisionRunList({ rows: [{ ...summary(0), mode: 'other' }], count: 1 }).ok).toBe(
			false
		);
		expect(parseDecisionRunList({ rows: [{ ...summary(0), stage_count: 9 }], count: 1 }).ok).toBe(
			false
		);
		expect(parseDecisionRunList({ rows: [{ ...summary(0), id: 1.5 }], count: 1 }).ok).toBe(false);
		expect(
			parseDecisionRunList({
				rows: [{ ...summary(0), pipeline_version: 'x'.repeat(DECISION_LIMITS.identifierChars + 1) }],
				count: 1
			}).ok
		).toBe(false);

		const malformed = fixture();
		malformed['run_id'] = Number.NaN;
		expect(parseDecisionRunDetail(malformed).ok).toBe(false);

		const tooManyModels = fixture();
		tooManyModels['model_refs'] = Array.from({ length: DECISION_LIMITS.modelRefs + 1 }, () => ({
			id: 'model',
			version: '1',
			weights_digest: null
		}));
		expect(parseDecisionRunDetail(tooManyModels).ok).toBe(false);

		const badStage = fixture();
		const stages = badStage['stages'];
		if (!Array.isArray(stages) || !isRecord(stages[0])) throw new Error('fixture stages');
		stages[0]['stage'] = 'unknown_stage';
		expect(parseDecisionRunDetail(badStage).ok).toBe(false);

		expect(parseReplayAgreement({ ...replay(), all_match: 'yes' }).ok).toBe(false);
		expect(parseReplayAgreement({ ...replay(), stages: [] }).ok).toBe(true);
	});

	test('decision_run_parser_strips_invisible_wire_text_without_html', () => {
		const hostile = fixture();
		hostile['pipeline_version'] = '1.32\u200B.11';
		const modelRefs = hostile['model_refs'];
		if (!Array.isArray(modelRefs) || !isRecord(modelRefs[0])) throw new Error('fixture models');
		modelRefs[0]['id'] = 'rules\u202Efixture';
		const contextRefs = hostile['context_refs'];
		if (!Array.isArray(contextRefs) || !isRecord(contextRefs[0])) throw new Error('fixture contexts');
		contextRefs[0]['evidence_id'] = 'evidence\u2066-1';
		const stages = hostile['stages'];
		if (!Array.isArray(stages) || !isRecord(stages[0])) throw new Error('fixture stages');
		stages[0]['algorithm'] = '<script>alert(1)</script>\uFEFF';

		const result = parseDecisionRunDetail(hostile);
		expect(result.ok).toBe(true);
		if (!result.ok) return;
		expect(result.value.pipelineVersion).toBe('1.32.11');
		expect(result.value.modelRefs[0]?.id).toBe('rulesfixture');
		expect(result.value.contextRefs[0]?.evidenceId).toBe('evidence-1');
		expect(result.value.stages[0]?.algorithm).toBe('<script>alert(1)</script>');
		expect(result.value.stages[0]?.outputPreview).not.toContain('\u200B');
	});

	test('decision_list_uses_only_limit_and_run_id_server_queries', () => {
		expect(decisionRunListQuery(20)).toEqual({ limit: 20 });
		expect(decisionRunListQuery(50, 42)).toEqual({ limit: 50, run_id: 42 });
		expect(decisionRunListQuery(0)).toBeNull();
		expect(decisionRunListQuery(51)).toBeNull();
		expect(decisionRunListQuery(20, -1)).toBeNull();
		expect(Object.keys(decisionRunListQuery(20, 42) ?? {})).toEqual(['limit', 'run_id']);
	});

	test('decision_run_parser_classifies_http_and_route_states', () => {
		expect(classifyDecisionStatus(401).kind).toBe('unauthorized');
		expect(classifyDecisionStatus(403).kind).toBe('forbidden');
		expect(classifyDecisionStatus(404).kind).toBe('not_found');
		expect(classifyDecisionStatus(409).kind).toBe('conflict');
		expect(classifyDecisionStatus(422).kind).toBe('bad_request');
		expect(classifyDecisionStatus(503).retryable).toBe(true);
		expect(parseDecisionRouteId('42')).toBe(42);
		expect(parseDecisionRouteId('-1')).toBeNull();
		expect(parseDecisionRouteId('1.2')).toBeNull();
		expect(parseDecisionRouteId(' 42')).toBeNull();
	});

	test('decision_replay_requires_operator_supplied_inputs', () => {
		const report = parseReplayAgreement(replay());
		expect(report.ok).toBe(true);
		if (!report.ok) return;
		expect(report.value.configHashMatch).toBe(true);
		expect(report.value.inputDigestMatch).toBe(true);
		expect(report.value.allMatch).toBe(true);
	});

	test('decision_replay_never_persists_or_logs_raw_inputs', () => {
		const first = createDecisionRequestGuard();
		const oldRequest = first.begin();
		const newRequest = first.begin();
		expect(first.isCurrent(oldRequest)).toBe(false);
		expect(first.isCurrent(newRequest)).toBe(true);
		expect(rawCaptureWithinLimit('synthetic')).toBe(true);
		expect(rawCaptureWithinLimit('x'.repeat(DECISION_LIMITS.rawCaptureBytes + 1))).toBe(false);
	});
});
