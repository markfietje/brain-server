/**
 * Dedicated-port browser harness.
 *
 * The shell tests boot a fresh, temporary kernel on 127.0.0.1:8799. The
 * operator's 8765 service is never queried or mutated. Both the R25 recall
 * fixture and the R32 decision fixture are synthetic and are seeded through
 * the kernel's public HTTP routes before the browser starts.
 */
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { createConnection } from 'node:net';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const shellRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const kernelRoot = join(shellRoot, '..');
const KERNEL_PORT = Number(process.env['E2E_KERNEL_PORT'] ?? 8799);
const KERNEL_URL = `http://127.0.0.1:${KERNEL_PORT}`;

let kernel: ChildProcess | null = null;
let dataDir: string | null = null;

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function requiredNumber(value: unknown, label: string): number {
	if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
		throw new Error(`e2e seed ${label} was not a non-negative safe integer`);
	}
	return value;
}

function requiredDigest(value: unknown, label: string): string {
	if (typeof value !== 'string' || !/^[0-9a-f]{64}$/.test(value)) {
		throw new Error(`e2e seed ${label} was not a lowercase SHA-256 digest`);
	}
	return value;
}

async function postJson(path: string, body: Record<string, unknown>): Promise<Record<string, unknown>> {
	const response = await fetch(`${KERNEL_URL}${path}`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(body)
	});
	if (!response.ok) throw new Error(`e2e seed POST ${path} failed (${response.status})`);
	const parsed: unknown = await response.json();
	if (!isRecord(parsed)) throw new Error(`e2e seed POST ${path} returned a non-object`);
	return parsed;
}

async function getJson(path: string): Promise<Record<string, unknown>> {
	const response = await fetch(`${KERNEL_URL}${path}`);
	if (!response.ok) throw new Error(`e2e verification GET ${path} failed (${response.status})`);
	const parsed: unknown = await response.json();
	if (!isRecord(parsed)) throw new Error(`e2e verification GET ${path} returned a non-object`);
	return parsed;
}

function portIsFree(port: number): Promise<boolean> {
	return new Promise((resolve) => {
		const socket = createConnection({ port, host: '127.0.0.1' });
		socket.once('connect', () => {
			socket.destroy();
			resolve(false);
		});
		socket.once('error', () => {
			socket.destroy();
			resolve(true);
		});
	});
}

async function waitForHealth(timeoutMs: number): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		if (kernel && kernel.exitCode !== null) {
			throw new Error(`the e2e kernel exited early (code ${kernel.exitCode})`);
		}
		try {
			const response = await fetch(`${KERNEL_URL}/health`);
			if (response.ok) return;
		} catch {
			// The child is still starting.
		}
		await new Promise((resolve) => setTimeout(resolve, 500));
	}
	throw new Error(`the e2e kernel did not become healthy on ${KERNEL_URL} in time`);
}

async function cleanup(): Promise<void> {
	const child = kernel;
	kernel = null;
	if (child && child.exitCode === null) {
		child.kill('SIGTERM');
		await new Promise<void>((resolve) => child.once('exit', () => resolve()));
	}
	const directory = dataDir;
	dataDir = null;
	if (directory) rmSync(directory, { recursive: true, force: true });
}

async function seedRecallFixture(): Promise<void> {
	const add = await postJson('/add', {
		text: 'StewardOS R32 decision explorer seed: r32 decision explorer seed supports a governed decision trace.',
		title: 'R32 e2e seed'
	});
	if (add['success'] !== true) throw new Error('the e2e /add seed did not report success');

	const recall = await postJson('/recall', {
		v: 1,
		query: 'verbatim export gate',
		limit: 10,
		trace: true
	});
	const traceId = requiredNumber(recall['trace_id'], 'recall trace_id');
	process.env['E2E_TRACE_ID'] = String(traceId);
	console.log(`[e2e] traced recall recorded: trace_id ${traceId}`);
}

