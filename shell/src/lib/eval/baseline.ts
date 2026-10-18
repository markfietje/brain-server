/**
 * The baseline harness (Plan 08 §3 run protocol).
 *
 * Boots a DEDICATED kernel on its own port over a temporary database, seeds the
 * frozen fixture's own corpus through the kernel's public routes, and runs the
 * fixture's judged queries. The operator's 8765 service is never queried or
 * mutated for the retrieval measurement — that service is used for exactly one
 * thing (see `unauthorizedProbe`), because a refusal can only be observed
 * against a kernel that actually refuses.
 *
 * `N >= 3` repeats per case is Plan 08 §3's protocol. Retrieval here is
 * DETERMINISTIC — the same query against the same corpus returns the same
 * ordered ids — so repeats measure the harness rather than the system, and the
 * protocol's reason (single-run deltas are noise on a 4096-token model) does not
 * apply to a retriever. The determinism is asserted rather than assumed: the
 * run repeats three times and fails if an ordered id sequence moves.
 */

import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { recallAtK, scoreRecall, type EvalFixture } from './fixture';
import { client, setApiBase } from '../api/client';
import type { paths } from '../api/schema';

/** The request body, taken from the GENERATED contract rather than written out.
 * This is the drift gate doing its job in the harness too: if `QueryDoc` loses
 * or renames a field, the harness stops compiling instead of quietly sending the
 * wrong body and recording a retrieval number for it. */
type RecallBody = paths['/recall']['post']['requestBody']['content']['application/json'];

export interface BootedKernel {
	base: string;
	stop: () => void;
}

async function waitForHealth(base: string, child: ChildProcess, timeoutMs: number): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		if (child.exitCode !== null) {
			throw new Error(`the baseline kernel exited early (code ${child.exitCode})`);
		}
		try {
			const response = await fetch(`${base}/health`);
			if (response.ok) return;
		} catch {
			// still starting
		}
		await new Promise((resolve) => setTimeout(resolve, 250));
	}
	throw new Error(`the baseline kernel did not become healthy on ${base}`);
}

/**
 * Boots the debug kernel this working tree builds, on its own port, over a
 * temporary database, with authentication OFF.
 *
 * Auth off is the LOOPBACK POSTURE, not a shortcut: a kernel with no token
 * resolved serves unauthenticated by design, and the retrieval measurement is
 * about retrieval. The refusal path is exercised separately against a kernel
 * that really does enforce auth, because a 401 manufactured by a local stub is
 * not evidence that the kernel refuses.
 */
export async function bootKernel(kernelRoot: string, port: number): Promise<BootedKernel> {
	const build = spawnSync('cargo', ['build', '--offline', '--locked', '--bin', 'brain-server'], {
		cwd: kernelRoot,
		stdio: 'inherit'
	});
	if (build.status !== 0) throw new Error('the baseline kernel build failed');

	const dataDir = mkdtempSync(join(tmpdir(), 'baseline-kernel-'));
	const childEnv = { ...process.env };
	for (const key of ['AUTH_TOKEN', 'AUTH_TOKEN_FILE', 'BRAIN_TOKEN', 'BRAIN_TOKEN_FILE']) {
		delete childEnv[key];
	}
	const child = spawn(join(kernelRoot, 'target/debug/brain-server'), [], {
		cwd: kernelRoot,
		env: {
			...childEnv,
			BIND_HOST: '127.0.0.1',
			BIND_PORT: String(port),
			BRAIN_DB_PATH: join(dataDir, 'brain.db'),
			BRAIN_WRITE_POSTURE: 'open',
			BRAIN_AUDIT_READ_EVENTS: 'off'
		},
		stdio: ['ignore', 'ignore', 'inherit']
	});
	const base = `http://127.0.0.1:${port}`;
	try {
		await waitForHealth(base, child, 120_000);
	} catch (error) {
		child.kill('SIGKILL');
		rmSync(dataDir, { recursive: true, force: true });
		throw error;
	}
	return {
		base,
		stop: () => {
			child.kill('SIGTERM');
			rmSync(dataDir, { recursive: true, force: true });
		}
	};
}

/** Seeds the fixture's own corpus. The FIXTURE's text, not `eval.rs`'s: the
 * fixture's rows are truncated to 99 characters and `eval.rs` carries the full
 * sentences, so the two corpora are not the same corpus. The baseline is bound
 * to the artifact Plan 08 §1 names, and the difference is recorded in the
 * preregistration rather than papered over. */
export async function seedCorpus(base: string, docs: string[]): Promise<void> {
	for (const [index, text] of docs.entries()) {
		const response = await fetch(`${base}/add`, {
			method: 'POST',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify({ text, title: `corpus doc ${index}` })
		});
		if (!response.ok) {
			throw new Error(`seeding corpus doc ${index} failed (${response.status})`);
		}
	}
}

/** One measured exchange, verbatim: the status, the ordered ids, the decision
 * and the confidence. Deliberately NOT a summary — this row IS the baseline. */
export interface QueryRow {
	id: string;
	vertical: string;
	query: string;
	relevant: number[];
	/** The kernel's ordered hit ids, mapped back to corpus indices. */
	ids: number[];
	/** The same ids as the SHELL's typed wire returned them — T1's subject. */
	typedIds: number[];
	decision: string;
	confidence: number | null;
}

async function postRecall(
	base: string,
	body: RecallBody
): Promise<{ status: number; json: Record<string, unknown> }> {
	const response = await fetch(`${base}/recall`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(body)
	});
	const json = (await response.json()) as Record<string, unknown>;
	return { status: response.status, json };
}

