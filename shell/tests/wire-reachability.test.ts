import { afterEach, beforeEach, expect, test, vi } from 'vitest';

/**
 * III-4 — the webview's wire can actually REACH the kernel.
 *
 * Two defects, found by driving the REAL bundle on 2026-10-20 and reading the
 * captured request off the wire, and both are invisible to a test that mocks
 * the credential:
 *
 *  1. **The first read was a race.** The layout kicks the credential read off
 *     (`void bootstrapToken()`), the keychain read is async, and the route's
 *     mount effect fired its ONE read before it resolved — so the request went
 *     out with NO `Authorization` header, against a kernel that requires one,
 *     with nothing to retry it. The captured request proved it: a plain GET
 *     with no bearer and no preflight.
 *  2. **A wire failure was unnamed.** The browser engine reports a blocked
 *     response, a rejected bearer and a dead port identically, so the shell
 *     showed one message for all three — the kind of generic refusal the
 *     shell's own law forbids.
 *
 * The credential is under this file's control (it resolves only when the test
 * says so), which is what makes the race reproducible instead of anecdotal.
 */

const credential = vi.hoisted(() => ({
	release: null as null | ((value: string | null) => void),
	kind: 'origin_not_allowed' as string
}));

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string) => {
		if (command === 'kernel_token') {
			return new Promise<string | null>((resolve) => {
				credential.release = resolve;
			});
		}
		if (command === 'kernel_wire_status') {
			return { kind: credential.kind, origin: 'tauri://localhost' };
		}
		return null;
	})
}));

beforeEach(() => {
	// The Tauri path: the credential comes from the core, not the page.
	(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
	credential.release = null;
	credential.kind = 'origin_not_allowed';
});

afterEach(() => {
	vi.unstubAllGlobals();
	vi.resetModules();
	delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

test('the_first_wire_read_waits_for_the_credential', async () => {
	vi.resetModules();
	const sent: string[] = [];
	vi.stubGlobal(
		'fetch',
		vi.fn(async (request: Request) => {
			sent.push(request.headers.get('Authorization') ?? '');
			return new Response(JSON.stringify([]), {
				status: 200,
				headers: { 'content-type': 'application/json' }
			});
		})
	);

	// M3-a: the wizard read this test drove retired with the wizard; the
	// subject here is the CREDENTIAL GATE, and the review queue's typed read
	// (`fetchPendingQueue` → GET /proposals, the one a live route owns) is
	// the same shape: one typed GET through the shared client.
	const { fetchPendingQueue } = await import('../src/lib/queue/client');
	const pending = fetchPendingQueue();

	// The credential has NOT resolved yet. The read must still be waiting:
	// before the fix, this request went out with no bearer at all.
	await new Promise((resolve) => setTimeout(resolve, 20));
	expect(sent, 'the read must not go out before the credential resolves').toHaveLength(0);

	credential.release?.('the-bearer');
	const read = await pending;
	expect(sent, 'the read must carry the credential').toEqual(['Bearer the-bearer']);
	expect(read, 'an empty queue reads as the kernel saying so').toEqual({ kind: 'ok', rows: [] });
});

test('a_wire_failure_names_its_fault', async () => {
	vi.resetModules();
	credential.kind = 'origin_not_allowed';
	// What the browser engine does when the kernel's CORS layer does not
	// allow this app's origin: the response is blocked and the fetch fails
	// with an opaque network error.
	vi.stubGlobal(
		'fetch',
		vi.fn(async () => {
			throw new TypeError('Failed to fetch');
		})
	);

	const { fetchPendingQueue } = await import('../src/lib/queue/client');
	const pending = fetchPendingQueue();
	// The credential resolves (the read waits for it — that is the fix), and
	// then the wire fails. The queue read NAMES the fault in its result —
	// the surface renders the refusal, never an empty queue.
	await new Promise((resolve) => setTimeout(resolve, 20));
	credential.release?.('the-bearer');
	await expect(pending).resolves.toMatchObject({ kind: 'refused', fault: 'origin_not_allowed' });

	// And a DIFFERENT fault is a different name — the message is not one
	// sentence for every failure.
	credential.kind = 'unauthorized';
	vi.resetModules();
	const again = await import('../src/lib/queue/client');
	const second = again.fetchPendingQueue();
	await new Promise((resolve) => setTimeout(resolve, 20));
	credential.release?.('the-bearer');
	await expect(second).resolves.toMatchObject({ kind: 'refused', fault: 'unauthorized' });
});

test('every_named_fault_has_its_own_message', async () => {
	const { wireFaultMessage } = await import('../src/lib/api/wire-fault');
	const faults = [
		'unreachable',
		'origin_not_allowed',
		'no_credential',
		'unauthorized',
		'unexpected',
		'unknown'
	] as const;
	const messages = faults.map((fault) => wireFaultMessage(fault));
	for (const message of messages) {
		expect(message.length, 'every fault renders a message').toBeGreaterThan(0);
	}
	// A NAMED fault must not render as the generic one — that would be the
	// unnamed refusal this round removed.
	expect(messages[0]).not.toBe(messages[5]);
	expect(messages[1]).not.toBe(messages[5]);
	expect(messages[2]).not.toBe(messages[5]);
	expect(messages[3]).not.toBe(messages[5]);
});
