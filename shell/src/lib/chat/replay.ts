/**
 * Replay — persisted chat turns mapped onto the page's `TurnRendering`.
 *
 * ## WHY A PURE MODULE (the M3-a module-split law)
 *
 * A rule that lives in a component is a rule a refactor can delete silently;
 * a rule that lives in a pure module is a rule the pins can drive directly.
 * The mapping is also the THIRD producer of the transcript's shape — the
 * deterministic `/recall` rendering and the chat-spine rendering (plan 25)
 * share the union — so it belongs beside `lib/chat/turn.ts`, not inside the
 * sidebar.
 *
 * ## THE MAPPING, AND THE ONE HONEST COMPRESSION
 *
 * The store persists PAIRS: a user row (seq n) and an assistant row
 * (seq n+1, carrying the grounding event and the tier). The transcript
 * renders one turn per pair — the question and its answer as one article —
 * exactly as the live surface does. Two turns that are not a complete pair
 * (an odd tail: the store's `every_persisted_user_row_has_its_assistant_
 * beside_it` pin says this cannot happen, and the pin is why this mapper
 * may TREAT it as impossible-but-named) are skipped rather than rendered
 * as a question with no answer.
 *
 * The assistant's text is the answer of record (deltas are never stored),
 * and the grounding hits ride the turn as PROVENANCE — the provenance-first
 * law survives replay: a replayed turn shows what it was grounded on, from
 * the grounding event the kernel stored verbatim.
 *
 * ## THE UNION MEMBER
 *
 * A replayed pair is a GENERATED turn: `kind: 'generated'`, carrying the
 * question, the answer text, the tier, and the passages the grounding hits
 * map to. `lib/chat/turn.ts` owns the union; this module is one of its
 * producers, and the live streaming consumer (M3-d) becomes the fourth.
 */

import type { Passage } from '../api/recall';
import type { StoredTurn } from '../shell/sessions.svelte.js';

/** One grounding hit, as the kernel's grounding event carries it. */
interface GroundingHit {
	id: number;
	title: string | null;
	score: number;
	domain: string | null;
	flagged: boolean;
}

/** Pull the hits array out of a stored grounding event. Malformed events
 * degrade to NO passages — the replay then shows the answer with no
 * evidence block, which is the honest shape of "we cannot prove what
 * grounded this" — never a fabricated citation. */
export function groundingHits(grounding: Record<string, unknown> | null): GroundingHit[] {
	if (grounding === null) return [];
	const raw = grounding['hits'];
	if (!Array.isArray(raw)) return [];
	const hits: GroundingHit[] = [];
	for (const item of raw) {
		if (typeof item !== 'object' || item === null) continue;
		const rec = item as Record<string, unknown>;
		const id = rec['id'];
		const score = rec['score'];
		if (typeof id !== 'number' || typeof score !== 'number') continue;
		const title = rec['title'];
		const domain = rec['domain'];
		hits.push({
			id,
			score,
			title: typeof title === 'string' ? title : null,
			domain: typeof domain === 'string' ? domain : null,
			flagged: rec['flagged'] === true
		});
	}
	return hits;
}

/** Map a grounding hit onto the page's `Passage` shape. The grounding event
 * carries ids/titles/scores/domains — the fields provenance renders — and
 * NOT the passage body, so `text` stays the empty string and the citation
 * renders its title and score without pretending to hold the source text
 * (the full text lives in the memory row, reachable through Memory). */
export function hitToPassage(hit: GroundingHit): Passage {
	return {
		id: hit.id,
		title: hit.title,
		text: '',
		snippet: null,
		domain: hit.domain,
		score: hit.score,
		valid_from: null,
		valid_to: null,
		flagged: hit.flagged,
		ingest_kind: null,
		memory_kind: null,
		authority: null
	};
}

/** The replayed turn: the page's `Turn` fields, produced purely. */
export interface ReplayedTurn {
	readonly rendering: {
		kind: 'generated';
		query: string;
		answer: string;
		tier: string | null;
		passages: Passage[];
	};
	/** The grounding passages, for the page's provenance block. */
	readonly passages: Passage[];
}

/**
 * Map a stored turn page onto the transcript's turns. Pairs walk in seq
 * order; a user row opens a pair and its assistant row closes it. Incomplete
 * tails are skipped (the store's own pin says they cannot exist; the skip is
 * the named posture, not a silent guess). Empty input maps to an empty
 * transcript — a stored session with zero turns IS empty.
 */
export function replayTurns(turns: readonly StoredTurn[]): ReplayedTurn[] {
	const out: ReplayedTurn[] = [];
	let open: { query: string } | null = null;
	for (const turn of turns) {
		if (turn.role === 'user') {
			// An unclosed previous pair (no assistant row followed) is skipped.
			open = { query: turn.content };
			continue;
		}
		if (open === null) continue;
		const hits = groundingHits(turn.grounding);
		const passages = hits.map(hitToPassage);
		out.push({
			rendering: {
				kind: 'generated',
				query: open.query,
				answer: turn.content,
				tier: turn.tier,
				passages
			},
			passages
		});
		open = null;
	}
	return out;
}
