/**
 * supportPack — the support vertical's Tier A manifest (plan 28 §4), as
 * adopted by C2 G-5: the ONE deliberate build act. The pack is signed
 * DATA; the shell sees its surface only because an operator translated
 * the pack's `ui/manifest.json` into THIS compiled module and added it to
 * the registry — there is no loader to change that, which is the point.
 * This file is the mechanism's proof in the only direction that exists:
 * data on disk, adopted by a commit.
 *
 * Translated from the support pack's `ui/manifest.json`
 * (support-pack@0.1.0, the C1 vertical's pack):
 *
 * ```json
 * {
 *   "id": "supportPack",
 *   "version": "0.1.0",
 *   "labelKeys": ["support.status.context"]
 * }
 * ```
 *
 * The contribution is deliberately minimal (READ FIRST ⑦'s choice,
 * recorded): NO nav entry — the route census is unchanged, because the
 * pack starts on surfaces that already exist (the chat spine IS the
 * support agent surface; the review queue IS its escalation path). What
 * ships is one statusContext member, the adopted voice in the bar's
 * fallback chain: when the kernel facts are live it can state the
 * vertical's posture; when the facts are absent it answers null — a
 * member with nothing to say says nothing, the chain's own law.
 */

import { m } from '$paraglide/messages.js';
import type { ExtensionManifest } from './types.js';

export const supportPack: ExtensionManifest = {
	id: 'supportPack',
	version: '0.1.0',
	contributes: {
		statusContext: [
			{
				order: 6,
				// the compiled paraglide m carries the catalogs' dotted keys
				// verbatim (string-named exports) — the same access form the
				// frame and every component use
				derive: (facts) => (facts?.version ? m['support.status.context']() : null)
			}
		]
	}
};
