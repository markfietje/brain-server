/**
 * II-7f — the `de` locale decision, as a STATED VALUE.
 *
 * ## THE PROBLEM, quoted because the plan's rule is the whole point
 *
 * Plan 02 §6: the shell ships en/de/fr/es/nl. AFM 3 Core on this machine reports
 * **24 supported languages including en, fr, es, nl — and not de**. The plan's
 * options are retrieval-only for `de`, route `de` through an external model
 * (Plan 03), or drop `de`, under the rule: *"Do not silently answer `de` in
 * English and call it German."*
 *
 * ## THE DECISION: `de` is RETRIEVAL-ONLY, and it is stated rather than defaulted
 *
 * **DECIDED: retrieval-only for `de`.** The two alternatives were considered
 * against facts, not preference:
 *
 * * **Drop `de`** — REJECTED on a contract cost, not a preference. Removing a
 *   locale is a KEYS-PARITY CONTRACT CHANGE, not a copy-paste: the five-way
 *   parity gate (`i18n-parity.test.ts`) holds all five catalogs to en's exact
 *   key set, and `project.inlang/settings.json` declares `locales`. Dropping
 *   `de` would mean editing a gate, a settings file and five catalogs to say
 *   less — spending a contract change to avoid stating a position.
 * * **Route `de` through an external model (Plan 03)** — REJECTED ON A FACT.
 *   Every external request is refused on this machine with
 *   `ModelManagerError 1046`, and this phase may not promise escalation. A
 *   decision that depends on a capability which is refused here is a decision
 *   that cannot be true today, and stating it as today's position would be the
 *   same error in the opposite direction.
 * * **Retrieval-only** — ADOPTED. It states exactly what is true: `de` renders,
 *   `de` retrieves, and `de` generates nothing.
 *
 * ## WHY THIS IS NOT "SILENTLY ENGLISH", which is the failure the rule names
 *
 * Two different things could be silently English, and the decision covers both:
 *
 * 1. **The GENERATED ANSWER.** There is no model session in the shipped app at
 *    all (II-7e's derivation: zero production conformers of
 *    `KernelRecallTransport` that can answer), so no locale generates anything
 *    and `de` cannot be answered in English — there is no answer to give. When
 *    a session is wired, `de` remains `retrieval_only` because AFM 3 Core has
 *    no German, which is the durable half of the finding.
 * 2. **THE SHELL'S OWN STRINGS.** These the shell controls outright, and
 *    `de.json` is genuinely German: 238 of 250 keys differ from `en.json`. The
 *    12 that match are genuine German loanwords (`Version`, `Pipeline`,
 *    `Kernel`, `Reranker`, …) and they are an EXPLICIT allow-list in
 *    `locale-capability.test.ts` — so a new untranslated key is red rather than
 *    invisible.
 *
 * ## WHY A TABLE AND NOT A BRANCH
 *
 * The rule is that the decision is "a stated value in the source, not a branch
 * that happens to fall through". A lookup table of stated values is that. The
 * alternative — a `locale === 'de' ? retrievalOnly : generate()` ternary — has
 * two defects a table does not: adding a locale silently inherits whatever the
 * ternary's else-branch does, and the per-locale answer is only readable by
 * reading the branch. Both are how a locale quietly becomes English.
 *
 * `generationFor` therefore returns a distinct `unknown_locale` for anything
 * outside the catalog set rather than defaulting to a capability: a new locale
 * with no stated decision is UNDECIDED, and undecided must fail closed.
 */

export const SHIPPED_LOCALES = ['en', 'de', 'fr', 'es', 'nl'] as const;
export type Locale = (typeof SHIPPED_LOCALES)[number];

/**
 * What a locale can GENERATE.
 *
 * `available` — the on-device model's roster covers it (AFM 3 Core reports 24
 * languages including en, fr, es, nl).
 * `retrieval_only` — retrieval and rendering work; generation does not.
 *
 * NOTE what is deliberately NOT here: there is no `english_fallback` member.
 * A fallback would be a name for the failure the plan forbids, and adding one
 * later would be the exact change this decision exists to make hard.
 */
export type GenerationCapability = 'available' | 'retrieval_only';

/** Returned for a locale with no stated decision. Never a capability. */
export const UNDECIDED = 'unknown_locale' as const;
export type Undecided = typeof UNDECIDED;

/**
 * THE DECISION, one stated value per shipped locale.
 *
 * `de` is `retrieval_only` because AFM 3 Core does not report German among its
 * 24 supported languages. The other four are `available` **as a roster fact** —
 * what the model COULD do once a session exists — and not as a claim that it
 * does today. II-7e derives that no session exists; this table does not
 * contradict it, and `generationFor` deliberately says nothing about whether a
 * session is present, because that is a transport fact and this is a roster
 * fact. Keeping them apart is what stops a future session landing and the table
 * being read as "de now generates".
 */
export const LOCALE_GENERATION: Readonly<Record<Locale, GenerationCapability>> = Object.freeze({
	en: 'available',
	de: 'retrieval_only',
	fr: 'available',
	es: 'available',
	nl: 'available'
});

/**
 * The capability for a locale — or `unknown_locale`, never a default.
 *
 * There is deliberately no second argument and no fallback chain. A function
 * that took `(locale, fallback)` would put the silent-English capability
 * back within one call site of being reachable, and the fallback parameter is
 * exactly the thing this module exists to make unnecessary.
 */
export function generationFor(locale: string): GenerationCapability | Undecided {
	if ((SHIPPED_LOCALES as readonly string[]).includes(locale)) {
		return LOCALE_GENERATION[locale as Locale];
	}
	return UNDECIDED;
}

/**
 * May this locale GENERATE? The question a surface asks before offering a
 * generate affordance.
 *
 * False for `unknown_locale`, which is the fail-closed direction: a locale with
 * no stated decision gets retrieval and no generate button, rather than a
 * generate button that would answer in whatever language the model defaults to.
 */
export function mayGenerate(locale: string): boolean {
	return generationFor(locale) === 'available';
}

/**
 * The one sentence a user reads when a `de` surface declines to generate.
 *
 * Stated here rather than assembled at the call site because the rule is about
 * WHAT IS SAID, and prose assembled from fragments is how a locale ends up
 * described as German while being answered in English. There is deliberately no
 * English fallback sentence here to reach for.
 */
export const RETRIEVAL_ONLY_NOTICE_DE =
	'Diese Ansicht antwortet nur aus dem Speicher. Die Berechnung auf dem Gerät ' +
	'unterstützt Deutsch nicht, daher wird hier nichts erzeugt — die übrigen ' +
	'Sprachen sind abrufbar und erzeugbar.';
