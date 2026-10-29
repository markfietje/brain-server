import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

/**
 * III-3 — the gate above the chrome, pinned at the IPC seam.
 *
 * The Rust contract is pinned in `src-tauri/src/sidecar_tests.rs`. This file
 * pins the only thing the webview owns: the STATE MACHINE. A refusal renders
 * no children; consent Cancel never starts; Continue starts once with
 * `consent: true`; a recorded consent starts without the dialog; the PWA
 * bypasses without invoking Tauri at all.
 *
 * The IPC stub is shared by `kernel_status`/`kernel_start` and by
 * `refreshBearer`'s `kernel_token` — the same import the app makes. A command
 * the component is not supposed to send throws, so an accidental second path
 * is a finding, not an `undefined`.
 */

const bridge = vi.hoisted(() => ({
	status: null as unknown,
	start: null as unknown,
	token: 'test-token' as string | null,
	calls: [] as Array<{ command: string; args?: unknown }>
}));

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string, args?: unknown) => {
		bridge.calls.push({ command, args });
		if (command === 'kernel_status') return bridge.status;
		if (command === 'kernel_start') return bridge.start;
		if (command === 'kernel_token') return bridge.token;
		throw new Error(`unexpected command ${command}`);
	})
}));

import { setBearer } from '../src/lib/api/token';
import KernelGateHost from './KernelGateHost.svelte';

const DATA_ROOT = '/Users/test/Library/Application Support/com.stewardmem.shell/kernel';
const SIDECAR = '/Applications/Brain Shell.app/Contents/MacOS/brain-server';

function setTauri(): void {
	(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}

function clearTauri(): void {
	delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}

function status(overrides: Record<string, unknown> = {}): Record<string, unknown> {
	return {
		state: 'notRunning',
		kind: null,
		expected: '1.29.23',
		observed: null,
		sidecar: SIDECAR,
		dataRoot: DATA_ROOT,
		waitedMs: null,
		lastError: null,
		consentGranted: false,
		version: null,
		pid: null,
		...overrides
	};
}

async function axeViolations(): Promise<unknown[]> {
	const { default: axe } = await import('axe-core');
	const results = await axe.run(document.body, {
		runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] }
	});
	return results.violations;
}

beforeEach(() => {
	setTauri();
	setBearer(null);
	bridge.status = status();
	bridge.start = status({ state: 'started', version: '1.29.23', pid: 4242, consentGranted: true });
	bridge.token = 'test-token';
	bridge.calls = [];
	vi.clearAllMocks();
});

afterEach(() => {
	clearTauri();
	setBearer(null);
	vi.clearAllMocks();
});

/**
 * P11 — a mismatch is a REFUSAL, not an empty app. Both versions are named,
 * no child content renders, and the gate never calls `kernel_start` over a
 * kernel it refused to adopt.
 */
test('mismatch_refuses_without_children', async () => {
	bridge.status = status({
		state: 'refused',
		kind: 'mismatch',
		expected: '1.29.23',
		observed: '9.9.9'
	});
	render(KernelGateHost);

	const refusal = await screen.findByTestId('kernel-refusal');
	expect(refusal.textContent).toContain('1.29.23');
	expect(refusal.textContent).toContain('9.9.9');
	expect(refusal.textContent).toContain(SIDECAR);
	expect(screen.queryByTestId('host-content')).toBeNull();
	expect(bridge.calls.filter((call) => call.command === 'kernel_start')).toHaveLength(0);

	expect(await axeViolations()).toEqual([]);
});

/**
 * P12a — Cancel is a blocked state, never a fall-through. The Rust side has
 * created nothing (consent=false returns before `create_dir_all`), so the
 * pin's UI half is "no start call, no children".
 */
test('consent_cancel_does_not_start', async () => {
	const user = userEvent.setup();
	render(KernelGateHost);

	const dialog = await screen.findByRole('dialog');
	expect(dialog.getAttribute('aria-modal')).toBe('true');
	expect(dialog.getAttribute('aria-labelledby')).toBe('kernel-consent-heading');
	expect(await axeViolations()).toEqual([]);

	await user.click(screen.getByRole('button', { name: 'Cancel' }));

	expect(await screen.findByTestId('kernel-blocked')).toBeTruthy();
	expect(bridge.calls.filter((call) => call.command === 'kernel_start')).toHaveLength(0);
	expect(screen.queryByTestId('host-content')).toBeNull();
});

/**
 * P12b — Continue starts ONCE with consent, then the bearer is read and the
 * app renders. The anti-vacuity arm asserts the IPC seam really received the
 * start and the token read.
 */
test('consent_continue_starts_then_ready_renders', async () => {
	const user = userEvent.setup();
	render(KernelGateHost);

	await screen.findByRole('dialog');
	await user.click(screen.getByRole('button', { name: 'Continue' }));

	expect(await screen.findByTestId('host-content')).toBeTruthy();
	const starts = bridge.calls.filter((call) => call.command === 'kernel_start');
	expect(starts).toHaveLength(1);
	expect(starts[0]?.args).toEqual({ consent: true });
	expect(bridge.calls.some((call) => call.command === 'kernel_token')).toBe(true);
});

/**
 * A recorded consent skips the dialog and starts on mount. This is the
 * second-launch path, and it must not show a consent screen it already has.
 */
test('a_recorded_consent_starts_without_the_dialog', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.start = status({
		state: 'alreadyRunning',
		version: '1.29.23',
		consentGranted: true
	});
	render(KernelGateHost);

	expect(await screen.findByTestId('host-content')).toBeTruthy();
	expect(screen.queryByRole('dialog')).toBeNull();
	expect(bridge.calls.filter((call) => call.command === 'kernel_start')).toHaveLength(1);
});

/**
 * A start that succeeds in Rust but leaves no readable bearer must NOT render
 * an app that will receive 401s on every call. This is the fail-closed half of
 * "authentication is mandatory".
 */
test('a_started_kernel_without_a_readable_token_refuses', async () => {
	const user = userEvent.setup();
	bridge.token = null;
	render(KernelGateHost);

	await screen.findByRole('dialog');
	await user.click(screen.getByRole('button', { name: 'Continue' }));

	const refusal = await screen.findByTestId('kernel-refusal');
	expect(refusal.textContent).toContain('keychain');
	expect(screen.queryByTestId('host-content')).toBeNull();
});

/**
 * The PWA keeps its serving-origin posture: no Tauri runtime means no gate,
 * no command, and children render immediately.
 */
test('the_pwa_bypasses_the_gate_and_invokes_nothing', async () => {
	clearTauri();
	render(KernelGateHost);

	expect(await screen.findByTestId('host-content')).toBeTruthy();
	expect(bridge.calls).toHaveLength(0);
});
