<script lang="ts">
	import { cn } from '#lib/utils.js';
	import type { HTMLLabelAttributes } from 'svelte/elements';

	// A native <label>. bits-ui wrapped this purely to forward a11y props it
	// did not need; the element already carries the semantics. `children` is
	// rendered explicitly — a Svelte 5 component has to ask for it, and
	// omitting it silently produces an EMPTY label, which fails axe
	// `label` (a labelled control needs an accessible NAME, not just a `for`).
	import type { Snippet } from 'svelte';

	let {
		ref = $bindable(null),
		class: className,
		children,
		...restProps
	}: HTMLLabelAttributes & { ref?: HTMLElement | null; children?: Snippet } = $props();
</script>

<label
	bind:this={ref}
	data-slot="label"
	class={cn(
		"gap-2 text-sm leading-none font-medium group-data-[disabled=true]:opacity-50 peer-disabled:opacity-50 flex items-center select-none group-data-[disabled=true]:pointer-events-none peer-disabled:cursor-not-allowed",
		className
	)}
	{...restProps}
	>{@render children?.()}</label
>