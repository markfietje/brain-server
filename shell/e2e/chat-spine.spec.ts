import { expect, test } from '@playwright/test';

/**
 * M3-d — the streaming composer over the REAL loopback kernel (plan 25 §5's
 * exit proof, the R139 e2e precedent extended to the shell).
 *
 * TWO tests, and neither ever SKIPS:
 *
 * 1. `the_composer_streams_the_machine_real_posture` — the machine's own
 *    posture IS the assertion. With the AFM bridge resolved by the harness,
 *    a sent question grounds (citations), streams (deltas under the rAF
 *    flush), and finals with the on-device tier. WITHOUT the bridge, the
 *    kernel answers `chat_bridge_unavailable` (503) by name, and the SAME
 *    test asserts the D1 fallback instead: the notice + a deterministic
 *    answer. The posture is never skipped — it is what the machine can prove.
 * 2. `the_fallback_posture_renders_a_deterministic_answer` — the fallback is
 *    STAGED deterministically on every machine: the page's POST /chat/turns
 *    is intercepted with the kernel's exact 503 envelope (measured off
 *    `src/handlers/chat.rs` — code `recall_unavailable`, the marker on the
 *    message), and the turn must render the posture notice beside a
 *    DETERMINISTIC /recall answer, with no fault banner (a refusal is an
 *    answer, never a wire fault).
 *
 * The kernel under test is the harness's own (e2e/global-setup.ts): a fresh
 * temporary kernel on 127.0.0.1:8799, seeded through its public routes. The
 * operator's 8765 service is never touched.
 */

const BRIDGE_PRESENT = Boolean(process.env['E2E_AFM_BRIDGE']);
const kernelUrl = `http://127.0.0.1:${process.env['E2E_KERNEL_PORT'] ?? 8799}`;

/** What one direct turn answers — the harness probe for the bridge posture
 * (Node fetch against the harness kernel, like sessions.spec's driveTurn). */
async function directTurn(message: string): Promise<{ ok: boolean; code: string }> {
	const response = await fetch(`${kernelUrl}/chat/turns`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ message, locale: 'en' })
	});
	if (!response.ok) {
		let code = `HTTP ${response.status}`;
		try {
			const body = (await response.json()) as { error?: { code?: string } };
			code = body.error?.code ?? code;
		} catch {
			// the status is the fact
		}
		return { ok: false, code };
	}
	const reader = response.body!.getReader();
	const decoder = new TextDecoder();
	let buffer = '';
	let sawFinal = false;
	let sawError = false;
	let done = false;
	while (!done) {
		const { value, done: finished } = await reader.read();
		if (finished) break;
		buffer += decoder.decode(value, { stream: true });
		let boundary: number;
		while ((boundary = buffer.indexOf('\n\n')) >= 0) {
			const frame = buffer.slice(0, boundary);
			buffer = buffer.slice(boundary + 2);
			if (frame.includes('event: final')) sawFinal = true;
			if (frame.includes('event: error')) sawError = true;
			if (sawFinal || sawError) done = true;
		}
	}
	return { ok: sawFinal && !sawError, code: sawError ? 'stream_error' : 'ok' };
}

const QUESTION = 'What does the StewardOS seed memory say?';

