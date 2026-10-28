/**
 * The sidebar's session region — the chat spine's durable store as the
 * frame sees it (plan 23 §2 D5).
 *
 * ## WHAT THIS MODULE IS
 *
 * A Svelte 5 universal-reactivity store (`.svelte.ts`, runes in modules —
 * no legacy writable stores): the session list, the active key, and the
 * four actions the region's rows need. The reads ride the TYPED wire
 * (`#lib/api/client.js`, generated from openapi.yaml) — there is no
 * hand-written endpoint shape here, the same law `lib/queue/client.ts`
 * holds.
 *
 * ## THE GLANCE LAW (and nothing else)
 *
 * The list refreshes at the GLANCE moments — app mount, window focus,
 * palette open, and after a turn's final — because those are the moments a
 * fact may have changed. There is NO polling loop and NO interval and NO
 * SSE: the kernel publishes no chat event (the review-queue's own law,
 * reused), and a poll would tax every idle second for a list that changes
 * when someone types. `noteFinal()` is the fourth trigger's seam: today the
 * page's own turn end calls it; the streaming composer (M3-d) calls the
 * same action at ITS final.
 *
 * ## THE KEYS ARE MINTED, NOT ASKED FOR
 *
 * "New chat" stays a pure client act (R141's law, unchanged): a fresh
 * `s-` + 8 hex key, never sent until something is said. The kernel has no
 * create route BY DESIGN — a session row exists only because a turn named
 * its key, so an abandoned new chat costs nothing. `fresh()` mints and
 * clears; it performs NO write.
 *
 * ## THE FOUR STATES THAT NEVER SHARE A RENDERING
 *
 * `ok` (the kernel answered — possibly with zero rows), `offline` (the
 * wire failed — named), `refused` (the kernel spoke and said no), and the
 * pre-first-read `unread`. The region renders NOTHING at zero rows
 * (chrome-by-exception, plan 21 §1) and the fault states are distinguishable
 * by their `kind`, never by a guess from emptiness.
 *
 * ## WHAT THIS MODULE DOES NOT DO
 *
 * No rename/delete dialogs (the component owns affordances); no relative
 * time (the row's own concern, locale-aware at the component); no transcript
 * replay mapping (`lib/chat/replay.ts` is the pure producer — this module
 * only carries the raw stored turns it fetched).
 */

import { client, type WireFault } from '#lib/api/client.js';
import { faultOf } from '#lib/api/wire-fault.js';
import { SvelteSet } from 'svelte/reactivity';

/** The wire's row, as openapi.yaml states it (parsed, never trusted). */
export interface SessionSummary {
	readonly session_key: string;
	readonly label: string;
	readonly turn_count: number;
	readonly created_at: number;
	readonly last_turn_at: number;
}

/** One stored turn, verbatim enough for replay to map (parse, don't trust). */
export interface StoredTurn {
	readonly seq: number;
	readonly role: 'user' | 'assistant';
	readonly content: string;
	readonly grounding: Record<string, unknown> | null;
	readonly tier: string | null;
	/** The kernel's cite-or-abstain verdict (LL-04, additive; assistant
	 * rows). Verbatim wire name — the page validates the vocabulary. */
	readonly support: string | null;
	readonly created_at: number;
}

/** The four read states. An empty list and a failed read are different facts. */
export type SessionsRead =
	| { readonly kind: 'ok'; readonly sessions: readonly SessionSummary[]; readonly total: number }
	| { readonly kind: 'offline'; readonly fault: WireFault }
	| { readonly kind: 'refused'; readonly status: number | null }
	| { readonly kind: 'malformed' };

/** The list ceilings (the kernel's own bounds; the shell never asks for more). */
export const SESSION_LIMITS = {
	maxLimit: 50,
	defaultLimit: 20,
	maxTurns: 50
} as const;

/** The label a row renders when no rename happened: the first user message,
 * ≤ 40 chars — the dsh behaviour, mirrored. The kernel does not guess it;
 * this is presentation over the row's own turn count. */
