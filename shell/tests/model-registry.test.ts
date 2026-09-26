import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import { stripInvisible } from '../src/lib/sanitize';
import {
	MODEL_REGISTRY_LIMITS,
	classifyModelRegistryStatus,
	isLegalLifecycleAction,
	lifecycleProposalContent,
	modelRegistryListQuery,
	modelRegistryListQueryParams,
	parseModelRef,
	parseModelRegistryDetail,
	parseModelRegistryList,
	REGISTRY_ROW_FIELDS
} from '../src/lib/model-registry';

/**
 * R36 M6-S2 runtime/parser contract. RED-first: this file landed before
 * src/lib/model-registry.ts existed and was recorded RED in the round evidence.
 *
 * Every assertion here is a security invariant in test form:
 *  - unknown vocabulary is refused, never guessed
 *  - digests are EXACT lowercase 64-hex and are never computed or truncated
 *  - outbound lifecycle bytes are VERBATIM wire values, never display-sanitized
 *  - there is no code path that constructs a direct status write
 *  - an evaluation join is DATA and can never move a status
 */

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function fixture(): Record<string, unknown> {
	const parsed: unknown = JSON.parse(
		readFileSync(join(process.cwd(), 'tests', 'fixtures', 'model-registry.json'), 'utf8')
	);
	if (!isRecord(parsed)) throw new Error('model-registry fixture must be an object');
	return parsed;
}

function withList(mutate: (rows: Record<string, unknown>[]) => void): unknown {
	const base = fixture();
	const list = structuredClone(base['list']) as Record<string, unknown>;
	const rows = structuredClone(list['rows']) as Record<string, unknown>[];
	mutate(rows);
	list['rows'] = rows;
	list['count'] = rows.length;
	return list;
}

function withDetail(mutate: (row: Record<string, unknown>) => void): unknown {
	const base = fixture();
	const row = structuredClone(base['detail']) as Record<string, unknown>;
	mutate(row);
	return row;
}

const HEX64 = 'a'.repeat(64);

/**
 * U+200B ZERO WIDTH SPACE, built from its codepoint.
 *
 * Written as a literal byte or an inline escape it is stripped by ordinary
 * tooling and by most editors, which silently turns the raw-value assertion
 * into "Acme models" === "Acme models" — a green test proving nothing. This
 * file was MEASURED at zero U+200B bytes after a literal attempt, so the
 * codepoint is built at runtime and the source stays pure ASCII.
 */
const ZWSP = String.fromCharCode(0x200b);
const INVISIBLE_NAME = `Acme${ZWSP}models`;

