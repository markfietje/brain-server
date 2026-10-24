import { join } from 'node:path';

/**
 * The shared test roots. (The wizard-era pack helpers — `corpusPack` and
 * `catalogPayload` — retired with the wizard in M3-a, plan
 * `21_CHAT_FIRST_IA`: their only consumers were the wizard's own tests and
 * the a11y recipe for the wizard `/`. The kernel's packs corpus stays where
 * it is; the committed `src/lib/api/schema.d.ts` and the drift gate still
 * hold the `/workflow/wizard/packs` wire, which remains registered.)
 *
 * Paths derive from process.cwd(): vitest always runs from the shell root
 * (its own package.json), and import.meta.url is an http URL under the jsdom
 * environment.
 */

export const shellRoot = process.cwd();
export const kernelRoot = join(shellRoot, '..');
