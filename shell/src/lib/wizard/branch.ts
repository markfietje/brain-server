/**
 * The branch engine — the client-side mirror of the kernel's answer
 * classification (classify_answer) and branch walk. THE LAW: an answer is
 * only ever a member of the question's CLOSED vocabulary; anything else is
 * refused (the caller renders the abstain card). No free text exists
 * anywhere in the flow, so injected "answers" cannot steer it.
 */

import type { ResolvedPack, WizardQuestion } from './pack';

export type Given = string | number | boolean;

/** The vocabulary key for a given answer, or null when it is out of
 * vocabulary (mistyped, unknown, out of range) — the kernel's
 * classify_answer returning None. */
export function classifyAnswer(q: WizardQuestion, given: unknown): string | null {
	switch (q.qtype) {
		case 'choice': {
			if (typeof given !== 'string') return null;
			return q.answerKeys.includes(given) ? given : null;
		}
		case 'score': {
			if (typeof given !== 'number' || !Number.isInteger(given) || given < 0) return null;
			const key = String(given);
			return q.answerKeys.includes(key) ? key : null;
		}
		case 'noul': {
			if (typeof given !== 'boolean') return null;
			return String(given);
		}
	}
}

/** The rendered (typed, free-text-free) answer text for a vocabulary key —
 * the display form of the answer, mirroring the kernel's render_options. */
export function renderedAnswer(q: WizardQuestion, key: string): string {
	switch (q.qtype) {
		case 'choice':
			return key;
		case 'score': {
			const levels = Array.isArray(q.criteria) ? (q.criteria as unknown[]) : [];
			const label = levels[Number(key)];
			return `level ${key}: ${typeof label === 'string' ? label : key}`;
		}
		case 'noul': {
			const criteria =
				q.criteria !== undefined && q.criteria !== null && typeof q.criteria === 'object'
					? (q.criteria as Record<string, unknown>)
					: {};
			const text = criteria[key];
			return typeof text === 'string' ? text : key === 'true' ? 'yes' : 'no';
		}
	}
}

export type WalkStep =
	| { kind: 'question'; id: string }
	| { kind: 'end' };

/** The next question id (or 'end') after an IN-VOCABULARY answer. Total
 * over validated packs: every vocabulary key has a branch by construction. */
export function branchOf(pack: ResolvedPack, q: WizardQuestion, key: string): WalkStep {
	const target = q.next[key] ?? 'end';
	return target === 'end' ? { kind: 'end' } : { kind: 'question', id: target };
}

/** The first question of a pack (or null when the id is somehow absent —
 * only possible for an unvalidated pack). */
export function firstStep(pack: ResolvedPack): WalkStep {
	return pack.byId.has(pack.first) ? { kind: 'question', id: pack.first } : { kind: 'end' };
}
