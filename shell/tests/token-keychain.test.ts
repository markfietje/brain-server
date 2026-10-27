import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

// The SvelteKit routing modules are mocked at the boundary, exactly as
// `command-palette.test.ts` does: AppShell imports all three.
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));
vi.mock('$app/state', () => ({
	page: { url: new URL('http://localhost/'), params: {} }
}));

// The IPC seam: a STATEFUL stub for the three commands the keychain round
// added. `vi.mock` factories are hoisted, so the state lives in `vi.hoisted`
// (a plain top-level `let` would red with "Cannot access before
// initialization"). `store_kernel_token` returns the ROTATION's outcome
// (III-3 §4.7) — never the token.
const keychain = vi.hoisted(() => ({
	stored: null as string | null,
	rotation: null as {
		kernelSynced: boolean;
		dataRootMissing: boolean;
		error: string | null;
	} | null
}));
// M3-b: AppShell reads the review queue on mount (the badge's wire). The
// host import below is STATIC, so the typed client would capture the real
// `fetch` before any stub could land — the queue read is mocked at the
// module boundary instead, answering an EMPTY queue: this file's subject is
// the palette/token flows, and an absent badge is the neutral chrome state.
vi.mock('#lib/queue/client.js', () => ({
	fetchPendingQueue: vi.fn(async () => ({ kind: 'ok', rows: [] }))
}));
// The sidebar frame's two other mount-time reads, mocked the same way.
vi.mock('#lib/api/kernel-facts.js', () => ({
	fetchKernelFacts: vi.fn(async () => ({
		kind: 'ok',
		facts: { version: null, count: null, model: null }
	}))
}));
vi.mock('#lib/api/audit.js', () => ({
	verifyAuditChain: vi.fn(async () => ({ kind: 'refused', status: 403, fault: 'unknown' }))
}));

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string, args?: { token?: string }) => {
		if (command === 'kernel_token') return keychain.stored;
		if (command === 'store_kernel_token') {
			keychain.stored = args?.token ?? null;
			return keychain.rotation;
		}
		if (command === 'clear_kernel_token') {
			keychain.stored = null;
			return null;
		}
		throw new Error(`unexpected command ${command}`);
	})
}));

import { invoke } from '@tauri-apps/api/core';
import { authHeaders, hasBearer, setBearer } from '../src/lib/api/token';
import { bootstrapToken } from '../src/lib/api/bootstrap';
import { shellRoot } from './fixtures';
import AppShellHost from './AppShellHost.svelte';

const SENTINEL = 'brain-shell-keychain-sentinel-DO-NOT-LEAK-5a7e';

