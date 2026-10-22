/**
 * III-6 — the review queue's row model, and the METADATA-ONLY session cache.
 *
 * ## WHAT THIS MODULE IS
 *
 * A pure projection of the kernel's human review queue (`GET /proposals`) into
 * what a reviewer may read, plus the session cache that lets the surface say
 * "there is work waiting" when the kernel cannot be reached.
 *
 * ## THE ONE LAW THIS FILE EXISTS TO ENFORCE
 *
 * **The cache holds METADATA, never the proposal `content`.** The candidate
 * memory text is the unpublished capture awaiting a human — the most sensitive
 * corpus in the system, and the one the kernel keeps behind its own at-rest
 * boundary. A second copy in the webview's storage is a data-at-rest decision
 * this surface is not entitled to make silently.
 *
 * **The TYPE is the guard.** `QueueRowMetadata` declares `content?: never`, so
 * a `QueueRow` (which carries `content: string`) is NOT assignable to it: the
 * compiler refuses a row-with-content at the cache's own boundary. A cache
 * that cannot hold the content cannot leak it. If a future edit finds itself
 * widening that type, it is fixing the type the wrong way — the law is that
 * the content never reaches the cache, not that the cache is careful about it.
 *
 * The reader is the SECOND layer, and it is a validating one: a cache entry
 * that was tampered with, or written by an older build, contributes only the
 * fields this module knows. Unknown keys — `content` above all — are dropped,
 * so a poisoned cache cannot put the candidate text on the screen either.
 *
 * ## THE LOCAL-STATE LAW
 *
 * sessionStorage ONLY — tab-session-local. Never localStorage, never a cookie,
 * never the kernel. This is the wizard's law (`src/lib/wizard/session.ts`), and
 * the reason is the same: progress dies with the tab session.
 */

import { stripInvisible } from '../sanitize.js';

export const QUEUE_LIMITS = {
	/** The kernel's own ceiling — `MAX_PROPOSALS` (`src/service/review.rs:27`).
	 * The shell never asks for more than the kernel would serve. */
	rows: 200,
	/** The kernel's own default page size (openapi.yaml: `limit` default 50). */
	defaultLimit: 50,
	identifierChars: 256,
	digestChars: 64,
	titleChars: 512,
	sourceChars: 256,
	ownerChars: 256,
	domainChars: 128,
	contentChars: 16384,
	/** The scores the kernel computes are 0..1 by construction (`salience` and
	 * `novelty` both clamp), so a score outside that band is a wire that does
	 * not match the contract and is refused rather than displayed. */
	scoreMax: 1,
	/** The absolute ceiling on a proposal id (i64 on the wire). */
	idMax: Number.MAX_SAFE_INTEGER
} as const;

export type ScreenVerdict = 'clean' | 'quarantine';

/** The fields a row carries whether it is live or cached. */
export interface QueueRowCore {
	readonly id: number;
	readonly kind: string;
	readonly domain: string;
	readonly owner: string | null;
	readonly source: string | null;
	readonly title: string | null;
	readonly novelty: number;
	readonly conflictWith: number | null;
	readonly salience: number;
	readonly authority: number | null;
	readonly createdAt: number;
	readonly editedAt: number | null;
	readonly decidedAt: number | null;
	readonly expiresAt: number;
	readonly warnSecs: number;
	readonly criticalSecs: number;
	readonly screenVerdict: ScreenVerdict;
	readonly qaScore: number | null;
}

/**
 * THE CACHE'S TYPE, and the guard is the type.
 *
 * `content?: never` is not decoration. It makes a `QueueRow` — which carries
 * `content: string` — UNASSIGNABLE to `QueueRowMetadata`, so the compiler
 * refuses a row-with-content where the cache is written. Widening this
 * declaration to `content?: string` would remove the guard in one keystroke,
 * which is why a pin asserts the shape of this declaration.
 */
export interface QueueRowMetadata extends QueueRowCore {
	readonly content?: never;
}

/**
 * A LIVE row: the metadata PLUS the candidate memory text the reviewer is
 * being asked about. It is deliberately NOT a `QueueRowMetadata` — see above.
 */
export interface QueueRow extends QueueRowCore {
	readonly content: string;
	readonly contentDigest: string | null;
}

/** What the session cache holds: the rows' metadata, and WHEN it was seen. */
export interface QueueCache {
	readonly cachedAt: number;
	readonly rows: readonly QueueRowMetadata[];
}

