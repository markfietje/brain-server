<script lang="ts">
	// The ⌘K command palette: @zag-js/combobox for filtering + active-descendant
	// keyboard navigation, @zag-js/dialog for the modality (focus trap, escape,
	// scroll lock). Both are the same machines bits-ui wrapped, used directly,
	// so the app no longer pulls `runed` — stale since 2025-12, and the last
	// thing in the tree still declaring a SvelteKit 2 peer.
	//
	// Highlighting is driven by the machine's own `data-highlighted` attribute
	// rather than a JS branch, so active-item styling can never drift out of
	// sync with what the machine considers active.
	import * as combobox from '@zag-js/combobox';
	import * as dialog from '@zag-js/dialog';
	import { normalizeProps, portal, useMachine } from '@zag-js/svelte';
	import { cn } from '#lib/utils.js';
	import type { Component } from 'svelte';

	export type PaletteItem = {
		value: string;
		label: string;
		Icon?: Component;
		onSelect: () => void;
	};
	export type PaletteGroup = { heading: string; items: PaletteItem[] };

	let {
		open = $bindable(false),
		groups,
		title,
		placeholder,
		emptyText,
		class: className
	}: {
		open?: boolean;
		groups: PaletteGroup[];
		title: string;
		placeholder: string;
		emptyText: string;
		class?: string;
	} = $props();

	// One component id, suffixed per machine — Svelte allows a single
	// `$props.id()` call, and each machine still needs a distinct DOM id root.
	const uid = $props.id();

	// Filtering is client-side and case-insensitive; the collection is rebuilt
	// from the surviving items whenever the query changes.
	let query = $state('');
	const filtered = $derived.by(() => {
		const q = query.trim().toLowerCase();
		if (q === '') return groups;
		return groups
			.map((g) => ({ ...g, items: g.items.filter((i) => i.label.toLowerCase().includes(q)) }))
			.filter((g) => g.items.length > 0);
	});
	const items = $derived(filtered.flatMap((g) => g.items));

	const collection = $derived(
		combobox.collection({
			items,
			itemToValue: (item: PaletteItem) => item.value,
			itemToString: (item: PaletteItem) => item.label
		})
	);

	function runSelected(value: string | string[]) {
		const chosen = Array.isArray(value) ? value[0] : value;
		const hit = groups.flatMap((g) => g.items).find((i) => i.value === chosen);
		hit?.onSelect();
	}

	const comboService = useMachine(combobox.machine, {
		id: `${uid}-combo`,
		get collection() {
			return collection;
		},
		open: true,
		// The first surviving item is pre-selected, so ⌘K -> type -> Enter
		// lands on the top result with no extra arrow press — the behaviour a
		// command palette is expected to have (bits-ui's Command did this by
		// default). NOTE: `autoHighlight` is NOT a boolean on this machine; it
		// is computed as `inputBehavior === 'autohighlight'`, so setting the
		// boolean silently does nothing.
		inputBehavior: 'autohighlight',
		onInputValueChange({ inputValue }) {
			query = inputValue;
		},
		onValueChange({ value }) {
			runSelected(value);
			query = '';
		}
	});
	const combo = $derived(combobox.connect(comboService, normalizeProps));

	const dialogService = useMachine(dialog.machine, {
		id: `${uid}-dialog`,
		get open() {
			return open;
		},
		onOpenChange(details) {
			open = details.open;
			if (details.open) query = '';
		}
	});
	const dlg = $derived(dialog.connect(dialogService, normalizeProps));
</script>

{#if dlg.open}
	<div
		use:portal
		{...dlg.getBackdropProps()}
		class="fixed inset-0 z-50 bg-black/50 data-[state=open]:animate-fade-in"
	></div>
	<div
		use:portal
		{...dlg.getPositionerProps()}
		class="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[16vh]"
	>
		<div
			{...dlg.getContentProps()}
			class={cn(
				"bg-popover text-popover-foreground ring-foreground/10 flex w-full max-w-lg flex-col overflow-hidden rounded-xl text-sm ring-1 outline-none",
				className
			)}
		>
			<h2 {...dlg.getTitleProps()} class="sr-only">{title}</h2>

			<div {...combo.getControlProps()} class="border-b">
				<input
					{...combo.getInputProps()}
					data-slot="command-input"
					class="h-11 w-full bg-transparent px-4 text-sm outline-none placeholder:text-muted-foreground"
					{placeholder}
					autocomplete="off"
					spellcheck="false"
				/>
			</div>

			<div {...combo.getPositionerProps()} class="max-h-72 overflow-x-hidden overflow-y-auto">
				<div {...combo.getContentProps()}>
					<ul {...combo.getListProps()} class="flex flex-col p-1" role="listbox">
						{#each filtered as group (group.heading)}
							<li {...combo.getItemGroupProps({ id: group.heading })} role="presentation">
								<span
									class="text-muted-foreground block px-2 py-1.5 text-xs font-medium"
								>
									{group.heading}
								</span>
								<ul role="group" class="flex flex-col">
									{#each group.items as item (item.value)}
										<li
											{...combo.getItemProps({ item })}
											data-slot="command-item"
											class="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 select-none data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground"
										>
											{#if item.Icon}
												<item.Icon aria-hidden="true" class="size-4 shrink-0 opacity-70" />
											{/if}
											{item.label}
										</li>
									{/each}
								</ul>
							</li>
						{/each}
						{#if items.length === 0}
							<li class="px-2 py-4 text-center text-muted-foreground">{emptyText}</li>
						{/if}
					</ul>
				</div>
			</div>
		</div>
	</div>
{/if}