import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * The ONE-source-of-truth fixtures: the SERVED pack bodies are the same
 * committed corpus files the kernel embeds (include_str!) and re-validates
 * (validate_wizard_pack) — the shell's tests run against exactly those
 * bytes, so pack drift breaks the kernel tests and the shell tests
 * TOGETHER, never apart. Paths derive from process.cwd(): vitest always
 * runs from the shell root (its own package.json), and import.meta.url is
 * an http URL under the jsdom environment.
 */

export const shellRoot = process.cwd();
export const kernelRoot = join(shellRoot, '..');

export function corpusPack(name: string): unknown {
	const bytes = readFileSync(
		`${kernelRoot}/crates/brain-fuzz/corpus/accounts/packs/${name}.json`,
		'utf8'
	);
	return JSON.parse(bytes) as unknown;
}

/** The catalog payload exactly as GET /workflow/wizard/packs serves it. */
export function catalogPayload(): {
	packs: Array<{ id: string; question_count: number; pack: unknown }>;
	count: number;
} {
	const names = ['capture-pre-screen', 'support-ticket', 'tele-health'];
	const packs = names.map((name) => {
		const pack = corpusPack(name) as { questions: unknown[] };
		return { id: name, question_count: pack.questions.length, pack };
	});
	return { packs, count: packs.length };
}