export const SNIPPET_MAX_CHARS = 40;

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Parse one session row. A row this module cannot prove is a row this
 * module does not render (the queue's `parseQueueRows` posture). */
function parseSession(value: unknown): SessionSummary | null {
	if (!isRecord(value)) return null;
	const key = value['session_key'];
	const label = value['label'];
	const turnCount = value['turn_count'];
	const createdAt = value['created_at'];
	const lastTurnAt = value['last_turn_at'];
	if (
		typeof key !== 'string' ||
		key.length === 0 ||
		typeof label !== 'string' ||
		typeof turnCount !== 'number' ||
		typeof createdAt !== 'number' ||
		typeof lastTurnAt !== 'number'
	) {
		return null;
	}
	return {
		session_key: key,
		label,
		turn_count: turnCount,
		created_at: createdAt,
		last_turn_at: lastTurnAt
	};
}

function parseSessions(body: unknown): { sessions: SessionSummary[]; total: number } | null {
	if (!isRecord(body)) return null;
	const total = body['total'];
	if (typeof total !== 'number' || !Number.isSafeInteger(total) || total < 0) return null;
	const raw = body['sessions'];
	if (!Array.isArray(raw)) return null;
	const sessions: SessionSummary[] = [];
	for (const row of raw) {
		const parsed = parseSession(row);
		if (parsed === null) return null;
		sessions.push(parsed);
	}
	return { sessions, total };
}

function parseTurn(value: unknown): StoredTurn | null {
	if (!isRecord(value)) return null;
	const seq = value['seq'];
	const role = value['role'];
	const content = value['content'];
	const createdAt = value['created_at'];
	if (
		typeof seq !== 'number' ||
		(role !== 'user' && role !== 'assistant') ||
		typeof content !== 'string' ||
		typeof createdAt !== 'number'
	) {
		return null;
	}
	const grounding = value['grounding'];
	const tier = value['tier'];
	const support = value['support'];
	return {
		seq,
		role,
		content,
		grounding: isRecord(grounding) ? grounding : null,
		tier: typeof tier === 'string' ? tier : null,
		support: typeof support === 'string' ? support : null,
		created_at: createdAt
	};
}

function parseTurns(body: unknown): { turns: StoredTurn[]; total: number } | null {
	if (!isRecord(body)) return null;
	const total = body['total'];
	if (typeof total !== 'number' || !Number.isSafeInteger(total) || total < 0) return null;
	const raw = body['turns'];
	if (!Array.isArray(raw)) return null;
	const turns: StoredTurn[] = [];
	for (const row of raw) {
		const parsed = parseTurn(row);
		if (parsed === null) return null;
		turns.push(parsed);
	}
	return { turns, total };
}

/** Mint a fresh session key: `s-` + 8 hex. Client-side only — the key is
 * never SENT until a turn names it (the lazy law's client half). */
export function mintSessionKey(): string {
	const hex = () => Math.floor(Math.random() * 16).toString(16);
	let suffix = '';
	for (let i = 0; i < 8; i += 1) suffix += hex();
	return `s-${suffix}`;
}

// ── the store ────────────────────────────────────────────────────────────

let sessions = $state<readonly SessionSummary[]>([]);
let total = $state(0);
let activeKey = $state<string | null>(null);
let readState = $state<'unread' | SessionsRead['kind']>('unread');
let lastFault = $state<WireFault | null>(null);
let lastRefusedStatus = $state<number | null>(null);

/** The list as the last glance read left it. Empty until the first read. */
export function currentSessions(): readonly SessionSummary[] {
	return sessions;
}

/** The honest total (the kernel's count of ALL the caller's sessions). */
export function currentTotal(): number {
	return total;
}

/** The active session's key, or `null` for the live (unsent) conversation. */
export function currentActiveKey(): string | null {
	return activeKey;
}

/** The last read's state — the region's own four-state law. */
export function currentReadState(): 'unread' | SessionsRead['kind'] {
	return readState;
}

/** The wire fault of the last OFFLINE read, for the status bar's sentence. */
export function currentSessionsFault(): WireFault | null {
	return lastFault;
}

/** The status of the last REFUSED read (the kernel spoke and said no). */
export function currentSessionsRefusedStatus(): number | null {
	return lastRefusedStatus;
}

/**
 * The GLANCE read: mount, focus, palette open, after a turn's final. Four
 * triggers, no loop.
 */
export async function refresh(): Promise<void> {
	try {
		const result = await client.GET('/chat/sessions', {
			params: { query: { limit: SESSION_LIMITS.defaultLimit, offset: 0 } }
		});
		if (result.response && result.response.status !== 200) {
			readState = 'refused';
			lastRefusedStatus = result.response.status;
			return;
		}
		const parsed = parseSessions(result.data);
		if (!parsed) {
			readState = 'malformed';
			return;
		}
		sessions = parsed.sessions;
		total = parsed.total;
		readState = 'ok';
		lastFault = null;
		lastRefusedStatus = null;
	} catch (error) {
		// A THROWN read is a wire failure — the kernel never spoke (a refusal
		// is a STATUS, and the branch above already named it). The fault is
		// recorded under the offline state, named where the core could name it
		// (the PWA posture answers `unknown` — the honest unknown).
		readState = 'offline';
		lastFault = faultOf(error);
		lastRefusedStatus = null;
	}
}

/**
 * Open a stored session: fetch its turns and hand them back for replay.
 * The caller maps them through `lib/chat/replay.ts` — this module carries
 * bytes, not renderings. Returns `null` for a session this caller cannot
 * read (offline/refused/unknown) — never an empty array, which would lie
 * that the transcript is empty.
 *
 * EXCEPT for a freshly minted key: `fresh()` records the keys it minted,
 * and a fresh key has NOTHING stored by the lazy law — the client knows
 * this without asking, so the fetch short-circuits to an honest empty
 * page (a 404 round-trip for a key that cannot exist yet is noise). The
 * set clears at the after-final glance: a final means some key became real.
 * (SvelteSet, not Set: the lint's reactivity rule — nothing derives from
 * it, but the cheaper honest form is the reactive one.)
 */
const freshKeys = new SvelteSet<string>();

export async function fetchTurns(key: string): Promise<readonly StoredTurn[] | null> {
	if (freshKeys.has(key)) return [];
	try {
		const result = await client.GET('/chat/sessions/{session}/turns', {
			params: {
				path: { session: key },
				query: { limit: SESSION_LIMITS.maxTurns, offset: 0 }
			}
		});
		if (result.response && result.response.status !== 200) return null;
		const parsed = parseTurns(result.data);
		if (!parsed) return null;
		activeKey = key;
		return parsed.turns;
	} catch {
		return null;
	}
}

/** Rename a session's label (the kernel holds the 80-byte fence; the input
 * is trimmed before it leaves). Refreshes the list — the label is on it. */
export async function rename(key: string, label: string): Promise<boolean> {
	const trimmed = label.trim();
	if (trimmed.length === 0) return false;
	try {
		const result = await client.PATCH('/chat/sessions/{session}', {
			params: { path: { session: key } },
			body: { label: trimmed }
		});
		if (result.response && result.response.status !== 200) return false;
		sessions = sessions.map((row) =>
			row.session_key === key ? { ...row, label: trimmed } : row
		);
		return true;
	} catch {
		return false;
	}
}

/** Erase a session (the kernel hard-deletes turns + row in one tx). The
 * CONFIRM lives with the affordance; this action is the confirmed act. On
 * success the row leaves the list and an active pointer to it clears. */
export async function remove(key: string): Promise<boolean> {
	try {
		const result = await client.DELETE('/chat/sessions/{session}', {
			params: { path: { session: key } }
		});
		if (result.response && result.response.status !== 200) return false;
		sessions = sessions.filter((row) => row.session_key !== key);
		total = Math.max(0, total - 1);
		if (activeKey === key) activeKey = null;
		return true;
	} catch {
		return false;
	}
}

/**
 * "New chat": mint a fresh key and make it active — a PURE client act with
 * NO write (the lazy law: nothing persists until something is said). The
 * conversation epoch (the frame store) is the transcript's reset signal;
 * this store only moves the active pointer.
 */
export function fresh(): string {
	const key = mintSessionKey();
	freshKeys.add(key);
	activeKey = key;
	return key;
}

/** Point the frame at a stored session (the row's click). The PAGE fetches
 * and replays the transcript on the change — this store carries pointers,
 * not renderings. */
export function activate(key: string): void {
	activeKey = key;
}

/** The after-final glance: the list's last_turn_at just changed (or is
 * about to be visible). M3-d's streaming composer calls this at ITS final;
 * today the page's turn end does. A read, never a write — and the moment a
 * final lands, no minted key is "unsent" any more: whichever key carried
 * the turn now exists server-side. */
export function noteFinal(): void {
	freshKeys.clear();
	void refresh();
}