describe('model registry parser', () => {
	test('model_registry_parser_accepts_bounded_list_and_detail', () => {
		const base = fixture();

		const list = parseModelRegistryList(base['list']);
		expect(list.ok).toBe(true);
		if (!list.ok) return;
		expect(list.value.count).toBe(2);
		expect(list.value.rows).toHaveLength(2);
		expect(list.value.rows[0]?.id).toBe('rules-r32-e2e');
		expect(list.value.rows[0]?.version).toBe('1.0.0');
		expect(list.value.rows[0]?.kind).toBe('deterministic-rules');
		expect(list.value.rows[0]?.status).toBe('candidate');
		// The LISTING carries artifact digest as a PRESENCE BOOLEAN only. The
		// value is available on the single-row read and nowhere else, so a
		// list row can never leak or invent the digest itself.
		expect(list.value.rows[0]?.artifactDigestPresent).toBe(true);
		expect(list.value.rows[1]?.artifactDigestPresent).toBe(false);
		expect(Object.keys(list.value.rows[0] ?? {})).not.toContain('artifactDigest');

		const detail = parseModelRegistryDetail(base['detail']);
		expect(detail.ok).toBe(true);
		if (!detail.ok) return;
		expect(detail.value.id).toBe('rules-r32-e2e');
		expect(detail.value.rowDigest).toBe(
			'9fbcf60dabc26c8905a2eb8820100e6709df27a0a409d49f04eab59d85a60037'
		);
		expect(detail.value.rowDigest).toMatch(/^[0-9a-f]{64}$/);
		// Nullable digest tiers stay null, never become '' or 'unknown'.
		expect(detail.value.calibrationRef).toBeNull();
		expect(detail.value.artifactDigest).toMatch(/^[0-9a-f]{64}$/);

		// The list is BOUNDED: more rows than the cap is a parse failure, not a
		// silent truncation, because a truncated list would read as authoritative.
		const overflow = withList((rows) => {
			for (let i = 0; i < MODEL_REGISTRY_LIMITS.listRows + 1; i += 1) {
				rows.push({ ...rows[0], id: `m${i}`, version: '1.0.0' });
			}
		});
		expect(parseModelRegistryList(overflow).ok).toBe(false);
	});

	test('model_registry_parser_rejects_unknown_status_kind_and_vocabulary', () => {
		// Every closed vocabulary refuses an unknown value rather than guessing
		// a default. An unknown status must never render as "candidate".
		for (const field of ['status', 'kind'] as const) {
			const bad = withDetail((row) => {
				row[field] = 'totally-not-a-real-value';
			});
			const parsed = parseModelRegistryDetail(bad);
			expect(parsed.ok, `${field} must refuse an unknown value`).toBe(false);
			if (parsed.ok) continue;
			expect(parsed.reason).toBe('invalid_detail');
		}

		// The output vocabulary is closed per item.
		const badVocab = withDetail((row) => {
			row['output_vocabulary'] = ['choice', 'not-a-vocabulary-word'];
		});
		expect(parseModelRegistryDetail(badVocab).ok).toBe(false);

		// A wrong TYPE is also a refusal, never a coercion.
		for (const bad of [null, [], 'a string', 42, true]) {
			expect(parseModelRegistryDetail(bad).ok, `detail ${JSON.stringify(bad)}`).toBe(false);
			expect(parseModelRegistryList(bad).ok, `list ${JSON.stringify(bad)}`).toBe(false);
		}

		// A missing required field is a refusal, not an invented default.
		for (const field of ['id', 'version', 'kind', 'name', 'status', 'row_digest']) {
			const missing = withDetail((row) => {
				delete row[field];
			});
			const parsed = parseModelRegistryDetail(missing);
			expect(parsed.ok, `detail missing ${field} must refuse`).toBe(false);
		}
	});

	test('model_registry_parser_enforces_digest_id_version_and_timestamp_bounds', () => {
		// Digests are EXACT lowercase 64-hex. The shell never computes,
		// truncates, uppercases, or repairs one.
		for (const badDigest of [
			'A'.repeat(64),
			'a'.repeat(63),
			'a'.repeat(65),
			'',
			'sha256:' + 'a'.repeat(64),
			`${'a'.repeat(63)}!`
		]) {
			const bad = withDetail((row) => {
				row['row_digest'] = badDigest;
			});
			expect(
				parseModelRegistryDetail(bad).ok,
				`row_digest ${JSON.stringify(badDigest)} must refuse`
			).toBe(false);
		}

		// Timestamps must be finite, safe, non-negative, and epoch-guarded
		// BEFORE any Date conversion — a hostile value must never reach `new Date`.
		for (const badStamp of [-1, Number.NaN, Number.POSITIVE_INFINITY, 1e18, '1750000000']) {
			const bad = withDetail((row) => {
				row['created_at'] = badStamp;
			});
			expect(
				parseModelRegistryDetail(bad).ok,
				`created_at ${String(badStamp)} must refuse`
			).toBe(false);
		}

		// id and version are length-bounded.
		const longId = withDetail((row) => {
			row['id'] = 'x'.repeat(MODEL_REGISTRY_LIMITS.identifierChars + 1);
		});
		expect(parseModelRegistryDetail(longId).ok).toBe(false);

		const longVersion = withDetail((row) => {
			row['version'] = 'v'.repeat(MODEL_REGISTRY_LIMITS.versionChars + 1);
		});
		expect(parseModelRegistryDetail(longVersion).ok).toBe(false);

		// The ref decoder accepts one whole {id}@{version} segment and refuses
		// a malformed one, mirroring the kernel's model_ref_invalid.
		expect(parseModelRef('rules-r32-e2e@1.0.0')).toEqual({
			id: 'rules-r32-e2e',
			version: '1.0.0'
		});
		for (const bad of ['nope', '@1.0.0', 'id@', 'a@b@c', '']) {
			expect(parseModelRef(bad), `ref ${JSON.stringify(bad)}`).toBeNull();
		}
	});

	test('model_registry_parser_preserves_raw_values_for_lifecycle_submission', () => {
		// Display sanitization must NEVER mutate outbound proposal bytes. The
		// fixture's name carries a canonical invisible character; the parsed
		// value keeps the RAW bytes for submission, and a separate
		// display-oriented projection is what a renderer may sanitize.
		//
		// MEASURED BEHAVIOUR, pinned here because it is a display hazard:
		// `stripInvisible` REMOVES the character, it does not replace it with a
		// separator. So "Acme<ZWSP>models" sanitizes to "Acmemodels" — the word
		// boundary is DESTROYED, not normalized. A renderer must not assume
		// whitespace survives sanitization, and must not re-insert a space to
		// "repair" it, because that would be inventing a fact.
		expect(INVISIBLE_NAME).not.toBe('Acme models');
		expect(stripInvisible(INVISIBLE_NAME)).toBe('Acmemodels');
		expect(stripInvisible(INVISIBLE_NAME)).not.toBe('Acme models');
		// The ZWSP REPLACES the space, so both strings are 11 code points: the
		// difference is the character at index 4, not the length.
		expect(INVISIBLE_NAME.length).toBe('Acme models'.length);
		expect([...INVISIBLE_NAME][4]).toBe(ZWSP);
		expect([...'Acme models'][4]).toBe(' ');

		const withInvisible = withDetail((row) => {
			row['name'] = INVISIBLE_NAME;
		});

		const parsed = parseModelRegistryDetail(withInvisible);
		expect(parsed.ok).toBe(true);
		if (!parsed.ok) return;

		// The retained value is the VERBATIM wire value, invisible byte INCLUDED.
		// This is the display/submission separation: sanitizing for display must
		// never mutate what goes back on the wire.
		expect(parsed.value.name).toBe(INVISIBLE_NAME);
		expect([...parsed.value.name]).toHaveLength([...INVISIBLE_NAME].length);
		expect(parsed.value.name).toContain(ZWSP);

		// And the proposal content built from it is byte-identical to the raw
		// wire value — no re-serialization, no sanitizing, no repair.
		const content = lifecycleProposalContent('promote', parsed.value);
		expect(content).toContain(INVISIBLE_NAME);
		const sent = JSON.parse(content) as Record<string, unknown>;
		expect(sent['action']).toBe('promote');
		expect(sent['id']).toBe('rules-r32-e2e');
		expect(sent['version']).toBe('1.0.0');
		expect(sent['row_digest']).toBe(
			'9fbcf60dabc26c8905a2eb8820100e6709df27a0a409d49f04eab59d85a60037'
		);
		// The row travels as evidence carrying the RAW name, invisible byte and
		// all: the outbound payload is never display-sanitized.
		const sentRow = sent['row'] as Record<string, unknown>;
		expect(sentRow['name']).toBe(INVISIBLE_NAME);
		expect(sentRow['name']).toContain(ZWSP);
		expect(stripInvisible(String(sentRow['name']))).toBe('Acmemodels');

		// THE ROW IS EXACTLY THE KERNEL'S FIELD SET. Two live 400s were found by
		// the E2E against the real wire and are pinned here so neither can
		// regress:
		//   registry_payload_invalid / unknown field `row_digest`  -- the pin
		//     travels BESIDE the row, never inside it, because RegistryRow is
		//     #[serde(deny_unknown_fields)] with no row_digest member;
		//   registry_payload_invalid / missing `approved_by`      -- the kernel
		//     row has no #[serde(default)], so the key must be present even
		//     when its value is null.
		expect(Object.keys(sentRow).sort()).toEqual([...REGISTRY_ROW_FIELDS].sort());
		expect(sentRow).not.toHaveProperty('row_digest');
		expect(Object.keys(sentRow)).toContain('approved_by');
		// And the pin is at the TOP level, exactly as captured.
		expect(sent['row_digest']).toBe(
			'9fbcf60dabc26c8905a2eb8820100e6709df27a0a409d49f04eab59d85a60037'
		);
	});

	test('model_registry_list_query_sends_only_limit_status_and_kind', () => {
		// The shell invents NO filter the server does not support. The server
		// accepts exactly limit, status, kind — so that is exactly what is sent.
		expect(modelRegistryListQuery({})).toBe('');
		expect(modelRegistryListQuery({ limit: 5 })).toBe('?limit=5');
		expect(modelRegistryListQuery({ status: 'candidate' })).toBe('?status=candidate');
		expect(modelRegistryListQuery({ kind: 'learned' })).toBe('?kind=learned');
		expect(modelRegistryListQuery({ limit: 20, status: 'retired', kind: 'reranker' })).toBe(
			'?limit=20&status=retired&kind=reranker'
		);
		// A limit outside the server's 1..50 bounds is refused, not clamped:
		// silently clamping would send a request the operator did not ask for.
		for (const bad of [0, 51, -1, 1.5, Number.NaN]) {
			expect(modelRegistryListQuery({ limit: bad }), `limit ${bad}`).toBeNull();
		}
		// An unknown filter value is refused, never forwarded.
		expect(modelRegistryListQuery({ status: 'nope' as never })).toBeNull();
		expect(modelRegistryListQuery({ kind: 'nope' as never })).toBeNull();

		// THE OBJECT FORM IS WHAT THE CLIENT ACTUALLY CONSUMES, and the string
		// form is a comparison artifact only. openapi-fetch 0.17's
		// createQuerySerializer iterates its argument only when
		// `typeof queryParams === 'object'`; handed a string it returns '' and
		// createFinalURL appends nothing — NO error, NO warning, the filters are
		// simply gone and an unfiltered request goes out. Pinned here so the two
		// forms can never be confused, and so the object form stays the easy one.
		expect(modelRegistryListQueryParams({ limit: 20, status: 'retired', kind: 'reranker' })).toEqual(
			{ limit: 20, status: 'retired', kind: 'reranker' }
		);
		expect(modelRegistryListQueryParams({})).toEqual({});
		expect(modelRegistryListQueryParams({ limit: 0 })).toBeNull();
		expect(modelRegistryListQueryParams({ status: 'nope' as never })).toBeNull();
		// The two forms are DERIVED from one another and cannot disagree.
		const input = { limit: 7, status: 'evaluated' as const };
		expect(modelRegistryListQuery(input)).toBe('?limit=7&status=evaluated');
		expect(Object.entries(modelRegistryListQueryParams(input) ?? {})).toHaveLength(2);
	});

	test('model_registry_lifecycle_actions_follow_the_closed_transition_table', () => {
		// The table is the kernel's, not the shell's invention
		// (src/workflow/registry.rs transition_legal):
		//   promote <- candidate | evaluated
		//   retire   <- candidate | evaluated | promoted
		//   anything else is illegal.
		expect(isLegalLifecycleAction('promote', 'candidate')).toBe(true);
		expect(isLegalLifecycleAction('promote', 'evaluated')).toBe(true);
		expect(isLegalLifecycleAction('promote', 'promoted')).toBe(false);
		expect(isLegalLifecycleAction('promote', 'retired')).toBe(false);

		expect(isLegalLifecycleAction('retire', 'candidate')).toBe(true);
		expect(isLegalLifecycleAction('retire', 'evaluated')).toBe(true);
		expect(isLegalLifecycleAction('retire', 'promoted')).toBe(true);
		expect(isLegalLifecycleAction('retire', 'retired')).toBe(false);

		// The action vocabulary is CLOSED: no third action exists, and an
		// invented one is illegal from every status.
		for (const action of ['approve', 'reject', 'set_status', 'delete', '']) {
			for (const from of ['candidate', 'evaluated', 'promoted', 'retired']) {
				expect(
					isLegalLifecycleAction(action, from),
					`${action} from ${from}`
				).toBe(false);
			}
		}
	});

	test('model_registry_evaluation_refs_are_parsed_as_data_not_status', () => {
		// An evaluation join is DATA. It must never be able to move a status,
		// and a non-empty evaluation_refs list must not imply "evaluated".
		const base = fixture();
		const detail = parseModelRegistryDetail(base['detail']);
		expect(detail.ok).toBe(true);
		if (!detail.ok) return;

		expect(detail.value.status).toBe('candidate');
		expect(detail.value.evaluationRefs).toEqual(['eval-r30-001', 'eval-r30-002']);
		// Two evaluation refs and still `candidate`: the join is non-authoritative.
		expect(detail.value.status).not.toBe('evaluated');

		// The parsed value exposes evaluation refs as a bounded, string-only
		// list — never as a status signal and never as a verdict.
		const detailValue = detail.value as unknown as Record<string, unknown>;
		expect(Object.keys(detailValue)).toContain('evaluationRefs');
		expect(Object.keys(detailValue)).not.toContain('evaluationStatus');
		expect(Object.keys(detailValue)).not.toContain('evaluationVerdict');

		// A non-string ref is a refusal rather than a coerced value.
		const badRefs = withDetail((row) => {
			row['evaluation_refs'] = ['ok', 42];
		});
		expect(parseModelRegistryDetail(badRefs).ok).toBe(false);
	});

	test('model_registry_cannot_construct_a_direct_status_write', () => {
		// The shell has no direct status write. A lifecycle action is a
		// PROPOSAL, not a mutation, and the payload carries no status field the
		// client could set freely.
		const base = fixture();
		const detail = parseModelRegistryDetail(base['detail']);
		expect(detail.ok).toBe(true);
		if (!detail.ok) return;

		const content = lifecycleProposalContent('retire', detail.value);
		const sent = JSON.parse(content) as Record<string, unknown>;
		expect(sent['action']).toBe('retire');
		// The row travels as evidence; the ACTION is what a human approves.
		expect(sent['row']).toBeDefined();
		// The proposal body is content+kind only -- there is no status channel.
		expect(Object.keys(sent).sort()).toEqual(['action', 'id', 'row', 'row_digest', 'version']);
		// And the row inside it is the kernel's exact field set, with the pin
		// OUTSIDE it. Both were live 400s before the E2E caught them.
		const row = sent['row'] as Record<string, unknown>;
		expect(Object.keys(row).sort()).toEqual([...REGISTRY_ROW_FIELDS].sort());
		expect(row).not.toHaveProperty('row_digest');
		// The fixture has no approved_by value, so the key is present and null --
		// present, because the kernel has no serde(default) for it.
		expect(Object.keys(row)).toContain('approved_by');
		expect(row['approved_by']).toBeNull();

		// And the status classification surface distinguishes a denial from an
		// empty list, so a 403 can never be rendered as "no models registered".
		const forbidden = classifyModelRegistryStatus(403);
		expect(forbidden.kind).toBe('forbidden');
		expect(forbidden.kind).not.toBe('empty');
		expect(classifyModelRegistryStatus(401).kind).toBe('unauthorized');
		expect(classifyModelRegistryStatus(409).kind).toBe('conflict');
		expect(classifyModelRegistryStatus(422).kind).toBe('invalid');
		expect(classifyModelRegistryStatus(500).kind).toBe('server');
		expect(classifyModelRegistryStatus(200).kind).toBe('ok');
		// An unknown status is `unknown`, never silently `ok`.
		expect(classifyModelRegistryStatus(418).kind).toBe('unknown');
	});
});
