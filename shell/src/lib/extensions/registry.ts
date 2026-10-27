/**
 * THE EXTENSION REGISTRY — the const array of first-party manifests the
 * bundler already sees (plan 26 §1). Tree-shaking, CSP and the audit surface
 * are unchanged BY CONSTRUCTION: no dynamic import, no eval, no remote
 * manifest — the structural pin scans `lib/extensions/**` for all three.
 *
 * Five nav contributors plus `coreSettings` (the settings dialog's four
 * sections). The frame renders the registry's arrays and owns none of the
 * surface literals anymore: NAV, the palette's navigate group, the chat
 * page's empty chips, the settings dialog's sections, and the status bar's
 * context chain all read from here — ONE source, the M3-a law.
 */

import { coreChat } from './core-chat.js';
import { coreMemory } from './core-memory.js';
import { coreReview } from './core-review.js';
import { coreAudit } from './core-audit.js';
import { coreGovernance } from './core-governance.js';
import { coreSettings } from './core-settings.js';
import { supportPack } from './support-pack.js';
import type {
	EmptyChip,
	ExtensionManifest,
	NavEntry,
	PaletteItem,
	SettingsSection,
	StatusContextMember
} from './types.js';
import type { KernelFacts } from '#lib/api/kernel-facts.js';

export const EXTENSIONS = [
	coreChat,
	coreMemory,
	coreReview,
	coreAudit,
	coreGovernance,
	coreSettings,
	supportPack
] as const;

export interface AssembledRegistry {
	readonly navEntries: readonly NavEntry[];
	readonly paletteItems: readonly PaletteItem[];
	readonly emptyChips: readonly EmptyChip[];
	readonly settingsSections: readonly SettingsSection[];
	readonly statusContextChain: readonly StatusContextMember[];
}

/**
 * THE SLOT-CLAIM LAW (dsh's discipline, kept): a nav entry is CLAIMED by
 * declaration, and a claim is exclusive — the same entry id or the same
 * route contributed twice is a registry REFUSAL, thrown here at module init.
 * The registry is static, so module init IS test time: a duplicate claim
 * reds every test that imports the frame, and the pin below drives this
 * same function with a fabricated duplicate for the readable red.
 */
export function assembleRegistry(extensions: readonly ExtensionManifest[]): AssembledRegistry {
	const claimedIds = new Set<string>();
	const claimedRoutes = new Set<string>();
	const navEntries: NavEntry[] = [];
	const paletteItems: PaletteItem[] = [];
	const emptyChips: EmptyChip[] = [];
	const settingsSections: SettingsSection[] = [];
	const statusContextChain: StatusContextMember[] = [];

	for (const extension of extensions) {
		const contributes = extension.contributes;
		for (const entry of contributes.navEntries ?? []) {
			if (claimedIds.has(entry.id)) {
				throw new Error(
					`the extension registry refuses a duplicate navEntries claim: "${extension.id}" re-claims entry id "${entry.id}"`
				);
			}
			if (claimedRoutes.has(entry.route)) {
				throw new Error(
					`the extension registry refuses a duplicate navEntries claim: "${extension.id}" re-claims route "${entry.route}"`
				);
			}
			claimedIds.add(entry.id);
			claimedRoutes.add(entry.route);
			navEntries.push(entry);
		}
		paletteItems.push(...(contributes.paletteItems ?? []));
		emptyChips.push(...(contributes.emptyChips ?? []));
		settingsSections.push(...(contributes.settingsSections ?? []));
		statusContextChain.push(...(contributes.statusContext ?? []));
	}

	navEntries.sort((a, b) => a.order - b.order);
	statusContextChain.sort((a, b) => a.order - b.order);
	return { navEntries, paletteItems, emptyChips, settingsSections, statusContextChain };
}

const assembled = assembleRegistry(EXTENSIONS);

/** The sidebar's entries, in `order` — the frame renders these verbatim. */
export const navEntries: readonly NavEntry[] = assembled.navEntries;

/** Extension palette items; the frame's navigate group renders them beside
 * the nav entries. (None contributed today.) */
export const paletteItems: readonly PaletteItem[] = assembled.paletteItems;

/** The chat empty state's chips, in contribution order. */
export const emptyChips: readonly EmptyChip[] = assembled.emptyChips;

/** The settings dialog's sections, in contribution order. */
export const settingsSections: readonly SettingsSection[] = assembled.settingsSections;

/** The status context chain, ordered — consumed only through
 * {@link resolveStatusContext}, so the frame and the pin drive ONE resolver. */
export const statusContextChain: readonly StatusContextMember[] = assembled.statusContextChain;

/** First non-null wins: the chain asked in order, `null` when no member has
 * a context to state. Pure — facts in, one string out. The frame and the
 * pin drive this ONE resolver (the pin passes its own members; the frame
 * passes the assembled chain). */
export function resolveStatusChain(
	members: readonly StatusContextMember[],
	facts: KernelFacts | null
): string | null {
	for (const member of members) {
		const value = member.derive(facts);
		if (value !== null) return value;
	}
	return null;
}

export function resolveStatusContext(facts: KernelFacts | null): string | null {
	return resolveStatusChain(statusContextChain, facts);
}
