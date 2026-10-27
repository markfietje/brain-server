<script lang="ts">
	// The kernel-origin override (loopback-enforced), SESSION-LOCAL: the
	// section's own state — nothing crosses the section boundary.
	import { m } from '$paraglide/messages.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { DEFAULT_BASE, apiBase, setApiBase } from '#lib/api/client.js';

	let originInput = $state(apiBase());
	let originState = $state<'idle' | 'saved' | 'invalid'>('idle');

	function saveOrigin(): void {
		if (setApiBase(originInput)) {
			originInput = apiBase();
			originState = 'saved';
		} else {
			originState = 'invalid';
		}
	}
</script>

<div class="grid gap-2">
	<Label for="setting-origin">{m["settings.origin.label"]()}</Label>
	<div class="flex gap-2">
		<Input id="setting-origin" bind:value={originInput} placeholder={DEFAULT_BASE} />
		<Button onclick={saveOrigin}>{m["settings.origin.save"]()}</Button>
	</div>
	<p class="text-xs text-muted-foreground">{m["settings.origin.hint"]()}</p>
	{#if originState === 'saved'}
		<p class="text-sm text-brand" role="status">{m["settings.origin.saved"]()}</p>
	{:else if originState === 'invalid'}
		<p class="text-sm text-destructive" role="alert">{m["settings.origin.invalid"]()}</p>
	{/if}
</div>
