/**
 * Node's resolver hook for the plugin's TypeScript sources.
 *
 * The plugin's own modules import each other with the `.js` extension the
 * TypeScript "NodeNext" convention prescribes (`import { sanitizeForBlock }
 * from "./format.js"`), because the HOST loads the plugin's TypeScript
 * directly and the host's loader resolves `.js` → `.ts`.
 *
 * **Node does not do that rewrite** — `node plugin/src/brain-client.ts` fails
 * with `ERR_MODULE_NOT_FOUND: .../src/format.js` (measured 2026-10-22). The
 * plugin's own test lane does not hit this, because vite resolves it.
 *
 * So the integration drill registers this hook: it retries a relative `.js`
 * specifier as `.ts` ONLY when the `.js` file does not exist. Nothing else is
 * touched — an absolute specifier, a bare specifier, or a `.js` file that
 * really exists all resolve exactly as they did before.
 */
export async function resolve(specifier, context, nextResolve) {
	if (
		specifier.startsWith('./') &&
		specifier.endsWith('.js') &&
		context.parentURL !== undefined
	) {
		try {
			return await nextResolve(specifier, context);
		} catch (error) {
			try {
				return await nextResolve(`${specifier.slice(0, -3)}.ts`, context);
			} catch {
				throw error;
			}
		}
	}
	return nextResolve(specifier, context);
}
