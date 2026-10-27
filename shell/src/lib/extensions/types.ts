/**
 * THE EXTENSION CONTRACT — Tier A (plan 26 §1, as executed by M3-f).
 *
 * An extension is a FIRST-PARTY, compile-time, declarative contributor of
 * shell SURFACE: nav entries, palette items, empty-state chips, settings
 * sections, and status-context members. The registry (`./registry.js`) is a
 * const array of these manifests the bundler already sees — there is no
 * loader, no sandbox, no manifest parser, no runtime installation, and no
 * dynamic import: a structural pin scans `lib/extensions/**` for all three,
 * forever.
 *
 * What an extension deliberately does NOT gain (plan 22 R6, narrowed by this
 * round as a NAMED ACT): a transport. "The shell is a pure protocol client"
 * stands in the DATA sense — every fact still arrives through the typed wire
 * — and an extension renders FACTS; it never decides what a fact means. What
 * the amendment buys is that "extensions" now names first-party SURFACE
 * contributions, not only kernel clients.
 *
 * This file is THE contract. Nothing else is an extension surface.
 */

import type { Component } from 'svelte';
import type * as messages from '$paraglide/messages.js';
import type { KernelFacts } from '#lib/api/kernel-facts.js';

/**
 * A catalog key the registry renders as a LABEL: the compiler's own
 * guarantee that the key exists AND takes no inputs (a registry label is
 * rendered bare — `m[labelKey]()` — so only the zero-input messages qualify;
 * a message needing `{ version }` is a fact-format, not a label). The
 * five-catalog parity pin is what makes the key exist in ALL five. The
 * barrel also re-exports itself as `m`; that name is not a message.
 */
export type MessageKey = {
	[K in Exclude<keyof typeof messages, 'm'>]: (typeof messages)[K] extends () => string
		? K
		: never;
}[Exclude<keyof typeof messages, 'm'>];

/**
 * The first-party surface set, compiler-enumerated. A new extension is a new
 * member here — the union is the audit surface: the registry cannot carry a
 * manifest the compiler did not enumerate.
 */
export type ExtensionId =
	| 'coreChat'
	| 'coreMemory'
	| 'coreReview'
	| 'coreAudit'
	| 'coreGovernance'
	| 'coreSettings'
	/** The support vertical's adopted pack manifest (C2 G-5): the ONE
	 * translation of a pack's ui/manifest.json into the compiled registry —
	 * the mechanism's proof, deliberately minimal (no nav entry). */
	| 'supportPack';

/**
 * A route a nav entry may name: the censused NAV surfaces. The census law
 * stays whole — `route-census.ts` carries the full eight-route set and a
 * module-load guard that refuses any nav route outside it — and this union
 * names the subset the sidebar lists. `/decisions` is deliberately absent:
 * the decision-run listing is an operator surface reached from the palette
 * and from Governance, not a daily destination.
 */
export type NavRoute = '/' | '/recall' | '/review' | '/audit' | '/governance';

/** One sidebar entry. The sidebar renders `{#each navEntries}` in `order`. */
export interface NavEntry {
	readonly id: string;
	readonly route: NavRoute;
	readonly labelKey: MessageKey;
	readonly Icon: Component;
	readonly order: number;
}

/** A command-palette item. The frame's `more` group keeps its own Settings
 * entry (it opens frame state, not a route); an extension's items join the
 * navigate group beside the nav entries. */
export interface PaletteItem {
	readonly group: 'navigate' | 'more';
	readonly labelKey: MessageKey;
	readonly run: () => void;
}

/**
 * One empty-state chip (the M3-b three-chip law). The `run` closures are
 * created at module init and capture NO page state — the ask chip focuses
 * the composer by its element id (the recorded jsdom discipline: the
 * component-bound ref is not populated under jsdom), the navigators go
 * through `$app/navigation` — so the page renders `{#each emptyChips}`
 * verbatim and a chip can never silently act on stale state.
 */
export interface EmptyChip {
	readonly id: string;
	readonly labelKey: MessageKey;
	readonly run: () => void;
}

/** One settings-dialog section, rendered in contribution order. */
export interface SettingsSection {
	readonly id: string;
	readonly component: Component;
}

/**
 * One member of the status bar's context chain: pure, facts in,
 * `string | null` out, first non-null wins by `order`. The facts are null
 * before the frame's first read — a member that derives from them narrows
 * and answers null; a member that reads the frame's own store does not care.
 * The frame never guesses a context sentence; a member with nothing to say
 * returns null.
 */
export interface StatusContextMember {
	readonly order: number;
	readonly derive: (facts: KernelFacts | null) => string | null;
}

/** THE manifest. `contributes` is the whole surface an extension may add. */
export interface ExtensionManifest {
	readonly id: ExtensionId;
	readonly version: string;
	readonly contributes: {
		/** THE claimed slot: nav entries may be contributed by EXACTLY ONE
		 * extension per id AND per route — the registry refuses a duplicate
		 * claim at module init (a thrown Error; the registry is static, so
		 * module init IS test time). */
		readonly navEntries?: readonly NavEntry[];
		readonly paletteItems?: readonly PaletteItem[];
		readonly emptyChips?: readonly EmptyChip[];
		readonly settingsSections?: readonly SettingsSection[];
		readonly statusContext?: readonly StatusContextMember[];
	};
}
