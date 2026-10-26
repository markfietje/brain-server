import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import {
	LOCALE_GENERATION,
	RETRIEVAL_ONLY_NOTICE_DE,
	SHIPPED_LOCALES,
	UNDECIDED,
	generationFor,
	mayGenerate
} from '../src/lib/locale-capability';
import { shellRoot } from './fixtures';

/**
 * II-7f — the `de` locale decision is STATED, not defaulted.
 *
 * Plan 02 §6's rule: "Do not silently answer `de` in English and call it
 * German." The pin's job is to make the decision a value that can be read, and
 * to make the two ways a locale could go silently English both fail.
 *
 * The pin reads the decision FROM THE SOURCE MODULE rather than restating it.
 * A test that carried its own copy of the table would agree with the source
 * forever, including after the source was changed — the drift shape this
 * repository keeps having to correct. So `de`'s verdict is asserted against
 * `LOCALE_GENERATION.de`, and the module is imported.
 */

const i18nDir = join(shellRoot, 'src', 'lib', 'i18n');

function catalog(locale: string): Record<string, string> {
	return JSON.parse(readFileSync(join(i18nDir, `${locale}.json`), 'utf8')) as Record<
		string,
		string
	>;
}

/**
 * The keys whose German is IDENTICAL to English, as an explicit allow-list.
 *
 * Every one is a genuine loanword in German technical writing — the plan's rule
 * is about answering in English and CALLING it German, not about forbidding
 * cognates. They are listed rather than tolerated by a threshold so that a new
 * untranslated key is a RED TEST naming the key, instead of a percentage that
 * drifts quietly as the catalog grows.
 */
const GERMAN_LOANWORDS = new Set([
	'nav.chat',
	'decisions.list.pipeline',
	'decisions.detail.model_version',
	'decisions.detail.limit',
	'decisions.detail.leg',
	'decisions.detail.kernel',
	// The sidebar frame's surfaces (2026-10-25): "Audit" and "Governance"
	// are the German technical terms, "Status" is a German word, and
	// "Steward" is the product name — a proper noun in every locale.
	'nav.audit',
	'nav.governance',
	'shell.brand',
	'audit.title',
	'audit.col.status',
	'governance.title'
]);
// (M3-a: the overview.*/models.* entries retired with their keys — a
// loanword entry for a key no catalog carries would be dead cover.)

/**
 * M3-b — the fr/es/nl arms of the SAME rule (READ FIRST ⑦ of the M3-b
 * prompt): `nav.chat` first shipped as "Chat" in all five locales, and fr's
 * nav block was otherwise fully translated — a smell the de-only gate could
 * not see. Each list is MEASURED (2026-10-24, 298 keys) and every entry is
 * a genuine loanword or a same-spelling cognate in that language, not an
 * untranslated string wearing a locale label.
 */
const FRENCH_LOANWORDS = new Set([
	// "Pipeline"/"Mode"/"Version"/"RRF k" are the same technical terms in
	// French; "Leg" is the retrieval leg, a term of art kept verbatim;
	// "Conversation"/"Scores"/"Salience" are French words of identical
	// spelling — cognates, not copies.
	'decisions.list.pipeline',
	'decisions.detail.mode',
	'decisions.detail.model_version',
	'decisions.detail.rrf_k',
	'decisions.detail.leg',
	'chat.title',
	'review.row.scores',
	'review.row.salience',
	// The sidebar frame (2026-10-25): "Audit", "Surfaces", "score" and
	// "relations" are French words of identical spelling; "Steward" is the
	// product name.
	'nav.audit',
	'shell.brand',
	'shell.nav',
	'memory.badge.score',
	'audit.title',
	'governance.domain.relations',
	// The session region (2026-10-26): "Conversation" is a French word of
	// identical spelling — the unlabelled row's noun, a cognate not a copy.
	'chat.sessions.unlabelled'
]);

const SPANISH_LOANWORDS = new Set([
	// "Chat", "Pipeline" and "Kernel" are established Spanish loanwords;
	// "Actor" and "RRF k" are the same terms of art.
	'nav.chat',
	'recall.trace.fields.actor',
	'decisions.list.pipeline',
	'decisions.detail.rrf_k',
	'decisions.detail.kernel',
	// The sidebar frame (2026-10-25): "Actor" and "Roles" are Spanish words
	// of identical spelling; "Steward" is the product name.
	'shell.brand',
	'audit.col.actor',
	'governance.roles'
]);

