<script lang="ts">
	// The app chrome (M3-a, plan `21_CHAT_FIRST_IA` — chat-first IA): a top
	// bar with NO nav — the brand mark, the ⌘K command palette button and the
	// settings gear are the ENTIRE permanent chrome — over the child routes,
	// the in-page palette (NO OS global shortcut — plan §7.4), and the session
	// settings panel (kernel-origin override + locale). The origin override is
	// LOOPBACK-ENFORCED client-side (client.ts refuses anything else —
	// fail-safe, never fail-open) and persists SESSION-LOCAL only.
	// Chrome appears by exception, never by default: the conversation is the
	// window; every other surface is the palette, a dialog, or a URL.
	import { m } from '$paraglide/messages.js';
	import { assertIsLocale, setLocale } from '$paraglide/runtime.js';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import ClipboardCheckIcon from '@lucide/svelte/icons/clipboard-check';
	import ListTreeIcon from '@lucide/svelte/icons/list-tree';
	import HistoryIcon from '@lucide/svelte/icons/history';
	import MessageCircleIcon from '@lucide/svelte/icons/message-circle';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import CommandIcon from '@lucide/svelte/icons/command';
	import { Button } from '#lib/components/ui/button/index.js';
	import { CommandPalette } from '#lib/components/ui/command/index.js';
	import { Dialog } from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { DEFAULT_BASE, apiBase, setApiBase } from '#lib/api/client.js';
	import { refreshBearer } from '#lib/api/bootstrap.js';

	let paletteOpen = $state(false);
	let settingsOpen = $state(false);

	let { children } = $props();

	function onKeydown(event: KeyboardEvent): void {
		if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
			event.preventDefault();
			paletteOpen = !paletteOpen;
		}
	}

	/**
	 * The palette's navigate group, EXPLICIT — not derived from the census.
	 * The census is the census, the palette is the palette: the two param'd
	 * detail routes (`/decisions/[id]`, `/recall/[trace_id]/trace`) are opened
	 * from their list pages and citations — a palette entry to a route whose
	 * id the palette does not have is a broken link, not navigation. The chat
	 * home is listed so every other surface is one ⌘K from getting back.
	 */
	const PALETTE_NAVIGATE = [
		{ route: '/', key: 'nav.chat' as const, Icon: MessageCircleIcon },
		{ route: '/decisions', key: 'nav.decisions' as const, Icon: ListTreeIcon },
		{ route: '/recall', key: 'nav.recall' as const, Icon: HistoryIcon },
		// The review queue — the operator's decision surface. NO badge yet:
		// that needs the queue-count wire (M3-b). Until then /review is
		// reachable here and by URL — by design, recorded, not forgotten.
		{ route: '/review', key: 'nav.review' as const, Icon: ClipboardCheckIcon }
	] as const;

	const PALETTE_GROUPS = $derived([
		{
			heading: m["palette.navigate"](),
			items: PALETTE_NAVIGATE.map((item) => ({
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
	let tokenState = $state<
		'idle' | 'saved' | 'rotated' | 'pending' | 'stale' | 'cleared' | 'empty' | 'error'
	>('idle');

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
			// The core answers with the ROTATION's outcome: the keychain holds
			// the token either way, but a running kernel reads its bearer from
			// the data root's auth-token. Reporting "saved" for a rotation
			// that did not reach the kernel would be a lie the user acts on.
			const outcome = (await invoke('store_kernel_token', { token })) as {
				kernelSynced?: boolean;
				dataRootMissing?: boolean;
				error?: string | null;
			} | null;
			if (outcome?.error) {
				tokenState = 'stale';
			} else if (outcome?.kernelSynced) {
				tokenState = 'rotated';
			} else if (outcome?.dataRootMissing) {
				tokenState = 'pending';
			} else {
				tokenState = 'saved';
			}
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

	// ── settings: the software update (III-4; Tauri only) ─────────────────
	// The Rust core owns the check and the install; the webview is granted no
	// updater permission and only ever sees a DTO. `unreachable` is its OWN
	// state: an endpoint that cannot be reached is never "you are up to date",
	// and a refusal names its reason rather than reading like an answer.
	type UpdateState =
		| 'idle'
		| 'checking'
		| 'current'
		| 'available'
		| 'unreachable'
		| 'refused'
		| 'error';
	let updateState = $state<UpdateState>('idle');
	let updateCurrentVersion = $state('');
	let updateCurrentKernel = $state('');
	let updateOfferedVersion = $state('');
	let updateOfferedKernel = $state('');
	let updateReason = $state('');

	type UpdateOutcome = {
		state?: string;
		currentVersion?: string;
		currentKernel?: string | null;
		offeredVersion?: string | null;
		offeredKernel?: string | null;
		expected?: string | null;
		observed?: string | null;
		lastError?: string | null;
	};

	/**
	 * Apply what the core answered. The install answers with the SAME DTO as
	 * the check, and its answer is the one that matters: a refusal names its
	 * reason, and dropping it would render a refusal as nothing at all.
	 */
	function applyOutcome(outcome: UpdateOutcome | null): void {
		updateCurrentVersion = outcome?.currentVersion ?? updateCurrentVersion;
		updateCurrentKernel = outcome?.currentKernel ?? updateCurrentKernel;
		updateOfferedVersion = outcome?.offeredVersion ?? '';
		updateOfferedKernel = outcome?.offeredKernel ?? '';
		updateReason = outcome?.lastError ?? '';
		const state = outcome?.state;
		if (state === 'available') {
			updateState = 'available';
		} else if (state === 'current') {
			updateState = 'current';
		} else if (state === 'unreachable') {
			updateState = 'unreachable';
		} else if (state === 'refused') {
			updateState = 'refused';
		} else {
			updateState = 'error';
		}
	}

	async function checkForUpdate(): Promise<void> {
		updateState = 'checking';
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			applyOutcome((await invoke('update_check')) as UpdateOutcome | null);
		} catch {
			updateState = 'error';
		}
	}

	async function installUpdate(): Promise<void> {
		updateState = 'checking';
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			applyOutcome((await invoke('update_install')) as UpdateOutcome | null);
		} catch {
			updateState = 'error';
		}
	}
</script>

<svelte:window onkeydown={onKeydown} />

<div data-slot="app-shell" class="mx-auto w-full max-w-3xl px-5 pb-16">
	<header class="flex items-center justify-between gap-3 pt-4" data-tauri-drag-region>
		<span class="text-sm font-semibold tracking-tight" data-tauri-drag-region>StewardOS</span>
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
				onclick={() => {
					settingsOpen = true;
					// The panel states the CURRENT version and the current
					// kernel version, and only the core knows either of them
					// (the kernel's comes from the sealed provenance). So
					// opening the panel asks the core once: the check is
					// bounded, and an endpoint that cannot be reached is its
					// OWN state, not an empty line.
					if (inTauri) void checkForUpdate();
				}}
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
				{:else if tokenState === 'rotated'}
					<p class="text-sm text-brand" role="status">{m["settings.token.rotated"]()}</p>
				{:else if tokenState === 'pending'}
					<p class="text-sm text-brand" role="status">{m["settings.token.pending"]()}</p>
				{:else if tokenState === 'stale'}
					<p class="text-sm text-destructive" role="alert">{m["settings.token.stale"]()}</p>
				{:else if tokenState === 'cleared'}
					<p class="text-sm text-brand" role="status">{m["settings.token.cleared"]()}</p>
				{:else if tokenState === 'empty'}
					<p class="text-sm text-destructive" role="alert">{m["settings.token.empty"]()}</p>
				{:else if tokenState === 'error'}
					<p class="text-sm text-destructive" role="alert">{m["settings.token.error"]()}</p>
				{/if}
			</div>
			<div class="grid gap-2">
				<h3 class="text-sm font-semibold">{m["settings.update.heading"]()}</h3>
				<p class="text-xs text-muted-foreground">
					{m["settings.update.current"]({
						version: updateCurrentVersion,
						kernel: updateCurrentKernel
					})}
				</p>
				<div class="flex flex-wrap gap-2">
					<Button onclick={checkForUpdate} disabled={updateState === 'checking'}>
						{m["settings.update.check"]()}
					</Button>
					{#if updateState === 'available'}
						<Button onclick={installUpdate}>{m["settings.update.install"]()}</Button>
					{/if}
				</div>
				{#if updateState === 'current'}
					<p class="text-sm text-brand" role="status">{m["settings.update.uptodate"]()}</p>
				{:else if updateState === 'available'}
					<p class="text-sm text-brand" role="status">
						{m["settings.update.available"]({
							version: updateOfferedVersion,
							kernel: updateOfferedKernel
						})}
					</p>
				{:else if updateState === 'unreachable'}
					<p class="text-sm text-destructive" role="alert">
						{m["settings.update.unreachable"]()}
					</p>
				{:else if updateState === 'refused'}
					<p class="text-sm text-destructive" role="alert">
						{m["settings.update.refused"]({ reason: updateReason })}
					</p>
				{:else if updateState === 'error'}
					<p class="text-sm text-destructive" role="alert">{m["settings.update.error"]()}</p>
				{/if}
			</div>
		{/if}
	</div>
</Dialog>
