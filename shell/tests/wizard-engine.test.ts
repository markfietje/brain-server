import { describe, expect, test } from 'vitest';
import { parsePack } from '../src/lib/wizard/pack';
import { classifyAnswer, branchOf, renderedAnswer } from '../src/lib/wizard/branch';
import { assembleAnswers } from '../src/lib/wizard/answers';
import { corpusPack } from './fixtures';

// The parser is the client-side mirror of the kernel's validate_wizard_pack;
// the ratified packs are the shared corpus fixtures (ONE source of truth).

describe('parsePack', () => {
	test('parses all three ratified packs (the corpus fixtures)', () => {
		for (const name of ['capture-pre-screen', 'support-ticket', 'tele-health']) {
			const parsed = parsePack(corpusPack(name));
			expect(parsed.ok, `${name}: ${!parsed.ok ? parsed.reason : ''}`).toBe(true);
			if (parsed.ok) {
				expect(parsed.pack.pack).toBe(name);
				expect(parsed.pack.questions.length).toBeGreaterThanOrEqual(3);
			}
		}
	});

	test('refuses hostile shapes by name, never throws', () => {
		for (const bad of [
			42,
			'pack',
			null,
			[],
			{ pack: 'p' },
			{ pack: '', first: 'q', questions: [] },
			{
				pack: 'p',
				first: 'q',
				questions: [{ id: 'q', qtype: 'essay', instructions: 'x', next: {} }]
			},
			{
				pack: 'p',
				first: 'q',
				questions: [{ id: 'q', qtype: 'noul', instructions: 'x', next: { false: 'end' } }]
			},
			{
				pack: 'p',
				first: 'q',
				questions: [
					{ id: 'a', qtype: 'choice', instructions: 'x', criteria: { yes: null }, next: { yes: 'b' } },
					{ id: 'b', qtype: 'noul', instructions: 'x', next: { false: 'a', true: 'end' } }
				]
			}
		]) {
			const result = parsePack(bad);
			expect(result.ok).toBe(false);
			if (!result.ok) expect(result.reason.startsWith('wizard_pack_invalid: ')).toBe(true);
		}
	});
});

describe('the branch engine (support-ticket fixture)', () => {
	const pack = parsePack(corpusPack('support-ticket'));
	if (!pack.ok) throw new Error('fixture must parse');

	test('classifies only the closed vocabulary', () => {
		const qClass = pack.pack.byId.get('q_class');
		expect(qClass).toBeDefined();
		if (!qClass) return;
		expect(classifyAnswer(qClass, 'billing')).toBe('billing');
		expect(classifyAnswer(qClass, 'legal')).toBeNull(); // out of vocabulary
		expect(classifyAnswer(qClass, ' payment ')).toBeNull(); // no trimming — closed
		expect(classifyAnswer(qClass, 42)).toBeNull();
		expect(classifyAnswer(qClass, undefined)).toBeNull();

		const qPriority = pack.pack.byId.get('q_priority');
		expect(qPriority).toBeDefined();
		if (!qPriority) return;
		expect(classifyAnswer(qPriority, 2)).toBe('2');
		expect(classifyAnswer(qPriority, 3)).toBeNull(); // out of range
		expect(classifyAnswer(qPriority, '2')).toBeNull(); // wrong type: string index

		const qKnown = pack.pack.byId.get('q_known');
		expect(qKnown).toBeDefined();
		if (!qKnown) return;
		expect(classifyAnswer(qKnown, true)).toBe('true');
		expect(classifyAnswer(qKnown, 'true')).toBeNull(); // wrong type
	});

	test('branches follow the ANSWER map, not text', () => {
		const qClass = pack.pack.byId.get('q_class');
		const qPriority = pack.pack.byId.get('q_priority');
		expect(qClass).toBeDefined();
		expect(qPriority).toBeDefined();
		if (!qClass || !qPriority) return;
		expect(branchOf(pack.pack, qClass, 'billing')).toEqual({ kind: 'question', id: 'q_priority' });
		expect(branchOf(pack.pack, qPriority, '2')).toEqual({ kind: 'end' });
		expect(branchOf(pack.pack, qPriority, '0')).toEqual({ kind: 'question', id: 'q_known' });
	});

	test('renders typed answer text (score/noul) — never free text', () => {
		const qPriority = pack.pack.byId.get('q_priority');
		const qKnown = pack.pack.byId.get('q_known');
		expect(qPriority).toBeDefined();
		expect(qKnown).toBeDefined();
		if (!qPriority || !qKnown) return;
		expect(renderedAnswer(qPriority, '2')).toBe('level 2: high');
		expect(renderedAnswer(qKnown, 'true')).toBe('yes, existing account');
	});
});

describe('assembleAnswers (the kernel walk, mirrored)', () => {
	test('walks the branch the answers drive and abstains on absence', () => {
		const pack = parsePack(corpusPack('support-ticket'));
		expect(pack.ok).toBe(true);
		if (!pack.ok) return;

		const full = assembleAnswers(pack.pack, {
			q_class: 'billing',
			q_priority: 1,
			q_known: false
		});
		expect(full.abstain).toBe(false);
		expect(full.answers).toEqual([
			{ question: 'q_class', answer: 'billing' },
			{ question: 'q_priority', answer: 'level 1: medium' },
			{ question: 'q_known', answer: 'no, new account' }
		]);

		// Unanswered: the reached question abstains, nothing invented.
		const empty = assembleAnswers(pack.pack, {});
		expect(empty.abstain).toBe(true);
		expect(empty.abstained).toEqual(['q_class']);
		expect(empty.answers).toEqual([]);

		// Out-of-vocabulary: refused at the engine; the walk sees no answer.
		const hostile = assembleAnswers(pack.pack, { q_class: 'DROP TABLE answers' });
		expect(hostile.abstain).toBe(true);
		expect(hostile.answers).toEqual([]);
	});
});
