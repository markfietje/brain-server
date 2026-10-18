/**
 * II-4 — the provenance block: Plan 06 §1's five fields, as TEXT NODES.
 *
 * Two rules, and the first is the one that gets broken.
 *
 * **1. Text nodes only.** `eslint.config.js` bans `{@html}`, and this module
 * never returns markup — it returns STRUCTURE, and the surface interpolates it
 * as text. That is deliberate: provenance that arrives as a string to be
 * injected is provenance one refactor away from being injected, and Plan 04 §3
 * is the write-up of what that costs. Every field here is a value the surface
 * places between tags.
 *
 * **2. Bi-temporality is the field that changes BEHAVIOUR.** Plan 06 §2: if
 * `valid_to` has passed, the row renders as HISTORICAL, not as a current fact.
 * A harness that shows a stale fact as current is the exact failure this
 * kernel exists to prevent, so the expiry test is computed here rather than
 * left to a surface to remember — a surface that forgets shows a 2019
 * retention policy as though it were in force.
 *
 * ## WHY `owner` AND `chain_head` ARE NOT HERE
 *
 * Plan 06 §1 lists five fields; two of them are not on the wire.
 * `RecallHit` (openapi.yaml) carries no owner, and the recall response carries
 * no chain head. Inventing either would be fabricating an authz fact and a
 * provenance fact respectively — the two things a boundary must never do — and
 * P1's register row already records that the core deliberately omits both.
 *
 * So this renders FOUR of the five, says which two are missing and why, and
 * the absence is visible in the block rather than papered over with a
 * placeholder that looks like data. `unavailable` is a discriminated arm so a
 * surface cannot quietly render `owner: —` and a reader cannot mistake an
 * absent fact for an empty one.
 */

import type { Passage } from '../api/recall';

/**
 * One field the kernel did not send, named rather than blanked.
 *
 * `kind: 'unavailable'` is the DISCRIMINANT and it is not decoration. Without
 * a literal discriminant on BOTH arms, TypeScript cannot narrow the union, so
 * `fact.value` is an error on the absent arm and every reader reaches for a
 * cast or an `in` check — which is how a provenance field silently degrades
 * into a blank string. The compiler is what guarantees the block stays total.
 */
export interface UnavailableFact {
	readonly kind: 'unavailable';
	/** Why it is absent — shown to the user, so it reads as a reason. */
	readonly because: string;
}

/**
 * A provenance line. `text` is a VALUE, never markup: the surface
 * interpolates it between tags and `{@html}` stays banned.
 */
export type Fact =
	| { readonly kind: 'present'; readonly label: string; readonly value: string }
	| UnavailableFact;

/** Confidence as a WORD plus the number — Plan 06 §2, never a bare number. */
export type Confidence =
	| { readonly kind: 'word'; readonly word: 'high' | 'medium' | 'low'; readonly value: number }
	| UnavailableFact;

/**
 * The bands. 0.82 must mean something to a reader, and "0.82" does not; the
 * number stays for the sceptical, which is the whole of the point of a
 * falsifiable claim.
 */
export function confidenceWord(value: number | null): Confidence {
	if (value === null) {
		return {
			kind: 'unavailable',
			because: 'the kernel returned no confidence; provenance was not requested or the store had none'
		};
	}
	const word = value >= 0.7 ? 'high' : value >= 0.4 ? 'medium' : 'low';
	return { kind: 'word', word, value };
}

/**
 * Bi-temporal validity, and the expired case is the load-bearing one.
 *
 * `valid_to` is compared against the caller's clock. Passing the clock IN
 * rather than reading `Date.now()` is what makes this pinnable: a rule that
 * reads the ambient clock can only be tested by waiting for it, and the
 * interesting cases here are all in the past.
 *
 * A row whose `valid_to` has passed is `historical: true` and the surface
 * marks it as such. It is still returned — a superseded fact is evidence, and
 * hiding it would make the citation block lie about what was found — but it
 * must never be drawn as current.
 */
export interface Validity {
	readonly validFrom: string | null;
	readonly validTo: string | null;
	readonly historical: boolean;
	/** What the surface shows. Absent validity is stated, not left blank. */
	readonly summary: string;
}

/** `true` when `validTo` is non-null and strictly in the past. */
export function isHistorical(passage: Passage, nowMs: number): boolean {
	if (passage.valid_to === null) return false;
	const parsed = Date.parse(passage.valid_to);
	// An unparseable `valid_to` is treated as EXPIRED rather than current: a
	// date the client cannot read is a date it cannot certify as future, and
	// Plan 06 §2's bias is toward calling a fact historical when unsure.
	if (Number.isNaN(parsed)) return true;
	return parsed < nowMs;
}

