<script lang="ts">
	import { cn } from '#lib/utils.js';
	import type { HTMLAttributes } from 'svelte/elements';

	// A native progressbar. `role="progressbar"` with the aria value triple is
	// the entire a11y contract for a determinate bar — no machine needed.
	// `indeterminate` is supported because a bar with no measurable value is
	// still a legitimate state, and it is the one case the old binding could
	// not express.
	let {
		ref = $bindable(null),
		class: className,
		max = 100,
		value = null,
		indeterminate = false,
		"data-slot": dataSlot = 'progress',
		...restProps
	}: HTMLAttributes<HTMLElement> & {
		ref?: HTMLElement | null;
		max?: number | null;
		value?: number | null;
		indeterminate?: boolean;
	} = $props();

	const percent = $derived(
		indeterminate || value == null ? 0 : Math.min(100, Math.max(0, (100 * value) / (max || 1)))
	);
</script>

<div
	bind:this={ref}
	role="progressbar"
	aria-valuemin={indeterminate ? undefined : 0}
	aria-valuemax={indeterminate ? undefined : (max ?? undefined)}
	aria-valuenow={indeterminate ? undefined : (value ?? undefined)}
	data-slot={dataSlot}
	class={cn("bg-muted h-1.5 rounded-full relative flex w-full items-center overflow-x-hidden", className)}
	{...restProps}
>
	<div
		data-slot="progress-indicator"
		class="bg-primary size-full flex-1 transition-all"
		style="transform: translateX(-{100 - percent}%)"
	></div>
</div>