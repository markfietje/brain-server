import { readFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';

/**
 * D9 — the five catalogs (en/de/fr/es/nl, the Dioxus parity set) must
 * carry EXACTLY the same key sets: a missing key in ANY locale is a red
 * test, not a runtime fallback surprise.
 */

const i18nDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'src', 'lib', 'i18n');
const LOCALES = ['en', 'de', 'fr', 'es', 'nl'] as const;

function keySet(locale: string): string[] {
	const raw = JSON.parse(readFileSync(join(i18nDir, `${locale}.json`), 'utf8')) as Record<
		string,
		unknown
	>;
	return Object.keys(raw).sort();
}

test('i18n_catalog_keys_are_parity_complete_en_de_fr_es_nl', () => {
	const files = readdirSync(i18nDir).filter((f) => f.endsWith('.json')).sort();
	expect(files).toEqual(LOCALES.map((l) => `${l}.json`).sort());
	const enKeys = keySet('en');
	expect(enKeys.length).toBeGreaterThan(5);
	for (const locale of LOCALES.slice(1)) {
		expect(keySet(locale), `locale ${locale} diverges from en`).toEqual(enKeys);
	}
	// And no catalog carries an EMPTY string (a hole in a translation).
	for (const locale of LOCALES) {
		const raw = JSON.parse(readFileSync(join(i18nDir, `${locale}.json`), 'utf8')) as Record<
			string,
			string
		>;
		for (const [key, value] of Object.entries(raw)) {
			expect(typeof value === 'string' && value.length > 0, `${locale}:${key} empty`).toBe(true);
		}
	}
});
