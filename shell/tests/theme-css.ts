import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { shellRoot } from './fixtures';

/**
 * The theme round's CSS reader (M3-e, plan 24): the REAL `src/app.css`, read
 * as text and parsed through the test environment's own CSSOM. Shared by the
 * theme pins and the forced-dark a11y arm so both sweep the same sheet.
 *
 * What jsdom's engine does with this sheet — measured 2026-10-27, the
 * ground the theme pins stand on:
 *
 *   - UNLAYERED rules cascade and compute, attribute selectors included;
 *     `@layer` blocks parse but are never entered for cascade.
 *   - `@media (prefers-color-scheme: dark)` is never entered, so the dark
 *     scheme is read STRUCTURALLY from the parsed block, and a test that
 *     wants the dark values in effect applies them itself
 *     (`forcedDarkCss()`).
 *   - `--custom-property` declarations cascade out of stylesheet rules and
 *     inherit, so `getComputedStyle(...).getPropertyValue('--bg')` reads a
 *     scheme's actual value.
 */

export const APP_CSS = readFileSync(join(shellRoot, 'src', 'app.css'), 'utf8');

export type Tokens = Map<string, string>;

/** Every `--custom-property` a declaration block carries, name → value. */
export function customProperties(style: CSSStyleDeclaration): Tokens {
	const tokens: Tokens = new Map();
	for (let i = 0; i < style.length; i += 1) {
		const name = style[i];
		if (name !== undefined && name.startsWith('--')) {
			tokens.set(name, style.getPropertyValue(name).trim());
		}
	}
	return tokens;
}

export function isDarkMedia(rule: CSSRule): rule is CSSMediaRule {
	return (
		rule instanceof CSSMediaRule &&
		rule.media.mediaText.replace(/\s+/g, '').includes('prefers-color-scheme:dark')
	);
}

/**
 * The two scheme blocks: the light scheme is every TOP-LEVEL `:root {}`
 * rule; the dark scheme is every `:root {}` inside a TOP-LEVEL
 * `@media (prefers-color-scheme: dark)`. The platform flavours
 * (`:root[data-platform=…]`) and the nested macOS reduced-transparency
 * fallbacks are NOT scheme blocks — they are the platform layer.
 */
export function schemeBlocks(): { light: Tokens; dark: Tokens } {
	const style = document.createElement('style');
	style.textContent = APP_CSS;
	document.head.append(style);
	const sheet = style.sheet;
	if (!sheet) throw new Error('app.css did not parse into a stylesheet');
	const light: Tokens = new Map();
	const dark: Tokens = new Map();
	for (const rule of sheet.cssRules) {
		if (rule instanceof CSSStyleRule && rule.selectorText === ':root') {
			for (const [k, v] of customProperties(rule.style)) light.set(k, v);
		} else if (isDarkMedia(rule)) {
			for (const inner of rule.cssRules) {
				if (inner instanceof CSSStyleRule && inner.selectorText === ':root') {
					for (const [k, v] of customProperties(inner.style)) dark.set(k, v);
				}
			}
		}
	}
	style.remove();
	return { light, dark };
}

/**
 * The dark scheme's declarations as ONE unlayered `:root` block — the media
 * gate pried open, for a test that must sweep with the dark values in
 * effect under an engine that never enters `prefers-color-scheme` media.
 */
export function forcedDarkCss(): string {
	const { dark } = schemeBlocks();
	const declarations = [...dark].map(([name, value]) => `${name}: ${value};`).join(' ');
	return `:root { ${declarations} }`;
}
