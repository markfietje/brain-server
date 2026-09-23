/**
 * D8 — intake telemetry, measured not opinioned, CLIENT-LOCAL. The three
 * preregistered reads (and NOTHING else):
 *   - field-level drop-off: the last-touched question id before abandon
 *   - time-to-first-meaningful-action: ms from flow start to the first
 *     recorded answer
 *   - error recurrence per question id: refused/out-of-vocabulary attempts
 * The buffer holds QUESTION IDS and COARSE NUMBERS only — never question
 * text, never answers, never free text. It lives in module state (the
 * session), writes NOTHING anywhere, and exports ONLY through
 * `exportTelemetry()` — the single door the answers-JSON download calls.
 * No kernel write surface exists (D3); the metrics-twins pattern applies to
 * any future kernel-side ingest.
 */

export interface TelemetryExport {
	readonly dropoffQuestionId: string | null;
	readonly msToFirstAction: number | null;
	readonly errorsByQuestionId: Readonly<Record<string, number>>;
}

let flowStartedAt: number | null = null;
let lastTouchedQuestionId: string | null = null;
let firstActionAt: number | null = null;
let abandoned = false;
const errorsByQuestionId = new Map<string, number>();

export function beginFlow(): void {
	flowStartedAt = Date.now();
	lastTouchedQuestionId = null;
	firstActionAt = null;
	abandoned = false;
	errorsByQuestionId.clear();
}

/** The user engaged a question (rendered/touched it). */
export function recordTouch(questionId: string): void {
	lastTouchedQuestionId = questionId;
}

/** The first recorded answer — the "meaningful action". */
export function recordAnswer(questionId: string): void {
	if (firstActionAt === null && flowStartedAt !== null) {
		firstActionAt = Date.now();
	}
	lastTouchedQuestionId = questionId;
}

/** A refused/out-of-vocabulary attempt on a question (ids only). */
export function recordError(questionId: string): void {
	errorsByQuestionId.set(questionId, (errorsByQuestionId.get(questionId) ?? 0) + 1);
}

/** The flow was abandoned where the user last touched. */
export function recordAbandon(): void {
	abandoned = true;
}

export function exportTelemetry(): TelemetryExport {
	return {
		dropoffQuestionId: abandoned ? lastTouchedQuestionId : null,
		msToFirstAction:
			firstActionAt !== null && flowStartedAt !== null ? firstActionAt - flowStartedAt : null,
		errorsByQuestionId: Object.fromEntries(errorsByQuestionId)
	};
}