export function validityOf(passage: Passage, nowMs: number): Validity {
	const historical = isHistorical(passage, nowMs);
	if (passage.valid_to !== null && historical) {
		return {
			validFrom: passage.valid_from,
			validTo: passage.valid_to,
			historical: true,
			summary:
				passage.valid_from !== null
					? `historical — was valid ${passage.valid_from} to ${passage.valid_to}, not current`
					: `historical — expired ${passage.valid_to}, not current`
		};
	}
	if (passage.valid_to !== null) {
		return {
			validFrom: passage.valid_from,
			validTo: passage.valid_to,
			historical: false,
			summary:
				passage.valid_from !== null
					? `valid ${passage.valid_from} to ${passage.valid_to}`
					: `valid until ${passage.valid_to}`
		};
	}
	if (passage.valid_from !== null) {
		return {
			validFrom: passage.valid_from,
			validTo: null,
			historical: false,
			summary: `valid from ${passage.valid_from}`
		};
	}
	return {
		validFrom: null,
		validTo: null,
		historical: false,
		summary: 'validity not stated by the kernel'
	};
}

/** One cited passage, with every field the surface may draw. */
export interface Citation {
	readonly id: number;
	readonly text: string;
	readonly title: Fact;
	readonly domain: Fact;
	readonly owner: Fact;
	readonly chainHead: Fact;
	readonly validity: Validity;
	readonly confidence: Confidence;
	readonly score: number;
	readonly flagged: boolean;
}

/** The two fields Plan 06 §1 names that `/recall` does not carry. */
const OWNER_ABSENT =
	'the kernel does not return an owner on /recall; this is Plan 06 field 2 and it is unavailable on this route';
const CHAIN_ABSENT =
	'the kernel does not return a chain head on /recall; Plan 06 field 3 is unavailable on this route';

/**
 * Builds one citation. The whole five-field set is always present as a value,
 * with the two unavailable ones carrying their reason — so a surface cannot
 * "helpfully" drop a field by forgetting to render an arm.
 */
export function citationFor(passage: Passage, confidence: number | null, nowMs: number): Citation {
	return {
		id: passage.id,
		text: passage.text,
		title:
			passage.title !== null && passage.title.length > 0
				? { kind: 'present' as const, label: 'title', value: passage.title }
				: { kind: 'unavailable' as const, because: 'no title on this row' },
		domain:
			passage.domain !== null && passage.domain.length > 0
				? { kind: 'present' as const, label: 'domain', value: passage.domain }
				: { kind: 'unavailable' as const, because: 'no domain on this row' },
		owner: { kind: 'unavailable', because: OWNER_ABSENT },
		chainHead: { kind: 'unavailable', because: CHAIN_ABSENT },
		validity: validityOf(passage, nowMs),
		confidence: confidenceWord(confidence),
		score: passage.score,
		flagged: passage.flagged
	};
}

/**
 * The block for one turn: the summary line plus every citation.
 *
 * The summary carries the COUNT, because a citation list with no count reads
 * as "everything" — and Plan 06 §2's shape shows `recalled N passages`
 * precisely so a reader can tell 3 of 3 from 3 of 900.
 */
export interface ProvenanceBlock {
	readonly count: number;
	readonly confidence: Confidence;
	readonly citations: readonly Citation[];
	/** Copy for ⌘⇧C: text WITH its provenance, so the receipt travels. */
	readonly receipt: string;
}

/**
 * `receipt` is the copy-with-provenance payload. Plan 02 §3 calls this "a
 * small decision with a large effect": it means a memory leaves the app
 * carrying its chain reference and its validity, which is the product's whole
 * claim made falsifiable by whoever holds the copied text.
 */
export function provenanceBlock(
	passages: readonly Passage[],
	confidence: number | null,
	nowMs: number
): ProvenanceBlock {
	const citations = passages.map((passage) => citationFor(passage, confidence, nowMs));
	const summary = confidenceWord(confidence);
	const receiptLines = citations.map((citation) => {
		const domain = citation.domain.kind === 'present' ? citation.domain.value : 'no domain';
		const validity = citation.validity.summary;
		return [
			`[${citation.id}] ${citation.title.kind === 'present' ? citation.title.value : 'untitled'}`,
			`    ${citation.text}`,
			`    domain: ${domain} · ${validity}`,
			citation.chainHead.kind === 'unavailable' ? `    ${citation.chainHead.because}` : '',
			citation.flagged ? '    quarantined at ingest' : ''
		]
			.filter((line) => line.length > 0)
			.join('\n');
	});
	const confidenceText = summary.kind === 'word' ? `${summary.word} (${summary.value.toFixed(2)})` : 'not stated';
	return {
		count: citations.length,
		confidence: summary,
		citations,
		receipt: [
			`recalled ${citations.length} passage${citations.length === 1 ? '' : 's'} · confidence ${confidenceText}`,
			...receiptLines
		].join('\n')
	};
}