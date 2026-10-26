import { expect, test } from '@playwright/test';

/**
 * M3-c — the session region over the REAL loopback kernel (plan 23 §4's
 * e2e): two sessions created BY TURN (the lazy law — no create route; the
 * store's rows exist because `/chat/turns` named the keys), then SWITCH
 * (both transcripts preserved through the replay mapper), RENAME, and
 * DELETE (the list and the kernel's own audit reflect it).
 *
 * The turns need the AFM bridge: a machine without one answers
 * `chat_bridge_unavailable` (503) by name, and this spec skips WITH THE
 * REASON — the R139 posture (the kernel's other surfaces stay unaffected;
 * the model is macOS-27-only, so an Ubuntu lane cannot run this drill).
 * A skip is loud, never a silent pass.
 */

const kernelUrl = `http://127.0.0.1:${process.env['E2E_KERNEL_PORT'] ?? 8799}`;
const SESSION_A = 'e2e-sessions-alpha';
const SESSION_B = 'e2e-sessions-beta';
const QUESTION_A = 'What is the gate law in one sentence?';
const QUESTION_B = 'Name the erasure posture for chat transcripts.';

interface TurnProbe {
	ok: boolean;
	finalText: string | null;
	groundingSeen: boolean;
	code?: string;
}

/** One real turn over the kernel's SSE spine: `grounding` → `delta`* →
 * `final`|`error`. Returns the FINAL text (the answer of record) and
 * whether the grounding event carried hits. */
async function driveTurn(session: string, message: string): Promise<TurnProbe> {
	const response = await fetch(`${kernelUrl}/chat/turns`, {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ session, message, locale: 'en' })
	});
	if (!response.ok || !response.body) {
		let code = `HTTP ${response.status}`;
		try {
			const body = (await response.json()) as { error?: { code?: string } };
			code = body.error?.code ?? code;
		} catch {
			// the status is the fact
		}
		return { ok: false, finalText: null, groundingSeen: false, code };
	}
	const reader = response.body.getReader();
	const decoder = new TextDecoder();
	let buffer = '';
	let finalText: string | null = null;
	let groundingSeen = false;
	let done = false;
	while (!done) {
		const { value, done: finished } = await reader.read();
		if (finished) break;
		buffer += decoder.decode(value, { stream: true });
		let boundary: number;
		while ((boundary = buffer.indexOf('\n\n')) >= 0) {
			const frame = buffer.slice(0, boundary);
			buffer = buffer.slice(boundary + 2);
			const eventLine = frame.split('\n').find((l) => l.startsWith('event:'));
			const dataLine = frame.split('\n').find((l) => l.startsWith('data:'));
			if (!eventLine || !dataLine) continue;
			const kind = eventLine.slice(6).trim();
			let payload: Record<string, unknown> = {};
			try {
				payload = JSON.parse(dataLine.slice(5).trim()) as Record<string, unknown>;
			} catch {
				continue;
			}
			if (kind === 'grounding') {
				const hits = payload['hits'];
				groundingSeen = Array.isArray(hits) && hits.length > 0;
			} else if (kind === 'final') {
				finalText = typeof payload['text'] === 'string' ? payload['text'] : null;
				done = true;
			} else if (kind === 'error') {
				return {
					ok: false,
					finalText: null,
					groundingSeen,
					code: String(payload['code'] ?? 'error')
				};
			}
		}
	}
	return { ok: finalText !== null, finalText, groundingSeen };
}

