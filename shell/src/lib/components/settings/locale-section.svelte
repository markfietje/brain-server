<script lang="ts">
	// The session locale's picker. The state lives in `#lib/shell/locale` —
	// the frame's relative session times read the SAME source, so the picker
	// and the clock cannot disagree.
	import { m } from '$paraglide/messages.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { LOCALES, chooseLocale, currentLocaleCode } from '#lib/shell/locale.svelte.js';

	const currentLocale = $derived(currentLocaleCode());

	function onSelect(event: Event): void {
		chooseLocale((event.currentTarget as HTMLSelectElement).value);
	}
</script>

<div class="grid gap-2">
	<Label for="setting-locale">{m["settings.locale.label"]()}</Label>
	<!-- The native select (the open list stays the platform's own popup —
	     the native-first control law). -->
	<select id="setting-locale" value={currentLocale} onchange={onSelect}>
		{#each LOCALES as l (l.code)}
			<option value={l.code}>{l.name}</option>
		{/each}
	</select>
</div>