async function seedDecisionFixture(): Promise<void> {
	const run = await postJson('/workflow/runs', {
		domain: 'global',
		kind: 'troubleshoot',
		state_json: '{}'
	});
	const runId = requiredNumber(run['run_id'], 'workflow run_id');

	const rulesConfig: Record<string, unknown> = {
		model_id: 'rules-r32-e2e',
		model_version: '1.0.0',
		rules: [
			{
				question_id: 'needs_human',
				min_evidence: 1,
				min_tier: 'untrusted',
				output: {
					Choice: { options: ['act', 'reject'], label: 'act' }
				}
			}
		]
	};
	const registration = await postJson('/workflow/model-registry/register', {
		kind: 'deterministic-rules',
		rules_config: rulesConfig
	});
	if (registration['id'] !== 'rules-r32-e2e' || registration['version'] !== '1.0.0') {
		throw new Error('the e2e model registration identity was not the synthetic candidate');
	}
	if (registration['status'] !== 'candidate') throw new Error('the e2e model was not a candidate');
	const configDigest = requiredDigest(registration['config_digest'], 'model config_digest');

	const config: Record<string, unknown> = {
		config_schema: 'harness.pipeline/v1',
		pipeline_id: 'r32-e2e',
		stages: [
			'normalize',
			'retrieve_context',
			'candidate_generation',
			'decision_model',
			'rules_policy',
			'rerank',
			'threshold',
			'action_escalation'
		],
		retrieval: { rrf_k: 60, limit: 5, leg: 'both' },
		model: { key: 'rules:rules-r32-e2e', digest: configDigest },
		thresholds: {
			act_labels: ['act'],
			reject_labels: ['reject'],
			score_act_at_or_above: 50,
			score_reject_at_or_below: 10,
			noul_act_when: true,
			fallback: 'approve'
		}
	};
	const decision = await postJson('/workflow/decision-runs', {
		config,
		rules_config: rulesConfig,
		run_id: runId,
		mode: 'exploratory',
		request_id: 'r32-e2e-request',
		question_id: 'needs_human',
		question_kind: 'choice',
		question_ids: ['needs_human'],
		query: 'r32 decision explorer seed'
	});
	const traceId = requiredNumber(decision['trace_id'], 'decision trace_id');
	const trace = await getJson(`/workflow/decision-runs/${traceId}`);
	if (trace['run_id'] !== runId || trace['mode'] !== 'exploratory') {
		throw new Error('the e2e decision trace identity/mode did not verify');
	}
	requiredDigest(trace['config_hash'], 'trace config_hash');
	requiredDigest(trace['input_digest'], 'trace input_digest');
	const modelRefs = trace['model_refs'];
	if (
		!Array.isArray(modelRefs) ||
		!modelRefs.some(
			(reference) =>
				isRecord(reference) &&
				reference['id'] === 'rules-r32-e2e' &&
				reference['version'] === '1.0.0' &&
				isRecord(reference['registry_ref']) &&
				reference['registry_ref']['registry_id'] === 'rules-r32-e2e' &&
				reference['registry_ref']['registry_version'] === '1.0.0'
		) ||
		!Array.isArray(trace['stages'])
	) {
		throw new Error('the e2e decision trace provenance did not verify');
	}
	process.env['E2E_DECISION_RUN_ID'] = String(runId);
	process.env['E2E_DECISION_TRACE_ID'] = String(traceId);
	process.env['E2E_DECISION_CONFIG_DIGEST'] = configDigest;
	console.log(`[e2e] decision trace recorded: trace_id ${traceId}`);
}

export default async function (): Promise<() => Promise<void>> {
	if (!(await portIsFree(KERNEL_PORT))) {
		throw new Error(
			`port ${KERNEL_PORT} is already in use — refusing to run e2e against a process this harness did not boot`
		);
	}

	try {
		console.log('[e2e] building the current kernel server binary (debug)…');
		const build = spawnSync('cargo', ['build', '--offline', '--locked', '--bin', 'brain-server'], {
			cwd: kernelRoot,
			stdio: 'inherit'
		});
		if (build.status !== 0) throw new Error('kernel server build failed');

		dataDir = mkdtempSync(join(tmpdir(), 'decision-e2e-kernel-'));
		const childEnv = { ...process.env };
		delete childEnv['AUTH_TOKEN'];
		delete childEnv['AUTH_TOKEN_FILE'];
		delete childEnv['BRAIN_TOKEN'];
		delete childEnv['BRAIN_TOKEN_FILE'];
		kernel = spawn(join(kernelRoot, 'target/debug/brain-server'), [], {
			cwd: kernelRoot,
			env: {
				...childEnv,
				BIND_HOST: '127.0.0.1',
				BIND_PORT: String(KERNEL_PORT),
				BRAIN_DB_PATH: join(dataDir, 'brain.db'),
				BRAIN_WRITE_POSTURE: 'open',
				BRAIN_AUDIT_READ_EVENTS: 'on',
				CORS_ORIGINS: 'http://127.0.0.1:4173'
			},
			stdio: ['ignore', 'pipe', 'pipe']
		});
		kernel.stdout?.on('data', (data) => process.stdout.write(`[kernel] ${data}`));
		kernel.stderr?.on('data', (data) => process.stderr.write(`[kernel!] ${data}`));
		await waitForHealth(120_000);
		console.log('[e2e] e2e kernel healthy');

		await seedRecallFixture();
		await seedDecisionFixture();
		return cleanup;
	} catch (error) {
			await cleanup();
		throw error;
	}
}
