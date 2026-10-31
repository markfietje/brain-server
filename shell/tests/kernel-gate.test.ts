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
	channel: 'direct' as string,
	posture: { posture: 'app', url: null, chosen: true } as unknown,
	install: null as unknown,
	detect: null as unknown,
	calls: [] as Array<{ command: string; args?: unknown }>
}));

/** The empty machine: every card absent, the port free — the report the
 * detection core returns over a bare home. */
const emptyReport = () => ({
	existing: null,
	openclaw: null,
	workspace: null,
	kernelOnPort: {
		running: false,
		version: null,
		holder: null,
		evidence: "GET /health on 127.0.0.1:8765 — the app's own probe"
	},
	updates: []
});

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string, args?: unknown) => {
		bridge.calls.push({ command, args });
		if (command === 'kernel_status') return bridge.status;
		if (command === 'kernel_start') return bridge.start;
		if (command === 'kernel_token') return bridge.token;
		if (command === 'build_channel') return bridge.channel;
		if (command === 'service_posture') return bridge.posture;
		if (command === 'store_service_posture') return bridge.posture;
		if (command === 'detect_cards') {
			if (bridge.detect === null) throw new Error('unexpected command detect_cards');
			return bridge.detect;
		}
		if (command === 'service_install') {
			return (
				bridge.install ?? {
					outcome: 'installed',
					serviceLabel: 'com.stewardmem.shell.service'
				}
			);
		}
		throw new Error(`unexpected command ${command}`);
	})
}));

import { setBearer } from '../src/lib/api/token';
import KernelGateHost from './KernelGateHost.svelte';

const DATA_ROOT = '/Users/test/Library/Application Support/com.stewardmem.shell/kernel';
const SIDECAR = '/Applications/StewardMem.app/Contents/MacOS/brain-server';

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
	bridge.channel = 'direct';
	bridge.posture = { posture: 'app', url: null, chosen: true };
	bridge.install = null;
	bridge.detect = emptyReport();
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

// ── W-2 — the posture decision (the ONE screen, offered once) ──────────────

/**
 * First run, consent recorded, NO posture marker: the ONE screen renders
 * with the three choices, A pre-selected, and NO start call yet — the
 * decision is the gate's only blocker, and B/C never block it.
 */
test('the_first_run_offers_the_posture_once_with_app_preselected', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	render(KernelGateHost);

	const screen_ = await screen.findByTestId('posture-screen');
	expect(screen_.getAttribute('aria-modal')).toBe('true');
	expect((screen.getByTestId('posture-app') as HTMLInputElement).checked).toBe(true);
	expect((screen.getByTestId('posture-service') as HTMLInputElement).checked).toBe(false);
	expect((screen.getByTestId('posture-external') as HTMLInputElement).checked).toBe(false);
	// The decision is the only blocker: nothing started behind it.
	expect(bridge.calls.filter((call) => call.command === 'kernel_start')).toHaveLength(0);
	expect(await axeViolations()).toEqual([]);
});

// ── W-T2 — the proposal cards: the machine's report beside the ONE choice ──

/**
 * D4 — a kernel already runs on 8765: the card names it with its version
 * and its evidence, and C arrives PRE-SELECTED with the loopback URL
 * prefilled — attach is the honest default when a kernel already runs.
 */
test('a_running_kernel_preselects_attach_and_the_card_names_it', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	bridge.detect = {
		...emptyReport(),
		kernelOnPort: {
			running: true,
			version: '1.29.38',
			holder: 'a kernel that answers as a Brain kernel, not started by this app',
			evidence: "GET /health on 127.0.0.1:8765 — the app's own probe"
		}
	};
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	await screen.findByTestId('detection-card-kernel');
	expect((screen.getByTestId('posture-external') as HTMLInputElement).checked).toBe(true);
	expect((screen.getByTestId('posture-app') as HTMLInputElement).checked).toBe(false);
	expect(
		(screen.getByTestId('posture-url') as HTMLInputElement).value
	).toBe('http://127.0.0.1:8765');
	expect(screen.getByTestId('detection-card-kernel').textContent).toContain('1.29.38');
	expect(screen.getByTestId('detection-evidence-kernel').textContent).toContain(
		'GET /health on 127.0.0.1:8765'
	);
});

/**
 * D1 — an existing install: the card names the service's version, its path,
 * and the plist it was read from. A claim without a path is a lie.
 */
