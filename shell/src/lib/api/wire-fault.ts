/**
 * III-4 — a wire failure is NAMED, never guessed.
 *
 * The renderer cannot tell a blocked response from a rejected bearer from a
 * dead port: the browser engine reports all three as one network failure.
 * The Rust core asks the kernel directly what the webview would experience
 * (`kernel_wire_status` → `unreachable` / `origin_not_allowed` /
 * `no_credential` / `unauthorized` / `unexpected`), and this module turns
 * that fact into the sentence the user reads.
 *
 * An unnamed fault falls back to the generic message, which is itself
 * honest — the shell never invents a cause it could not establish.
 */
import { m } from '$paraglide/messages.js';
import type { WireFault } from './client';

/** The fault a thrown error carries, or `unknown` when it carries none. */
export function faultOf(error: unknown): WireFault {
	if (
		typeof error === 'object' &&
		error !== null &&
		'fault' in error &&
		typeof (error as { fault: unknown }).fault === 'string'
	) {
		return (error as { fault: WireFault }).fault;
	}
	return 'unknown';
}

/** The sentence for a fault. */
export function wireFaultMessage(fault: WireFault): string {
	switch (fault) {
		case 'unreachable':
			return m['wire.error.unreachable']();
		case 'origin_not_allowed':
			return m['wire.error.origin_not_allowed']();
		case 'no_credential':
			return m['wire.error.no_credential']();
		case 'unauthorized':
			return m['wire.error.unauthorized']();
		default:
			return m['wire.error']();
	}
}
