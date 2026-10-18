import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import { shellRoot } from './fixtures';
import type { Passage, RecallOutcome } from '../src/lib/api/recall';
import { isAnswerLike, renderTurn, renderingToken, type TurnRendering } from '../src/lib/chat/turn';
import {
	citationFor,
	confidenceWord,
	isHistorical,
	provenanceBlock,
	validityOf
} from '../src/lib/chat/provenance';

/**
 * II-3 / II-4 — the turn model and the provenance block.
 *
 * Both are pure modules, so every arm below drives the REAL subject. The
 * outcome fixtures are hand-built to match the core's serde representation
 * (P1's register row records that the internal tag FLATTENS the variant, so a
 * refusal genuinely has no `passages` key on the wire).
 */

const NOW = Date.parse('2026-10-18T12:00:00Z');

function passage(over: Partial<Passage> = {}): Passage {
	return {
		id: 7,
		title: 'Retention policy',
		text: 'Audit rows are retained for 400 days by default.',
		snippet: null,
		domain: 'infra',
		score: 0.82,
		valid_from: '2026-03-02',
		valid_to: null,
		flagged: false,
		ingest_kind: null,
		memory_kind: null,
		authority: null,
		...over
	};
}

const ANSWERED: RecallOutcome = {
	state: 'answered',
	decision: 'ok',
	passages: [passage()],
	confidence: 0.82,
	searched: { query: 'retention', domain: null, limit: 5, domains_searched: [], included_global: null }
};
const ABSENT: RecallOutcome = {
	state: 'absent',
	decision: 'ok',
	passages: [],
	confidence: 0.1,
	searched: { query: 'retention', domain: null, limit: 5, domains_searched: [], included_global: null }
};
// `state` is what the CORE derived, not what the caller claims: the core maps
// `decision: low_confidence` to `abstained` and `decision: ok` with zero
// passages to `absent`. Hand-building the union is the only way to drive both
// arms without a kernel, and getting it wrong here is exactly the confusion the
// taxonomy exists to prevent -- the first draft of this fixture typed
// `low_confidence` under `state: 'absent'` and two arms caught it.
const ABSTAINED: RecallOutcome = {
	state: 'abstained',
	decision: 'low_confidence',
	passages: [],
	confidence: 0.04,
	searched: { query: 'retention', domain: null, limit: 5, domains_searched: [], included_global: null }
};
const REFUSED: RecallOutcome = { state: 'refused', status: 403, scope: 'recall' };
const UNREACHABLE: RecallOutcome = {
	state: 'unreachable',
	status: 0,
	detail: 'the recall transport failed: ECONNREFUSED'
};
const VIOLATION: RecallOutcome = {
	state: 'contract_violation',
	status: 200,
	detail: 'the recall body is not the published contract'
};

describe('II-5 the four renderings stay distinct in the DOM-facing token', () => {
	test('refusal_renders_differently_from_absence', () => {
		const refused = renderTurn(REFUSED, 'retention');
		const absent = renderTurn(ABSENT, 'retention');
		expect(refused.kind).toBe('refused');
		expect(absent.kind).toBe('absent');
		expect(renderingToken(refused)).not.toBe(renderingToken(absent));

		// The substantive half: a refusal carries the status and the scope, so
		// a surface has the information to say WHY it refused rather than
		// falling back to "no results".
		expect(refused).toMatchObject({ status: 403, scope: 'recall' });
	});

	test('abstention_is_its_own_rendering_and_is_not_an_error', () => {
		const abstained = renderTurn(ABSTAINED, 'retention');
		expect(abstained.kind).toBe('abstained');
		expect(abstained.kind).not.toBe('absent');
		// Plan 06 §3: abstention is an ANSWER, emphatically not an error.
		expect(isAnswerLike(abstained)).toBe(true);
		expect(isAnswerLike(renderTurn(REFUSED, 'q'))).toBe(false);
		expect(isAnswerLike(renderTurn(UNREACHABLE, 'q'))).toBe(false);
		// And it carries what was searched, which is Plan 06 §3's offer.
		expect(abstained).toMatchObject({ searched: 'retention', limit: 5 });
	});

	test('all_six_states_reach_a_distinct_rendering', () => {
		const kinds = [
			renderTurn(ANSWERED, 'q'),
			renderTurn(ABSENT, 'q'),
			renderTurn(ABSTAINED, 'q'),
			renderTurn(REFUSED, 'q'),
			renderTurn(UNREACHABLE, 'q'),
			renderTurn(VIOLATION, 'q')
		].map(renderingToken);
		expect(new Set(kinds).size, `renderings collapsed: ${kinds.join(',')}`).toBe(6);
	});

	test('no_rendering_is_reached_by_emptiness', () => {
		// THE CONFIDENTIALITY ARM, and it is a source read rather than a
		// behaviour check because the defect is textual: a renderer that
		// branches on `passages?.length` or calls `hasPassages()` reintroduces
		// the leak P1 closed, and the compiler will not stop it because both
		// accessors type-check against an absence.
		//
		// So the guard is that neither appears, and that the switch is over
		// `state`. Read as source, because "the pin drives the real function"
		// cannot detect a branch that would only fire on a future outcome.
		const source = readFileSync(join(shellRoot, 'src/lib/chat/turn.ts'), 'utf8');
		const code = source
			.split('\n')
			.filter((line) => {
				const trimmed = line.trim();
				return !trimmed.startsWith('//') && !trimmed.startsWith('*') && !trimmed.startsWith('/*');
			})
			.join('\n');
		// Comment prose is excluded for the reason the Swift guard records: a
		// comment does not execute. The doc comments above NAME these
		// accessors precisely to say this module does not use them.
		expect(code, 'turn.ts branches on emptiness').not.toMatch(/passages\s*\??\.\s*length/);
		expect(code, 'turn.ts uses hasPassages').not.toContain('hasPassages');
		expect(code, 'turn.ts uses the throwing accessor').not.toContain('recallPassages');
		expect(code, 'turn.ts must switch on state').toMatch(/switch\s*\(\s*outcome\.state\s*\)/);
	});
});

