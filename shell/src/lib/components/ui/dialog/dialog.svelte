<script lang="ts">
	// A dialog on @zag-js/dialog, the same state machine bits-ui was built on —
	// used directly now, so the app carries no `runed` (stale since 2025-12,
	// and its `@sveltejs/kit: ^2` peer was the last thing blocking SvelteKit 3).
	//
	// Zag is framework-agnostic: `useMachine` owns the machine, `connect` maps
	// it to a props object, and `portal` does what a bits-ui `Dialog.Portal`
	// did. Focus trap, escape handling, scroll lock, inert-outside and the
	// aria wiring are the machine's, not ours.
	import * as dialog from '@zag-js/dialog';
	import { mergeProps, normalizeProps, portal, useMachine } from '@zag-js/svelte';
	import XIcon from '@lucide/svelte/icons/x';
	import { Button } from '#lib/components/ui/button/index.js';
	import { cn } from '#lib/utils.js';
	import type { Snippet } from 'svelte';

	let {
		open = $bindable(false),
		title,
		description,
		class: className,
		showCloseButton = true,
		closeLabel = 'Close',
		children
	}: {
		open?: boolean;
		title: string;
		description?: string;
		class?: string;
		showCloseButton?: boolean;
		closeLabel?: string;
		children?: Snippet;
	} = $props();

	const uid = $props.id();
	const service = useMachine(dialog.machine, {
		id: uid,
		get open() {
			return open;
		},
		onOpenChange(details) {
			open = details.open;
		}
	});
	const api = $derived(dialog.connect(service, normalizeProps));
	// mergeProps (not a raw spread) keeps the machine's button props and the
	// design-system props in one object without the union TS cannot reduce.
	const closeProps = $derived(
		mergeProps([api.getCloseTriggerProps(), { variant: 'ghost', size: 'icon-sm', class: 'absolute top-4 right-4' }])
	);
</script>

{#if api.open}
	<div
		use:portal
		{...api.getBackdropProps()}
		class="fixed inset-0 z-50 bg-black/50 data-[state=open]:animate-in data-[state=open]:fade-in-0"
	></div>
	<div
		use:portal
		{...api.getPositionerProps()}
		class="fixed inset-0 z-50 flex items-center justify-center p-4"
	>
		<div
			{...api.getContentProps()}
			class={cn(
				"bg-popover text-popover-foreground ring-foreground/10 grid max-w-[calc(100%_-_2rem)] w-full gap-6 rounded-xl p-6 text-sm ring-1 outline-none sm:max-w-md",
				className
			)}
		>
			<h2 {...api.getTitleProps()} class="text-base leading-none font-semibold">
				{title}
			</h2>
			{#if description}
				<p {...api.getDescriptionProps()} class="text-muted-foreground text-sm">
					{description}
				</p>
			{/if}
			{@render children?.()}
			{#if showCloseButton}
				<Button {...closeProps} data-slot="dialog-close">
					<XIcon />
					<span class="sr-only">{closeLabel}</span>
				</Button>
			{/if}
		</div>
	</div>
{/if}