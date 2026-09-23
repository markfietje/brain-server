/**
 * D5 — the typed wire is the ONLY wire. The client is generated from the
 * kernel's openapi.yaml (`pnpm gen:api` → schema.d.ts, drift-gated by the
 * named test); every call is typed against THAT contract — no hand-written
 * endpoint shapes exist anywhere in the shell.
 *
 * Base URL: the kernel origin, default the pinned 127.0.0.1:8765.
 * Overridable, but LOOPBACK-ONLY is enforced client-side: a non-loopback
 * override silently falls back to the default (fail-safe, not fail-open).
 */

import createClient, { type Middleware } from 'openapi-fetch';
import type { paths } from './schema';
import { authHeaders } from './token';

const DEFAULT_BASE = 'http://127.0.0.1:8765';

function isLoopbackUrl(raw: string): boolean {
	try {
		const url = new URL(raw);
		if (url.protocol !== 'http:' && url.protocol !== 'https:') return false;
		const host = url.hostname;
		return host === '127.0.0.1' || host === 'localhost' || host === '[::1]' || host === '::1';
	} catch {
		return false;
	}
}

function resolveBase(): string {
	const override = import.meta.env['VITE_BRAIN_API_BASE'];
	if (typeof override === 'string' && override.length > 0 && isLoopbackUrl(override)) {
		return override.replace(/\/$/, '');
	}
	return DEFAULT_BASE;
}

export const apiBase = resolveBase();

/** Injects the runtime-held bearer (D7) — the token lives in token.ts and
 * nowhere else. */
const bearerMiddleware: Middleware = {
	onRequest({ request }) {
		for (const [name, value] of Object.entries(authHeaders())) {
			request.headers.set(name, value);
		}
		return request;
	}
};

export const client = createClient<paths>({ baseUrl: apiBase });
// Middleware registers imperatively in this openapi-fetch line (0.17).
client.use(bearerMiddleware);

/** One typed catalog read — the renderer's entire kernel surface this
 * round (D3: no intake submission, no telemetry write, no answer RPC). */
export async function fetchWizardPacks(): Promise<{
	packs: Array<{ id: string; question_count: number; pack: unknown }>;
	count: number;
}> {
	const { data, error } = await client.GET('/workflow/wizard/packs');
	if (error !== undefined || data === undefined) {
		throw new Error('wire_error');
	}
	return data;
}
