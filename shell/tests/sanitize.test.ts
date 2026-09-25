import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'vitest';
import { sanitizeTemplateText, stripInvisible } from '../src/lib/sanitize';
import { kernelRoot } from './fixtures';

type InvisibleFixture = {
	classes: Array<{ ranges: Array<[string, string]> }>;
	'visible-samples': string[];
};

const fixture = JSON.parse(
	readFileSync(join(kernelRoot, 'plugin/fixtures/invisible-classes.json'), 'utf8')
) as InvisibleFixture;
const canonicalScalars = new Set<number>();
for (const fixtureClass of fixture.classes) {
	for (const [startText, endText] of fixtureClass.ranges) {
		const start = Number.parseInt(startText, 16);
		const end = Number.parseInt(endText, 16);
		for (let scalar = start; scalar <= end; scalar += 1) {
			canonicalScalars.add(scalar);
		}
	}
}

const visibleSamples = fixture['visible-samples'].map((sample) =>
	String.fromCodePoint(Number.parseInt(sample, 16))
);
const explicitNoncanonicalNegatives = [
	0x2064,
	...Array.from({ length: 6 }, (_, index) => 0x206a + index)
];

test('shell_invisible_set_matches_canonical_fixture_exactly', () => {
	const mismatches: string[] = [];
	let scalarCount = 0;

	for (let scalar = 0; scalar <= 0x10ffff; scalar += 1) {
		if (scalar >= 0xd800 && scalar <= 0xdfff) continue;
		scalarCount += 1;
		const text = String.fromCodePoint(scalar);
		const expected = canonicalScalars.has(scalar) ? '' : text;
		const stripped = stripInvisible(text);
		const templateText = sanitizeTemplateText(text);
		if (stripped !== expected || templateText !== expected) {
			if (mismatches.length < 8) {
				mismatches.push(
					`U+${scalar.toString(16).toUpperCase().padStart(4, '0')}: ` +
						`strip=${JSON.stringify(stripped)} template=${JSON.stringify(templateText)} ` +
						`expected=${JSON.stringify(expected)}`
				);
			}
		}
	}

	expect(scalarCount).toBe(0x110000 - 0x800);
	expect(mismatches).toEqual([]);
});

test('shell_invisible_set_preserves_visible_samples_and_is_idempotent', () => {
	for (const sample of visibleSamples) {
		expect(stripInvisible(sample)).toBe(sample);
		expect(sanitizeTemplateText(sample)).toBe(sample);
	}
	for (const scalar of explicitNoncanonicalNegatives) {
		const text = String.fromCodePoint(scalar);
		expect(stripInvisible(text)).toBe(text);
		expect(sanitizeTemplateText(text)).toBe(text);
	}

	const corpus = [
		'',
		'plain text',
		'plain\ttext\nwith whitespace',
		...visibleSamples,
		...Array.from(canonicalScalars, (scalar) => String.fromCodePoint(scalar)),
		...explicitNoncanonicalNegatives.map((scalar) => String.fromCodePoint(scalar))
	];
	for (const input of corpus) {
		const original = input;
		const once = stripInvisible(input);
		expect(stripInvisible(once)).toBe(once);
		expect(sanitizeTemplateText(input)).toBe(once);
		expect(input).toBe(original);
	}
});
