#!/usr/bin/env node
/**
 * Injects the production Content-Security-Policy meta (D6) into every built
 * HTML file. The directives stay strict ('self' + the pinned loopback kernel
 * origin) AND SvelteKit's inline bootstrap scripts are allowed the precise
 * way: each inline script's sha256 is computed HERE, at build time, and
 * added to script-src as 'sha256-…' — never 'unsafe-inline'. Dev serves no
 * CSP at all (the inline bootstrap + the HMR websocket are dev-only).
 */
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const root = join(process.cwd(), 'build');

function htmlFiles(dir) {
	const out = [];
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		const p = join(dir, entry.name);
		if (entry.isDirectory()) out.push(...htmlFiles(p));
		else if (entry.name.endsWith('.html')) out.push(p);
	}
	return out;
}

const CSP_META_RE = /<meta http-equiv="Content-Security-Policy"[^>]*>/;
const INLINE_SCRIPT_RE = /<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g;

for (const file of htmlFiles(root)) {
	const html = readFileSync(file, 'utf8');
	const hashes = new Set();
	for (const m of html.matchAll(INLINE_SCRIPT_RE)) {
		if (m[1].trim().length > 0) {
			hashes.add(
				`'sha256-${createHash('sha256').update(m[1], 'utf8').digest('base64')}'`
			);
		}
	}
	const scriptSrc = ["'self'", ...hashes].join(' ');
	const directives = [
		"default-src 'self'",
		`script-src ${scriptSrc}`,
		"style-src 'self'",
		"img-src 'self' data:",
		"connect-src 'self' http://127.0.0.1:8765",
		"object-src 'none'",
		"base-uri 'self'",
		"frame-ancestors 'none'"
	].join('; ');
	const meta = `<meta http-equiv="Content-Security-Policy" content="${directives}">`;
	const next = CSP_META_RE.test(html)
		? html.replace(CSP_META_RE, meta)
		: html.replace('</head>', `${meta}\n\t</head>`);
	writeFileSync(file, next);
	console.log(`[inject-csp] ${file}: script-src ${scriptSrc}`);
}
