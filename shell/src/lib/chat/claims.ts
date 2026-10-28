/**
 * LL-04 §1.4 — the pure sentence↔claim mapping behind the support render.
 *
 * WHY A PURE MODULE (the M3-a module-split law, one more time): which
 * sentence of the answer carries a provenance chip and which renders dimmed
 * with an `unsupported` chip is a DECISION, and a decision that lives in a
 * component is a decision a refactor can delete silently. The page draws
 * what this module matches; the pins drive this module directly.
 *
 * THE HONESTY RULES, stated where they can be checked:
 *
 * - The kernel's typed final carries the model's OWN sentences (`claims`),
 *   verbatim, with the hit indices they assert. The verdict on those
 *   indices is the kernel's `support` event — this module NEVER judges a
 *   cite. It only answers "which sentence is this claim's sentence".
 * - A sentence with a matching claim renders WITH its chip (the claim's
 *   indices resolve to titles/domains by the PAGE, from the grounding
 *   hits — this module does not see hits).
 * - A sentence with no matching claim renders dimmed with the
 *   `unsupported` chip: visible, never deleted, never silently kept as
 *   supported — the same law as the kernel's, on the render side.
 * - No claims (an old kernel, or the model asserted none): the answer
 *   renders as plain text — no fabricated structure over prose the kernel
 *   never typed. `null` claims and empty claims render the SAME plain way,
 *   but the turn-level support line still shows the kernel's label.
 *
 * Matching is normalization-only (case, whitespace, [n] markers) with a
 * substring test in BOTH directions: a claim may quote less of its
 * sentence than the answer carries, and the model may emit the sentence
 * slightly shorter in the claim than in the prose. No fuzzy matching — a
 * match that cannot be proven is no match, and the dimmed sentence is the
 * honest floor.
 */

/** One claim from the typed final, as the spine parsed it. */
export interface AnswerClaim {
	readonly text: string;
	readonly cites: readonly number[];
}

/** One renderable piece of the answer: the sentence's text and the claim
 * that covers it, when one does. */
export interface SentenceRender {
	readonly text: string;
	readonly claim: AnswerClaim | null;
}

/** Normalize for matching: lowercase, collapse whitespace, strip the [n]
 * citation markers the prose may carry, and trim edge punctuation noise. */
function normalize(text: string): string {
	return text
		.toLowerCase()
		.replace(/\[\d+\]/g, '')
		.replace(/[.,;:!?"'']+$/g, '')
		.replace(/\s+/g, ' ')
		.trim();
}

/** Split the answer into sentences. Deliberately simple and deterministic:
 * split after `.`, `!`, `?`, `…` when followed by whitespace or end — the
 * model's prose, not poetry. A sentence that grew no terminator (the tail
 * of a streaming answer) is still a sentence. */
export function splitSentences(answer: string): string[] {
	const out: string[] = [];
	let current = '';
	for (let i = 0; i < answer.length; i += 1) {
		const char = answer[i];
		current += char;
		const isTerminator = char === '.' || char === '!' || char === '?' || char === '…';
		const next = i + 1 >= answer.length ? '' : (answer[i + 1] ?? '');
		const nextIsBoundary = next === '' || /\s/.test(next);
		if (isTerminator && nextIsBoundary) {
			if (current.trim().length > 0) out.push(current.trim());
			current = '';
		}
	}
	if (current.trim().length > 0) out.push(current.trim());
	return out;
}

/** Map the answer's sentences onto the model's claims. A claim covers the
 * FIRST sentence it matches (a model asserting the same sentence twice is
 * one fact); a sentence no claim matches carries `claim: null` — the
 * dimmed, `unsupported`-chipped one. */
export function mapClaimsToSentences(
	sentences: readonly string[],
	claims: readonly AnswerClaim[] | null
): SentenceRender[] {
	if (claims === null || claims.length === 0) {
		return sentences.map((text) => ({ text, claim: null }));
	}
	const normalizedClaims = claims.map((claim) => ({ claim, key: normalize(claim.text) }));
	return sentences.map((sentence) => {
		const key = normalize(sentence);
		if (key.length === 0) return { text: sentence, claim: null };
		const match = normalizedClaims.find(({ key: claimKey }) => {
			if (claimKey.length === 0) return false;
			// Either direction: the claim may be a shorter quote of the
			// sentence, or the sentence a re-flow of the claim.
			return claimKey.includes(key) || key.includes(claimKey);
		});
		return { text: sentence, claim: match?.claim ?? null };
	});
}

/** The hit titles/domains one claim's indices resolve to, from the
 * grounding hits the turn already carries. A cite is a 0-based INDEX into
 * the hits array — the kernel's own universe (its verdict counts an
 * out-of-range index `unknown_refs`); resolving by hit ID here would be a
 * second, drifted gate. An out-of-range index resolves to nothing: the
 * LABEL of a bad cite is the kernel's `support` decision, and this helper
 * exists only to name the hits an in-range cite points at. */
export function citeTitles(
	cites: readonly number[],
	hits: readonly { readonly id: number; readonly title: string | null; readonly domain: string | null }[]
): string[] {
	return cites
		.map((index) => (index >= 0 && index < hits.length ? hits[index] : undefined))
		.filter((hit) => hit !== undefined)
		.map((hit) => {
			const title = hit.title ?? '';
			if (title.length === 0) return hit.domain ?? '';
			return hit.domain !== null && hit.domain.length > 0 ? `${title} · ${hit.domain}` : title;
		})
		.filter((label) => label.length > 0);
}
