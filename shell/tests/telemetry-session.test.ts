import { describe, expect, test, beforeEach } from 'vitest';
import {
	beginFlow,
	recordTouch,
	recordAnswer,
	recordError,
	recordAbandon,
	exportTelemetry
} from '../src/lib/telemetry/buffer';

/**
 * M3-a (plan `21_CHAT_FIRST_IA`): the wizard retired, and with it the
 * wizard-session describe below (it tested `lib/wizard/session.ts`, deleted
 * with the lib). The TELEMETRY half stays — the buffer module stays — with
 * the wizard's name dropped from the two tests that described the buffer's
 * own laws, not the wizard's.
 *
 * NOT SETTLED HERE, recorded: with the wizard page gone the buffer has no
 * production caller. It is kept because this round's scope is the IA, not a
 * sweep for orphaned lib modules; its retirement (or a caller) is a later
 * round's decision, made on its own measurement.
 */
describe('D8 telemetry — ids and coarse numbers ONLY', () => {
	beforeEach(() => {
		beginFlow();
	});

	test('dropoff_field_is_recorded_anonymized', () => {
		recordTouch('q_class');
		recordAnswer('q_class');
		recordTouch('q_priority');
		// The user abandons HERE: the drop-off is the last-touched QUESTION
		// ID — no question text, no answer values, no free text anywhere in
		// the export.
		recordAbandon();
		const t = exportTelemetry();
		expect(t.dropoffQuestionId).toBe('q_priority');
		const json = JSON.stringify(t);
		expect(json).not.toContain('How urgent');
		expect(json).not.toContain('billing');
		expect(json).not.toContain('medium');
	});

	test('time-to-first-meaningful-action is a coarse number or absent', () => {
		expect(exportTelemetry().msToFirstAction).toBeNull(); // nothing yet
		recordAnswer('q_class');
		const t = exportTelemetry();
		expect(t.msToFirstAction).not.toBeNull();
		expect(t.msToFirstAction as number).toBeLessThan(60_000);
	});

	test('error_recurrence_is_measured_not_opinion', () => {
		recordError('q_class');
		recordError('q_class');
		recordError('q_known');
		const t = exportTelemetry();
		expect(t.errorsByQuestionId).toEqual({ q_class: 2, q_known: 1 });
		// Recurrence is counted per question id — there is no score, no
		// judgement, no optimization logic attached.
		expect(Object.keys(t.errorsByQuestionId).every((k) => k.startsWith('q_'))).toBe(true);
	});

	test('no abandon recorded → dropoff stays null (completed flows export no drop-off)', () => {
		recordAnswer('q_class');
		const t = exportTelemetry();
		expect(t.dropoffQuestionId).toBeNull();
	});
});