describe('II-4 provenance', () => {
	test('an_expired_valid_to_renders_as_historical_never_as_current', () => {
		const expired = passage({ valid_from: '2019-01-01', valid_to: '2021-06-30' });
		expect(isHistorical(expired, NOW)).toBe(true);

		const validity = validityOf(expired, NOW);
		expect(validity.historical).toBe(true);
		expect(validity.summary).toContain('historical');
		expect(validity.summary).toContain('not current');
		// The negative that matters: it must not read as an open-ended fact.
		expect(validity.summary).not.toContain('valid until 2021-06-30');

		const live = passage({ valid_from: '2026-01-01', valid_to: '2027-01-01' });
		expect(isHistorical(live, NOW)).toBe(false);
		expect(validityOf(live, NOW).summary).toContain('valid');
	});

	test('an_unparseable_valid_to_is_treated_as_expired_not_current', () => {
		// A date the client cannot read is a date it cannot certify as future,
		// and Plan 06 §2's bias is toward calling a fact historical when unsure.
		expect(isHistorical(passage({ valid_to: 'not-a-date' }), NOW)).toBe(true);
		expect(validityOf(passage({ valid_to: 'not-a-date' }), NOW).historical).toBe(true);
	});

	test('a_turn_that_recalls_cannot_render_without_a_provenance_block', () => {
		const block = provenanceBlock([passage()], 0.82, NOW);
		expect(block.count).toBe(1);
		expect(block.citations).toHaveLength(1);
		// Every citation carries all five Plan 06 fields as VALUES, with the
		// two the kernel does not send marked unavailable and REASONED — so a
		// surface cannot drop one by forgetting a branch, and a reader cannot
		// mistake an absent fact for an empty one.
		const citation = block.citations[0]!;
		// The unions are discriminated, so these arms NARROW rather than cast:
		// if a field stopped carrying its `present`/`unavailable` discriminant
		// the assertions would stop compiling, which is the property wanted
		// from a provenance block whose whole job is to be total.
		if (citation.title.kind === 'present') expect(citation.title.value).toBe('Retention policy');
		else throw new Error('title should be present');
		if (citation.domain.kind === 'present') expect(citation.domain.value).toBe('infra');
		else throw new Error('domain should be present');
		// Narrowing, not casting: if either arm lost its `kind` discriminant
		// these would stop COMPILING, which is the property a provenance block
		// wants -- every field present on every citation, either as a value or
		// as a stated reason for its absence.
		if (citation.owner.kind !== 'unavailable') throw new Error('owner must be unavailable');
		if (citation.chainHead.kind !== 'unavailable') throw new Error('chainHead must be unavailable');
		expect(citation.owner.because).toContain('owner');
		expect(citation.chainHead.because).toContain('chain head');
		if (citation.confidence.kind === 'word') expect(citation.confidence.word).toBe('high');
		else throw new Error('confidence should be a word');
	});

	test('confidence_renders_as_a_word_and_never_a_bare_number', () => {
		expect(confidenceWord(0.82)).toMatchObject({ kind: 'word', word: 'high', value: 0.82 });
		expect(confidenceWord(0.5)).toMatchObject({ word: 'medium' });
		expect(confidenceWord(0.1)).toMatchObject({ word: 'low' });
		// Absent confidence is stated as absent, never as zero — zero is a
		// number a reader would trust.
		const unknown = confidenceWord(null);
		expect(unknown.kind).toBe('unavailable');
		if (unknown.kind === 'unavailable') expect(unknown.because).toContain('no confidence');
	});

	test('copy_carries_the_provenance_so_the_receipt_travels', () => {
		const block = provenanceBlock([passage(), passage({ id: 8, title: 'Audit runbook' })], 0.82, NOW);
		expect(block.receipt).toContain('recalled 2 passages');
		expect(block.receipt).toContain('[7]');
		expect(block.receipt).toContain('[8]');
		// The receipt states the missing fields rather than omitting them, so a
		// reader holding the copied text knows they are missing BY DESIGN.
		expect(block.receipt).toContain('chain head');
	});

	test('provenance_renders_as_text_nodes_and_html_stays_banned', () => {
		// The ban is enforced by lint; this arm asserts the ban still exists
		// and that no chat module reaches for the escape hatch, because a
		// disabled rule is not a passing one.
		const eslintConfig = readFileSync(join(shellRoot, 'eslint.config.js'), 'utf8');
		expect(eslintConfig, 'the {@html} ban must still be configured').toContain('@html');

		for (const file of ['turn.ts', 'provenance.ts']) {
			const source = readFileSync(join(shellRoot, 'src/lib/chat', file), 'utf8');
			const code = source
				.split('\n')
				.filter((line) => {
					const trimmed = line.trim();
					return !trimmed.startsWith('//') && !trimmed.startsWith('*') && !trimmed.startsWith('/*');
				})
				.join('\n');
			expect(code, `${file} uses {@html}`).not.toContain('@html');
		}
	});
});