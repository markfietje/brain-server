/**
 * The canonical invisible-Unicode boundary (D9). Renderer-facing text passes
 * through this pure, idempotent removal step before it can be put on screen.
 * This is deliberately not an HTML sanitizer: it neither parses nor emits
 * markup, and the existing `{@html}` lint ban remains the markup boundary.
 */

// This expression is the exact scalar membership set in
// plugin/fixtures/invisible-classes.json and src/strip_invisible.rs. The `u`
// flag makes the supplementary ranges operate on code points rather than
// UTF-16 code units.
const INVISIBLE =
	/[\u00AD\u034F\u061C\u115F-\u1160\u180E\u200B-\u200F\u202A-\u202E\u2060-\u2063\u2066-\u2069\uFE00-\uFE0F\uFEFF\uFFF9-\uFFFB\u{E0000}-\u{E007F}\u{E0100}-\u{E01EF}]/gu;

export function stripInvisible(text: string): string {
	return text.replace(INVISIBLE, '');
}

/** The one door rendered pack text passes through. */
export function sanitizeTemplateText(text: string): string {
	return stripInvisible(text);
}

// ── the attribute tier (grammar twin of the server read seam) ─────────────
// NOT a sanitizer: this mirror covers the ATTRIBUTE grammar only — the same
// tokenizer, the same `=` lookahead, the same hostility probe. Element
// stripping stays server-side, so a swept hostile element opener is still a
// hostile element opener (only its attributes are judged here). Rendered
// shell content passes through the server seam first; this twin exists so
// the shell's own composition never re-opens the spacing grammar the seam
// closed (whitespace around `=` must not smuggle an attribute past either
// tree). The one asymmetry with the server, stated: the server strips
// control whitespace a layer earlier (so `\r`/`\x0c`-spaced forms die
// there); this twin has no such layer, so its lookahead kills all five
// whitespace forms uniformly.

const URL_ATTRIBUTES = new Set([
	'href',
	'src',
	'action',
	'formaction',
	'xlink:href',
	'poster',
	'background'
]);

function isAsciiWhitespace(ch: string): boolean {
	return ch === ' ' || ch === '\t' || ch === '\n' || ch === '\f' || ch === '\r';
}

function isWsOrControl(ch: string): boolean {
	const code = ch.codePointAt(0) ?? 0;
	return /\s/.test(ch) || (code < 0x20) || (code >= 0x7f && code <= 0x9f);
}

function trimWsControlQuotes(value: string): string {
	let start = 0;
	let end = value.length;
	while (start < end && (isWsOrControl(value[start] ?? '') || value[start] === '"' || value[start] === "'")) {
		start += 1;
	}
	while (end > start && (isWsOrControl(value[end - 1] ?? '') || value[end - 1] === '"' || value[end - 1] === "'")) {
		end -= 1;
	}
	return value.slice(start, end);
}

const NAMED_ENTITIES: Array<[string, string]> = [
	['colon', ':'],
	['tab', '\t'],
	['newline', '\n']
];

function decodeEntitiesOnce(s: string): string {
	let out = '';
	let i = 0;
	while (i < s.length) {
		if (s[i] !== '&') {
			out += s[i];
			i += 1;
			continue;
		}
		const rest = s.slice(i + 1);
		if (rest.startsWith('#')) {
			const hex = rest[1] === 'x' || rest[1] === 'X';
			const start = hex ? 2 : 1;
			const maxDigits = hex ? 6 : 7;
			let end = start;
			while (
				end < rest.length &&
				end - start < maxDigits &&
				/[0-9]/.test(rest[end] ?? '') &&
				(!hex || /[0-9a-fA-F]/.test(rest[end] ?? ''))
			) {
				end += 1;
			}
			if (end > start && rest[end] === ';') {
				const code = Number.parseInt(rest.slice(start, end), hex ? 16 : 10);
				const ch = Number.isSafeInteger(code) ? String.fromCodePoint(code) : null;
				out += ch ?? s.slice(i, i + end + 2);
				i += end + 2;
				continue;
			}
			out += '&';
			i += 1;
			continue;
		}
		let matched = false;
		for (const [name, ch] of NAMED_ENTITIES) {
			if (rest.length >= name.length + 1 && rest.slice(0, name.length).toLowerCase() === name && rest[name.length] === ';') {
				out += ch;
				i += 1 + name.length + 1;
				matched = true;
				break;
			}
		}
		if (!matched) {
			out += '&';
			i += 1;
		}
	}
	return out;
}

