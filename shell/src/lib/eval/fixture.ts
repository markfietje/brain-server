/**
 * The frozen eval fixture's parser and scorer (Plan 08 §1).
 *
 * ONE parser, shared by the preregistration pin and the baseline runner, so
 * the criterion and the measurement cannot be derived from two different
 * readings of the same file. It reads the FIXTURE
 * (`tests/fixtures/eval_queries.md`) and never `tests/eval.rs`: the fixture is
 * the frozen artifact, and the plan's rule is that a harness which changes
 * between the baseline and a candidate measures two different things.
 *
 * The parser is deliberately STRICT. Every deviation it could tolerate is a
 * place where a silently-empty query set would look like a clean run.
 */

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/** One judged query: its text, the corpus indices that answer it, and its vertical. */
export interface JudgedQuery {
	/** The fixture's own id (`Q1`, `Q2`, …) — stable across re-parses. */
	id: string;
	vertical: string;
	query: string;
	/** 0-based `DOCS` indices. EMPTY for a negation probe — deliberately so. */
	relevant: number[];
}

export interface EvalFixture {
	sha256: string;
	/** `DOCS`, 0-based, in fixture order. */
	docs: string[];
	queries: JudgedQuery[];
	/** The queries whose gold set is empty: they measure that absent terms stay absent. */
	negationProbes: JudgedQuery[];
}

function sha256(text: string): string {
	return createHash('sha256').update(text, 'utf8').digest('hex');
}

/** The `| 0 | text |` corpus rows. */
function parseCorpus(markdown: string): string[] {
	const docs: string[] = [];
	for (const line of markdown.split('\n')) {
		const row = /^\|\s*(\d+)\s*\|\s*(.*?)\s*\|\s*$/.exec(line);
		if (row === null) continue;
		const index = Number(row[1]);
		if (index !== docs.length) {
			throw new Error(`eval fixture corpus is not 0-based contiguous at row ${index}`);
		}
		docs.push(row[2] ?? '');
	}
	return docs;
}

/** The `### Qn — [vertical]` / `Query:` / `Relevant:` blocks. */
function parseQueries(markdown: string, docCount: number): JudgedQuery[] {
	const queries: JudgedQuery[] = [];
	const lines = markdown.split('\n');
	for (let i = 0; i < lines.length; i++) {
		const header = /^###\s+(Q\d+)\s+—\s+\[([^\]]+)\]\s*$/.exec(lines[i] ?? '');
		if (header === null) continue;
		const id = header[1] as string;
		const vertical = header[2] as string;
		const queryLine = lines[i + 1] ?? '';
		const relevantLine = lines[i + 2] ?? '';
		const query = /^Query:\s*"(.*)"\s*$/.exec(queryLine);
		const relevant = /^Relevant:\s*\[([^\]]*)\]\s*$/.exec(relevantLine);
		if (query === null || relevant === null) {
			throw new Error(`eval fixture query ${id} is malformed (Query/Relevant on the lines after it)`);
		}
		const indices = (relevant[1] ?? '')
			.split(',')
			.map((part) => part.trim())
			.filter((part) => part.length > 0)
			.map((part) => {
				const n = Number(part);
				if (!Number.isInteger(n) || n < 0 || n >= docCount) {
					throw new Error(`eval fixture query ${id} references out-of-bounds doc ${part}`);
				}
				return n;
			});
		queries.push({ id, vertical, query: query[1] ?? '', relevant: indices });
	}
	return queries;
}

/** Parses the frozen fixture's bytes. Throws rather than returning a partial set. */
export function parseEvalFixture(markdown: string): EvalFixture {
	const docs = parseCorpus(markdown);
	if (docs.length === 0) throw new Error('eval fixture carries no corpus rows');
	const queries = parseQueries(markdown, docs.length);
	if (queries.length === 0) throw new Error('eval fixture carries no judged queries');
	const seen = new Set<string>();
	for (const q of queries) {
		if (seen.has(q.id)) throw new Error(`eval fixture query id ${q.id} appears twice`);
		seen.add(q.id);
	}
	return {
		sha256: sha256(markdown),
		docs,
		queries,
		negationProbes: queries.filter((q) => q.relevant.length === 0)
	};
}

/** Reads the frozen fixture from the kernel root. */
export function loadEvalFixture(kernelRoot: string): EvalFixture {
	const path = join(kernelRoot, 'tests', 'fixtures', 'eval_queries.md');
	return parseEvalFixture(readFileSync(path, 'utf8'));
}

/**
 * recall@k: the fraction of a query's relevant docs found in the top k.
 * An empty gold set scores 1.0 by convention — a negation probe passes by
 * returning NOTHING relevant, so scoring it 0.0 would invert its meaning.
 * `k` is applied to the ORDERED id list, so a count-based shortcut cannot
 * pass: the order is the measurement.
 */
export function recallAtK(orderedIds: number[], relevant: number[], k: number): number {
	if (relevant.length === 0) return 1.0;
	const topK = new Set(orderedIds.slice(0, k));
	return relevant.filter((r) => topK.has(r)).length / relevant.length;
}

/** recall@k over the whole judged set, and the per-query rows behind it. */
export function scoreRecall(
	queries: JudgedQuery[],
	results: Map<string, number[]>,
	k: number
): { recall: number; rows: Array<{ id: string; query: string; relevant: number[]; ids: number[]; recall: number }> } {
	const rows = queries.map((q) => {
		const ids = results.get(q.id) ?? [];
		return {
			id: q.id,
			query: q.query,
			relevant: q.relevant,
			ids,
			recall: recallAtK(ids, q.relevant, k)
		};
	});
	const recall = rows.reduce((sum, row) => sum + row.recall, 0) / rows.length;
	return { recall, rows };
}
