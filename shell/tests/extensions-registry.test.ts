import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { shellRoot } from './fixtures';
import { CENSUS_ROUTES } from './route-census';
import {
	EXTENSIONS,
	assembleRegistry,
	emptyChips,
	navEntries,
	resolveStatusChain,
	settingsSections,
	statusContextChain
} from '../src/lib/extensions/registry.js';
import type { ExtensionManifest, StatusContextMember } from '../src/lib/extensions/types.js';

/**
 * M3-f — the Tier A extension registry's OWN police (plan 26 §1, §3).
 *
 * The registry is static — module init IS test time — so its laws are
 * enforced at load (the census guard in `route-census.ts`, the duplicate
 * refusal in `assembleRegistry`) and restated HERE as pins with readable
 * reds. The behavioral judges live where the surfaces live: the census
 * exact-set arms in `route-parity.test.ts`, the three-chip law and the
 * mount focus in `chat-home-state.test.ts`, the palette in
 * `command-palette.test.ts`, the settings dialog in the chrome pins, and
 * the status context's slot in the chrome + memory-surface pins.
 *
 * ## WHAT AN EXTENSION MAY NEVER GAIN
 *
 * A transport, a loader, a sandbox, a manifest parser, a market. The scan
 * below holds `lib/extensions/**` to the declarative shape forever: no
 * dynamic import, no eval, no remote anything.
 */

const extensionsDir = join(shellRoot, 'src', 'lib', 'extensions');

// ── the slot-claim law ────────────────────────────────────────────────────

test('the_registry_refuses_a_duplicate_naventries_claim_at_module_init', () => {
	const base: ExtensionManifest = {
		id: 'coreChat',
		version: '0.1.1',
		contributes: {
			navEntries: [
				{ id: 'chat', route: '/', labelKey: 'nav.chat', Icon: undefined as never, order: 1 }
			]
		}
	};
	// The same entry ID claimed by a second extension.
	const sameId: ExtensionManifest = {
		...base,
		id: 'coreMemory',
		contributes: {
			navEntries: [
				{ id: 'chat', route: '/recall', labelKey: 'nav.memory', Icon: undefined as never, order: 2 }
			]
		}
	};
	// The same ROUTE claimed by a second extension.
	const sameRoute: ExtensionManifest = {
		...base,
		id: 'coreMemory',
		contributes: {
			navEntries: [
				{ id: 'memory', route: '/', labelKey: 'nav.memory', Icon: undefined as never, order: 2 }
			]
		}
	};
	expect(() => assembleRegistry([base, sameId]), 'a re-claimed entry id is a refusal').toThrow(
		/refuses a duplicate navEntries claim.*"chat"/
	);
	expect(() => assembleRegistry([base, sameRoute]), 'a re-claimed route is a refusal').toThrow(
		/refuses a duplicate navEntries claim.*"\/"/
	);
	// And the real registry assembled: the law's proof that the check RUNS at
	// module init is that this file imported it at the top.
	expect(assembleRegistry([...EXTENSIONS]).navEntries.length).toBeGreaterThan(0);
});

// ── the census law, at the registry ───────────────────────────────────────

test('every_naventry_route_is_in_the_census', () => {
	// WHY THE GUARD LIVES HERE AND NOT IN `route-census.ts` (measured, not
	// assumed): the prompt's mechanism — the census module importing the
	// registry — pulled the registry's whole import graph (the settings
	// sections reach `#lib/api/client`, whose `createClient` runs at module
	// load) into files that stub `fetch` in their BODY, and route-a11y's
	// sweep bound the unstubbed fetch: all 13 render proofs failed. The
	// census module stays dependency-free; the census law at the registry is
	// held three ways instead — the `NavRoute` union (compiler), this pin
	// (readable red), and route-parity's registry-driven exact-set arms.
	expect(navEntries.length).toBeGreaterThan(0);
	for (const entry of navEntries) {
		expect(
			CENSUS_ROUTES as readonly string[],
			`the registry nav entry "${entry.id}" names "${entry.route}", which is not a censused route — ` +
				'a nav surface is a census surface or it does not ship'
		).toContain(entry.route);
	}
});

// ── the declarative shape, forever ────────────────────────────────────────

