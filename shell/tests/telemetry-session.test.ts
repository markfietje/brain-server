import { describe, expect, test, beforeEach } from 'vitest';
import {
	beginFlow,
	recordTouch,
	recordAnswer,
	recordError,
	recordAbandon,
	exportTelemetry
} from '../src/lib/telemetry/buffer';
import { saveProgress, restoreProgress, clearProgress } from '../src/lib/wizard/session';

describe('D8 telemetry — ids and coarse numbers ONLY', () => {
	beforeEach(() => {
		beginFlow();
	});

	test('wizard_dropoff_field_is_recorded_anonymized', () => {
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

	test('wizard_error_recurrence_is_measured_not_opinion', () => {
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

describe('session progress (D1 resume)', () => {
	test('wizard_resume_restores_saved_progress', () => {
		const answers = { q_class: 'billing', q_priority: 1 };
		saveProgress(
			{ pack: 'support-ticket' } as Parameters<typeof saveProgress>[0],
			'q_known',
			answers
		);
		const restored = restoreProgress('support-ticket');
		expect(restored).not.toBeNull();
		expect(restored?.currentQuestionId).toBe('q_known');
		expect(restored?.answers).toEqual(answers);

		clearProgress('support-ticket');
		expect(restoreProgress('support-ticket')).toBeNull();
	});

	test('a saved blob for ANOTHER pack never restores (the key is the pack id)', () => {
		saveProgress({ pack: 'tele-health' } as Parameters<typeof saveProgress>[0], 'q_modality', {});
		expect(restoreProgress('support-ticket')).toBeNull();
		clearProgress('tele-health');
	});

	test('corrupt session blobs restore as null, never throw', () => {
		window.sessionStorage.setItem('brain-wizard-progress:support-ticket', '{not json');
		expect(restoreProgress('support-ticket')).toBeNull();
		window.sessionStorage.setItem(
			'brain-wizard-progress:support-ticket',
			JSON.stringify({ packId: 'other', currentQuestionId: 5, answers: null })
		);
		expect(restoreProgress('support-ticket')).toBeNull();
		window.sessionStorage.removeItem('brain-wizard-progress:support-ticket');
	});
});
