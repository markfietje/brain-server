/**
 * II-3 — the chat turn: the pure model a surface renders.
 *
 * This module holds no Svelte and no wire. It takes a finished `RecallOutcome`
 * and produces the ONE thing a renderer is allowed to draw, which is a
 * discriminated union over the core's six states. The surface's job becomes
 * mechanical, and a surface that branches on the wrong thing is a type error
 * rather than a confidentiality leak.
 *
 * ## WHY THE RENDERER NEVER SEES A `RecallOutcome`
 *
 * P1 proved the data distinguishes refusal from absence: on a refusal there is
 * no `passages` key at all. The leak the prompt warns about is a renderer
 * that checks `outcome.passages?.length` and falls through to an empty state,
 * which re-introduces the row P1 spent eight pins closing — "you may not read
 * this" drawn as "there is nothing here", which tells a reader that rows exist
 * behind a refusal.
 *
 * So the union below is built from `outcome.state` and NEVER from emptiness,
 * and the two states that both have zero passages (`absent`, `abstained`) are
 * separate cases with separate copy. `TurnRendering` carries a `kind` whose
 * discriminant is a SINGLE literal per member, for the compiler reason recorded
 * in `recall.ts`: a two-literal discriminant stops TypeScript narrowing, and
 * then `return outcome.passages` type-checks against a failure arm.
 *
 * ## WHAT A TURN IS NOT
 *
 * There is no model in this turn. P2's transport decision (see
 * `docs/gate-law-register.md`) is that no Swift host fills
 * `KernelRecallTransport` in the shipped app, so the on-device model's tool has
 * no live transport and there are no model deltas to stream. A turn is
 * therefore retrieval plus its evidence — which is why Plan 02 §4's
 * "model unavailable degrades to working retrieval" is not a fallback here but
 * the ONLY mode. `provenance` is what makes that honest: a retrieval turn still
 * carries citations, so the user sees what the memory said rather than a
 * sentence with nothing behind it.
 */

import type { Passage, RecallOutcome } from '../api/recall';

/**
 * What the user is shown for one turn. Each member is one rendering, and the
 * copy is attached rather than looked up so a surface cannot substitute a
 * generic empty-state string for a refusal by forgetting a branch.
 *
 * `kind` values are exactly the core's states plus `pending`. Nothing is
 * merged: `absent` and `abstained` stay apart because they are different facts
 * (the search ran and found nothing vs the store could not answer), and
 * `refused` stays apart from both because it is a fact about the READER.
 */
export type TurnRendering =
	| { kind: 'pending'; query: string }
	| { kind: 'answered'; query: string; passages: Passage[]; confidence: number | null }
	| { kind: 'absent'; query: string; searched: string; limit: number }
	| { kind: 'abstained'; query: string; searched: string; limit: number; confidence: number | null }
	| { kind: 'refused'; query: string; status: number; scope: string }
	| { kind: 'unreachable'; query: string; status: number; detail: string }
	| { kind: 'contract_violation'; query: string; status: number; detail: string };

/**
 * Projects a finished outcome onto what may be drawn.
 *
 * The switch is over `outcome.state` and nothing else. In particular there is
 * no `hasPassages(outcome)` branch and no `passages.length` test: a renderer
 * that reached for either would be re-deriving the distinction the core already
 * made, and the first bug it would introduce is collapsing a refusal into an
 * absence. `recallPassages()` — the accessor that THROWS on a refusal — is
 * deliberately not used here either, because this function must not throw: a
 * refusal is a legitimate rendering, not an error.
 */
export function renderTurn(outcome: RecallOutcome, query: string): TurnRendering {
	switch (outcome.state) {
		case 'answered':
			return {
				kind: 'answered',
				query,
				passages: outcome.passages,
				confidence: outcome.confidence
			};
		case 'absent':
			return {
				kind: 'absent',
				query,
				searched: outcome.searched.query,
				limit: outcome.searched.limit
			};
		case 'abstained':
			return {
				kind: 'abstained',
				query,
				searched: outcome.searched.query,
				limit: outcome.searched.limit,
				confidence: outcome.confidence
			};
		case 'refused':
			return { kind: 'refused', query, status: outcome.status, scope: outcome.scope };
		case 'unreachable':
			return { kind: 'unreachable', query, status: outcome.status, detail: outcome.detail };
		case 'contract_violation':
			return {
				kind: 'contract_violation',
				query,
				status: outcome.status,
				detail: outcome.detail
			};
	}
}

/**
 * True only for the state that renders as an ANSWER.
 *
 * Plan 06 §3 is explicit that abstention is the product's sharpest behaviour,
 * so the surface must be able to ask "is this an answer?" without re-deriving
 * it. Abstention IS an answer — "I could not find that" — and is NOT an error,
 * so `abstained` is included here and `refused`/`unreachable` are not.
 */
export function isAnswerLike(turn: TurnRendering): boolean {
	return turn.kind === 'answered' || turn.kind === 'abstained';
}

/**
 * The `data-rendering` attribute a surface puts on its turn container.
 *
 * One stable token per state, so a DOM-level assertion can distinguish the
 * four renderings without depending on copy (which is translated, and which
 * therefore cannot be an assertion target across five locales). This is the
 * mechanical half of "four visibly distinct renderings": the pin reads the
 * attribute, and a surface that collapses two states into one markup shape
 * fails even if its wording differs.
 */
export function renderingToken(turn: TurnRendering): string {
	return turn.kind;
}