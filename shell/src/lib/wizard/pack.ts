/**
 * The total pack parser — the client-side mirror of the kernel's
 * `validate_wizard_pack` (src/workflow/wizard.rs). Same laws, same
 * closedness: a pack is either FULLY valid (resolved, with every question's
 * answer vocabulary derived) or refused by name. Never throws; never
 * invents. The kernel remains the schema's authority — this mirror exists
 * so the renderer refuses hostile/ambiguous packs WITHOUT a network round
 * trip and never renders a branch the kernel would refuse.
 */

export type QType = 'choice' | 'score' | 'noul';

export interface WizardQuestion {
	readonly id: string;
	readonly qtype: QType;
	readonly instructions: string;
	/** choice: label → description|null; score: level labels; noul: absent or {false,true} texts */
	readonly criteria?: unknown;
	/** answer key → question id | 'end' — total over the answer vocabulary */
	readonly next: Readonly<Record<string, string>>;
	/** derived: the CLOSED answer vocabulary (choice labels | score indices | true/false) */
	readonly answerKeys: readonly string[];
}

export interface ResolvedPack {
	readonly pack: string;
	readonly first: string;
	readonly questions: readonly WizardQuestion[];
	readonly byId: ReadonlyMap<string, WizardQuestion>;
}

export type ParseResult =
	| { ok: true; pack: ResolvedPack }
	| { ok: false; reason: string };

const ID_MAX = 64;
const OPTION_CEILING = 20;
const END = 'end';
const QTYPES: readonly QType[] = ['choice', 'score', 'noul'];

function refuse(reason: string): ParseResult {
	return { ok: false, reason: `wizard_pack_invalid: ${reason}` };
}

function isNonEmptyBoundedString(v: unknown): v is string {
	return typeof v === 'string' && v.length > 0 && v.length <= ID_MAX;
}

/** The closed answer vocabulary of one question, derived from its qtype. */
function answerKeysOf(qtype: QType, criteria: unknown): string[] | null {
	switch (qtype) {
		case 'choice': {
			if (criteria === null || typeof criteria !== 'object' || Array.isArray(criteria)) {
				return null;
			}
			const keys = Object.keys(criteria);
			if (keys.length < 1 || keys.length > OPTION_CEILING) return null;
			return keys;
		}
		case 'score': {
			if (!Array.isArray(criteria) || criteria.length < 1 || criteria.length > OPTION_CEILING) {
				return null;
			}
			return criteria.map((_, i) => String(i));
		}
		case 'noul': {
			// noul criteria are OPTIONAL (the ratified packs carry both shapes).
			if (criteria === undefined) return ['false', 'true'];
			if (criteria === null || typeof criteria !== 'object' || Array.isArray(criteria)) {
				return null;
			}
			const keys = Object.keys(criteria);
			return keys.length === 2 && keys.includes('false') && keys.includes('true')
				? ['false', 'true']
				: null;
		}
	}
}

export function parsePack(value: unknown): ParseResult {
	if (value === null || typeof value !== 'object' || Array.isArray(value)) {
		return refuse('pack must be a JSON object');
	}
	const obj = value as Record<string, unknown>;
	if (!isNonEmptyBoundedString(obj['pack'])) {
		return refuse('pack id must be a non-empty string of at most 64 chars');
	}
	if (!isNonEmptyBoundedString(obj['first'])) {
		return refuse('first must be a non-empty string of at most 64 chars');
	}
	const rawQuestions = obj['questions'];
	if (!Array.isArray(rawQuestions) || rawQuestions.length === 0) {
		return refuse('questions must be a non-empty array');
	}

	const seen = new Set<string>();
	const questions: WizardQuestion[] = [];
	for (const raw of rawQuestions) {
		if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
			return refuse('question must be a JSON object');
		}
		const q = raw as Record<string, unknown>;
		if (!isNonEmptyBoundedString(q['id'])) {
			return refuse('question id must be a non-empty string of at most 64 chars');
		}
		const id = q['id'];
		if (seen.has(id)) return refuse(`duplicate question id ${id}`);
		seen.add(id);
		if (typeof q['qtype'] !== 'string' || !QTYPES.includes(q['qtype'] as QType)) {
			return refuse(`question ${id}: unknown question type`);
		}
		const qtype = q['qtype'] as QType;
		if (typeof q['instructions'] !== 'string') {
			return refuse(`question ${id}: instructions must be a string`);
		}
		if (q['next'] === null || typeof q['next'] !== 'object' || Array.isArray(q['next'])) {
			return refuse(`question ${id}: next must be an object`);
		}
		const nextMap = q['next'] as Record<string, unknown>;
		const keys = answerKeysOf(qtype, q['criteria']);
		if (keys === null) {
			return refuse(`question ${id}: criteria do not fit the closed ${qtype} vocabulary`);
		}
		const next: Record<string, string> = {};
		for (const key of keys) {
			const target = nextMap[key];
			if (!isNonEmptyBoundedString(target) || !(target === END || target.length <= ID_MAX)) {
				return refuse(`question ${id}: next must map answer ${JSON.stringify(key)} to a question id or "end"`);
			}
			next[key] = target;
		}
		if (Object.keys(nextMap).length !== keys.length) {
			return refuse(`question ${id}: next carries keys outside the answer vocabulary`);
		}
		questions.push({
			id,
			qtype,
			instructions: q['instructions'],
			criteria: q['criteria'],
			next,
			answerKeys: keys
		});
	}

	// Branch targets must exist.
	for (const q of questions) {
		for (const target of Object.values(q.next)) {
			if (target !== END && !seen.has(target)) {
				return refuse(`question ${q.id}: branch target ${JSON.stringify(target)} does not exist`);
			}
		}
	}

	// Reachable from first, and no cycles (DFS colors — the kernel's walk).
	const byId = new Map<string, WizardQuestion>(questions.map((q) => [q.id, q]));
	const color = new Map<string, 1 | 2>();
	const visit = (id: string): string | null => {
		const c = color.get(id);
		if (c === 2) return null;
		if (c === 1) return `branch cycle through ${id}`;
		color.set(id, 1);
		const q = byId.get(id);
		if (!q) return `first names absent question ${id}`;
		for (const target of Object.values(q.next)) {
			if (target !== END) {
				const err = visit(target);
				if (err) return err;
			}
		}
		color.set(id, 2);
		return null;
	};
	const cycleErr = visit(obj['first']);
	if (cycleErr) return refuse(cycleErr);
	for (const q of questions) {
		if (!color.has(q.id)) return refuse(`question ${q.id} is unreachable from first`);
	}

	return {
		ok: true,
		pack: { pack: obj['pack'], first: obj['first'], questions, byId }
	};
}
