/**
 * The frame's shared state — the three facts the sidebar, the status bar
 * and the surfaces exchange, and nothing else.
 *
 * ## WHAT LIVES HERE
 *
 * - **The conversation epoch.** "New chat" is a sidebar act that must reach
 *   the chat surface whether or not that surface is mounted; a counter the
 *   surface keys its transcript on is the whole mechanism. The transcript
 *   itself stays in the surface — sessions arrive with their own listing
 *   endpoints, and until then nothing here persists.
 * - **The status context.** The bottom bar's middle slot is the surface's
 *   own sentence ("3 memories", "4 citations"). A surface SETS it while it
 *   is on screen and CLEARS it when it leaves; the bar never guesses one.
 * - **The kernel facts.** Version, corpus count and model are read ONCE per
 *   glance moment by the frame and shared with every surface that shows
 *   them, so two readers cannot disagree about the kernel they are looking
 *   at. The read itself lives in `#lib/api/kernel-facts` — this module only
 *   holds the last answer.
 *
 * ## WHAT DOES NOT
 *
 * No polling, no interval, no watchdog: every value here changes because a
 * surface or the frame observed something. The wire fault has its own
 * module (`chat/wire-status`) with its own law; it is not duplicated here.
 */

import type { KernelFacts } from '#lib/api/kernel-facts.js';

let conversationEpoch = $state(0);
let credentialEpoch = $state(0);
let statusContext = $state<string | null>(null);
let kernelFacts = $state<KernelFacts | null>(null);

/** The current conversation's epoch; a surface keys its transcript on it. */
export function currentConversationEpoch(): number {
	return conversationEpoch;
}

/**
 * The credential epoch (M3-f, moved from the frame's inline effect): the
 * settings token section NOTES a credential change here and the frame
 * re-runs its three glance reads on it. A credential change is the one
 * moment the facts can go from refused to answered (or back) — measured
 * 2026-10-24: a token saved over a stale one left the sidebar reading
 * "integrity not verified" until relaunch.
 */
export function currentCredentialEpoch(): number {
	return credentialEpoch;
}

/** The token section's note that a credential changed (saved, rotated,
 * pending or cleared). */
export function noteCredentialChange(): void {
	credentialEpoch += 1;
}

/** Start a new conversation: the chat surface drops its transcript. */
export function startNewConversation(): void {
	conversationEpoch += 1;
}

/** The surface's own status sentence, or `null` when none is on screen. */
export function currentStatusContext(): string | null {
	return statusContext;
}

/** A surface states what the bottom bar should say beside the wire health. */
export function setStatusContext(text: string | null): void {
	statusContext = text;
}

/** The last kernel facts the frame read, or `null` before the first read. */
export function currentKernelFacts(): KernelFacts | null {
	return kernelFacts;
}

/** The frame records what it read; surfaces only ever read this. */
export function recordKernelFacts(facts: KernelFacts | null): void {
	kernelFacts = facts;
}