const DUTCH_LOANWORDS = new Set([
	// "Chat", "Pipeline" and "Kernel" are established Dutch loanwords;
	// "Actor", "Model" and "Scores" are same-spelling cognates; "conflict"
	// and "Conflict" are Dutch words in their own right.
	'nav.chat',
	'recall.conflict',
	'recall.trace.fields.actor',
	'decisions.list.pipeline',
	'decisions.detail.model_id',
	'decisions.detail.kernel',
	'review.row.scores',
	'review.row.conflict',
	// The sidebar frame (2026-10-25): "Audit", "Governance", "Status",
	// "Actor", "model", "score", "Procedure"/"Procedures" are Dutch words or
	// established loanwords of identical spelling; "Steward" is the product
	// name.
	'nav.audit',
	'nav.governance',
	'shell.brand',
	'shell.status.model',
	'memory.filter.procedures',
	'memory.badge.score',
	'memory.add.kind.procedure',
	'audit.title',
	'audit.col.actor',
	'audit.col.status',
	'governance.title',
	// The streaming composer (2026-10-26): "Stop" is the Dutch imperative of
	// "stoppen" — the genuine word for the button, spelled as English.
	'chat.stop'
]);

const LOANWORD_LISTS: Readonly<Record<string, ReadonlySet<string>>> = {
	de: GERMAN_LOANWORDS,
	fr: FRENCH_LOANWORDS,
	es: SPANISH_LOANWORDS,
	nl: DUTCH_LOANWORDS
};