function setTauri(): void {
	(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}

function clearTauri(): void {
	delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}

function consoleSink(): { calls: string[]; restore: () => void } {
	const calls: string[] = [];
	const originals = (['log', 'info', 'warn', 'error', 'debug'] as const).map((channel) => {
		const original = console[channel].bind(console);
		const spy = vi.spyOn(console, channel).mockImplementation((...args: unknown[]) => {
			calls.push(args.map((arg) => (typeof arg === 'string' ? arg : String(arg))).join(' '));
			original(...args);
		});
		return { spy, original };
	});
	return {
		calls,
		restore: () => {
			for (const { spy, original } of originals) {
				spy.mockRestore();
				void original;
			}
		}
	};
}

beforeEach(() => {
	setTauri();
	keychain.stored = null;
	keychain.rotation = null;
	setBearer(null);
	vi.clearAllMocks();
});

afterEach(() => {
	clearTauri();
	setBearer(null);
	// Clear call history only: the IPC stub's implementation lives in the
	// module mock and must survive into the next test. Each spy restores
	// itself in the test that created it.
	vi.clearAllMocks();
});

/**
 * The bootstrap read goes through the Tauri command and never writes the
 * token to browser storage. The serving origin's PWA path is untouched
 * (there is no Tauri runtime), which the third arm pins.
 */
test('the_bootstrap_reads_the_keychain_command_and_never_stores_it', async () => {
	keychain.stored = SENTINEL;
	const storage = vi.spyOn(Storage.prototype, 'setItem');
	const consoleSpy = consoleSink();
	try {
		await bootstrapToken();

		expect(hasBearer()).toBe(true);
		expect(authHeaders()['Authorization']).toBe(`Bearer ${SENTINEL}`);
		expect(vi.mocked(invoke)).toHaveBeenCalledWith('kernel_token');
		for (const call of storage.mock.calls) {
			expect(JSON.stringify(call)).not.toContain(SENTINEL);
		}
		expect(consoleSpy.calls.join('\n')).not.toContain(SENTINEL);

		// The PWA arm: outside Tauri the in-memory bearer is NOT cleared.
		clearTauri();
		setBearer('serving-origin-bearer');
		await bootstrapToken();
		expect(hasBearer()).toBe(true);
	} finally {
		consoleSpy.restore();
		storage.mockRestore();
	}
});

/**
 * The Settings writer: type a token, Save writes it through the command, the
 * input is emptied, the status flips to configured; Clear removes it and the
 * status returns to anonymous.
 */
test('the_settings_saves_to_the_keychain_and_clears_it', async () => {
	const user = userEvent.setup();
	render(AppShellHost);

	await user.click(screen.getByRole('button', { name: 'Settings' }));
	await screen.findByRole('dialog');
	const input = (await screen.findByLabelText('Kernel token')) as HTMLInputElement;
	expect(screen.getByText(/No token configured/)).toBeTruthy();

	// The frame's glance reads ran once at mount; a credential change must
	// run them again, because it is the one moment a refused fact can
	// become an answered one (measured: a stale sidebar until relaunch).
	const { fetchKernelFacts } = await import('#lib/api/kernel-facts.js');
	const { verifyAuditChain } = await import('#lib/api/audit.js');
	const factsBefore = vi.mocked(fetchKernelFacts).mock.calls.length;
	const chainBefore = vi.mocked(verifyAuditChain).mock.calls.length;

	await user.type(input, SENTINEL);
	await user.click(screen.getByRole('button', { name: 'Save token' }));

	await waitFor(() =>
		expect(vi.mocked(invoke)).toHaveBeenCalledWith('store_kernel_token', { token: SENTINEL })
	);
	await waitFor(() => expect(keychain.stored).toBe(SENTINEL));
	expect(input.value).toBe('');
	expect(await screen.findByText('A kernel token is configured.')).toBeTruthy();
	await waitFor(() => {
		expect(vi.mocked(fetchKernelFacts).mock.calls.length, 'the facts re-read after a save').toBe(factsBefore + 1);
		expect(vi.mocked(verifyAuditChain).mock.calls.length, 'the chain re-read after a save').toBe(chainBefore + 1);
	});

	await user.click(screen.getByRole('button', { name: 'Clear token' }));
	await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith('clear_kernel_token'));
	await waitFor(() => expect(keychain.stored).toBeNull());
	expect(await screen.findByText(/No token configured/)).toBeTruthy();
	await waitFor(() => {
		expect(vi.mocked(fetchKernelFacts).mock.calls.length, 'the facts re-read after a clear').toBe(factsBefore + 2);
	});
});

/**
 * III-3 §4.7 — the Settings save must say what happened to the RUNNING
 * kernel. The keychain always holds the token, but a running kernel reads
 * its bearer from the data root's auth-token, so the core rotates that file
 * and reports whether it did.
 */
test('the_settings_reports_that_the_running_kernel_took_the_token', async () => {
	keychain.rotation = { kernelSynced: true, dataRootMissing: false, error: null };
	const user = userEvent.setup();
	render(AppShellHost);

	await user.click(screen.getByRole('button', { name: 'Settings' }));
	await screen.findByRole('dialog');
	const input = (await screen.findByLabelText('Kernel token')) as HTMLInputElement;
	await user.type(input, SENTINEL);
	await user.click(screen.getByRole('button', { name: 'Save token' }));

	expect(
		await screen.findByText('Token saved; the running kernel uses it now.')
	).toBeTruthy();
});

