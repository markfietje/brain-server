import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';

/**
 * W-2/W-4 — "How StewardMem runs", pinned at the IPC seam: the section is
 * direct-channel ONLY (the mas channel renders NOTHING — the runtime half
 * of the config boundary), every transition ends in a NAMED outcome, the
 * uninstall names what it removed and OFFERS the data with its exact path
 * (never a silent deletion), and the offered removal is its own named act.
 */

const bridge = vi.hoisted(() => ({
	channel: 'direct' as string,
	posture: { posture: 'app', url: null, chosen: true } as unknown,
	install: null as unknown,
	uninstall: null as unknown,
	removed: false as boolean,
	calls: [] as Array<{ command: string; args?: unknown }>
}));

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string, args?: unknown) => {
		bridge.calls.push({ command, args });
		if (command === 'build_channel') return bridge.channel;
		if (command === 'service_posture') return bridge.posture;
		if (command === 'store_service_posture') return bridge.posture;
		if (command === 'service_install') {
			return bridge.install ?? { outcome: 'installed' };
		}
		if (command === 'service_uninstall') {
			return (
				bridge.uninstall ?? {
					outcome: 'uninstalled',
					removed: ['the launchd agent (com.stewardmem.shell.service)'],
					offered: [],
					residue: []
				}
			);
		}
		if (command === 'service_remove_data') {
			bridge.removed = true;
			return null;
		}
		throw new Error(`unexpected command ${command}`);
	})
}));

import RunModeSection from '../src/lib/components/settings/run-mode-section.svelte';

function setTauri(): void {
	(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}

function clearTauri(): void {
	delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}

beforeEach(() => {
	setTauri();
	bridge.channel = 'direct';
	bridge.posture = { posture: 'app', url: null, chosen: true };
	bridge.install = null;
	bridge.uninstall = null;
	bridge.removed = false;
	bridge.calls = [];
	vi.clearAllMocks();
});

afterEach(() => {
	clearTauri();
	vi.clearAllMocks();
});

test('the_mas_channel_renders_nothing', async () => {
	bridge.channel = 'mas';
	render(RunModeSection);
	expect(screen.queryByTestId('settings-runmode')).toBeNull();
	expect(bridge.calls.filter((call) => call.command === 'service_posture')).toHaveLength(0);
});

test('the_current_posture_renders_and_change_discloses_three_choices', async () => {
	const user = userEvent.setup();
	render(RunModeSection);

	const section = await screen.findByTestId('settings-runmode');
	expect(section.textContent).toContain('Inside the app');
	await user.click(screen.getByText('Change'));

	expect((screen.getByTestId('settings-runmode-app') as HTMLInputElement).checked).toBe(true);
	expect(screen.getByTestId('settings-runmode-service')).toBeTruthy();
	expect(screen.getByTestId('settings-runmode-external')).toBeTruthy();
});

test('applying_the_service_with_a_held_port_names_the_holder', async () => {
	const user = userEvent.setup();
	bridge.install = {
		outcome: 'portHeld',
		kind: 'portHeld',
		holder: 'a kernel that answers as a Brain kernel, not started by this app'
	};
	render(RunModeSection);

	await screen.findByTestId('settings-runmode');
	await user.click(screen.getByText('Change'));
	await user.click(screen.getByTestId('settings-runmode-service'));
	await user.click(screen.getByRole('button', { name: 'Apply' }));

	const outcome = await screen.findByTestId('settings-runmode-outcome');
	expect(outcome.textContent).toContain('not started by this app');
});

test('uninstall_names_what_it_removed_and_offers_the_data_never_silent', async () => {
	const user = userEvent.setup();
	bridge.posture = { posture: 'service', url: null, chosen: true };
	bridge.uninstall = {
		outcome: 'uninstalled',
		removed: ['the launchd agent (com.stewardmem.shell.service)', '/Users/test/Library/LaunchAgents/com.stewardmem.shell.service.plist'],
		offered: ['/Users/test/Library/Application Support/com.stewardmem.shell/service'],
		residue: ['/Users/test/Library/Application Support/com.stewardmem.shell/service']
	};
	render(RunModeSection);

	await screen.findByTestId('settings-runmode');
	await user.click(screen.getByRole('button', { name: 'Stop using the background service' }));

	const outcome = await screen.findByTestId('settings-runmode-outcome');
	expect(outcome.textContent).toContain('com.stewardmem.shell.service');
	const offered = await screen.findByTestId('settings-runmode-offered');
	expect(offered.textContent).toContain(
		'/Users/test/Library/Application Support/com.stewardmem.shell/service'
	);
	// The data still stands until its OWN named act.
	expect(bridge.removed).toBe(false);

	await user.click(screen.getByRole('button', { name: 'Delete it' }));
	expect(await screen.findByTestId('settings-runmode-outcome')).toBeTruthy();
	expect(bridge.removed).toBe(true);
	expect(screen.queryByTestId('settings-runmode-offered')).toBeNull();
});

test('a_ghost_uninstall_is_named_not_installed_without_an_error', async () => {
	const user = userEvent.setup();
	bridge.posture = { posture: 'service', url: null, chosen: true };
	bridge.uninstall = { outcome: 'notInstalled', removed: [], offered: [], residue: [] };
	render(RunModeSection);

	await screen.findByTestId('settings-runmode');
	await user.click(screen.getByRole('button', { name: 'Stop using the background service' }));

	const outcome = await screen.findByTestId('settings-runmode-outcome');
	expect(outcome.textContent).toContain('Nothing was installed.');
});
