/**
 * M3-b — the ONE classified wire fault the chrome may show (plan 21 §1, plan
 * 22 R2).
 *
 * ## WHAT THIS MODULE IS
 *
 * The single piece of shared state behind the home page's fault banner. A
 * wire fault is classified when a surface's READ failed at the wire — the
 * transport threw (`WireError.fault`, named by the Rust core's
 * `kernel_wire_status` because the renderer cannot tell a blocked response
 * from a dead port) — and the classification is cleared when the wire is
 * observed healthy again. Nothing else lives here: no polling, no interval,
 * no watchdog. The banner is REACTIVE, and this module is how a fault found
 * by one surface's read becomes the one interrupt another surface renders.
 *
 * ## THE LAWS THIS MODULE OBEYS
 *
 * - **A refusal is not a fault.** A 403 answer means the kernel SPOKE. Only
 *   a wire-level failure (unreachable, origin_not_allowed, no_credential,
 *   …) is ever classified here — the six-state taxonomy's whole point.
 * - **Silent while green.** `null` is the resting state, and a healthy read
 *   clears a stale fault rather than leaving a banner over a live wire.
 * - **One fault, not a history.** The newest classification wins; the banner
 *   names the CURRENT fact, not the last N of them.
 *
 * ## WHY A MODULE, NOT COMPONENT STATE
 *
 * The banner renders on the home page, but faults are found wherever a read
 * happens — a chat turn's transport, the badge's queue read on window focus.
 * A rule that lives in one component is a rule the other cannot reach; a
 * module-scoped `$state` with three functions is the whole surface, and the
 * compiler keeps every reader reactive to it.
 */

import type { WireFault } from '#lib/api/client.js';

let classified = $state<WireFault | null>(null);

/** The fault the banner should name, or `null` while the wire is healthy. */
export function currentWireFault(): WireFault | null {
	return classified;
}

/** Classify a wire-level fault — the named kind, never a guess. */
export function classifyWireFault(fault: WireFault): void {
	classified = fault;
}

/** Clear a stale fault — called when a read observed the wire healthy. */
export function clearWireFault(): void {
	classified = null;
}
