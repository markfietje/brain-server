import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import AxeBuilder from 'axe-core';

const here = dirname(fileURLToPath(import.meta.url));
const shellRoot = join(here, '..');

/**
 * The full pack body catalog exactly as the kernel route serves it, built
 * from the SHARED corpus fixtures (ONE source of truth).
 */
function catalogFetch() {
	const names = ['capture-pre-screen', 'support-ticket', 'tele-health'];
	const packs = names.map((name) => {
		const pack = JSON.parse(
			readFileSync(
				join(shellRoot, '../crates/brain-fuzz/corpus/accounts/packs', `${name}.json`),
				'utf8'
			)
		) as { questions: unknown[] };
		return { id: name, question_count: pack.questions.length, pack };
	});
	return { packs, count: packs.length };
}

function stubWire(): void {
	vi.stubGlobal(
		'fetch',
		vi.fn(async () => new Response(JSON.stringify(catalogFetch()), { status: 200 }))
	);
}

async function renderPage() {
	const mod = await import('../src/routes/+page.svelte');
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	return render(mod.default as any);
}

test('wizard_pack_renders_as_branched_conversation', async () => {
	stubWire();
	const user = userEvent.setup();
	await renderPage();
	// The picker lists the served packs.
	await waitFor(() => expect(screen.getByText('Choose a form')).toBeTruthy());
	await user.click(screen.getByRole('button', { name: /support-ticket/ }));
	// ONE question at a time — the first question's instructions.
	await waitFor(() =>
		expect(screen.getByRole('heading', { name: 'Which class does this ticket belong to?' })).toBeTruthy()
	);
	expect(screen.queryByText('How urgent is this ticket?')).toBeNull();
	// Answer billing → the flow BRANCHES to the priority question.
	await user.click(screen.getByText('Billing'));
	await user.click(screen.getByRole('button', { name: 'Continue' }));
	await waitFor(() => expect(screen.getByRole('heading', { name: 'How urgent is this ticket?' })).toBeTruthy());
	expect(screen.queryByText('The account is already known to support.')).toBeNull();
	// Score 0 (low) → the KNOWN question (the next-map says so).
	await user.click(screen.getByText('low'));
	await user.click(screen.getByRole('button', { name: 'Continue' }));
	await waitFor(() =>
		expect(screen.getByText('The account is already known to support.')).toBeTruthy()
	);
	// noul true → end → the done state with the export door (the SWITCH).
	await user.click(screen.getByRole('switch'));
	await user.click(screen.getByRole('button', { name: 'Continue' }));
	await waitFor(() => expect(screen.getByText('All questions answered.')).toBeTruthy());
	expect(screen.getByTestId('export')).toBeTruthy();
});

test('wizard_branch_follows_answer_not_free_text', async () => {
	stubWire();
	const user = userEvent.setup();
	await renderPage();
	await waitFor(() => expect(screen.getByText('Choose a form')).toBeTruthy());
	await user.click(screen.getByRole('button', { name: /support-ticket/ }));
	await waitFor(() =>
		expect(screen.getByRole('heading', { name: 'Which class does this ticket belong to?' })).toBeTruthy()
	);
	// The TECHNICAL branch drives to a DIFFERENT next question than billing
	// would: the ANSWER map steers the flow — the choice control cannot
	// express anything but the three ratified labels, so no text can steer.
	await user.click(screen.getByText('Technical'));
	await user.click(screen.getByRole('button', { name: 'Continue' }));
	// technical → q_priority (same target here), so take score 2 (high) →
	// end per the next-map (0/1 would have gone to q_known).
	await waitFor(() => expect(screen.getByRole('heading', { name: 'How urgent is this ticket?' })).toBeTruthy());
	await user.click(screen.getByText('high'));
	await user.click(screen.getByRole('button', { name: 'Continue' }));
	await waitFor(() => expect(screen.getByText('All questions answered.')).toBeTruthy());
});

