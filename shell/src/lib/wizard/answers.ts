/**
 * The assembled-answers builder — the client-side mirror of the kernel's
 * `assemble_answers` walk: start at `first`, answer each question from the
 * recorded answers, follow the branch the ANSWER drives, and ABSTAIN
 * (never invent) on anything ambiguous or unanswered — the walk ends
 * rather than guessing a branch. The exported case names its origin
 * channel (the closed vocabulary, 'wizard' for this renderer) and carries
 * typed answers only.
 */

import type { ResolvedPack } from './pack';
import { branchOf, classifyAnswer, firstStep, renderedAnswer } from './branch';
import { exportTelemetry } from '$lib/telemetry/buffer';

export interface TypedAnswer {
	readonly question: string;
	readonly answer: string;
}

export interface AssembledCase {
	readonly origin: 'wizard';
	readonly pack: string;
	readonly abstain: boolean;
	readonly answers: readonly TypedAnswer[];
	readonly abstained: readonly string[];
}

/** Walk the pack along the recorded answers. `answers` maps question id →
 * the GIVEN value (string | number | boolean). Unknown ids and out-of-
 * vocabulary values are simply never reached (the branch engine refused
 * them at input time); an unanswered reached question ends the walk in an
 * abstain. */
export function assembleAnswers(pack: ResolvedPack, answers: Readonly<Record<string, unknown>>): AssembledCase {
	const typed: TypedAnswer[] = [];
	const abstained: string[] = [];
	let step = firstStep(pack);
	let guard = 0;
	while (step.kind === 'question') {
		guard += 1;
		if (guard > pack.questions.length + 1) {
			// Defensive: the parser refuses cycles; the walk never spins.
			break;
		}
		const q = pack.byId.get(step.id);
		if (!q) break;
		const given = answers[q.id];
		const key = given === undefined ? null : classifyAnswer(q, given);
		if (key === null) {
			abstained.push(q.id);
			break;
		}
		typed.push({ question: q.id, answer: renderedAnswer(q, key) });
		step = branchOf(pack, q, key);
	}
	return {
		origin: 'wizard',
		pack: pack.pack,
		abstain: abstained.length > 0,
		answers: typed,
		abstained
	};
}

/** The user-facing evidence artifact (the Art. 22-engineering export): the
 * assembled case PLUS the local-only telemetry block (D8) — question ids
 * and coarse timings, never question text, never answers beyond the case
 * itself. The ONLY door the telemetry buffer has. */
export function buildAnswersExport(pack: ResolvedPack, answers: Readonly<Record<string, unknown>>): string {
	return JSON.stringify(
		{
			case: assembleAnswers(pack, answers),
			telemetry: exportTelemetry()
		},
		null,
		2
	);
}
