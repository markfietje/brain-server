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
			const status = (await invoke('kernel_status')) as KernelStatus;
			if (status.state === 'ready') {
				gate = { phase: 'ready' };
				return;
			}
			if (status.state === 'notRunning') {
				if (status.consentGranted) {
					await start(true);
				} else {
					gate = { phase: 'consent', dataRoot: status.dataRoot ?? '' };
				}
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
		if (event.key === 'Escape') {
			event.preventDefault();
			cancel();
			return;
		}
		if (event.key !== 'Tab' || panel === null) return;
		const focusable = Array.from(
			panel.querySelectorAll<HTMLElement>(
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
	// never left behind the gate.
	$effect(() => {
		if (gate.phase === 'consent' && panel !== null) {
			panel.querySelector<HTMLElement>('button:not([disabled])')?.focus();
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
