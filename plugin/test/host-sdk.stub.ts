import type { PluginEntry } from 'openclaw/plugin-sdk/plugin-entry';

/**
 * THE TEST-LANE STUB for the host's `definePluginEntry` (III-8 · D2 shape (b)).
 *
 * ── WHAT IT STANDS IN FOR ───────────────────────────────────────────────────
 *
 * The host's OpenClaw plugin SDK exports:
 *
 *   definePluginEntry(entry: PluginEntry): PluginEntry
 *
 * — the declaration the plugin's own `plugin/ambient.d.ts` carries, copied
 * from the real SDK (2026.6.x). The host's package cannot be installed here
 * (`@openclaw/plugin-sdk` is 404 on npm — measured 2026-10-22), and the ONE
 * thing the plugin needs from it at runtime is this helper.
 *
 * ── THE STUB'S CONTRACT IS THE HOST'S, AND THE DIFFERENCE IS NAMED ────────
 *
 * The declared type says the helper is an IDENTITY: an entry in, the same
 * entry out. This stub is that identity, PLUS a check that the entry carries
 * the five fields the declared `PluginEntry` names (`id`, `name`,
 * `description`, `configSchema`, `register`).
 *
 * **THE DIFFERENCE, STATED PLAINLY SO NOBODY CONFUSES THE TWO:** the host's
 * helper's exact failure mode is NOT KNOWN HERE — it cannot be run from this
 * repository — so this stub is STRICTLY STRICTER than the host is known to
 * be. That direction is deliberate: a stricter stand-in can refuse an entry
 * the host would accept (a false red in this lane), while a laxer one would
 * accept an entry the host REFUSES (a false green in this lane, which is the
 * direction that ships broken code). The plugin's own entry carries all five
 * fields, so the check does not fire today; it exists to catch the plugin's
 * own entry drifting away from the contract it declares.
 *
 * The residual — the suite proving the plugin against a STUB rather than
 * against the REAL host — is named in the gate-law register's row and is
 * closed by the integration drill, which drives the plugin's REAL client
 * against the SHIPPED SIGNED binary.
 */
export function definePluginEntry(entry: PluginEntry): PluginEntry {
	if (typeof entry !== 'object' || entry === null) {
		throw new TypeError('definePluginEntry: the plugin entry must be an object');
	}
	for (const field of ['id', 'name', 'description'] as const) {
		if (typeof entry[field] !== 'string' || entry[field].length === 0) {
			throw new TypeError(
				`definePluginEntry: the plugin entry's ${field} must be a non-empty string`
			);
		}
	}
	if (typeof entry.configSchema === 'undefined') {
		throw new TypeError('definePluginEntry: the plugin entry must carry a configSchema');
	}
	if (typeof entry.register !== 'function') {
		throw new TypeError('definePluginEntry: the plugin entry must carry a register function');
	}
	return entry;
}
