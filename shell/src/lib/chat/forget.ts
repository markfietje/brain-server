/**
 * II-6 — `forget`: the erasure receipt, and what it can honestly claim.
 *
 * ## WHAT THE KERNEL ACTUALLY RETURNS, because the plan assumes more
 *
 * Plan 06 §4 renders `forget` as:
 *
 *     forgot brain-server #a41 · tombstoned 14:31 · chain 8f2e… → 91cf…
 *       DSAR certificate: dsar-2026-10-15-0042
 *
 * and says "the kernel already emits one". **It does not, on this route.**
 * `POST /ump/forget` returns exactly `{"result": "tombstoned"}` — verified
 * against `openapi.yaml` and `src/handlers/ump_ops.rs`, whose handler ends at
 * `Ok(Json(json!({ "result": result })))`. There is no chain head and no
 * certificate on that response.
 *
 * A DSAR certificate is emitted by a DIFFERENT route: `POST /dsar` (the Art
 * 15/17 workflow, Admin-only, subject-addressed) and re-read at
 * `GET /dsar/{id}/certificate`. Those are whole-subject erasures, not
 * citation-level forgets, and they carry different authority.
 *
 * So this module has three honest outcomes and NO fourth:
 *
 *   - `tombstoned`  — the kernel confirmed it, and the audit chain carries a
 *                     row naming the target. That IS a chain transition, and
 *                     it is verifiable through `/ump/audit`.
 *   - `refused`     — the kernel declined (legal hold, or the authority the
 *                     destructive path requires). A refusal is a RESULT, not
 *                     a failure: the row was not erased and saying so is the
 *                     whole point.
 *   - `unavailable` — the kernel could not be reached, or answered something
 *                     this client cannot read.
 *
 * ## WHY THERE IS NO `certificate` FIELD
 *
 * The obvious move is to synthesise `dsar-<date>-<n>` from the tombstone
 * time. That is the exact failure Plan 08 §0 exists to prevent: a receipt
 * that is not a receipt. A user holding a fabricated reference would take it
 * to an auditor and find nothing, which costs more than showing no reference
 * at all. The prompt's pin says "`forget` shows a DSAR certificate reference,
 * or fails" — the `or fails` arm is the honest one here, and taking it is
 * recorded rather than hidden.
 *
 * ## WHY `humanGated` IS A VALUE AND NOT A COMMENT
 *
 * `⌘⇧X` opens a human-gated proposal. The kernel removed
 * `brain_memory_proposal_decide` in 1.29.18 precisely so a model cannot
 * promote its own suggestion, and the server role-gates both proposal verbs
 * to humans. A shell that could auto-approve would make the kernel's gate
 * decorative, so `Receipt.humanGated` is a field a surface must render rather
 * than a note in a README — and the pin asserts no auto-approve path exists.
 */

/** What the kernel confirmed. */
export interface Tombstoned {
	readonly outcome: 'tombstoned';
	readonly result: 'tombstoned' | 'erased';
	readonly recordId: number;
	/** The audit row that proves the chain moved. Null when unread. */
	readonly chainEvidence: ChainEvidence | null;
	/**
	 * Always absent on this route, and named as such. See the module header:
	 * `/ump/forget` emits no certificate, and one is never synthesised.
	 */
	readonly dsarCertificate: null;
	readonly certificateAbsentBecause: string;
}

/** A refusal is a result, not an error — the row was NOT erased. */
export interface RefusedForget {
	readonly outcome: 'refused';
	readonly recordId: number;
	readonly status: number;
	readonly reason: string;
	readonly dsarCertificate: null;
}

/** The kernel could not be reached, or answered something unreadable. */
export interface UnavailableForget {
	readonly outcome: 'unavailable';
	readonly recordId: number;
	readonly status: number;
	readonly detail: string;
}

export type ForgetOutcome = Tombstoned | RefusedForget | UnavailableForget;

/**
 * The chain evidence a tombstone leaves.
 *
 * `/ump/audit` returns HASHES ONLY — `target_hash` is a SHA-256 of the
 * affected id, never the id itself. So this is evidence that a row for that
 * target exists on the chain at a time, not a row a user can read back. The
 * field is named `targetHash` rather than `recordId` for the same reason the
 * core omits `owner`: presenting a hash where a reader expects an id is how a
 * receipt starts lying.
 */
export interface ChainEvidence {
	readonly auditId: number;
	readonly ts: string;
	readonly kind: string;
	readonly targetHash: string;
	readonly status: string;
	readonly domain: string | null;
}