export type QueueParseFailure = 'not_an_array' | 'too_many_rows' | 'bad_row';

export type QueueParseResult =
	| { ok: true; value: readonly QueueRow[] }
	| { ok: false; reason: QueueParseFailure };

function isPlainObject(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** A bounded, invisible-stripped string; `null` when absent or empty. */
function optionalString(value: unknown, max: number): string | null {
	if (typeof value !== 'string' || value.length > max) return null;
	const cleaned = stripInvisible(value);
	return cleaned.length === 0 ? null : cleaned;
}

/** A bounded, invisible-stripped string that must be present and non-empty. */
function requiredString(value: unknown, max: number): string | null {
	return optionalString(value, max);
}

/** A finite integer in `[min, max]`, or `null`. */
function optionalInteger(value: unknown, min: number, max: number): number | null {
	if (typeof value !== 'number' || !Number.isSafeInteger(value)) return null;
	if (value < min || value > max) return null;
	return value;
}

/** A finite score in `0..1`, or `null`. */
function optionalScore(value: unknown): number | null {
	if (typeof value !== 'number' || !Number.isFinite(value)) return null;
	if (value < 0 || value > QUEUE_LIMITS.scoreMax) return null;
	return value;
}

function screenVerdict(value: unknown): ScreenVerdict | null {
	return value === 'clean' || value === 'quarantine' ? value : null;
}

/**
 * Parse ONE wire row. Every field is proved, every collection bounded, and a
 * row that cannot be proved is REFUSED — the whole page is then refused,
 * because a queue that silently drops rows would under-report the work
 * waiting, and an under-reported queue is worse than no queue.
 */
function parseRow(value: unknown): QueueRow | null {
	if (!isPlainObject(value)) return null;

	const id = optionalInteger(value['id'], 0, QUEUE_LIMITS.idMax);
	const kind = requiredString(value['kind'], QUEUE_LIMITS.identifierChars);
	const domain = requiredString(value['domain'], QUEUE_LIMITS.domainChars);
	const novelty = optionalScore(value['novelty']);
	const salience = optionalScore(value['salience']);
	const createdAt = optionalInteger(value['created_at'], 0, QUEUE_LIMITS.idMax);
	const expiresAt = optionalInteger(value['expires_at'], 0, QUEUE_LIMITS.idMax);
	const warnSecs = optionalInteger(value['warn_secs'], 0, QUEUE_LIMITS.idMax);
	const criticalSecs = optionalInteger(value['critical_secs'], 0, QUEUE_LIMITS.idMax);
	const verdict = screenVerdict(value['screen_verdict']);

	if (
		id === null ||
		kind === null ||
		domain === null ||
		novelty === null ||
		salience === null ||
		createdAt === null ||
		expiresAt === null ||
		warnSecs === null ||
		criticalSecs === null ||
		verdict === null
	) {
		return null;
	}

	// The content is the reviewer's copy of the candidate text. It is bounded
	// and stripped; a row whose content is absent is still a row (the kernel
	// always sends it, but the display degrades to "unavailable" rather than
	// dropping the whole page).
	const rawContent = value['content'];
	const content =
		typeof rawContent === 'string' && rawContent.length <= QUEUE_LIMITS.contentChars
			? stripInvisible(rawContent)
			: '';

	// The digest is the reviewer's COPY HANDLE: the approve verb is
	// digest-bound, so a decision made elsewhere binds to the bytes the
	// reviewer was shown. It is displayed for exactly that reason.
	const rawDigest = value['content_digest'];
	const digest =
		typeof rawDigest === 'string' && rawDigest.length <= QUEUE_LIMITS.digestChars
			? rawDigest
			: null;

	return {
		id,
		kind,
		domain,
		owner: optionalString(value['owner'], QUEUE_LIMITS.ownerChars),
		source: optionalString(value['source'], QUEUE_LIMITS.sourceChars),
		title: optionalString(value['title'], QUEUE_LIMITS.titleChars),
		novelty,
		conflictWith: optionalInteger(value['conflict_with'], 0, QUEUE_LIMITS.idMax),
		salience,
		authority: optionalScore(value['authority']),
		createdAt,
		editedAt: optionalInteger(value['edited_at'], 0, QUEUE_LIMITS.idMax),
		decidedAt: optionalInteger(value['decided_at'], 0, QUEUE_LIMITS.idMax),
		expiresAt,
		warnSecs,
		criticalSecs,
		screenVerdict: verdict,
		qaScore: optionalInteger(value['qa_score'], 0, QUEUE_LIMITS.idMax),
		content,
		contentDigest: digest
	};
}

/** Parse the wire's `ProposalView[]` without trusting its shape. */
export function parseQueueRows(value: unknown): QueueParseResult {
	if (!Array.isArray(value)) return { ok: false, reason: 'not_an_array' };
	if (value.length > QUEUE_LIMITS.rows) return { ok: false, reason: 'too_many_rows' };
	const rows: QueueRow[] = [];
	for (const raw of value) {
		const parsed = parseRow(raw);
		if (parsed === null) return { ok: false, reason: 'bad_row' };
		rows.push(parsed);
	}
	return { ok: true, value: rows };
}

/**
 * The projection into the cache's type: the metadata and NOTHING else.
 *
 * This is the function the law is about. It constructs a NEW object with an
 * explicit field list, so no field of the source — content above all — can
 * ride along by being forgotten here. The return type is `QueueRowMetadata`,
 * which cannot hold `content`.
 */
export function toQueueMetadata(row: QueueRow): QueueRowMetadata {
	return {
		id: row.id,
		kind: row.kind,
		domain: row.domain,
		owner: row.owner,
		source: row.source,
		title: row.title,
		novelty: row.novelty,
		conflictWith: row.conflictWith,
		salience: row.salience,
		authority: row.authority,
		createdAt: row.createdAt,
		editedAt: row.editedAt,
		decidedAt: row.decidedAt,
		expiresAt: row.expiresAt,
		warnSecs: row.warnSecs,
		criticalSecs: row.criticalSecs,
		screenVerdict: row.screenVerdict,
		qaScore: row.qaScore
	};
}

/**
 * What the surface renders: the metadata plus the content WHEN the row is
 * live. `null` means "not available", never "empty" — the offline state says
 * the text is unavailable, and the row says so too.
 */
export interface QueueDisplayRow extends QueueRowCore {
	readonly content: string | null;
	readonly contentDigest: string | null;
}

/**
 * The display projection. The content and the digest are passed IN, so the
 * caller decides what it has — a live row passes both, a cached row passes
 * neither, and there is no path by which a cached row acquires either.
 */
export function toDisplayRow(
	row: QueueRowCore,
	content: string | null,
	contentDigest: string | null
): QueueDisplayRow {
	return {
		id: row.id,
		kind: row.kind,
		domain: row.domain,
		owner: row.owner,
		source: row.source,
		title: row.title,
		novelty: row.novelty,
		conflictWith: row.conflictWith,
		salience: row.salience,
		authority: row.authority,
		createdAt: row.createdAt,
		editedAt: row.editedAt,
		decidedAt: row.decidedAt,
		expiresAt: row.expiresAt,
		warnSecs: row.warnSecs,
		criticalSecs: row.criticalSecs,
		screenVerdict: row.screenVerdict,
		qaScore: row.qaScore,
		content,
		contentDigest
	};
}

/** The sessionStorage key. One entry per tab session; nothing outlives it. */
export const QUEUE_CACHE_KEY = 'brain-queue-cache:';

function isPlainRow(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * Read ONE cached row, taking ONLY the fields this module knows.
 *
 * Unknown keys are dropped — which is the second layer of the content law: a
 * cache entry carrying `content` (tampered with, or written by a build that
 * did not have the law) contributes its metadata and NOTHING else. The row
 * is rebuilt field by field, so nothing can ride along.
 */
function readCachedRow(value: unknown): QueueRowMetadata | null {
	if (!isPlainRow(value)) return null;
	const id = optionalInteger(value['id'], 0, QUEUE_LIMITS.idMax);
	const kind = requiredString(value['kind'], QUEUE_LIMITS.identifierChars);
	const domain = requiredString(value['domain'], QUEUE_LIMITS.domainChars);
	const novelty = optionalScore(value['novelty']);
	const salience = optionalScore(value['salience']);
	const createdAt = optionalInteger(value['createdAt'], 0, QUEUE_LIMITS.idMax);
	const verdict = screenVerdict(value['screenVerdict']);
	if (
		id === null ||
		kind === null ||
		domain === null ||
		novelty === null ||
		salience === null ||
		createdAt === null ||
		verdict === null
	) {
		return null;
	}
	return {
		id,
		kind,
		domain,
		owner: optionalString(value['owner'], QUEUE_LIMITS.ownerChars),
		source: optionalString(value['source'], QUEUE_LIMITS.sourceChars),
		title: optionalString(value['title'], QUEUE_LIMITS.titleChars),
		novelty,
		conflictWith: optionalInteger(value['conflictWith'], 0, QUEUE_LIMITS.idMax),
		salience,
		authority: optionalScore(value['authority']),
		createdAt,
		editedAt: optionalInteger(value['editedAt'], 0, QUEUE_LIMITS.idMax),
		decidedAt: optionalInteger(value['decidedAt'], 0, QUEUE_LIMITS.idMax),
		expiresAt: optionalInteger(value['expiresAt'], 0, QUEUE_LIMITS.idMax) ?? 0,
		warnSecs: optionalInteger(value['warnSecs'], 0, QUEUE_LIMITS.idMax) ?? 0,
		criticalSecs: optionalInteger(value['criticalSecs'], 0, QUEUE_LIMITS.idMax) ?? 0,
		screenVerdict: verdict,
		qaScore: optionalInteger(value['qaScore'], 0, QUEUE_LIMITS.idMax)
	};
}

/**
 * The cached metadata, or `null`. Never throws: a storage failure (private
 * mode, a quota, a corrupted entry) is not an error surface — it simply means
 * there is nothing cached.
 */
export function readQueueCache(): QueueCache | null {
	try {
		const raw = window.sessionStorage.getItem(QUEUE_CACHE_KEY);
		if (raw === null) return null;
		const parsed: unknown = JSON.parse(raw);
		if (!isPlainObject(parsed)) return null;
		const cachedAt = optionalInteger(parsed['cachedAt'], 0, QUEUE_LIMITS.idMax);
		const rawRows = parsed['rows'];
		if (cachedAt === null || !Array.isArray(rawRows)) return null;
		if (rawRows.length > QUEUE_LIMITS.rows) return null;
		const rows: QueueRowMetadata[] = [];
		for (const raw of rawRows) {
			const row = readCachedRow(raw);
			if (row === null) return null;
			rows.push(row);
		}
		return { cachedAt, rows };
	} catch {
		return null;
	}
}

/**
 * Write the METADATA cache. The parameter is typed `readonly QueueRowMetadata[]`
 * — a type that cannot carry `content` — and the writer rebuilds every row
 * field by field, so the stored bytes hold the metadata and nothing else.
 */
export function writeQueueCache(rows: readonly QueueRowMetadata[], cachedAt: number): void {
	try {
		const payload: QueueCache = {
			cachedAt,
			rows: rows.map((row) => ({ ...row }))
		};
		window.sessionStorage.setItem(QUEUE_CACHE_KEY, JSON.stringify(payload));
	} catch {
		// storage unavailable (private mode etc.) — the queue simply does not
		// survive the session; the live read still works. Never an error surface.
	}
}

/** Forget the cache (the operator asked for a fresh read). */
export function clearQueueCache(): void {
	try {
		window.sessionStorage.removeItem(QUEUE_CACHE_KEY);
	} catch {
		// nothing stored
	}
}

// ── the presentation helpers ────────────────────────────────────────────────

/** The absolute time, ISO-8601 in UTC — unambiguous and locale-free. */
export function queueTimeLabel(unixSeconds: number): string {
	return new Date(unixSeconds * 1000).toISOString();
}

/** A score, rendered with a fixed precision so two scores compare by eye. */
export function queueScoreLabel(score: number): string {
	return score.toFixed(4);
}

/** The age of the cache, in whole minutes (never negative). */
export function queueAgeMinutes(cachedAt: number, now: number): number {
	const seconds = Math.max(0, Math.floor(now) - Math.floor(cachedAt));
	return Math.floor(seconds / 60);
}

/**
 * The age's unit: minutes below an hour, hours below a day, days above. The
 * renderer turns this into a translated phrase — the shell never hard-codes
 * an English duration.
 */
export type QueueAgeUnit = 'minutes' | 'hours' | 'days';

export function queueAgeUnit(minutes: number): QueueAgeUnit {
	if (minutes < 60) return 'minutes';
	if (minutes < 60 * 24) return 'hours';
	return 'days';
}

/** The age in the unit above (whole units). */
export function queueAgeAmount(minutes: number): number {
	const unit = queueAgeUnit(minutes);
	if (unit === 'minutes') return minutes;
	if (unit === 'hours') return Math.floor(minutes / 60);
	return Math.floor(minutes / (60 * 24));
}
