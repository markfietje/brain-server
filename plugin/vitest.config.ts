import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

/**
 * III-8 · the plugin's own vitest configuration.
 *
 * WHY THIS FILE EXISTS AT ALL: until this round the plugin's suite had NO
 * runner configuration and NO lane, so 1,768 lines of pins across seven files
 * executed NOWHERE. `npx vitest run` collected them by vitest's defaults, five
 * of the seven failed to LOAD, and the two that loaded ran 70 tests — in a
 * developer's terminal, on a good day, and in no CI lane at all.
 *
 * ── THE ALIAS, AND THE TRADE IT BUYS ────────────────────────────────────────
 *
 * `plugin/index.ts` imports the HOST's `definePluginEntry` — a VALUE — and
 * calls it at module scope, so importing the plugin's entry point requires the
 * host's package to be present. The host's package is not published
 * (`@openclaw/plugin-sdk` is 404 on npm — measured 2026-10-22), so it cannot
 * be installed; the plugin's TYPES are already ambient
 * (`plugin/ambient.d.ts` declares the whole module), so only this one VALUE
 * import stands between the suite and the host.
 *
 * The lane therefore resolves that ONE specifier to a local stub
 * (`test/host-sdk.stub.ts`). **THE TRADE, RECORDED AND NOT BLURRED:** this
 * buys the shipped code not moving AT ALL — `index.ts` is byte-identical to
 * what the host loads — and it spends FIDELITY: the suite proves the plugin
 * against a STUB of the host's entry helper, not the host's. The residual is
 * named in the register row and is closed by the integration drill, which
 * drives the plugin's REAL client against the SHIPPED SIGNED binary.
 *
 * The alternative (injecting the helper into `index.ts` as a parameter) was
 * rejected on a MEASURED fact: the plugin is published into the host's
 * ecosystem and loaded by the HOST (`openclaw.extensions: ["./index.ts"]`),
 * so an injected seam would require the host's call site to move — in a
 * repository this one does not own, against a package that cannot be
 * installed here.
 */
export default defineConfig({
	resolve: {
		alias: {
			'openclaw/plugin-sdk/plugin-entry': fileURLToPath(
				new URL('./test/host-sdk.stub.ts', import.meta.url)
			)
		}
	},
	test: {
		/**
		 * The suite's files, NAMED — not left to vitest's default glob. The
		 * lane's pin reads this list and holds every suite file against it,
		 * so a suite file that no longer runs is a RED lane rather than a
		 * file that quietly stopped being collected.
		 */
		include: ['test/**/*.test.ts', 'src/**/*.test.ts', 'src/**/*.spec.ts']
	}
});
