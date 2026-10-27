<script lang="ts">
	// The software update (III-4; Tauri only — the section renders NOTHING in
	// a plain browser). The Rust core owns the check and the install; the
	// webview is granted no updater permission and only ever sees a DTO.
	// `unreachable` is its OWN state: an endpoint that cannot be reached is
	// never "you are up to date", and a refusal names its reason rather than
	// reading like an answer.
	import { m } from '$paraglide/messages.js';
	import { Button } from '#lib/components/ui/button/index.js';

	const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

	type UpdateState =
		| 'idle'
		| 'checking'
		| 'current'
		| 'available'
		| 'unreachable'
		| 'refused'
		| 'error';
	let updateState = $state<UpdateState>('idle');
	let updateCurrentVersion = $state('');
	let updateCurrentKernel = $state('');
	let updateOfferedVersion = $state('');
	let updateOfferedKernel = $state('');
	let updateReason = $state('');

	type UpdateOutcome = {
		state?: string;
		currentVersion?: string;
		currentKernel?: string | null;
		offeredVersion?: string | null;
		offeredKernel?: string | null;
		expected?: string | null;
		observed?: string | null;
		lastError?: string | null;
	};

	/**
	 * Apply what the core answered. The install answers with the SAME DTO as
	 * the check, and its answer is the one that matters: a refusal names its
	 * reason, and dropping it would render a refusal as nothing at all.
	 */
	function applyOutcome(outcome: UpdateOutcome | null): void {
		updateCurrentVersion = outcome?.currentVersion ?? updateCurrentVersion;
		updateCurrentKernel = outcome?.currentKernel ?? updateCurrentKernel;
		updateOfferedVersion = outcome?.offeredVersion ?? '';
		updateOfferedKernel = outcome?.offeredKernel ?? '';
		updateReason = outcome?.lastError ?? '';
		const state = outcome?.state;
		if (state === 'available') {
			updateState = 'available';
		} else if (state === 'current') {
			updateState = 'current';
		} else if (state === 'unreachable') {
			updateState = 'unreachable';
		} else if (state === 'refused') {
			updateState = 'refused';
		} else {
			updateState = 'error';
		}
	}

	async function checkForUpdate(): Promise<void> {
		updateState = 'checking';
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			applyOutcome((await invoke('update_check')) as UpdateOutcome | null);
		} catch {
			updateState = 'error';
		}
	}

	async function installUpdate(): Promise<void> {
		updateState = 'checking';
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			applyOutcome((await invoke('update_install')) as UpdateOutcome | null);
		} catch {
			updateState = 'error';
		}
	}

	// The dialog mounts its content only while open, so mount IS open: the
	// panel states the CURRENT version and the current kernel version, and
	// only the core knows either of them (the kernel's comes from the sealed
	// provenance). Opening the panel asks the core once — the old
	// settings-button call, moved to where the state lives.
	$effect(() => {
		if (inTauri) void checkForUpdate();
	});
</script>

{#if inTauri}
	<div class="grid gap-2">
		<h3 class="text-sm font-semibold">{m["settings.update.heading"]()}</h3>
		<p class="text-xs text-muted-foreground">
			{m["settings.update.current"]({
				version: updateCurrentVersion,
				kernel: updateCurrentKernel
			})}
		</p>
		<div class="flex flex-wrap gap-2">
			<Button onclick={checkForUpdate} disabled={updateState === 'checking'}>
				{m["settings.update.check"]()}
			</Button>
			{#if updateState === 'available'}
				<Button onclick={installUpdate}>{m["settings.update.install"]()}</Button>
			{/if}
		</div>
		{#if updateState === 'current'}
			<p class="text-sm text-brand" role="status">{m["settings.update.uptodate"]()}</p>
		{:else if updateState === 'available'}
			<p class="text-sm text-brand" role="status">
				{m["settings.update.available"]({
					version: updateOfferedVersion,
					kernel: updateOfferedKernel
				})}
			</p>
		{:else if updateState === 'unreachable'}
			<p class="text-sm text-destructive" role="alert">
				{m["settings.update.unreachable"]()}
			</p>
		{:else if updateState === 'refused'}
			<p class="text-sm text-destructive" role="alert">
				{m["settings.update.refused"]({ reason: updateReason })}
			</p>
		{:else if updateState === 'error'}
			<p class="text-sm text-destructive" role="alert">{m["settings.update.error"]()}</p>
		{/if}
	</div>
{/if}