function stripCssComments(s: string): string {
	let out = '';
	let i = 0;
	while (i < s.length) {
		if (s[i] === '/' && s[i + 1] === '*') {
			const close = s.indexOf('*/', i + 2);
			if (close === -1) return out;
			i = close + 2;
			continue;
		}
		out += s[i];
		i += 1;
	}
	return out;
}

function decodeCssEscapesOnce(s: string): string {
	let out = '';
	let i = 0;
	while (i < s.length) {
		if (s[i] !== '\\') {
			out += s[i];
			i += 1;
			continue;
		}
		const rest = s.slice(i + 1);
		let hexEnd = 0;
		while (hexEnd < rest.length && hexEnd < 6 && /[0-9a-fA-F]/.test(rest[hexEnd] ?? '')) {
			hexEnd += 1;
		}
		if (hexEnd > 0) {
			const code = Number.parseInt(rest.slice(0, hexEnd), 16);
			const ch = Number.isSafeInteger(code) ? String.fromCodePoint(code) : null;
			if (ch !== null) out += ch;
			let after = hexEnd;
			if (rest[after] === ' ') after += 1;
			else if (rest.startsWith('\r\n', after)) after += 2;
			i += 1 + after;
		} else {
			out += rest[0] ?? '\\';
			i += 1 + (rest[0] ?? '').length;
		}
	}
	return out;
}

function stripWsControl(s: string): string {
	return [...s].filter((ch) => !isWsOrControl(ch)).join('');
}

/** Trim, one entity-decode pass, browser whitespace removal, prefix match. */
function schemeIsDangerous(value: string): boolean {
	const lowered = stripWsControl(decodeEntitiesOnce(trimWsControlQuotes(value))).toLowerCase();
	return (
		lowered.startsWith('javascript:') ||
		lowered.startsWith('vbscript:') ||
		lowered.startsWith('data:')
	);
}

/** A `style=` value expresses a network fetch through `url(` / `image-set(`. */
function cssValueFetches(value: string): boolean {
	const unquoted = trimWsControlQuotes(value);
	const decoded = decodeEntitiesOnce(unquoted);
	const noComments = stripCssComments(decoded);
	const unescaped = decodeCssEscapesOnce(noComments);
	const lowered = stripWsControl(unescaped).toLowerCase();
	return lowered.includes('url(') || lowered.includes('image-set(');
}

/**
 * Hostility decision for ONE attribute token. Spaced forms arrive REJOINED
 * by the sweep's `=` lookahead, so this function always sees the whole
 * `name=value` and never a bare fragment.
 */
export function attrIsHostile(token: string): boolean {
	const eq = token.indexOf('=');
	const name = eq === -1 ? token : token.slice(0, eq);
	const value = eq === -1 ? null : token.slice(eq + 1);
	const lower = name.toLowerCase();
	if (lower.length > 2 && lower.startsWith('on') && /^[a-z]+$/.test(lower.slice(2))) {
		return true;
	}
	if (lower === 'ping') return true;
	if (value !== null && URL_ATTRIBUTES.has(lower)) return schemeIsDangerous(value);
	if (lower === 'style' && value !== null) return cssValueFetches(value);
	return false;
}

function scanTagEnd(html: string, from: number): number | null {
	let quote: string | null = null;
	for (let i = from; i < html.length; i += 1) {
		const ch = html[i] ?? '';
		if (quote !== null) {
			if (ch === quote) quote = null;
		} else if (ch === '"' || ch === "'") {
			quote = ch;
		} else if (ch === '>') {
			return i + 1;
		}
	}
	return null;
}