/** Why no certificate — the sentence a user reads. */
const NO_CERTIFICATE =
	'the kernel issues no DSAR certificate on /ump/forget; certificates come from the Admin-only /dsar workflow, which erases a whole subject rather than one citation. None is shown because none exists.';

/**
 * Builds the tombstone receipt.
 *
 * `chainEvidence` is OPTIONAL and its absence is not an error: the audit read
 * is a second call that can fail on its own, and a tombstone the kernel
 * confirmed must still render when the evidence read does not. The receipt
 * says what it can prove and names what it cannot.
 */
export function tombstonedReceipt(
	result: 'tombstoned' | 'erased',
	recordId: number,
	chainEvidence: ChainEvidence | null
): Tombstoned {
	return {
		outcome: 'tombstoned',
		result,
		recordId,
		chainEvidence,
		dsarCertificate: null,
		certificateAbsentBecause: NO_CERTIFICATE
	};
}

/** 401/403 and the destructive-path refusals all land here. */
export function refusedForget(recordId: number, status: number, reason: string): RefusedForget {
	return { outcome: 'refused', recordId, status, reason, dsarCertificate: null };
}

export function unavailableForget(
	recordId: number,
	status: number,
	detail: string
): UnavailableForget {
	return { outcome: 'unavailable', recordId, status, detail };
}

/**
 * DID THE BYTES GO? — read from the RESULT, never from the outcome's name.
 *
 * **This was wrong until II-7c, and the old form is recorded because it is the
 * shape a name-based predicate takes.** It read `outcome.outcome ===
 * 'tombstoned'`, which is TRUE for a tombstone — so every soft forget reported
 * that data had been ERASED, when what happened is that a flag was set and the
 * row is still retrievable. The distinction is invisible in the outcome's name
 * and total in the `result` the kernel actually returns.
 *
 * It survived a green suite because the pins asserted the behaviour rather than
 * the predicate: `chat-forget.test.ts` called it on a tombstone and expected
 * `true`, so the pin and the defect agreed with each other. Nothing could have
 * caught it from the outside — it took wiring the core's real `result` through
 * to make the question askable.
 *
 * Note the honest consequence, which is the point: through the shell's soft
 * path this is now ALWAYS `false`, and it stays in the vocabulary because
 * `tombstonedReceipt` accepts `'erased'` and a caller must be able to ask.
 */
export function didErase(outcome: ForgetOutcome): boolean {
	return outcome.outcome === 'tombstoned' && outcome.result === 'erased';
}

/**
 * True when the receipt may claim the chain moved.
 *
 * A tombstone whose evidence read returned nothing may NOT claim it: the
 * kernel confirmed the flag, but the chain row is a separate fact, and a
 * receipt that asserts a chain transition it did not observe is the softer
 * form of the fabricated certificate this module refuses to produce.
 */
export function chainTransitionObserved(outcome: ForgetOutcome): boolean {
	return outcome.outcome === 'tombstoned' && outcome.chainEvidence !== null;
}

/**
 * The receipt a copy action carries, and the auto-approve guard.
 *
 * `humanGated` is TRUE unconditionally and is not a parameter. The kernel
 * removed `brain_memory_proposal_decide` (1.29.18) and role-gates both
 * proposal verbs to humans; a shell path that could approve its own proposal
 * would make that gate decorative. Making it a constant rather than a flag
 * means a caller cannot pass `false`.
 */
export interface ForgetReceipt {
	readonly text: string;
	readonly humanGated: true;
	readonly autoApprovable: false;
}

export function forgetReceipt(outcome: ForgetOutcome): ForgetReceipt {
	let text: string;
	switch (outcome.outcome) {
		case 'tombstoned': {
			const evidence = outcome.chainEvidence;
			text = [
				`forgot brain-server #${outcome.recordId} · ${outcome.result}`,
				evidence === null
					? '  the kernel confirmed the tombstone; the chain row was not read, so no chain transition is claimed'
					: `  chain row ${evidence.auditId} at ${evidence.ts} (target ${evidence.targetHash.slice(0, 12)}…)`,
				`  DSAR certificate: none — ${outcome.certificateAbsentBecause}`
			].join('\n');
			break;
		}
		case 'refused':
			text = [
				`forget refused for #${outcome.recordId} (HTTP ${outcome.status})`,
				`  ${outcome.reason}`,
				'  nothing was erased'
			].join('\n');
			break;
		case 'unavailable':
			text = [
				`forget unavailable for #${outcome.recordId} (${outcome.status})`,
				`  ${outcome.detail}`,
				'  erasure is degraded, not empty'
			].join('\n');
			break;
	}
	return { text, humanGated: true, autoApprovable: false };
}