describe('the de locale decision', () => {
	test('de_is_a_STATED_retrieval_only_value_and_not_a_default', () => {
		// THE PIN. `de` carries an explicit `retrieval_only`, read from the
		// source module's own table. If someone changed it to `available`, this
		// fails; if someone DELETED the key, TypeScript fails first.
		expect(LOCALE_GENERATION.de).toBe('retrieval_only');
		expect(mayGenerate('de')).toBe(false);

		// It is stated, not defaulted: the entry EXISTS, rather than being
		// absent and falling through a lookup. This is the distinction the
		// prompt names, and it is why the table is a Record over the five
		// literals rather than a Partial.
		expect(Object.prototype.hasOwnProperty.call(LOCALE_GENERATION, 'de')).toBe(true);
		expect(LOCALE_GENERATION).toHaveProperty('de');

		// And the other four are stated individually too, so `de` is not the
		// only decided one and the table is not `de`-plus-a-default.
		for (const locale of ['en', 'fr', 'es', 'nl']) {
			expect(
				LOCALE_GENERATION[locale as keyof typeof LOCALE_GENERATION],
				`${locale} must carry its own stated capability`
			).toBe('available');
		}

		// THE REASON is in the source, not only in a comment elsewhere. Plan
		// 02 §6's finding is that AFM 3 Core reports 24 languages including
		// fr/es/nl and NOT de, so the table's shape must be legible next to the
		// claim. Asserted as source text so deleting the reasoning is possible
		// but visible.
		const source = readFileSync(
			join(shellRoot, 'src', 'lib', 'locale-capability.ts'),
			'utf8'
		);
		expect(source).toContain('retrieval_only');
		expect(source).toMatch(/AFM 3 Core/);
		expect(source).toMatch(/1046/);
		// The two rejected alternatives are named, so a later reader can see
		// this was a choice with a blast radius rather than an omission.
		expect(source).toMatch(/Drop .?de|drop `de`/i);
		expect(source).toMatch(/external model/i);

		// And the notice a user reads exists in GERMAN, is non-empty, and is
		// not the English string with a locale label swapped on it.
		expect(RETRIEVAL_ONLY_NOTICE_DE.length).toBeGreaterThan(40);
		expect(RETRIEVAL_ONLY_NOTICE_DE).not.toContain('Deutsch wird');
		// It must not promise generation for de.
		expect(RETRIEVAL_ONLY_NOTICE_DE.toLowerCase()).toContain('nicht');
	});

	test('no_locale_falls_through_to_another_locale_or_to_a_default', () => {
		// THE FAIL-CLOSED ARM. An unknown locale is UNDECIDED, not `available`
		// and not `en`. A function that returned a capability for anything would
		// hand the next added locale an English answer by default — the exact
		// failure, arriving one locale later.
		for (const unknown of ['it', 'pt', 'zh', '', 'DE', 'de-AT', 'en_US', 'xx']) {
			expect(
				generationFor(unknown),
				`${JSON.stringify(unknown)} has no stated decision and must be ${UNDECIDED}`
			).toBe(UNDECIDED);
			expect(
				mayGenerate(unknown),
				`${JSON.stringify(unknown)} must not be offered a generate affordance`
			).toBe(false);
		}

		// Anti-vacuity: the five real locales are NOT undecided.
		for (const locale of SHIPPED_LOCALES) {
			expect(generationFor(locale)).not.toBe(UNDECIDED);
		}

		// There is no fallback parameter and no second-locale argument: the
		// capability function's arity is 1, so the silent-English path cannot
		// be reached by supplying a fallback at a call site.
		expect(generationFor.length).toBe(1);
		expect(mayGenerate.length).toBe(1);

		// The capability union has no `english_fallback` member to reach for.
		// Asserted over the DECLARED set, so adding one is a deliberate act that
		// fails here rather than a quiet widening.
		const declared = new Set(Object.values(LOCALE_GENERATION));
		expect(declared.has('available' as never)).toBe(true);
		expect(declared.has('retrieval_only' as never)).toBe(true);
		expect(declared.has('english_fallback' as never)).toBe(false);
		expect(generationFor('de')).not.toBe('en');
	});

	test('the_table_is_total_over_the_catalogs_that_actually_ship', () => {
		// The table must cover every catalog on disk and no others, so a locale
		// added to `project.inlang/settings.json` without a stated decision is a
		// red test rather than an undecided locale discovered in production.
		const onDisk = readdirSync(i18nDir)
			.filter((f) => f.endsWith('.json'))
			.map((f) => f.replace('.json', ''))
			.sort();
		const declared = [...SHIPPED_LOCALES].sort();
		expect(onDisk, 'the table must cover exactly the catalogs that ship').toEqual(declared);
		expect(Object.keys(LOCALE_GENERATION).sort()).toEqual(declared);
		expect(new Set(SHIPPED_LOCALES).size).toBe(SHIPPED_LOCALES.length);
	});

	test('the_de_catalog_is_really_German_and_not_the_english_strings_relabelled', () => {
		// THE OTHER HALF OF THE RULE. `de`'s decision is about GENERATION; this
		// is about the shell's own RENDERED strings, which the shell controls
		// outright. A `de` locale whose catalog is English would be the silent
		// answer the plan names, one layer below the model.
		const en = catalog('en');
		const de = catalog('de');
		const identical = Object.keys(en).filter((key) => en[key] === de[key]);

		// Every key identical to English must be an ALLOW-LISTED loanword. A
		// threshold would drift quietly as the catalog grows; a named list makes
		// a new untranslated key a red test that NAMES it.
		expect(
			identical.filter((key) => !GERMAN_LOANWORDS.has(key)),
			`these de keys are byte-identical to en and are not declared loanwords: ${identical
				.filter((key) => !GERMAN_LOANWORDS.has(key))
				.join(', ')}`
		).toEqual([]);

		// And the allow-list is not a blank cheque: it must not have grown to
		// cover most of the catalog, which is what "declared loanwords" would
		// become if each failure were answered by adding the key.
		expect(
			identical.length / Object.keys(en).length,
			`${identical.length} of ${Object.keys(en).length} de keys are identical to en. The ` +
				'allow-list has stopped being an exception list.'
		).toBeLessThan(0.1);

		// Anti-vacuity: the catalogs really do differ, so the two arms above
		// are measuring a translated catalog rather than two identical files.
		const differing = Object.keys(en).filter((key) => en[key] !== de[key]);
		expect(differing.length).toBeGreaterThan(Object.keys(en).length * 0.9);
		expect(Object.keys(de).length).toBe(Object.keys(en).length);
	});

	test('the_fr_es_nl_catalogs_are_really_translated_and_not_the_english_strings_relabelled', () => {
		// M3-b — the "Chat"-in-French lesson: the identical-to-en check now
		// covers EVERY locale, each with its own measured allow-list. A new
		// untranslated key in fr, es or nl is a RED TEST naming the key, the
		// same fail-closed shape the de arm has had since II-7f.
		const en = catalog('en');
		for (const locale of ['fr', 'es', 'nl']) {
			const cat = catalog(locale);
			expect(Object.keys(cat).length, `${locale} must be key-complete with en`).toBe(
				Object.keys(en).length
			);
			const identical = Object.keys(en).filter((key) => en[key] === cat[key]);
			expect(
				identical.filter((key) => !LOANWORD_LISTS[locale]!.has(key)),
				`these ${locale} keys are byte-identical to en and are not declared loanwords: ${identical
					.filter((key) => !LOANWORD_LISTS[locale]!.has(key))
					.join(', ')}`
			).toEqual([]);
			// The allow-list stays an exception list, not a blank cheque.
			expect(
				identical.length / Object.keys(en).length,
				`${identical.length} of ${Object.keys(en).length} ${locale} keys are identical to en`
			).toBeLessThan(0.1);
			// Anti-vacuity: the catalog measurably differs from en.
			const differing = Object.keys(en).filter((key) => en[key] !== cat[key]);
			expect(differing.length, `${locale} differs from en on too few keys`).toBeGreaterThan(
				Object.keys(en).length * 0.9
			);
		}
	});
});