/**
 * A rotation that did NOT reach the kernel must not be reported as a saved
 * token: the user would act on the lie (they would not relaunch).
 */
test('a_rotation_that_failed_is_not_reported_as_saved', async () => {
	keychain.rotation = { kernelSynced: false, dataRootMissing: false, error: 'insecureAuthFile' };
	const user = userEvent.setup();
	render(AppShellHost);

	await user.click(screen.getByRole('button', { name: 'Settings' }));
	await screen.findByRole('dialog');
	const input = (await screen.findByLabelText('Kernel token')) as HTMLInputElement;
	await user.type(input, SENTINEL);
	await user.click(screen.getByRole('button', { name: 'Save token' }));

	expect(
		await screen.findByText(/the running kernel was not updated/)
	).toBeTruthy();
	expect(screen.queryByText('Token saved to the login keychain.')).toBeNull();
});

/**
 * The PWA keeps its serving-origin bearer: outside Tauri the section does not
 * exist at all, so there is no second writer competing with the origin.
 */
test('the_token_entry_is_hidden_outside_tauri', async () => {
	clearTauri();
	const user = userEvent.setup();
	render(AppShellHost);

	await user.click(screen.getByRole('button', { name: 'Settings' }));
	await screen.findByRole('dialog');
	expect(screen.queryByLabelText('Kernel token')).toBeNull();
	expect(screen.queryByRole('button', { name: 'Save token' })).toBeNull();
});

/** The section joins the Settings dialog, so the dialog's AA profile covers it. */
test('the_settings_surface_is_axe_clean', async () => {
	const user = userEvent.setup();
	render(AppShellHost);
	await user.click(screen.getByRole('button', { name: 'Settings' }));
	await screen.findByRole('dialog');
	await screen.findByLabelText('Kernel token');

	const { default: axe } = await import('axe-core');
	const results = await axe.run(document.body, {
		runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
	});
	expect(results.violations).toEqual([]);
});

/**
 * The token never reaches storage, a console line, or rendered markup during
 * the Save drill — and the anti-vacuity arm proves the invoke DID receive it,
 * so a no-op Save cannot pass.
 */
test('the_token_never_reaches_storage_logs_or_markup', async () => {
	const user = userEvent.setup();
	const storage = vi.spyOn(Storage.prototype, 'setItem');
	const consoleSpy = consoleSink();
	try {
		render(AppShellHost);
		await user.click(screen.getByRole('button', { name: 'Settings' }));
		await screen.findByRole('dialog');
		const input = (await screen.findByLabelText('Kernel token')) as HTMLInputElement;
		await user.type(input, SENTINEL);
		await user.click(screen.getByRole('button', { name: 'Save token' }));

		await waitFor(() =>
			expect(vi.mocked(invoke)).toHaveBeenCalledWith('store_kernel_token', { token: SENTINEL })
		);
		await waitFor(() => expect(keychain.stored).toBe(SENTINEL));

		// Anti-vacuity: the token really crossed the IPC seam.
		expect(JSON.stringify(vi.mocked(invoke).mock.calls)).toContain(SENTINEL);

		// …and nowhere else.
		expect(document.body.innerHTML).not.toContain(SENTINEL);
		expect(input.value).toBe('');
		for (const call of storage.mock.calls) {
			expect(JSON.stringify(call)).not.toContain(SENTINEL);
		}
		expect(consoleSpy.calls.join('\n')).not.toContain(SENTINEL);
	} finally {
		consoleSpy.restore();
		storage.mockRestore();
	}

	// The modules that hold the token have no browser-storage back door.
	for (const file of ['src/lib/api/token.ts', 'src/lib/api/bootstrap.ts']) {
		const source = readFileSync(join(shellRoot, file), 'utf8');
		expect(source).not.toMatch(/indexedDB/);
		expect(source).not.toMatch(/document\.cookie/);
	}
	const bootstrapSrc = readFileSync(join(shellRoot, 'src/lib/api/bootstrap.ts'), 'utf8');
	expect(bootstrapSrc).not.toMatch(/import\.meta\.env\s*[.[]/);
});
