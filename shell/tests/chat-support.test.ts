import { describe, expect, test } from 'vitest';
import { citeTitles, mapClaimsToSentences, splitSentences } from '../src/lib/chat/claims';
import {
	generatedFromFinal,
	parseSupportLabel,
	withSupportLabel,
	type TurnRendering
} from '../src/lib/chat/turn';
import { replayTurns } from '../src/lib/chat/replay';
import type { StoredTurn } from '../src/lib/shell/sessions.svelte';

/**
 * LL-04 §1.4 — the support render's PURE half, pinned at the module
 * boundary (the M3-a module-split law: the page draws what these functions
 * decide, so the pins drive the functions, not the DOM).
 *
 * The law this file holds: the kernel's verdict is THE label — the shell
 * renders it and never re-derives it; the sentence mapping is honest
 * structure over the model's OWN typed claims (a sentence no claim covers
 * renders dimmed with the `unsupported` chip, never deleted, never kept
 * silently as supported); and a cite resolves BY INDEX into the hits the
 * kernel sent — the kernel's own universe, the exact distinction the
 * flagship red-proof turned on.
 */

describe('splitSentences', () => {
	test('splits_on_terminators_followed_by_a_boundary_and_keeps_an_terminated_tail', () => {
		expect(splitSentences('One sentence. Two follows! Three?')).toEqual([
			'One sentence.',
			'Two follows!',
			'Three?'
		]);
		// The tail of a streaming answer grew no terminator — still a sentence.
		expect(splitSentences('One. Two')).toEqual(['One.', 'Two']);
		// A terminator with NO following whitespace is an abbreviation shape,
		// not a boundary — deterministic, no dictionary.
		expect(splitSentences('v1.2 of the spec. Done.')).toEqual(['v1.2 of the spec.', 'Done.']);
		expect(splitSentences('')).toEqual([]);
	});
});

describe('mapClaimsToSentences', () => {
	const claims = [
		{ text: 'Green tea is steamed Camellia sinensis.', cites: [0] },
		{ text: 'It contains EGCG [1].', cites: [1] }
	];

	test('a_sentence_a_claim_covers_carries_it_and_one_without_renders_uncovered', () => {
		const mapped = mapClaimsToSentences(
			['Green tea is steamed Camellia sinensis.', 'A sentence the model never claimed.'],
			claims
		);
		expect(mapped[0]?.claim).toEqual(claims[0]);
		expect(mapped[1]?.claim).toBeNull();
	});

	test('matching_survives_the_prose_citation_markers_and_case', () => {
		const mapped = mapClaimsToSentences(['it contains egcg [2], which is real.'], claims);
		expect(mapped[0]?.claim).toEqual(claims[1]);
	});

	test('a_null_or_empty_claim_set_renders_every_sentence_plain', () => {
		const sentences = ['One.', 'Two.'];
		for (const claims of [null, []]) {
			const mapped = mapClaimsToSentences(sentences, claims);
			expect(mapped).toEqual([
				{ text: 'One.', claim: null },
				{ text: 'Two.', claim: null }
			]);
		}
	});
});

describe('citeTitles', () => {
	const hits = [
		{ id: 42, title: 'Green tea', domain: 'global' },
		{ id: 7, title: 'Gate law', domain: null },
		{ id: 99, title: null, domain: 'kitchen' }
	];

	test('a_cite_resolves_by_INDEX_into_the_hits_not_by_hit_id', () => {
		// THE FLAGSHIP'S SHELL-SIDE ECHO: cites[1] is the SECOND hit (id 7),
		// never "the hit whose id is 1" — resolving by id is the drifted
		// second gate the kernel's index arithmetic exists to prevent.
		expect(citeTitles([1], hits)).toEqual(['Gate law']);
		expect(citeTitles([0], hits)).toEqual(['Green tea · global']);
	});

	test('an_out_of_range_index_resolves_to_nothing_the_label_is_the_kernels', () => {
		expect(citeTitles([7], hits), 'index 7 of 3 hits names no hit here').toEqual([]);
		expect(citeTitles([-1], hits)).toEqual([]);
	});

	test('a_hit_without_a_title_still_names_its_domain_and_a_barren_hit_names_nothing', () => {
		expect(citeTitles([2], hits)).toEqual(['kitchen']);
	});
});

describe('the support label on the turn', () => {
	test('parseSupportLabel_admits_only_the_five_word_vocabulary', () => {
		for (const label of ['supported', 'partial', 'unsupported', 'abstained', 'degraded']) {
			expect(parseSupportLabel(label)).toBe(label);
		}
		expect(parseSupportLabel('hallucinated')).toBeNull();
		expect(parseSupportLabel(undefined)).toBeNull();
		expect(parseSupportLabel(42)).toBeNull();
	});

	test('generatedFromFinal_carries_the_typed_fields_and_starts_unlabeled', () => {
		const rendering = generatedFromFinal(
			'q',
			{
				text: 'An answer.',
				tier: 'on_device',
				abstain: false,
				claims: [{ text: 'An answer.', cites: [0] }]
			},
			[]
		);
		expect(rendering).toMatchObject({
			kind: 'generated',
			abstain: false,
			claims: [{ text: 'An answer.', cites: [0] }],
			support: null
		});
	});

	test('an_absent_typed_pair_reads_as_the_untyped_posture', () => {
		const rendering = generatedFromFinal('q', { text: 'x', tier: null, abstain: null, claims: null }, []);
		expect(rendering).toMatchObject({ abstain: false, claims: null, support: null });
	});

	test('withSupportLabel_patches_the_generated_turn_and_ignores_every_other_member', () => {
		const generated = generatedFromFinal('q', { text: 'x', tier: null, abstain: null, claims: null }, []);
		const labeled = withSupportLabel(generated, 'unsupported');
		expect(labeled).toMatchObject({ kind: 'generated', support: 'unsupported' });
		const pending: TurnRendering = { kind: 'pending', query: 'q' };
		expect(withSupportLabel(pending, 'supported')).toBe(pending);
	});
});

describe('replay carries the stored verdict', () => {
	function storedTurns(support: string | null): StoredTurn[] {
		return [
			{
				seq: 1,
				role: 'user',
				content: 'q',
				grounding: null,
				tier: null,
				support: null,
				created_at: 1
			},
			{
				seq: 2,
				role: 'assistant',
				content: 'a',
				grounding: null,
				tier: 'on_device',
				support,
				created_at: 1
			}
		];
	}

	test('an_unsupported_turn_stays_flagged_in_history_and_junk_reads_null', () => {
		const replayed = replayTurns(storedTurns('unsupported'));
		expect(replayed[0]?.rendering.support).toBe('unsupported');
		expect(replayed[0]?.rendering.abstain).toBe(false);
		const junk = replayTurns(storedTurns('hallucinated'));
		expect(junk[0]?.rendering.support, 'a name outside the vocabulary is not a label').toBeNull();
		const old = replayTurns(storedTurns(null));
		expect(old[0]?.rendering.support, 'pre-LL-04 rows render unlabeled').toBeNull();
	});
});
