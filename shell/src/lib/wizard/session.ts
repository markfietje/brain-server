/**
 * Session-local progress (D1's saved-progress/resume). sessionStorage ONLY:
 * progress dies with the tab session — never localStorage, never a cookie,
 * never the kernel (D3: no write surface). The saved shape carries the
 * question IDS and the typed answers, nothing else.
 */

import type { ResolvedPack } from './pack';

const KEY_PREFIX = 'brain-wizard-progress:';

export interface SavedProgress {
	readonly packId: string;
	readonly currentQuestionId: string;
	readonly answers: Readonly<Record<string, unknown>>;
}

export function saveProgress(pack: ResolvedPack, currentQuestionId: string, answers: Readonly<Record<string, unknown>>): void {
	try {
		const saved: SavedProgress = {
			packId: pack.pack,
			currentQuestionId,
			answers
		};
		window.sessionStorage.setItem(KEY_PREFIX + pack.pack, JSON.stringify(saved));
	} catch {
		// storage unavailable (private mode etc.) — progress simply does not
		// survive; the flow still works. Never an error surface.
	}
}

export function clearProgress(packId: string): void {
	try {
		window.sessionStorage.removeItem(KEY_PREFIX + packId);
	} catch {
		// as above
	}
}

export function restoreProgress(packId: string): SavedProgress | null {
	try {
		const raw = window.sessionStorage.getItem(KEY_PREFIX + packId);
		if (raw === null) return null;
		const parsed: unknown = JSON.parse(raw);
		if (parsed === null || typeof parsed !== 'object') return null;
		const candidate = parsed as Partial<SavedProgress>;
		if (
			candidate['packId'] !== packId ||
			typeof candidate['currentQuestionId'] !== 'string' ||
			candidate['answers'] === null ||
			typeof candidate['answers'] !== 'object'
		) {
			return null;
		}
		return {
			packId: packId,
			currentQuestionId: candidate['currentQuestionId'],
			answers: candidate['answers'] as Record<string, unknown>
		};
	} catch {
		return null;
	}
}