/**
 * Delete-only attribute sweep over ONE surviving tag (both brackets). The
 * `=` lookahead rejoins `name = value` spacings before the hostility
 * probe; kept tokens emit verbatim, so an all-clean tag is byte-identical.
 */
export function sweepSurvivingTag(tag: string): string {
	const nameMatch = /^<([A-Za-z0-9]+)/.exec(tag);
	if (nameMatch === null) return tag;
	const nameLen = 1 + (nameMatch[1]?.length ?? 0);
	const rest = tag.slice(nameLen, tag.endsWith('>') ? -1 : undefined);
	const spans: Array<[number, number]> = [];
	{
		let j = 0;
		while (j < rest.length) {
			if (isAsciiWhitespace(rest[j] ?? '')) {
				j += 1;
				continue;
			}
			const start = j;
			let quote: string | null = null;
			while (j < rest.length) {
				const ch = rest[j] ?? '';
				if (quote !== null) {
					if (ch === quote) quote = null;
				} else if (ch === '"' || ch === "'") {
					quote = ch;
				} else if (isAsciiWhitespace(ch)) {
					break;
				}
				j += 1;
			}
			spans.push([start, j]);
		}
	}
	const toks = spans.map(([s, e]) => rest.slice(s, e));
	const drop = new Array<boolean>(toks.length).fill(false);
	for (let i = 0; i < toks.length; i += 1) {
		if (drop[i] === true) continue;
		const tok = toks[i] ?? '';
		let joined: { candidate: string; unit: number[] } | null = null;
		// RED-PROOF MUTANT (reverted before ship): lookahead disabled.
		if (tok === '=') {
			const prev = i > 0 ? (toks[i - 1] ?? '') : null;
			const next = toks[i + 1] ?? null;
			if (prev !== null && next !== null && !prev.includes('=')) {
				joined = { candidate: `${prev}=${next}`, unit: [i - 1, i, i + 1] };
			}
		} else if (tok.startsWith('=')) {
			const prev = i > 0 ? (toks[i - 1] ?? '') : null;
			if (prev !== null && tok.length > 1 && !prev.includes('=')) {
				joined = { candidate: `${prev}${tok}`, unit: [i - 1, i] };
			}
		} else if (tok.endsWith('=') && tok.length > 1) {
			const next = toks[i + 1] ?? null;
			if (next !== null) {
				joined = { candidate: `${tok}${next}`, unit: [i, i + 1] };
			}
		}
		if (joined !== null) {
			if (attrIsHostile(joined.candidate)) {
				for (const k of joined.unit) drop[k] = true;
			}
		} else if (attrIsHostile(tok)) {
			drop[i] = true;
		}
	}
	let out = tag.slice(0, nameLen);
	let j = 0;
	spans.forEach(([s, e], k) => {
		out += rest.slice(j, s);
		if (drop[k] !== true) out += rest.slice(s, e);
		j = e;
	});
	out += rest.slice(j);
	out += '>';
	return out;
}

/**
 * Sweep the attributes of every well-formed `<name …>` region in a string,
 * leaving text and malformed markup byte-identical.
 */
export function sweepAttributesInTags(html: string): string {
	let out = '';
	let i = 0;
	while (i < html.length) {
		const open = html.indexOf('<', i);
		if (open === -1) {
			out += html.slice(i);
			break;
		}
		const after = html[open + 1] ?? '';
		if (!/[A-Za-z]/.test(after)) {
			out += html.slice(i, open + 1);
			i = open + 1;
			continue;
		}
		const end = scanTagEnd(html, open + 1);
		if (end === null) {
			out += html.slice(i);
			break;
		}
		out += html.slice(i, open);
		out += sweepSurvivingTag(html.slice(open, end));
		i = end;
	}
	return out;
}