test('the_existing_service_card_names_the_version_the_path_and_the_evidence', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	bridge.detect = {
		...emptyReport(),
		existing: {
			binaryPath: '/Users/mark/.local/bin/brain-server',
			version: '1.29.37',
			running: false,
			evidence:
				'/Users/mark/Library/LaunchAgents/com.brain.server.plist → ProgramArguments[0] = /Users/mark/.local/bin/brain-server'
		}
	};
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	await screen.findByTestId('detection-card-existing');
	// The existing service does NOT pre-select anything — the machine's fact
	// is named; the choice stays the user's (A keeps the HIG default).
	expect((screen.getByTestId('posture-app') as HTMLInputElement).checked).toBe(true);
	const card = screen.getByTestId('detection-card-existing');
	expect(card.textContent).toContain('1.29.37');
	expect(card.textContent).toContain('/Users/mark/.local/bin/brain-server');
	expect(screen.getByTestId('detection-evidence-existing').textContent).toContain(
		'Library/LaunchAgents/com.brain.server.plist'
	);
});

/**
 * D3 — the workspace memory: counted and sized, NEVER opened, and the
 * import is explicitly deferred to Settings (the installer never migrates
 * data).
 */
test('the_workspace_card_counts_and_defers_the_import', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	bridge.detect = {
		...emptyReport(),
		workspace: {
			databases: [
				{ name: 'brain-global.db', sizeBytes: 0, modifiedEpoch: 1 },
				{ name: 'brain-import-dom-10345000.db', sizeBytes: 8192, modifiedEpoch: 2 }
			],
			totalBytes: 8192,
			evidence: '/Users/mark/.openclaw/workspace — names, sizes, mtimes'
		}
	};
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	const card = await screen.findByTestId('detection-card-workspace');
	expect(card.textContent).toContain('2');
	expect(card.textContent).toContain('8 KB');
	expect(card.textContent).toContain('import them later from Settings');
	expect(screen.getByTestId('detection-evidence-workspace').textContent).toContain(
		'/Users/mark/.openclaw/workspace'
	);
});

/**
 * D5 — the update card names BOTH versions: installed X, offered Y.
 */
test('the_update_card_names_both_versions', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	bridge.detect = {
		...emptyReport(),
		updates: [
			{
				binary: 'brain-server',
				installedVersion: '1.29.37',
				offeredVersion: '1.29.38',
				evidence:
					'/Users/mark/.local/bin/brain-server (--version) vs /offer/brain-server (--version)'
			}
		]
	};
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	const card = await screen.findByTestId('detection-card-update');
	expect(card.textContent).toContain('brain-server');
	expect(card.textContent).toContain('1.29.37');
	expect(card.textContent).toContain('1.29.38');
	expect(screen.getByTestId('detection-evidence-update').textContent).toContain('--version');
});

/**
 * D2 — the OpenClaw integration is named with the config that carries it.
 */
test('the_openclaw_card_names_its_config', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	bridge.detect = {
		...emptyReport(),
		openclaw: { pluginEnabled: true, evidence: '/Users/mark/.openclaw/openclaw.json' }
	};
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	expect(await screen.findByTestId('detection-card-openclaw').then((el) => el.textContent)).toContain('OpenClaw');
	expect(screen.getByTestId('detection-evidence-openclaw').textContent).toContain(
		'openclaw.json'
	);
});

/**
 * An EMPTY machine: A keeps the HIG default, and the absence is NAMED —
 * "nothing was touched", never a silent blank.
 */
test('an_empty_machine_leaves_app_preselected_and_names_the_nothing_found_law', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	expect((screen.getByTestId('posture-app') as HTMLInputElement).checked).toBe(true);
	expect((await screen.findByTestId('detection-nothing-found')).textContent).toContain(
		'nothing was touched'
	);
	expect(screen.queryByTestId('detection-card-existing')).toBeNull();
});

/**
 * An older core without the detector: no cards, the gate stands as it was —
 * a missing detection is never a crash and never a guess.
 */
test('an_older_core_without_the_detector_leaves_a_standing_gate', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	bridge.detect = null; // the handler throws — the command does not exist
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	expect((screen.getByTestId('posture-app') as HTMLInputElement).checked).toBe(true);
	expect(screen.queryByTestId('detection-cards')).toBeNull();
	expect(await axeViolations()).toEqual([]);
});

/**
 * Continue with A: the choice is STORED first, then the kernel starts —
 * the same start-once contract as the consent screen.
 */
