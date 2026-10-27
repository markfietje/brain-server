<script lang="ts">
	// The kernel token (III-2; Tauri only — the section renders NOTHING in a
	// plain browser). The writer the login keychain needs. The token is
	// entered here, written by the Rust core, and never retained in the
	// webview beyond the IPC call: the input is cleared after Save AND after
	// a failure, and the status line says configured/anonymous without naming
	// which source supplied it.
	import { m } from '$paraglide/messages.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { refreshBearer } from '#lib/api/bootstrap.js';
	import { noteCredentialChange } from '#lib/shell/frame.svelte.js';

	const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
	let tokenInput = $state('');
	let tokenConfigured = $state(false);
	let tokenState = $state<
		'idle' | 'saved' | 'rotated' | 'pending' | 'stale' | 'cleared' | 'empty' | 'error'
	>('idle');

	// The dialog mounts its content only while open, so mount IS open: the
	// status is read once per opening (the frame's old settingsOpen-keyed
	// effect, moved to where the state lives).
	$effect(() => {
		if (inTauri) {
			tokenState = 'idle';
			void refreshTokenStatus();
		}
	});

	async function refreshTokenStatus(): Promise<void> {
		tokenConfigured = await refreshBearer();
	}

	async function saveToken(): Promise<void> {
		const token = tokenInput;
		if (!token.trim()) {
			tokenState = 'empty';
			return;
		}
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			// The core answers with the ROTATION's outcome: the keychain holds
			// the token either way, but a running kernel reads its bearer from
			// the data root's auth-token. Reporting "saved" for a rotation
			// that did not reach the kernel would be a lie the user acts on.
			const outcome = (await invoke('store_kernel_token', { token })) as {
				kernelSynced?: boolean;
				dataRootMissing?: boolean;
				error?: string | null;
			} | null;
			if (outcome?.error) {
				tokenState = 'stale';
			} else if (outcome?.kernelSynced) {
				tokenState = 'rotated';
			} else if (outcome?.dataRootMissing) {
				tokenState = 'pending';
			} else {
				tokenState = 'saved';
			}
		} catch {
			tokenState = 'error';
		} finally {
			tokenInput = '';
			await refreshTokenStatus();
			// A credential change is the one moment the frame's facts can go
			// from refused to answered (or back) — the frame re-runs its three
			// glance reads on the epoch (measured 2026-10-24: a token saved
			// over a stale one left the sidebar reading "integrity not
			// verified" until relaunch).
			if (tokenState !== 'empty' && tokenState !== 'error') noteCredentialChange();
		}
	}

	async function clearToken(): Promise<void> {
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			await invoke('clear_kernel_token');
			tokenState = 'cleared';
		} catch {
			tokenState = 'error';
		} finally {
			tokenInput = '';
			await refreshTokenStatus();
			if (tokenState === 'cleared') noteCredentialChange();
		}
	}
</script>

{#if inTauri}
	<div class="grid gap-2">
		<h3 class="text-sm font-semibold">{m["settings.token.heading"]()}</h3>
		<Label for="setting-token">{m["settings.token.label"]()}</Label>
		<div class="flex flex-wrap gap-2">
			<Input
				id="setting-token"
				type="password"
				autocomplete="off"
				spellcheck="false"
				autocapitalize="none"
				autocorrect="off"
				aria-describedby="setting-token-hint"
				bind:value={tokenInput}
			/>
			<Button onclick={saveToken}>{m["settings.token.save"]()}</Button>
			<Button variant="outline" onclick={clearToken}>
				{m["settings.token.clear"]()}
			</Button>
		</div>
		<p id="setting-token-hint" class="text-xs text-muted-foreground">
			{m["settings.token.hint"]()}
		</p>
		<p class="text-sm text-muted-foreground" role="status">
			{tokenConfigured
				? m["settings.token.status.configured"]()
				: m["settings.token.status.anonymous"]()}
		</p>
		{#if tokenState === 'saved'}
			<p class="text-sm text-brand" role="status">{m["settings.token.saved"]()}</p>
		{:else if tokenState === 'rotated'}
			<p class="text-sm text-brand" role="status">{m["settings.token.rotated"]()}</p>
		{:else if tokenState === 'pending'}
			<p class="text-sm text-brand" role="status">{m["settings.token.pending"]()}</p>
		{:else if tokenState === 'stale'}
			<p class="text-sm text-destructive" role="alert">{m["settings.token.stale"]()}</p>
		{:else if tokenState === 'cleared'}
			<p class="text-sm text-brand" role="status">{m["settings.token.cleared"]()}</p>
		{:else if tokenState === 'empty'}
			<p class="text-sm text-destructive" role="alert">{m["settings.token.empty"]()}</p>
		{:else if tokenState === 'error'}
			<p class="text-sm text-destructive" role="alert">{m["settings.token.error"]()}</p>
		{/if}
	</div>
{/if}
