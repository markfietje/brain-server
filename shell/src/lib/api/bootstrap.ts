/**
 * The D7 runtime bootstrap: when running inside Tauri, ask the Rust core
 * for the operator-supplied token (`kernel_token` reads BRAIN_SHELL_TOKEN
 * from the environment — system integration, not business logic) and hand
 * it to the token module. In the PWA build (no Tauri runtime) this is a
 * no-op: the operator's serving origin calls setBearer itself (documented
 * in the README). The token NEVER transits import.meta.env (D7).
 */
import { setBearer } from './token';

export async function bootstrapToken(): Promise<void> {
	// withGlobalTauri is FALSE (D6) — detect the runtime via the injected
	// internals object instead of a global API handle.
	if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
		return;
	}
	try {
		const { invoke } = await import('@tauri-apps/api/core');
		const token = await invoke<string | null>('kernel_token');
		setBearer(token);
	} catch {
		// No runtime, no token: the loopback posture needs none. Never an
		// error surface — and never a log of ANY token material.
		setBearer(null);
	}
}