/** Maps a hit back to its corpus index by EXACT content, then by a 40-character
 * prefix. Exact-first because a prefix match alone would collapse two docs that
 * share an opening, and a silent collapse there would look like a retrieval
 * miss rather than a mapping fault. */
function corpusMapper(docs: string[]): (content: string) => number | null {
	const prefixes = docs.map((doc, index) => ({ prefix: doc.slice(0, 40), index }));
	return (content: string) => {
		const exact = docs.indexOf(content);
		if (exact >= 0) return exact;
		return prefixes.find((entry) => content.startsWith(entry.prefix))?.index ?? null;
	};
}

/**
 * Runs the fixture. Two wires per query: the kernel over plain `fetch` (the
 * control) and the shell's TYPED client (the subject). T1 compares the two
 * ORDERED id sequences.
 */
export async function runBaseline(
	base: string,
	fixture: EvalFixture,
	repeats: number
): Promise<{ rows: QueryRow[]; unmapped: number }> {
	const mapContent = corpusMapper(fixture.docs);
	const rows: QueryRow[] = [];
	let unmapped = 0;

	for (const judged of fixture.queries) {
		const body: RecallBody = { v: 1, query: judged.query, limit: 5, provenance: true };
		const control = await postRecall(base, body);
		if (control.status !== 200) {
			throw new Error(`query ${judged.id} returned ${control.status} from the control wire`);
		}
		const hits = (control.json['hits'] ?? []) as Array<{ content?: string }>;
		const ids = hits.map((hit) => {
			const index = mapContent(hit.content ?? '');
			if (index === null) unmapped += 1;
			// An unmappable hit is recorded as -1 rather than dropped: dropping
			// it would shorten the sequence and quietly improve recall@k.
			return index ?? -1;
		});

		// T1's other half: the SHELL's typed wire, through the generated client
		// the app actually calls. `setApiBase` refuses a non-loopback origin, so
		// this cannot silently aim the measurement at a remote kernel.
		const typed = await runTypedWire(base, body, mapContent);

		// Determinism (Plan 08 §3's N>=3, applied where it means something):
		// a retriever that moves rows between identical runs is not measuring
		// what this baseline claims to measure.
		for (let repeat = 1; repeat < repeats; repeat++) {
			const again = await postRecall(base, body);
			const againIds = ((again.json['hits'] ?? []) as Array<{ content?: string }>).map(
				(hit) => mapContent(hit.content ?? '') ?? -1
			);
			if (againIds.join(',') !== ids.join(',')) {
				throw new Error(
					`query ${judged.id} is NOT deterministic: repeat ${repeat} returned ` +
						`[${againIds.join(',')}] against [${ids.join(',')}]`
				);
			}
		}

		rows.push({
			id: judged.id,
			vertical: judged.vertical,
			query: judged.query,
			relevant: judged.relevant,
			ids,
			typedIds: typed.ids,
			decision: String(control.json['decision'] ?? ''),
			confidence:
				typeof (control.json['telemetry'] as { confidence?: unknown } | undefined)
					?.confidence === 'number'
					? ((control.json['telemetry'] as { confidence: number }).confidence)
					: null
		});
	}
	return { rows, unmapped };
}

/**
 * The same request over the shell's TYPED wire — `client.POST('/recall', …)`
 * from `src/lib/api/client.ts`, the generated, drift-gated client the app calls.
 *
 * This is the subject of T1. The control above is plain `fetch`; if the typed
 * wire loses, duplicates or reorders a row, the two ordered sequences differ
 * and the conclusion is about the BRIDGE, never about retrieval quality.
 */
async function runTypedWire(
	base: string,
	body: RecallBody,
	mapContent: (content: string) => number | null
): Promise<{ status: number; ids: number[] }> {
	setApiBase(base);
	const { data, response } = await client.POST('/recall', { body });
	const hits = (data as { hits?: Array<{ content?: string }> } | undefined)?.hits ?? [];
	return {
		status: response.status,
		ids: hits.map((hit) => mapContent(hit.content ?? '') ?? -1)
	};
}

/** recall@5 over the run, from the shared scorer so the pin and the record
 * cannot be derived from two different readings of the metric. */
export function scoreRun(fixture: EvalFixture, rows: QueryRow[]): {
	recall5: number;
	zeroHit: string[];
} {
	const results = new Map(rows.map((row) => [row.id, row.ids]));
	const { recall } = scoreRecall(fixture.queries, results, 5);
	return {
		recall5: recall,
		zeroHit: rows.filter((row) => row.ids.length === 0).map((row) => row.id)
	};
}

export { recallAtK };

/**
 * The refusal probe — and this is the one measurement taken against the
 * OPERATOR's live kernel rather than the temporary one.
 *
 * A refusal must be observed where auth is actually enforced. Manufacturing a
 * 401 with a local stub would prove that the client can tell 401 from 200, and
 * nothing about whether the kernel ever says 401. So: an unauthenticated call
 * to the operator's own loopback kernel, which is expected to refuse.
 */
export async function unauthorizedProbe(operatorBase: string): Promise<{
	status: number;
	refused: boolean;
}> {
	const response = await fetch(`${operatorBase}/recall`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ v: 1, query: 'baseline refusal probe', limit: 1 })
	});
	return { status: response.status, refused: response.status === 401 || response.status === 403 };
}
