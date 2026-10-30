import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { render, screen, waitFor, cleanup } from '@testing-library/svelte';

// The App Store fork's SURFACE law (PLAN_MAS_FORK §1 + §3, 2026-10-30),
// pinned in the renderer:
//
//   1. The store edition renders NO updater surface — the channel answer
//      gates the section, and a mas build never names the updater's
//      commands (they are compiled out; the invoke would be a lie).
//   2. The optional Signal gateway module is DESCRIBE + DETECT only: the
//      section names the two postures (running / not installed) and the
//      "separate module" law — no install, no download, no configure
//      affordance exists anywhere in the markup (guideline 2.4.5(iv), and
//      4.2.3(i): the app is fully functional without it).
//
// The IPC seam is mocked at the module boundary, the same discipline as
// every pin here: the channel and the gateway probe are the two commands
// the fork added, and both are answered from the test's own state.

const state = vi.hoisted(() => ({
	channel: 'direct' as 'direct' | 'mas',
	gateway: { present: false, address: '127.0.0.1:8080' } as {
		present: boolean;
		address: string;
	}
}));

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string) => {
		if (command === 'build_channel') return state.channel;
		if (command === 'kernel_token') return null;
		if (command === 'signal_gateway_status') return state.gateway;
		if (command === 'update_check') throw new Error('update_check must not be called on mas');
		return null;
	})
}));

// The settings page reads the kernel facts at mount (the About pane) and
// the audit chain (the sidebar's integrity dot) — both mocked at the same
// boundary, the neutral answers this file's subject does not touch.
vi.mock('#lib/api/kernel-facts.js', () => ({
	fetchKernelFacts: vi.fn(async () => ({
		kind: 'ok',
		facts: { version: null, count: null, model: null }
	}))
}));
vi.mock('#lib/api/audit.js', () => ({
	verifyAuditChain: vi.fn(async () => ({ kind: 'refused', status: 403, fault: 'unknown' }))
}));

// The SvelteKit routing modules, mocked as the frame's pins do.
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (p: string) => p }));
vi.mock('$app/state', () => ({
	page: { url: new URL('http://localhost/'), params: {} }
}));

// The frame and the page open REAL api modules once the Tauri gate is open
// (AppShell's settings://open listener, the About pane's getVersion) — and
// their internal `./core` import bypasses the mock registry above, so both
// seams are mocked at the module boundary (the keychain pins' law).
vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {})
}));
vi.mock('@tauri-apps/api/app', () => ({
	getVersion: vi.fn(async () => '0.0.0-mas-pin')
}));

import SettingsPageHost from './SettingsPageHost.svelte';

beforeEach(() => {
	state.channel = 'direct';
	state.gateway = { present: false, address: '127.0.0.1:8080' };
	(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

afterEach(() => {
	cleanup();
	delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
	vi.clearAllMocks();
});

describe('the App Store fork surface', () => {
	test('the_mas_edition_renders_no_updater_surface', async () => {
		state.channel = 'mas';
		const user = (await import('@testing-library/user-event')).default.setup();
		render(SettingsPageHost);
		await user.click(screen.getByRole('button', { name: 'Updates' }));
		await waitFor(() => {
			expect(
				screen.queryByText('Software update'),
				'the store edition must render no updater surface — the store owns updates'
			).toBeNull();
		});
		expect(
			screen.queryByRole('button', { name: 'Check for updates' }),
			'the mas build must never name the updater commands'
		).toBeNull();
	});

	test('the_direct_edition_still_renders_the_updater', async () => {
		const user = (await import('@testing-library/user-event')).default.setup();
		render(SettingsPageHost);
		await user.click(screen.getByRole('button', { name: 'Updates' }));
		expect(
			await screen.findByRole('button', { name: 'Check for updates' }),
			'the direct channel keeps its updater surface'
		).toBeTruthy();
	});

	test('the_signal_surface_describes_and_detects_nothing_more', async () => {
		state.gateway = { present: true, address: '127.0.0.1:8080' };
		const user = (await import('@testing-library/user-event')).default.setup();
		render(SettingsPageHost);
		await user.click(screen.getByRole('button', { name: 'Advanced' }));
		expect(
			await screen.findByTestId('settings-signal-detected'),
			'a running gateway is NAMED'
		).toBeTruthy();
		// 2.4.5(iv) — the detection carries no install/download/configure
		// affordance: no button of any kind inside the module's surface.
		const surface = screen.getByTestId('settings-signal');
		expect(
			surface.querySelectorAll('button'),
			'the Signal surface must carry no affordance — describe + detect, nothing more'
		).toHaveLength(0);
	});

	test('the_signal_absent_posture_reads_honest_and_never_as_an_error', async () => {
		state.gateway = { present: false, address: '127.0.0.1:8080' };
		const user = (await import('@testing-library/user-event')).default.setup();
		render(SettingsPageHost);
		await user.click(screen.getByRole('button', { name: 'Advanced' }));
		expect(
			await screen.findByTestId('settings-signal-absent'),
			'"no gateway installed" is the resting state, not a fault'
		).toBeTruthy();
	});
});