test('sessions_create_by_turn_switch_rename_delete_over_the_real_kernel', async ({ page }) => {
	// THE BRIDGE PROBE: one real turn decides whether this machine can run
	// the drill at all. A named 503 is a SKIP with the reason; a real answer
	// is the gate to the rest.
	const probe = await driveTurn(SESSION_A, QUESTION_A);
	test.skip(!probe.ok, `the AFM bridge is unavailable on this machine (${probe.code}) — the session drill needs real turns`);
	expect(probe.finalText, 'the probe turn answered').toBeTruthy();

	// The kernel PERSISTED the pair: the session exists without any create
	// call having been made (the lazy law, measured on the wire).
	const wireList = await fetch(`${kernelUrl}/chat/sessions?limit=50`);
	expect(wireList.ok, 'GET /chat/sessions over the live wire').toBe(true);
	const wireSessions = (await wireList.json()) as {
		sessions: Array<{ session_key: string; turn_count: number }>;
		total: number;
	};
	const alphaEntry = wireSessions.sessions.find((s) => s.session_key === SESSION_A);
	expect(alphaEntry, 'the session row exists because a turn named it').toBeTruthy();
	// turn_count counts PAIRS (one persist per turn); the stored rows are
	// two — the turns wire is where the pair is visible.
	expect(alphaEntry!.turn_count, 'one pair = one counted turn').toBe(1);
	const turnsWire = await fetch(`${kernelUrl}/chat/sessions/${SESSION_A}/turns?limit=50`);
	expect(turnsWire.ok, 'the turns wire').toBe(true);
	const turnsBody = (await turnsWire.json()) as {
		turns: Array<{ seq: number; role: string }>;
		total: number;
	};
	expect(turnsBody.total, 'user + assistant persisted').toBe(2);
	expect(turnsBody.turns.map((t) => t.role)).toEqual(['user', 'assistant']);

	// ── the second session, by turn ──
	const second = await driveTurn(SESSION_B, QUESTION_B);
	expect(second.ok, 'the second session turn answered').toBeTruthy();

// ── the shell: open the frame, the region lists BOTH sessions ──
	await page.goto('/');
	await expect(page.getByTestId('session-region')).toBeVisible();
	await expect(page.getByTestId('session-row')).toHaveCount(2);

	// ── RENAME FIRST (the switch legs click by NAME, not by position: the
	//    list orders by last_turn_at and both turns are seconds apart).
	//    Row 0 is the newest — whichever key that is, the wire tells us. ──
	const items = page.getByTestId('session-item');
	await items.nth(0).hover();
	await items.nth(0).getByTestId('session-rename').click();
	await page.getByTestId('session-rename-input').fill('Beta drill');
	await page.getByTestId('session-rename-input').press('Enter');
	await expect(items.nth(0)).toContainText('Beta drill');
	const named = await fetch(`${kernelUrl}/chat/sessions?limit=50`);
	const namedBody = (await named.json()) as {
		sessions: Array<{ session_key: string; label: string }>;
	};
	const betaKey = namedBody.sessions.find((s) => s.label === 'Beta drill')!.session_key;
	const alphaKey = namedBody.sessions.find((s) => s.label === '')!.session_key;
	expect(betaKey === SESSION_B || betaKey === SESSION_A).toBe(true);
	expect(alphaKey).not.toBe(betaKey);

	// ── SWITCH: opening a session renders ITS transcript (question AND
	//    answer — the replay mapping preserves the pair), and switching to
	//    the other preserves BOTH. ──
	const openByName = async (name: string): Promise<void> => {
		await page.getByTestId('session-row').filter({ hasText: name }).first().click();
		await expect(page.getByTestId('chat-turn')).toHaveCount(1);
	};
	await openByName('Beta drill');
	const turnB = page.getByTestId('chat-turn').first();
	await expect(turnB).toContainText(QUESTION_B, { ignoreCase: true });
	await expect(turnB).toContainText(second.finalText!.slice(0, 30));

	await openByName('Conversation');
	// The unlabelled row is the only one left showing the unlabelled
	// sentence (Beta was renamed above) — find it by its li, then rename
	// through the affordance scoped to THAT item (the affordances are
	// siblings of the row button, not descendants — a button-scoped locator
	// never resolves).
	const unlabelledItem = page
		.getByTestId('session-item')
		.filter({ has: page.getByTestId('session-row').filter({ hasText: 'Conversation' }) })
		.first();
	const unlabelledRow = unlabelledItem.getByTestId('session-row');
	await unlabelledRow.click();
	await unlabelledItem.getByTestId('session-rename').click();
	await page.getByTestId('session-rename-input').fill('Alpha drill');
	await page.getByTestId('session-rename-input').press('Enter');
	await expect(page.getByTestId('session-row').filter({ hasText: 'Alpha drill' })).toHaveCount(1);

	await openByName('Alpha drill');
	const turnA = page.getByTestId('chat-turn').first();
	await expect(turnA).toContainText(QUESTION_A, { ignoreCase: true });
	// The ANSWER TEXT is on the replayed turn (the answer of record).
	await expect(turnA).toContainText(probe.finalText!.slice(0, 30));

	// The grounding evidence is on the replayed turn (the provenance-first
	// law survives replay): the evidence affordance exists and opens.
	await turnA.getByTestId('chat-evidence').click();
	await expect(turnA.getByTestId('chat-citation').first()).toBeVisible();

	// ── DELETE: confirmed, then the list AND the kernel reflect it — the
	//    erasure is total (turns + row) and audited. ──
	const doomedItem = page.getByTestId('session-item').filter({ hasText: 'Alpha drill' });
	await doomedItem.hover();
	await doomedItem.getByTestId('session-delete').click();
	await page.getByTestId('session-delete-confirm').click();
	await expect(page.getByTestId('session-row')).toHaveCount(1);
	const afterDelete = await fetch(`${kernelUrl}/chat/sessions?limit=50`);
	const afterBody = (await afterDelete.json()) as {
		sessions: Array<{ session_key: string }>;
		total: number;
	};
	expect(
		afterBody.sessions.find((s) => s.session_key === SESSION_A),
		"the erased session is gone from the kernel's list"
	).toBeUndefined();

	// Clean the second session too, so the drill leaves no rows behind.
	const delB = await fetch(`${kernelUrl}/chat/sessions/${SESSION_B}`, { method: 'DELETE' });
	expect(delB.ok, 'the cleanup delete').toBe(true);
});