test('branch_engine_refuses_unknown_answers_with_the_abstain_card', async () => {
	// Engine level: the refusal IS the abstain path.
	const { parsePack } = await import('../src/lib/wizard/pack');
	const { classifyAnswer } = await import('../src/lib/wizard/branch');
	const { assembleAnswers } = await import('../src/lib/wizard/answers');
	const packValue = JSON.parse(
		readFileSync(
			join(shellRoot, '../crates/brain-fuzz/corpus/accounts/packs/support-ticket.json'),
			'utf8'
		) as string
	);
	const parsed = parsePack(packValue);
	expect(parsed.ok).toBe(true);
	if (!parsed.ok) return;
	const q = parsed.pack.byId.get('q_class');
	expect(q).toBeDefined();
	if (!q) return;
	expect(classifyAnswer(q, '<script>alert(1)</script>')).toBeNull();
	expect(classifyAnswer(q, 'DROP TABLE answers')).toBeNull();
	const hostile = assembleAnswers(parsed.pack, { q_class: '<script>alert(1)</script>' });
	expect(hostile.abstain).toBe(true);
	expect(hostile.answers).toEqual([]);
	// UI level: the abstain card renders with the question id — a human
	// takes over; nothing was invented.
	stubWire();
	await renderPage();
	await waitFor(() => expect(screen.getByText('Choose a form')).toBeTruthy());
	await userEvent.click(screen.getByRole('button', { name: /support-ticket/ }));
	await waitFor(() =>
		expect(screen.getByRole('heading', { name: 'Which class does this ticket belong to?' })).toBeTruthy()
	);
	// Force the refusal path through the page: no option can be invalid via
	// the form, so drive the abstain by finishing with an empty walk —
	// instead, unit-level proof is above; here assert the card component
	// itself.
	const { default: AbstainCard } = await import('../src/lib/wizard/AbstainCard.svelte');
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	render(AbstainCard as any, { props: { questionId: 'q_class' } });
	expect(screen.getByTestId('abstain-card')).toBeTruthy();
	expect(screen.getByText('Needs a human')).toBeTruthy();
});

test('wizard_dropoff_field_is_recorded_anonymized', async () => {
	// The export the download button produces carries the telemetry block —
	// ids only. Build the export from the shared fixture and scan it.
	const { parsePack } = await import('../src/lib/wizard/pack');
	const { buildAnswersExport } = await import('../src/lib/wizard/answers');
	const { beginFlow, recordAnswer, recordTouch, recordAbandon } = await import(
		'../src/lib/telemetry/buffer'
	);
	const packValue = JSON.parse(
		readFileSync(
			join(shellRoot, '../crates/brain-fuzz/corpus/accounts/packs/support-ticket.json'),
			'utf8'
		) as string
	);
	const parsed = parsePack(packValue);
	if (!parsed.ok) throw new Error('fixture parses');
	beginFlow();
	recordTouch('q_class');
	recordAnswer('q_class');
	recordTouch('q_priority');
	recordAbandon();
	const json = buildAnswersExport(parsed.pack, { q_class: 'billing' });
	expect(json.includes('How urgent')).toBe(false); // never question text
	expect(json).toContain('"dropoffQuestionId": "q_priority"');
});

test('wizard_error_recurrence_is_measured_not_opinion', async () => {
	const { beginFlow, recordError, exportTelemetry } = await import('../src/lib/telemetry/buffer');
	beginFlow();
	recordError('q_priority');
	recordError('q_priority');
	const t = exportTelemetry();
	expect(t.errorsByQuestionId['q_priority']).toBe(2);
	expect(JSON.stringify(t).includes('opinion')).toBe(false);
});

test('wizard_resume_restores_saved_progress', async () => {
	stubWire();
	const { saveProgress } = await import('../src/lib/wizard/session');
	// A "previous session" left off at q_known with two answers recorded.
	saveProgress(
		{
			pack: 'support-ticket',
			first: 'q_class',
			questions: [],
			byId: new Map()
		} as unknown as Parameters<typeof saveProgress>[0],
		'q_known',
		{ q_class: 'billing', q_priority: 0 }
	);
	await renderPage();
	await waitFor(() => expect(screen.getByText('Choose a form')).toBeTruthy());
	await userEvent.click(screen.getByRole('button', { name: /support-ticket/ }));
	// The flow RESUMES at q_known — not at the first question.
	await waitFor(() =>
		expect(screen.getByText('The account is already known to support.')).toBeTruthy()
	);
	expect(screen.queryByText('Which class does this ticket belong to?')).toBeNull();
});

test('renderer_passes_axe_aa_checks', async () => {
	stubWire();
	await renderPage();
	await waitFor(() => expect(screen.getByText('Choose a form')).toBeTruthy());
	const { default: axe } = await import('axe-core');
	const results = await axe.run(document.body, {
		runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
	});
	expect(results.violations).toEqual([]);
});
