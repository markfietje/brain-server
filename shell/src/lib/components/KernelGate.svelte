<script lang="ts">
	// III-3 — the gate ABOVE the chrome. Detect → consent → start → ready, or a
	// named refusal with NO nav, NO app content and NO empty-state lie. The
	// version the gate expects is the sealed provenance's `x_api_version`,
	// read by the Rust core; the webview never derives it.
	//
	// In the PWA build there is no Tauri runtime: the serving origin owns the
	// wire and the gate bypasses immediately. It never invokes a command and
	// never blocks the page.
	import { onMount } from 'svelte';
	import { m } from '$paraglide/messages.js';
	import { refreshBearer } from '#lib/api/bootstrap.js';
	import { apiBase, SESSION_BASE_KEY } from '#lib/api/client.js';
	import { LOOPBACK_KERNEL_URL, type DetectDto } from '#lib/detection.js';
	import DetectionCards from '#lib/components/settings/detection-cards.svelte';
	import { Button } from '#lib/components/ui/button/index.js';

	interface KernelStatus {
		state: 'ready' | 'notRunning' | 'started' | 'alreadyRunning' | 'refused';
		kind?: string | null;
		expected?: string | null;
		observed?: string | null;
		sidecar?: string | null;
		dataRoot?: string | null;
		waitedMs?: number | null;
		lastError?: string | null;
		consentGranted: boolean;
		version?: string | null;
		pid?: number | null;
	}

	type GateState =
		| { phase: 'checking' }
		| { phase: 'consent'; dataRoot: string }
		| { phase: 'posture'; outcome: InstallDto | null; working: boolean }
		| { phase: 'starting' }
		| { phase: 'blocked'; reason: string }
		| { phase: 'ready' }
		| {
				phase: 'refused';
				kind: string;
				expected?: string;
				observed?: string;
				sidecar?: string;
				dataRoot?: string;
				waitedMs?: number;
				lastError?: string;
		  };

	// W-2 — the service posture's wire shapes (the Rust core owns every law;
	// this is what the DTOs look like over the IPC seam).
	interface PostureDto {
		posture: 'app' | 'service' | 'external';
		url?: string | null;
		chosen: boolean;
	}

	interface InstallDto {
		outcome: string;
		serviceLabel?: string | null;
		plistPath?: string | null;
		stoppedAppKernel?: boolean;
		holder?: string | null;
		expected?: string | null;
		observed?: string | null;
		kind?: string | null;
		error?: string | null;
	}

	let postureChoice = $state<'app' | 'service' | 'external'>('app');
	let externalUrl = $state('');
	let externalInvalid = $state(false);
	let posturePanel = $state<HTMLDivElement | null>(null);
	let detectReport = $state<DetectDto | null>(null);

	let { children } = $props();

	let gate = $state<GateState>({ phase: 'checking' });
	let panel = $state<HTMLDivElement | null>(null);

	function inTauri(): boolean {
		return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
	}

	function refusalText(kind: string): string {
		switch (kind) {
			case 'mismatch':
				return m['gate.refused.mismatch']();
			case 'foreign':
				return m['gate.refused.foreign']();
			case 'inconsistent':
				return m['gate.refused.inconsistent']();
			case 'missingSidecar':
				return m['gate.refused.missingSidecar']();
			case 'missingProvenance':
				return m['gate.refused.missingProvenance']();
			case 'tokenUnavailable':
				return m['gate.refused.tokenUnavailable']();
			case 'consentRefused':
				return m['gate.refused.consentRefused']();
			case 'insecureAuthFile':
				return m['gate.refused.insecureAuthFile']();
			case 'startTimeout':
				return m['gate.refused.startTimeout']();
			case 'spawnFailed':
				return m['gate.refused.spawnFailed']();
			case 'statusFailed':
				return m['gate.refused.statusFailed']();
			case 'startFailed':
				return m['gate.refused.startFailed']();
			default:
				return m['gate.refused.unknown']();
		}
	}

	function refusedFrom(dto: KernelStatus): GateState {
		return {
			phase: 'refused',
			kind: dto.kind ?? 'unknown',
			expected: dto.expected ?? undefined,
			observed: dto.observed ?? undefined,
			sidecar: dto.sidecar ?? undefined,
			dataRoot: dto.dataRoot ?? undefined,
			waitedMs: dto.waitedMs ?? undefined,
			lastError: dto.lastError ?? undefined
		};
	}

	async function check(): Promise<void> {
		if (!inTauri()) {
			// PWA: the serving origin owns the wire; never invoke Tauri here.
			gate = { phase: 'ready' };
			return;
		}
		gate = { phase: 'checking' };
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			// W-2 — posture C adopts its kernel BEFORE the local probe: the wire
			// base must point at the chosen kernel before the app can render.
			// (Direct channel only; the store edition never asks and never has
			// these commands — build_channel is the runtime half of the same
			// config boundary that compiles the module out of the mas build.)
			if ((await directChannel(invoke)) && (await adoptExternalIfChosen(invoke))) {
				return;
			}
			const status = (await invoke('kernel_status')) as KernelStatus;
			if (status.state === 'ready') {
				gate = { phase: 'ready' };
				return;
			}
			if (status.state === 'notRunning') {
				if (!status.consentGranted) {
					gate = { phase: 'consent', dataRoot: status.dataRoot ?? '' };
					return;
				}
				if (await directChannel(invoke)) {
					const marker = (await invoke('service_posture')) as PostureDto;
					if (!marker.chosen) {
						// The ONE decision, offered once. Everything else deferred
						// (the HIG's postpone law); A is pre-selected and the
						// primary button — unless the machine itself proposes
						// otherwise: W-T2's detections load beside the screen and
						// a kernel already running on 8765 pre-selects C (attach
						// is the honest default; the cards are if-branches over
						// the detection core's DTO — no model, no guess).
						gate = { phase: 'posture', outcome: null, working: false };
						detectReport = await loadDetections(invoke);
						return;
					}
					if (marker.posture === 'service') {
						await startService(invoke);
						return;
					}
				}
				await start(true);
				return;
			}
			gate = refusedFrom(status);
		} catch (error) {
			gate = {
				phase: 'refused',
				kind: 'statusFailed',
				lastError: error instanceof Error ? error.message : String(error)
			};
		}
	}

	type Invoker = typeof import('@tauri-apps/api/core').invoke;

	// W-T2 — the machine's report beside the ONE choice. An older core
	// without the command is no cards and A stands (today's behavior); the
	// store build never reaches this path (the direct-channel bypass above).
	async function loadDetections(invoke: Invoker): Promise<DetectDto | null> {
		try {
			const report = (await invoke('detect_cards')) as DetectDto;
			if (report.kernelOnPort.running) {
				postureChoice = 'external';
				externalUrl = LOOPBACK_KERNEL_URL;
			}
			return report;
		} catch {
			return null;
		}
	}

	async function directChannel(invoke: Invoker): Promise<boolean> {
		try {
			return (await invoke('build_channel')) === 'direct';
		} catch {
			// An older core without the marker: today's behavior stands.
			return false;
		}
	}

	/** Posture C, already chosen: point the wire at the chosen kernel. The
	 * base only ever changes ONCE (a reload re-enters with the marker and
	 * the base already agrees). Local kernels only, this release — the
	 * loopback law is the wire's own, named in the refusal below. */
	async function adoptExternalIfChosen(invoke: Invoker): Promise<boolean> {
		const marker = (await invoke('service_posture')) as PostureDto;
		if (!marker.chosen || marker.posture !== 'external' || !marker.url) return false;
		const normalized = marker.url.replace(/\/$/, '');
		if (apiBase() === normalized) {
			// The wire points at the chosen kernel; the local probe is not the
			// gate for this posture — the wire's own named faults diagnose it.
			gate = { phase: 'ready' };
			return true;
		}
		sessionStorage.setItem(SESSION_BASE_KEY, normalized);
		location.reload();
		return true;
	}

	/** Posture B's start: install is IDEMPOTENT (an already-bootstrapped
	 * agent comes back named as such), and the app then ATTACHES — it never
	 * spawns a kernel of its own while the service owns the port. */
	async function startService(invoke: Invoker): Promise<void> {
		gate = { phase: 'posture', outcome: null, working: true };
		const install = (await invoke('service_install')) as InstallDto;
		if (install.outcome === 'installed' || install.outcome === 'alreadyInstalled') {
			const status = (await invoke('kernel_status')) as KernelStatus;
			if (
				status.state === 'ready' ||
				status.state === 'started' ||
				status.state === 'alreadyRunning'
			) {
				gate = { phase: 'ready' };
				return;
			}
			gate = refusedFrom(status);
			return;
		}
		// The named refusal renders ON the decision screen — A and C are
		// still right there, and a held port names its holder.
		gate = { phase: 'posture', outcome: install, working: false };
	}

	/** The decision screen's Continue: every branch stores the choice FIRST
	 * (it is durable), then acts, and every ending is named. */
	async function choosePosture(): Promise<void> {
		if (!inTauri()) {
			gate = { phase: 'ready' };
			return;
		}
		const { invoke } = await import('@tauri-apps/api/core');
		if (postureChoice === 'app') {
			await invoke('store_service_posture', { posture: 'app' });
			await start(true);
			return;
		}
		if (postureChoice === 'service') {
			await invoke('store_service_posture', { posture: 'service' });
			await startService(invoke);
			return;
		}
		const url = externalUrl.trim();
		if (!isLoopbackUrl(url)) {
			externalInvalid = true;
			return;
		}
		await invoke('store_service_posture', { posture: 'external', url });
		gate = { phase: 'posture', outcome: null, working: false };
		await adoptExternalIfChosen(invoke);
	}

	/** The wire's standing local-only law (client.ts's isLoopbackUrl, mirrored
	 * here for the choice's door — the full check lives with the wire). */
	function isLoopbackUrl(raw: string): boolean {
		try {
			const parsed = new URL(raw);
			if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return false;
			const host = parsed.hostname;
			return host === '127.0.0.1' || host === 'localhost' || host === '[::1]' || host === '::1';
		} catch {
			return false;
		}
	}

	async function start(consent: boolean): Promise<void> {
		if (!inTauri()) {
			gate = { phase: 'ready' };
			return;
		}
		gate = { phase: 'starting' };
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			const result = (await invoke('kernel_start', { consent })) as KernelStatus;
			if (result.state === 'started' || result.state === 'alreadyRunning') {
				// The Rust side stored the bearer; this is the only token read
				// after a start. A start that could not make the token readable
				// refuses rather than rendering an app that cannot authenticate.
				const readable = await refreshBearer();
				if (!readable) {
					gate = {
						phase: 'refused',
						kind: 'tokenUnavailable',
						dataRoot: result.dataRoot ?? undefined
					};
					return;
				}
				gate = { phase: 'ready' };
				return;
			}
			gate = refusedFrom(result);
		} catch (error) {
			gate = {
				phase: 'refused',
				kind: 'startFailed',
				lastError: error instanceof Error ? error.message : String(error)
			};
		}
	}

	function cancel(): void {
		// Cancel is a BLOCKED state, never a fall-through: the Rust side has
		// created nothing (consent=false returns before create_dir_all).
		gate = { phase: 'blocked', reason: 'consentRefused' };
	}

	function onDialogKeydown(event: KeyboardEvent): void {
		trapFocus(event, panel);
	}

	function onPostureKeydown(event: KeyboardEvent): void {
		trapFocus(event, posturePanel);
	}

	function trapFocus(event: KeyboardEvent, host: HTMLDivElement | null): void {
		if (event.key === 'Escape') {
			event.preventDefault();
			cancel();
			return;
		}
		if (event.key !== 'Tab' || host === null) return;
		const focusable = Array.from(
			host.querySelectorAll<HTMLElement>(
				'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
			)
		).filter((element) => !element.hasAttribute('disabled'));
		if (focusable.length === 0) return;
		const first = focusable[0];
		const last = focusable[focusable.length - 1];
		if (first === undefined || last === undefined) return;
		if (event.shiftKey && document.activeElement === first) {
			event.preventDefault();
			last.focus();
		} else if (!event.shiftKey && document.activeElement === last) {
			event.preventDefault();
			first.focus();
		}
	}

	// Focus enters the dialog on open (and only then), so a keyboard user is
	// never left behind the gate. The posture screen's focus lands on the
	// PRIMARY choice — the pre-selected A (the HIG's default-first law).
	$effect(() => {
		if (gate.phase === 'consent' && panel !== null) {
			panel.querySelector<HTMLElement>('button:not([disabled])')?.focus();
		}
	});
	$effect(() => {
		if (gate.phase === 'posture' && posturePanel !== null) {
			posturePanel.querySelector<HTMLElement>('input:checked')?.focus();
		}
	});

	onMount(() => {
		void check();
	});
