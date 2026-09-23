<script lang="ts">
	// The app chrome (M2-S1): top bar + nav over the child routes, the ⌘K
	// command palette (in-page; NO OS global shortcut — plan §7.4), and the
	// session settings panel (kernel-origin override + locale). The origin
	// override is LOOPBACK-ENFORCED client-side (client.ts refuses anything
	// else — fail-safe, never fail-open) and persists SESSION-LOCAL only.
	// Nav marks the active route with aria-current — never color alone.
	import { _, locale, getLocaleFromNavigator } from 'svelte-i18n';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import HouseIcon from '@lucide/svelte/icons/house';
	import ClipboardListIcon from '@lucide/svelte/icons/clipboard-list';
	import SearchIcon from '@lucide/svelte/icons/search';
	import HistoryIcon from '@lucide/svelte/icons/history';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import CommandIcon from '@lucide/svelte/icons/command';
	import { Button } from '$lib/components/ui/button';
	import * as Command from '$lib/components/ui/command';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import { DEFAULT_BASE, apiBase, setApiBase } from '$lib/api/client';

	const NAV = [
		{ route: '/overview', key: 'nav.overview', Icon: HouseIcon },
		{ route: '/', key: 'nav.wizard', Icon: ClipboardListIcon },
		{ route: '/search', key: 'nav.search', Icon: SearchIcon },
		{ route: '/recall', key: 'nav.recall', Icon: HistoryIcon }
	] as const;

	let paletteOpen = $state(false);
	let settingsOpen = $state(false);

	let { children } = $props();

	function isCurrent(href: string): boolean {
		const path = page.url.pathname;
		return href === '/' ? path === '/' : path.startsWith(href);
	}

	function onKeydown(event: KeyboardEvent): void {
		if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
			event.preventDefault();
			paletteOpen = !paletteOpen;
		}
	}

	function runItem(run: () => void): void {
		paletteOpen = false;
		run();
	}

	// ── settings: session locale ─────────────────────────────────────────
	const LOCALES = [
		{ code: 'en', name: 'English' },
		{ code: 'de', name: 'Deutsch' },
		{ code: 'fr', name: 'Français' },
		{ code: 'es', name: 'Español' },
		{ code: 'nl', name: 'Nederlands' }
	] as const;
	const LOCALE_KEY = 'shell.locale';
	let currentLocale = $state('en');

	$effect(() => {
		let stored: string | null = null;
		try {
			stored = sessionStorage.getItem(LOCALE_KEY);
		} catch {
			// private mode: nothing persisted
		}
		const code =
			stored && LOCALES.some((l) => l.code === stored)
				? stored
				: (getLocaleFromNavigator() ?? 'en');
		currentLocale = code;
		void locale.set(code);
	});

	function chooseLocale(event: Event): void {
		const code = (event.currentTarget as HTMLSelectElement).value;
		currentLocale = code;
		void locale.set(code);
		try {
			sessionStorage.setItem(LOCALE_KEY, code);
		} catch {
			// private mode: the choice stays live for this session only
		}
	}

	// ── settings: the kernel-origin override (loopback-enforced) ────────
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

<svelte:window onkeydown={onKeydown} />

<div data-slot="app-shell" class="mx-auto w-full max-w-3xl px-5 pb-16">
	<header class="flex items-center justify-between gap-3 pt-4" data-tauri-drag-region>
		<span class="text-sm font-semibold tracking-tight" data-tauri-drag-region>StewardOS</span>
		<nav aria-label={$_('nav.label')} class="flex items-center gap-1">
			{#each NAV as item (item.route)}
				<a
					href={resolve(item.route)}
					aria-current={isCurrent(resolve(item.route)) ? 'page' : undefined}
					class="rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors hover:bg-muted aria-current-page:text-brand aria-current-page:font-semibold"
				>
					<item.Icon class="mr-1 inline size-4 align-[-2px]" aria-hidden="true" />
					<span class="hidden sm:inline">{$_(item.key)}</span>
					<span class="sr-only sm:hidden">{$_(item.key)}</span>
				</a>
			{/each}
		</nav>
		<div class="flex items-center gap-1">
			<Button
				variant="ghost"
				size="icon-sm"
				aria-label={$_('palette.open')}
				onclick={() => (paletteOpen = true)}
			>
				<CommandIcon aria-hidden="true" />
			</Button>
			<Button
				variant="ghost"
				size="icon-sm"
				aria-label={$_('nav.settings')}
				onclick={() => (settingsOpen = true)}
			>
				<SettingsIcon aria-hidden="true" />
			</Button>
		</div>
	</header>

	<main class="pt-6">
		{@render children()}
	</main>
</div>

<Command.Dialog bind:open={paletteOpen} title={$_('palette.title')}>
	<Command.Input placeholder={$_('palette.placeholder')} />
	<Command.List>
		<Command.Empty>{$_('palette.empty')}</Command.Empty>
		<Command.Group heading={$_('palette.navigate')}>
			{#each NAV as item (item.route)}
				<Command.Item onSelect={() => runItem(() => goto(resolve(item.route)))}>
					<item.Icon aria-hidden="true" />
					{$_(item.key)}
				</Command.Item>
			{/each}
		</Command.Group>
		<Command.Group heading={$_('palette.more')}>
			<Command.Item onSelect={() => runItem(() => (settingsOpen = true))}>
				<SettingsIcon aria-hidden="true" />
				{$_('nav.settings')}
			</Command.Item>
		</Command.Group>
	</Command.List>
</Command.Dialog>

<Dialog.Root bind:open={settingsOpen}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>{$_('settings.title')}</Dialog.Title>
			<Dialog.Description>{$_('settings.description')}</Dialog.Description>
		</Dialog.Header>
		<div class="grid gap-5 py-2">
			<div class="grid gap-2">
				<Label for="setting-origin">{$_('settings.origin.label')}</Label>
				<div class="flex gap-2">
					<Input id="setting-origin" bind:value={originInput} placeholder={DEFAULT_BASE} />
					<Button onclick={saveOrigin}>{$_('settings.origin.save')}</Button>
				</div>
				<p class="text-xs text-muted-foreground">{$_('settings.origin.hint')}</p>
				{#if originState === 'saved'}
					<p class="text-sm text-brand" role="status">{$_('settings.origin.saved')}</p>
				{:else if originState === 'invalid'}
					<p class="text-sm text-destructive" role="alert">{$_('settings.origin.invalid')}</p>
				{/if}
			</div>
			<div class="grid gap-2">
				<Label for="setting-locale">{$_('settings.locale.label')}</Label>
				<!-- The native select (the open list stays the platform's own
				     popup — the native-first control law). -->
				<select id="setting-locale" value={currentLocale} onchange={chooseLocale}>
					{#each LOCALES as l (l.code)}
						<option value={l.code}>{l.name}</option>
					{/each}
				</select>
			</div>
		</div>
	</Dialog.Content>
</Dialog.Root>