test('the_composer_streams_the_machine_real_posture', async ({ page }) => {
	await page.goto('/');

	if (BRIDGE_PRESENT) {
		// ── THE REAL STREAM: grounding, deltas, the on-device tier. ──
		await page.getByTestId('chat-composer').fill(QUESTION);
		await page.getByTestId('chat-send').click();

		// The turn exists, leaves `pending`, and streams: the text node GROWS
		// (≥1 delta rendered) before the terminal replaces it. The node is
		// empty (and Playwright calls an empty node hidden) until the first
		// delta lands — the bridge takes a breath before its first token — so
		// the assert is ATTACHED-then-NON-EMPTY, not merely visible.
		const turn = page.getByTestId('chat-turn').last();
		await expect(turn).toBeVisible();
		const streamText = page.getByTestId('chat-stream-text');
		await expect(streamText).toBeAttached();
		await expect(streamText).not.toHaveText('', { timeout: 60_000 });

		// The grounding card is up (collapsed by default, the M3-b law) — the
		// kernel's grounding event named the seed memory, so the citations
		// count is real. Open it and read the citation ids off the DOM.
		await expect(page.getByTestId('chat-evidence')).toBeVisible();
		await page.getByTestId('chat-evidence').click();
		await expect(page.getByTestId('chat-citation').first()).toBeVisible();

		// The terminal: the turn lands as `generated` (the ANSWER of record)
		// with the tier named — never an error state, never the banner.
		await expect(turn).toHaveAttribute('data-rendering', 'generated', { timeout: 120_000 });
		await expect(page.getByTestId('chat-tier')).toContainText('on_device');
		await expect(page.getByTestId('chat-fault-banner')).toHaveCount(0);
		await expect(page.getByTestId('chat-stream-failed')).toHaveCount(0);

		// The persisted pair (M3-c): the session region reflects the turn —
		// the glance law's fourth trigger fired at the final.
		await expect(page.getByTestId('session-region')).toBeVisible();

		// Leave no rows behind: the turn persisted into the kernel's `default`
		// session (the page had no active key), and the sessions spec asserts
		// EXACT counts against this same harness kernel.
		const cleanup = await fetch(`${kernelUrl}/chat/sessions/default`, { method: 'DELETE' });
		expect(cleanup.ok, 'the drill erases the session it created').toBe(true);
	} else {
		// ── THE MACHINE'S POSTURE: no bridge, so the kernel refuses by name
		//    and D1 renders the fallback. The same assertions plan 25 §5.7
		//    demands of a model-less runner — never skipped. ──
		const probe = await directTurn(QUESTION);
		expect(probe.ok, 'the model-less posture must refuse by name, not hang').toBe(false);
		expect(probe.code).toBe('recall_unavailable');

		await page.getByTestId('chat-composer').fill(QUESTION);
		await page.getByTestId('chat-send').click();
		await expect(page.getByTestId('chat-fallback-notice')).toBeVisible();
		await expect(page.getByTestId('chat-fallback-notice')).toHaveAttribute(
			'data-posture',
			'bridge_unavailable'
		);
		// A DETERMINISTIC answer beside the notice: the seeded memory answers
		// the question through /recall — an answered turn with evidence, or
		// the store's honest absence. Never the banner; the composer lives.
		const turn = page.getByTestId('chat-turn').last();
		await expect(turn).not.toHaveAttribute('data-rendering', 'pending');
		const rendering = await turn.getAttribute('data-rendering');
		expect(['answered', 'absent', 'abstained']).toContain(rendering);
		await expect(page.getByTestId('chat-fault-banner')).toHaveCount(0);
	}
});

test('the_fallback_posture_renders_a_deterministic_answer', async ({ page }) => {
	// STAGE the posture with the kernel's own 503 envelope — the exact bytes
	// a model-less kernel answers (src/handlers/chat.rs: the code is
	// `recall_unavailable`; the MESSAGE prefix is the marker the spine's
	// posture table classifies on).
	await page.route('**/chat/turns', (route) =>
		route.fulfill({
			status: 503,
			contentType: 'application/json',
			body: JSON.stringify({
				error: {
					code: 'recall_unavailable',
					message: 'chat_bridge_unavailable — no brain-afm binary beside the kernel'
				}
			})
		})
	);

	await page.goto('/');
	await page.getByTestId('chat-composer').fill(QUESTION);
	await page.getByTestId('chat-send').click();

	// THE NOTICE names the posture…
	const notice = page.getByTestId('chat-fallback-notice');
	await expect(notice).toBeVisible();
	await expect(notice).toHaveAttribute('data-posture', 'bridge_unavailable');

	// …and the DETERMINISTIC answer renders beside it: the real kernel's
	// /recall answered the fallback (the route intercept touches only the
	// spine), so the turn is a full answer with its evidence — or the store's
	// honest absence. Never the banner (a refusal is an answer); the composer
		// is alive: typing a new question re-enables Send (the fallback was a
		// turn end, not a lock-up).
	const turn = page.getByTestId('chat-turn').last();
	await expect(turn).not.toHaveAttribute('data-rendering', 'pending');
	const rendering = await turn.getAttribute('data-rendering');
	expect(['answered', 'absent', 'abstained']).toContain(rendering);
	await expect(page.getByTestId('chat-fault-banner')).toHaveCount(0);
	// The composer is ALIVE: the fallback was a turn end, not a lock-up — a
	// new question re-enables Send.
	await page.getByTestId('chat-composer').fill('next question');
	await expect(page.getByTestId('chat-send')).toBeEnabled();
});
