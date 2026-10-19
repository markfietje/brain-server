<script lang="ts">
	// The app chrome (M2-S1): top bar + nav over the child routes, the ⌘K
	// command palette (in-page; NO OS global shortcut — plan §7.4), and the
	// session settings panel (kernel-origin override + locale). The origin
	// override is LOOPBACK-ENFORCED client-side (client.ts refuses anything
	// else — fail-safe, never fail-open) and persists SESSION-LOCAL only.
	// Nav marks the active route with aria-current — never color alone.
	import { m } from '$paraglide/messages.js';
	import { assertIsLocale, setLocale } from '$paraglide/runtime.js';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import HouseIcon from '@lucide/svelte/icons/house';
	import ClipboardListIcon from '@lucide/svelte/icons/clipboard-list';
	import SearchIcon from '@lucide/svelte/icons/search';
	import ListTreeIcon from '@lucide/svelte/icons/list-tree';
	import CpuIcon from '@lucide/svelte/icons/cpu';
	import HistoryIcon from '@lucide/svelte/icons/history';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import CommandIcon from '@lucide/svelte/icons/command';
	import { Button } from '#lib/components/ui/button/index.js';
	import { CommandPalette } from '#lib/components/ui/command/index.js';
	import { Dialog } from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { DEFAULT_BASE, apiBase, setApiBase } from '#lib/api/client.js';
	import { refreshBearer } from '#lib/api/bootstrap.js';

	const NAV = [
		{ route: '/overview', key: 'nav.overview', Icon: HouseIcon },
		{ route: '/', key: 'nav.wizard', Icon: ClipboardListIcon },
		{ route: '/search', key: 'nav.search', Icon: SearchIcon },
		{ route: '/decisions', key: 'nav.decisions', Icon: ListTreeIcon },
		{ route: '/models', key: 'nav.models', Icon: CpuIcon },
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

	// The palette's two groups, derived so a NAV change is reflected in the
	// command list without a second source of truth.
	const PALETTE_GROUPS = $derived([
		{
			heading: m["palette.navigate"](),
			items: NAV.map((item) => ({
				value: item.route,
				label: m[item.key](),
				Icon: item.Icon,
				onSelect: () => runItem(() => goto(resolve(item.route)))
			}))
		},
		{
			heading: m["palette.more"](),
			items: [
				{
					value: 'settings',
					label: m["nav.settings"](),
					Icon: SettingsIcon,
					onSelect: () => runItem(() => (settingsOpen = true))
				}
			]
		}
	]);

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
		// Session storage wins; otherwise the browser's own preference, narrowed
		// to a locale the catalog set actually carries.
		const browser = typeof navigator === 'undefined' ? '' : navigator.language.slice(0, 2);
		const code = stored && LOCALES.some((l) => l.code === stored)
			? stored
			: LOCALES.some((l) => l.code === browser)
				? browser
				: 'en';
		currentLocale = code;
		// Paraglide types the locale as a literal union; assert narrows the
		// runtime string the picker produced without widening the catalog.
		setLocale(assertIsLocale(code));
	});

	function chooseLocale(event: Event): void {
		const code = (event.currentTarget as HTMLSelectElement).value;
		currentLocale = code;
		// Paraglide types the locale as a literal union; assert narrows the
		// runtime string the picker produced without widening the catalog.
		setLocale(assertIsLocale(code));
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

	// ── settings: the kernel token (III-2; Tauri only) ───────────────────
	// The writer the login keychain needs. The token is entered here, written
	// by the Rust core, and never retained in the webview beyond the IPC call:
	// the input is cleared after Save AND after a failure, and the status line
	// says configured/anonymous without naming which source supplied it.
	const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
	let tokenInput = $state('');
	let tokenConfigured = $state(false);
	let tokenState = $state<'idle' | 'saved' | 'cleared' | 'empty' | 'error'>('idle');

	$effect(() => {
		if (settingsOpen && inTauri) {
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
			await invoke('store_kernel_token', { token });
			tokenState = 'saved';
		} catch {
			tokenState = 'error';
		} finally {
			tokenInput = '';
			await refreshTokenStatus();
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
		}
	}
</script>

<svelte:window onkeydown={onKeydown} />

<div data-slot="app-shell" class="mx-auto w-full max-w-3xl px-5 pb-16">
	<header class="flex items-center justify-between gap-3 pt-4" data-tauri-drag-region>
		<span class="text-sm font-semibold tracking-tight" data-tauri-drag-region>StewardOS</span>
		<nav aria-label={m["nav.label"]()} class="flex items-center gap-1">
			{#each NAV as item (item.route)}
				<a
					href={resolve(item.route)}
					aria-current={isCurrent(resolve(item.route)) ? 'page' : undefined}
					class="rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors hover:bg-muted aria-current-page:text-brand aria-current-page:font-semibold"
				>
					<item.Icon class="mr-1 inline size-4 align-[-2px]" aria-hidden="true" />
					<span class="hidden sm:inline">{m[item.key]()}</span>
					<span class="sr-only sm:hidden">{m[item.key]()}</span>
				</a>
			{/each}
		</nav>
		<div class="flex items-center gap-1">
			<Button
				variant="ghost"
				size="icon-sm"
				aria-label={m["palette.open"]()}
				onclick={() => (paletteOpen = true)}
			>
				<CommandIcon aria-hidden="true" />
			</Button>
			<Button
				variant="ghost"
				size="icon-sm"
				aria-label={m["nav.settings"]()}
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

<CommandPalette
	bind:open={paletteOpen}
	title={m["palette.title"]()}
	placeholder={m["palette.placeholder"]()}
	emptyText={m["palette.empty"]()}
	groups={PALETTE_GROUPS}
/>

<Dialog
	bind:open={settingsOpen}
	title={m["settings.title"]()}
	description={m["settings.description"]()}
	closeLabel={m["palette.open"]()}
>
	<div class="grid gap-5 py-2">
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
		<div class="grid gap-2">
			<Label for="setting-locale">{m["settings.locale.label"]()}</Label>
			<!-- The native select (the open list stays the platform's own
			     popup — the native-first control law). -->
			<select id="setting-locale" value={currentLocale} onchange={chooseLocale}>
				{#each LOCALES as l (l.code)}
					<option value={l.code}>{l.name}</option>
				{/each}
			</select>
		</div>
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
				{:else if tokenState === 'cleared'}
					<p class="text-sm text-brand" role="status">{m["settings.token.cleared"]()}</p>
				{:else if tokenState === 'empty'}
					<p class="text-sm text-destructive" role="alert">{m["settings.token.empty"]()}</p>
				{:else if tokenState === 'error'}
					<p class="text-sm text-destructive" role="alert">{m["settings.token.error"]()}</p>
				{/if}
			</div>
		{/if}
	</div>
</Dialog>