</script>

{#if gate.phase === 'ready'}
	{@render children()}
{:else if gate.phase === 'consent'}
	<div class="fixed inset-0 z-50 flex items-center justify-center bg-background p-6">
		<div
			bind:this={panel}
			role="dialog"
			aria-modal="true"
			aria-labelledby="kernel-consent-heading"
			aria-describedby="kernel-consent-body"
			tabindex="-1"
			class="w-full max-w-lg rounded-lg border border-border bg-card p-6 text-card-foreground shadow-lg"
			onkeydown={onDialogKeydown}
		>
			<h1 id="kernel-consent-heading" class="text-lg font-semibold">
				{m['gate.consent.heading']()}
			</h1>
			<p id="kernel-consent-body" class="mt-2 text-sm text-muted-foreground">
				{m['gate.consent.body']()}
			</p>
			<p class="mt-3 text-sm">{m['gate.consent.localOnly']()}</p>
			<p class="mt-1 text-sm">{m['gate.consent.modes']()}</p>
			<dl class="mt-4 grid gap-1 text-sm">
				<dt class="font-medium">{m['gate.consent.pathLabel']()}</dt>
				<dd><code data-testid="kernel-data-root">{gate.dataRoot}</code></dd>
			</dl>
			<div class="mt-6 flex justify-end gap-2">
				<Button onclick={() => void start(true)}>{m['gate.consent.continue']()}</Button>
				<Button variant="outline" onclick={cancel}>{m['gate.consent.cancel']()}</Button>
			</div>
		</div>
	</div>
{:else if gate.phase === 'posture'}
	<div class="fixed inset-0 z-50 flex items-center justify-center bg-background p-6">
		<div
			bind:this={posturePanel}
			role="dialog"
			aria-modal="true"
			aria-labelledby="posture-heading"
			tabindex="-1"
			class="w-full max-w-lg rounded-lg border border-border bg-card p-6 text-card-foreground shadow-lg"
			onkeydown={onPostureKeydown}
			data-testid="posture-screen"
		>
			<h1 id="posture-heading" class="text-lg font-semibold">{m['posture.heading']()}</h1>
			<DetectionCards report={detectReport} />
			<fieldset class="mt-4 grid gap-2" data-testid="posture-choices">
				<legend class="sr-only">{m['posture.heading']()}</legend>
				<label
					class="flex items-start gap-3 rounded-md border border-border p-3 has-[:checked]:border-brand"
				>
					<input
						type="radio"
						name="posture"
						value="app"
						bind:group={postureChoice}
						class="mt-1"
						data-testid="posture-app"
					/>
					<span>
						<span class="block text-sm font-medium">{m['posture.a.title']()}</span>
						<span class="block text-sm text-muted-foreground">{m['posture.a.detail']()}</span>
					</span>
				</label>
				<label
					class="flex items-start gap-3 rounded-md border border-border p-3 has-[:checked]:border-brand"
				>
					<input
						type="radio"
						name="posture"
						value="service"
						bind:group={postureChoice}
						class="mt-1"
						data-testid="posture-service"
					/>
					<span>
						<span class="block text-sm font-medium">{m['posture.b.title']()}</span>
						<span class="block text-sm text-muted-foreground">{m['posture.b.detail']()}</span>
					</span>
				</label>
				<label
					class="flex items-start gap-3 rounded-md border border-border p-3 has-[:checked]:border-brand"
				>
					<input
						type="radio"
						name="posture"
						value="external"
						bind:group={postureChoice}
						class="mt-1"
						data-testid="posture-external"
					/>
					<span>
						<span class="block text-sm font-medium">{m['posture.c.title']()}</span>
						<span class="block text-sm text-muted-foreground">{m['posture.c.detail']()}</span>
					</span>
				</label>
			</fieldset>
			{#if postureChoice === 'external'}
				<div class="mt-3 grid gap-1">
					<label for="posture-url" class="text-sm font-medium">
						{m['posture.c.urlLabel']()}
					</label>
					<input
						id="posture-url"
						data-testid="posture-url"
						bind:value={externalUrl}
						placeholder="http://127.0.0.1:8765"
						class="rounded-md border border-border bg-background px-3 py-2 text-sm"
					/>
					{#if externalInvalid}
						<p class="text-sm text-destructive" role="alert" data-testid="posture-url-invalid">
							{m['posture.c.urlInvalid']()}
						</p>
					{/if}
				</div>
			{/if}
			{#if postureChoice === 'service'}
				<p class="mt-3 text-xs text-muted-foreground" data-testid="posture-signal">
					{m['posture.signal']()}
				</p>
			{/if}
			{#if gate.working}
				<p class="mt-4 text-sm text-muted-foreground" aria-live="polite" data-testid="posture-working">
					{m['posture.installing']()}
				</p>
			{/if}
			{#if gate.outcome !== null}
				<div class="mt-4" data-testid="posture-outcome">
					{#if gate.outcome.outcome === 'portHeld'}
						<p class="text-sm" role="alert">
							{m['posture.outcome.portHeld']({ holder: gate.outcome.holder ?? '' })}
						</p>
					{:else}
						<p class="text-sm" role="alert">
							{m['posture.outcome.refused']({ error: gate.outcome.error ?? (gate.outcome.kind ?? '') })}
						</p>
					{/if}
				</div>
			{/if}
			<div class="mt-6 flex justify-end gap-2">
				<Button
					onclick={() => void choosePosture()}
					disabled={gate.working}
					data-testid="posture-continue"
				>
					{m['posture.continue']()}
				</Button>
			</div>
		</div>
	</div>
{:else if gate.phase === 'refused'}
	<main class="mx-auto w-full max-w-2xl px-5 py-10" data-testid="kernel-refusal">
		<h1 class="text-lg font-semibold">{m['gate.refused.heading']()}</h1>
		<p class="mt-2 text-sm" role="alert">{refusalText(gate.kind)}</p>
		<dl class="mt-4 grid gap-2 text-sm">
			{#if gate.expected}
				<div>
					<dt class="font-medium">{m['gate.refused.expected']()}</dt>
					<dd><code>{gate.expected}</code></dd>
				</div>
			{/if}
			{#if gate.observed}
				<div>
					<dt class="font-medium">{m['gate.refused.observed']()}</dt>
					<dd><code>{gate.observed}</code></dd>
				</div>
			{/if}
			{#if gate.sidecar}
				<div>
					<dt class="font-medium">{m['gate.refused.sidecar']()}</dt>
					<dd><code>{gate.sidecar}</code></dd>
				</div>
			{/if}
			{#if gate.dataRoot}
				<div>
					<dt class="font-medium">{m['gate.refused.dataRoot']()}</dt>
					<dd><code>{gate.dataRoot}</code></dd>
				</div>
			{/if}
			{#if gate.waitedMs !== undefined}
				<div>
					<dt class="font-medium">{m['gate.refused.waited']()}</dt>
					<dd><code>{gate.waitedMs} ms</code></dd>
				</div>
			{/if}
			{#if gate.lastError}
				<div>
					<dt class="font-medium">{m['gate.refused.lastError']()}</dt>
					<dd><code>{gate.lastError}</code></dd>
				</div>
			{/if}
		</dl>
		<div class="mt-6">
			<Button onclick={() => void check()}>{m['gate.refused.retry']()}</Button>
		</div>
	</main>
{:else if gate.phase === 'blocked'}
	<main class="mx-auto w-full max-w-2xl px-5 py-10" data-testid="kernel-blocked">
		<h1 class="text-lg font-semibold">{m['gate.refused.heading']()}</h1>
		<p class="mt-2 text-sm" role="alert">{m['gate.consent.cancelled']()}</p>
		<div class="mt-6">
			<Button onclick={() => void check()}>{m['gate.refused.retry']()}</Button>
		</div>
	</main>
{:else}
	<main class="mx-auto w-full max-w-2xl px-5 py-10" aria-busy="true" data-testid="kernel-starting">
		<p class="text-sm text-muted-foreground">
			{gate.phase === 'checking' ? m['gate.checking']() : m['gate.starting']()}
		</p>
	</main>
{/if}