test('the_registry_is_declarative_no_dynamic_import_or_eval_or_remote_manifest', () => {
	// Comment prose is stripped first: the modules' doc comments NAME the
	// ban precisely to say why it exists (a comment does not execute — the
	// same rule chat-turn.test.ts records).
	const sources: { file: string; code: string }[] = [];
	const walk = (dir: string): void => {
		for (const entry of readdirSync(dir, { withFileTypes: true })) {
			const path = join(dir, entry.name);
			if (entry.isDirectory()) walk(path);
			else if (entry.name.endsWith('.ts')) {
				sources.push({
					file: path,
					code: readFileSync(path, 'utf8')
						.split('\n')
						.filter((line) => {
							const trimmed = line.trim();
							return (
								!trimmed.startsWith('//') &&
								!trimmed.startsWith('*') &&
								!trimmed.startsWith('/*')
							);
						})
						.join('\n')
				});
			}
		}
	};
	walk(extensionsDir);
	expect(sources.length, 'the registry modules were found').toBeGreaterThanOrEqual(7);
	for (const { file, code } of sources) {
		expect(code, `${file} must not load code dynamically`).not.toMatch(/import\s*\(/);
		expect(code, `${file} must not evaluate strings as code`).not.toMatch(/\beval\s*\(/);
		expect(code, `${file} must not name a remote manifest — the registry is built, never fetched`)
			.not.toMatch(/https?:\/\//);
	}
});

// ── the catalog law, at the registry ──────────────────────────────────────

test('every_labelkey_the_registry_renders_exists_in_all_five_catalogs', () => {
	const LOCALES = ['en', 'de', 'fr', 'es', 'nl'] as const;
	const catalogs = Object.fromEntries(
		LOCALES.map((locale) => [
			locale,
			new Set(
				Object.keys(
					JSON.parse(
						readFileSync(join(shellRoot, 'src', 'lib', 'i18n', `${locale}.json`), 'utf8')
					) as Record<string, string>
				)
			)
		])
	);
	const labelKeys = [
		...navEntries.map((entry) => entry.labelKey),
		...emptyChips.map((chip) => chip.labelKey)
	];
	expect(labelKeys.length).toBeGreaterThanOrEqual(8);
	for (const key of labelKeys) {
		for (const locale of LOCALES) {
			expect(
				catalogs[locale]!.has(key),
				`the registry renders "${key}", which the ${locale} catalog does not carry — ` +
					'an extension cannot ship half-translated'
			).toBe(true);
		}
	}
});

// ── the three-chip law, at the registry ───────────────────────────────────

test('the_registry_contributes_exactly_the_three_acting_chips', () => {
	// The BEHAVIORAL judge is chat-home-state.test.ts (the page renders these
	// and the pins click them); this is the structural half — the exact set,
	// so an added or removed chip reds here by name before any mount.
	expect(emptyChips.map((chip) => chip.id)).toEqual(['ask', 'trace', 'review']);
	for (const chip of emptyChips) {
		expect(typeof chip.run, `the "${chip.id}" chip must carry its own act`).toBe('function');
	}
});

// ── the settings dialog's section order ───────────────────────────────────

test('the_settings_dialog_renders_the_registry_s_sections_in_contribution_order', () => {
	expect(settingsSections.map((section) => section.id)).toEqual([
		'origin',
		'locale',
		'token',
		'update'
	]);
});

// ── the status context chain ──────────────────────────────────────────────

const FACTS = { version: '1.29.34', count: 3, model: 'on-device' };

test('the_status_context_chain_is_facts_in_string_or_null_out_first_non_null_wins', () => {
	const member = (order: number, value: string | null): StatusContextMember => ({
		order,
		derive: () => value
	});
	// First non-null wins…
	expect(
		resolveStatusChain([member(1, 'the first speaks'), member(2, 'the second is never asked')], FACTS)
	).toBe('the first speaks');
	// …a null member is skipped, never rendered as an empty slot…
	expect(resolveStatusChain([member(1, null), member(2, 'the second speaks')], FACTS)).toBe(
		'the second speaks'
	);
	// …an all-null chain states nothing (the bar never guesses)…
	expect(resolveStatusChain([member(1, null), member(2, null)], FACTS)).toBeNull();
	expect(resolveStatusChain([], FACTS)).toBeNull();
	// …and the chain is ORDERED, not arrival-ordered: assemble sorts by the
	// members' own order across extensions.
	const assembled = assembleRegistry([
		{
			id: 'coreChat',
			version: '0.1.1',
			contributes: { statusContext: [member(9, 'the late order never jumps the queue')] }
		},
		{
			id: 'coreMemory',
			version: '0.1.1',
			contributes: { statusContext: [member(2, 'the low order speaks first')] }
		}
	]);
	expect(resolveStatusChain(assembled.statusContextChain, FACTS)).toBe(
		'the low order speaks first'
	);
	// The real chain is live: its first member bridges the frame store, and
	// the frame renders through the same resolver the memory-surface pins
	// drive.
	expect(statusContextChain.length).toBeGreaterThanOrEqual(1);
});