test('choosing_app_stores_the_marker_and_starts_once', async () => {
	const user = userEvent.setup();
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	await user.click(screen.getByTestId('posture-continue'));

	expect(await screen.findByTestId('host-content')).toBeTruthy();
	const stores = bridge.calls.filter((call) => call.command === 'store_service_posture');
	expect(stores).toHaveLength(1);
	expect(stores[0]?.args).toEqual({ posture: 'app' });
	const starts = bridge.calls.filter((call) => call.command === 'kernel_start');
	expect(starts).toHaveLength(1);
	expect(starts[0]?.args).toEqual({ consent: true });
});

/**
 * A held port REFUSES posture B naming its holder, ON the decision screen —
 * A and C remain right there. No children render over a refusal.
 */
test('a_held_port_refuses_posture_b_naming_the_holder', async () => {
	const user = userEvent.setup();
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	bridge.install = {
		outcome: 'portHeld',
		kind: 'portHeld',
		holder: 'a kernel that answers as a Brain kernel, not started by this app'
	};
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	await user.click(screen.getByTestId('posture-service'));
	await user.click(screen.getByTestId('posture-continue'));

	const outcome = await screen.findByTestId('posture-outcome');
	expect(outcome.textContent).toContain('not started by this app');
	// No start was attempted over the refused install: the app never spawns
	// a kernel of its own while the service owns the port.
	expect(bridge.calls.filter((call) => call.command === 'kernel_start')).toHaveLength(0);
	expect(screen.queryByTestId('host-content')).toBeNull();
});

/**
 * The Signal sentence renders in B's confirmation — the factual sentence,
 * with NO install affordance for the gateway (the MAS law: describe only).
 */
test('the_signal_sentence_renders_in_the_b_confirmation', async () => {
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	expect(screen.queryByTestId('posture-signal')).toBeNull();
	const user = userEvent.setup();
	await user.click(screen.getByTestId('posture-service'));
	expect(screen.getByTestId('posture-signal').textContent).toContain('Signal gateway');
	// Describe only: nothing on this screen installs, downloads, or executes
	// a gateway — the invoke stub refuses any command beyond the posture's own.
});

/**
 * Posture C with a non-loopback URL is refused at the door and stores
 * NOTHING — the wire's local-only law, named.
 */
test('a_non_loopback_url_is_refused_and_stores_nothing', async () => {
	const user = userEvent.setup();
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	render(KernelGateHost);

	await screen.findByTestId('posture-screen');
	await user.click(screen.getByTestId('posture-external'));
	await user.type(screen.getByTestId('posture-url'), 'https://kernel.example.com:8765');
	await user.click(screen.getByTestId('posture-continue'));

	expect(await screen.findByTestId('posture-url-invalid')).toBeTruthy();
	expect(
		bridge.calls.filter((call) => call.command === 'store_service_posture')
	).toHaveLength(0);
});

/**
 * The store edition NEVER shows the posture screen: build_channel 'mas'
 * goes straight to today's behavior (the runtime half of the config
 * boundary that compiles the commands out of the mas build).
 */
test('the_mas_channel_never_shows_the_posture_screen', async () => {
	bridge.channel = 'mas';
	bridge.status = status({ consentGranted: true });
	bridge.posture = { posture: 'app', url: null, chosen: false };
	render(KernelGateHost);

	expect(await screen.findByTestId('host-content')).toBeTruthy();
	expect(screen.queryByTestId('posture-screen')).toBeNull();
	expect(bridge.calls.some((call) => call.command === 'service_posture')).toBe(false);
	expect(bridge.calls.filter((call) => call.command === 'kernel_start')).toHaveLength(1);
});

/**
 * The wizard makes NO network call anywhere in the posture flow: the only
 * transport is the IPC seam (source-scan over the two surfaces the wizard
 * owns).
 */
test('the_wizard_surfaces_hold_no_wire_transport', async () => {
	const { readFileSync } = await import('node:fs');
	for (const file of [
		'../src/lib/components/KernelGate.svelte',
		'../src/lib/components/settings/run-mode-section.svelte'
	]) {
		const source = readFileSync(new URL(file, import.meta.url), 'utf8');
		// fetch( appears ONLY in comments if at all; no XHR, no EventSource,
		// no import of the wire modules beyond the base accessor the gate
		// already owned (the posture phase itself never sends).
		expect(source).not.toMatch(/(?<!\/\/.*)\bfetch\s*\(/);
		expect(source).not.toContain('XMLHttpRequest');
		expect(source).not.toContain('new EventSource');
	}
});
