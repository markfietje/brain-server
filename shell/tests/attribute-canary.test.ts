import { expect, test } from 'vitest';
import { attrIsHostile, sweepAttributesInTags, sweepSurvivingTag } from '../src/lib/sanitize';

// The attribute-grammar canary family: every spacing form around `=`
// judges as one joined `name=value`, so a dangerous scheme, a
// fetch-bearing style, or a click beacon dies spaced exactly as it dies
// tight. Unlike the server seam (which strips control whitespace a layer
// earlier), this twin has no such layer — its lookahead kills all five
// whitespace forms uniformly, and the family asserts exactly that.

const URL_ATTRS = ['href', 'src', 'action', 'formaction', 'xlink:href', 'poster', 'background'];
const ALL_ATTRS = [...URL_ATTRS, 'style', 'ping'];
const LIVE_WS = [' ', '\t', '\n'];
const ALL_WS = [' ', '\t', '\n', '\r', '\f'];

function hostileValue(attr: string): string {
	if (attr === 'style') return 'background:url(https://evil.example/s.png)';
	if (attr === 'ping') return 'https://evil.example/click';
	return 'javascript:alert(1)';
}

function hostileInput(attr: string, w: string): string {
	return `<a ${attr}${w}=${w}"${hostileValue(attr)}">x</a>`;
}

test('attribute_canary_spaced_equals_cannot_smuggle_a_hostile_attribute', () => {
	// Control: the tight form dies, so every spaced death below is
	// attributable to the spacing grammar and not a dead fixture.
	const tight = '<a href="javascript:alert(1)">x</a>';
	expect(sweepAttributesInTags(tight)).not.toContain('javascript:');

	const survived: string[] = [];
	for (const attr of ALL_ATTRS) {
		for (const w of ALL_WS) {
			const input = hostileInput(attr, w);
			const out = sweepAttributesInTags(input);
			if (out.includes('evil.example') || out.includes(attr)) {
				survived.push(`${attr} ws=${JSON.stringify(w)}: ${out}`);
			}
		}
		for (const input of [
			`<a ${attr}= "${hostileValue(attr)}">x</a>`,
			`<a ${attr} ="${hostileValue(attr)}">x</a>`
		]) {
			const out = sweepAttributesInTags(input);
			if (out.includes('evil.example') || out.includes(attr)) {
				survived.push(`${attr} half-form: ${out}`);
			}
		}
	}
	expect(survived).toEqual([]);
});

test('attribute_canary_spaced_benign_attributes_pass_through_verbatim', () => {
	for (const w of LIVE_WS) {
		const link = `<a href${w}=${w}"https://good.example/page">docs</a>`;
		expect(sweepAttributesInTags(link)).toBe(link);
		const calm = `<div style${w}=${w}"color:red">calm</div>`;
		expect(sweepAttributesInTags(calm)).toBe(calm);
		const sentence = `before <a href${w}=${w}"https://good.example/p">docs</a> after`;
		expect(sweepAttributesInTags(sentence)).toBe(sentence);
	}
	const mixed = '<a class="keep" href = "javascript:alert(1)" title="also keep">text</a>';
	const out = sweepAttributesInTags(mixed);
	expect(out).toContain('class="keep"');
	expect(out).toContain('title="also keep"');
	expect(out.toLowerCase()).not.toContain('javascript:');
	expect(out).toContain('>text</a>');
	// Idempotent: a second pass is the identity.
	expect(sweepAttributesInTags(out)).toBe(out);
});

test('attribute_canary_probe_units_judge_whole_tokens', () => {
	expect(attrIsHostile('href="javascript:alert(1)"')).toBe(true);
	expect(attrIsHostile('href')).toBe(false);
	expect(attrIsHostile('=')).toBe(false);
	expect(attrIsHostile('"javascript:alert(1)"')).toBe(false);
	expect(attrIsHostile('ping')).toBe(true);
	expect(attrIsHostile('style="color:red"')).toBe(false);
	expect(attrIsHostile('style="background:url(https://evil.example/x)"')).toBe(true);
	expect(sweepSurvivingTag('<a href="https://good.example/p" title="t">')).toBe(
		'<a href="https://good.example/p" title="t">'
	);
});
