<script lang="ts">
	// The optional Signal gateway module (PLAN_MAS_FORK §1, the operator's
	// decision 2026-10-30): DESCRIBE + DETECT, nothing more. The gateway is a
	// SEPARATE, AGPL-licensed process the user installs themselves from the
	// website — this surface never installs, downloads, configures, or
	// executes it (App Store guideline 2.4.5(iv)), and the app is fully
	// functional without it (4.2.3(i)). The Rust core's bounded TCP probe of
	// the gateway's documented loopback address is the whole of the
	// "detection"; the address rides the DTO and is never guessed client-side.
	import { m } from '$paraglide/messages.js';
	import { onMount } from 'svelte';

	type GatewayStatus = { present: boolean; address: string };

	let present = $state<boolean | null>(null);
	let address = $state('');

	onMount(async () => {
		if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			const status = await invoke<GatewayStatus>('signal_gateway_status');
			present = status.present;
			address = status.address;
		} catch {
			// The core did not answer: "not detected" is the honest resting
			// state, never an error banner over an optional module.
			present = false;
		}
	});
</script>

<div class="grid gap-2" data-testid="settings-signal">
	<h3 class="text-sm font-semibold">{m['settings.signal.heading']()}</h3>
	{#if present === null}
		<p class="text-sm text-muted-foreground" aria-live="polite">
			{m['settings.signal.checking']()}
		</p>
	{:else if present}
		<p class="text-sm text-brand" role="status" data-testid="settings-signal-detected">
			{m['settings.signal.detected']()}
		</p>
	{:else}
		<p class="text-sm text-muted-foreground" data-testid="settings-signal-absent">
			{m['settings.signal.absent']()}
		</p>
	{/if}
	<p class="text-xs text-muted-foreground">{m['settings.signal.hint']()}</p>
</div>
