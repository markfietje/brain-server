/**
 * D7 — the bearer token's ONLY home. The token NEVER enters the Vite
 * bundle: no `import.meta.env` read exists here (VITE_* variables are
 * compile-time-exposed to the bundle and are therefore FORBIDDEN for the
 * token). It is supplied at RUNTIME — via the Tauri command (`kernel_token`,
 * which reads the operator's own environment in the Rust core) or, in the
 * PWA build, by the operator's serving origin calling `setBearer` from its
 * own bootstrap script. It is held in module memory only: never logged,
 * never rendered, never persisted (keyring storage is the recorded M2
 * deferral). The redaction test pins all of this.
 */

let bearer: string | null = null;

/** Supply the token at runtime (Tauri bootstrap or the operator's serving
 * origin). Passing null clears it. */
export function setBearer(token: string | null): void {
	bearer = typeof token === 'string' && token.length > 0 ? token : null;
}

export function hasBearer(): boolean {
	return bearer !== null;
}

/** The Authorization headers for the typed client's middleware — empty
 * when no token is held (the loopback posture needs none). */
export function authHeaders(): Record<string, string> {
	return bearer === null ? {} : { Authorization: `Bearer ${bearer}` };
}

/** True when the given sink text would leak the token — used by the
 * redaction test (and by any future log sink's own assertion). */
export function leaksToken(sink: string): boolean {
	return bearer !== null && bearer.length > 0 && sink.includes(bearer);
}
