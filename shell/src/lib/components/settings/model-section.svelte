<script lang="ts">
	// The retrieval model's download surface (the operator's UX call,
	// 2026-10-30: "I want to see the download embedding model (name of the
	// model)"). BIND-FIRST (the S-4 clean-machine finding): the kernel binds
	// and serves WHILE the first-run HuggingFace download runs in a
	// background thread — so this section names the posture truthfully:
	// `loading` (the one-time fetch, with the model's name), `ready`, or
	// `failed` (with the named error). The READ lives in
	// `#lib/api/kernel-facts.ts` (the route-parity law: typed-wire reads
	// belong to api modules); the poll runs ONLY while loading — 1.5 s
	// cadence, stopped at ready/failed — never a standing request.
	import { m } from '$paraglide/messages.js';
	import { fetchModelState } from '#lib/api/kernel-facts.js';
	import { onMount } from 'svelte';

	let modelId = $state<string | null>(null);
	let modelPosture = $state<'loading' | 'ready' | 'failed'>('loading');
	let modelError = $state<string | null>(null);
	let mounted = $state(true);

	onMount(() => {
		mounted = true;
		void poll();
		return () => {
			mounted = false;
		};
	});

	async function poll(): Promise<void> {
		while (mounted && modelPosture === 'loading') {
			const read = await fetchModelState();
			if (read.state !== 'unknown') {
				modelPosture = read.state;
				modelId = read.model ?? modelId;
				modelError = read.error;
				return; // terminal state — the poll stops
			}
			if (read.model) modelId = read.model;
			await new Promise((r) => setTimeout(r, 1500));
		}
	}
</script>

<div class="grid gap-2">
	<h3 class="text-sm font-semibold">{m['settings.model.heading']()}</h3>
	<p class="flex items-center gap-2 text-sm" data-testid="settings-model-state">
		{#if modelPosture === 'loading'}
			<span
				class="inline-block size-3 shrink-0 animate-spin rounded-full border-2 border-muted-foreground/30 border-t-foreground"
				aria-hidden="true"></span>
			{m['settings.model.loading']({ model: modelId ?? m['settings.model.fallback_name']() })}
		{:else if modelPosture === 'ready'}
			<span class="inline-block size-2 shrink-0 rounded-full bg-ok" aria-hidden="true"></span>
			{m['settings.model.ready']({ model: modelId ?? m['settings.model.fallback_name']() })}
		{:else}
			<span class="inline-block size-2 shrink-0 rounded-full bg-destructive" aria-hidden="true"></span>
			{m['settings.model.failed']()}
		{/if}
	</p>
	<p class="text-xs text-muted-foreground">{m['settings.model.hint']()}</p>
	{#if modelPosture === 'failed' && modelError}
		<p class="text-xs text-destructive" role="alert">{modelError}</p>
	{/if}
</div>
