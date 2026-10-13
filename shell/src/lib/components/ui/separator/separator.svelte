<script lang="ts">
	import { cn } from '#lib/utils.js';
	import type { HTMLAttributes } from 'svelte/elements';

	// A native separator. `role="separator"` plus `aria-orientation` is the
	// whole contract; there is no interactive state, so no machine is needed.
	let {
		ref = $bindable(null),
		class: className,
		orientation = 'horizontal',
		decorative = false,
		'data-slot': dataSlot = 'separator',
		...restProps
	}: HTMLAttributes<HTMLElement> & {
		ref?: HTMLElement | null;
		orientation?: 'horizontal' | 'vertical';
		decorative?: boolean;
	} = $props();
</script>

<div
	bind:this={ref}
	role={decorative ? 'none' : 'separator'}
	aria-orientation={decorative ? undefined : orientation}
	aria-hidden={decorative ? 'true' : undefined}
	data-slot={dataSlot}
	data-orientation={orientation}
	class={cn(
		'shrink-0 bg-border data-[orientation=horizontal]:h-px data-[orientation=horizontal]:w-full data-[orientation=vertical]:w-px data-[orientation=vertical]:self-stretch',
		className
	)}
	{...restProps}
></div>