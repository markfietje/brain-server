<script lang="ts">
	// The app chrome — the frame every surface renders inside, shaped after
	// the harness it borrows from: a sidebar column (brand + kernel version,
	// "New chat", the surface list, the store's facts) beside a main column
	// (the surface, then a one-line status bar). The conversation is the home
	// surface and the first entry in the list; every other surface is one
	// click away and one ⌘K away. The settings panel stays a dialog (the
	// kernel-origin override, LOOPBACK-ENFORCED client-side and SESSION-LOCAL,
	// plus locale, token and update), and the palette stays in-page — no OS
	// global shortcut.
	//
	// Chrome states facts it has read and never guesses them: the review
	// count rides the Review entry only while the queue read returned rows,
	// the sidebar's integrity line says "not verified" until the chain read
	// answered, and the status bar's model is the one the kernel reported.
	import { m } from '$paraglide/messages.js';
	import { assertIsLocale, setLocale } from '$paraglide/runtime.js';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import ClipboardCheckIcon from '@lucide/svelte/icons/clipboard-check';
	import ListTreeIcon from '@lucide/svelte/icons/list-tree';
	import LayoutGridIcon from '@lucide/svelte/icons/layout-grid';
	import ListIcon from '@lucide/svelte/icons/list';
	import MessageCircleIcon from '@lucide/svelte/icons/message-circle';
	import PanelLeftIcon from '@lucide/svelte/icons/panel-left';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import CommandIcon from '@lucide/svelte/icons/command';
	import { Button } from '#lib/components/ui/button/index.js';
	import { CommandPalette } from '#lib/components/ui/command/index.js';
	import { Dialog } from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { DEFAULT_BASE, apiBase, setApiBase } from '#lib/api/client.js';
	import { refreshBearer } from '#lib/api/bootstrap.js';
	import { wireFaultMessage } from '#lib/api/wire-fault.js';
	import { fetchKernelFacts } from '#lib/api/kernel-facts.js';
	import { verifyAuditChain } from '#lib/api/audit.js';
	import { fetchPendingQueue } from '#lib/queue/client.js';
	import {
		classifyWireFault,
		clearWireFault,
		currentWireFault
	} from '#lib/chat/wire-status.svelte.js';
	import {
		currentKernelFacts,
		currentStatusContext,
		recordKernelFacts,
		startNewConversation
	} from '#lib/shell/frame.svelte.js';

	let paletteOpen = $state(false);
	let settingsOpen = $state(false);

	let { children } = $props();

	function onKeydown(event: KeyboardEvent): void {
		if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
			event.preventDefault();
			paletteOpen = !paletteOpen;
		}
	}

	// ── the surface list ──────────────────────────────────────────────────
	//
	// The sidebar's entries, in order: the conversation first, then the
	// surfaces a conversation cites. `/decisions` is deliberately NOT here —
	// the decision-run listing is an operator surface reached from the
	// palette and from Governance, not a daily destination. The two param'd
	// detail routes are opened from their list pages and from citations.
	const NAV = [
		{ id: 'chat', route: '/', key: 'nav.chat' as const, Icon: MessageCircleIcon },
		{ id: 'memory', route: '/recall', key: 'nav.memory' as const, Icon: LayoutGridIcon },
		{ id: 'review', route: '/review', key: 'nav.review' as const, Icon: ClipboardCheckIcon },
		{ id: 'audit', route: '/audit', key: 'nav.audit' as const, Icon: ListIcon },
		{
			id: 'governance',
			route: '/governance',
			key: 'nav.governance' as const,
			Icon: ShieldCheckIcon
		}
	] as const;

	const activePath = $derived(page.url?.pathname ?? '/');

	/** The home entry matches exactly; every other entry owns its subtree. */
	function isActive(route: string): boolean {
		if (route === '/') return activePath === '/';
		return activePath === route || activePath.startsWith(`${route}/`);
	}

	const activeLabel = $derived.by(() => {
		const entry = NAV.find((item) => isActive(item.route));
		return entry === undefined ? m['shell.brand']() : m[entry.key]();
	});

	// The document title names the surface on screen, the way a browser tab
	// would; the home surface carries the brand.
	$effect(() => {
		document.title = activeLabel;
	});

	// ── the sidebar's width: open or rail ─────────────────────────────────
	//
	// Collapsed is a rail of icons with their names in the accessible tree;
	// the choice persists for the session only (sessionStorage, like the
	// locale), so a fresh launch opens the sidebar the way the design does.
	const SIDEBAR_KEY = 'shell.sidebar';
	let collapsed = $state(false);

	$effect(() => {
		try {
			collapsed = sessionStorage.getItem(SIDEBAR_KEY) === 'collapsed';
		} catch {
			// private mode: the sidebar opens
		}
	});

	function toggleSidebar(): void {
		collapsed = !collapsed;
		try {
			sessionStorage.setItem(SIDEBAR_KEY, collapsed ? 'collapsed' : 'open');
		} catch {
			// private mode: the choice stays live for this session only
		}
	}

	function newChat(): void {
		startNewConversation();
		void goto(resolve('/'));
	}

	const PALETTE_GROUPS = $derived([
		{
			heading: m['palette.navigate'](),
			items: [
				...NAV.map((item) => ({
					value: item.route,
					label: m[item.key](),
					Icon: item.Icon,
					onSelect: () => runItem(() => goto(resolve(item.route)))
				})),
				{
					value: '/decisions',
					label: m['nav.decisions'](),
					Icon: ListTreeIcon,
					onSelect: () => runItem(() => goto(resolve('/decisions')))
				}
			]
		},
		{
			heading: m['palette.more'](),
			items: [
				{
					value: 'settings',
					label: m['nav.settings'](),
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

	// ── the review count on the Review entry ──────────────────────────────
	//
	// The count's wire is the queue read ITSELF (the proposal path publishes
	// NO event, so there is nothing to subscribe to). It refreshes at the
	// GLANCE moments — app mount, window focus, palette open — three
	// triggers, no loop: a polling interval would tax every idle second for
	// a fact that changes rarely. The read is bounded by `QUEUE_LIMITS`; the
	// entry needs EXISTENCE, and the count in its label is the count the
	// read happened to return.
	//
	// A failed read renders NO count (an unreadable queue is not a non-empty
	// queue), but an OFFLINE read is a wire-level fault — the one fact that
	// feeds the status bar and the home page's banner, exactly as a turn's
	// transport fault does.
	let queueCount = $state<number | null>(null);

	async function refreshQueue(): Promise<void> {
		const read = await fetchPendingQueue();
		if (read.kind === 'ok') {
			// The kernel answered — the wire is healthy, so a stale banner
			// over a live wire comes down (silence while green).
			clearWireFault();
			queueCount = read.rows.length;
			return;
		}
		// Unknown means absent: the count is chrome, not a fault surface.
		queueCount = null;
		if (read.kind === 'offline') classifyWireFault(read.fault);
		// A refusal means the kernel SPOKE — never a fault (the taxonomy's
		// law), and never a count either: the queue stays unread, not "empty".
	}

	// ── the store's facts: version, rows, integrity ───────────────────────
	//
	// Read once at mount, shared through the frame store so a surface that
	// shows the same fact shows the same reading. The chain verdict is the
	// kernel recomputing every domain's chain — an Admin read — so it runs
	// at mount and when the audit surface asks, never on every focus.
	type ChainState = 'unread' | 'verified' | 'broken' | 'unverified';
	let chain = $state<ChainState>('unread');

	async function refreshFacts(): Promise<void> {
		const read = await fetchKernelFacts();
		if (read.kind === 'ok') {
			recordKernelFacts(read.facts);
			return;
		}
		recordKernelFacts(null);
		classifyWireFault(read.fault);
	}

	async function refreshChain(): Promise<void> {
		const read = await verifyAuditChain();
		if (read.kind === 'ok') {
			chain = read.verified ? 'verified' : 'broken';
			return;
		}
		// A refusal or an unreadable answer is NOT a verdict: the line says
		// the chain was not verified, which is the honest reading of a read
		// that did not happen. It never says "verified" on a guess.
		chain = 'unverified';
	}

	$effect(() => {
		void refreshQueue();
		void refreshFacts();
		void refreshChain();
	});

	$effect(() => {
		if (paletteOpen) void refreshQueue();
	});

	const facts = $derived(currentKernelFacts());
	const statusContext = $derived(currentStatusContext());
	const wireFault = $derived(currentWireFault());

	const rowsLabel = $derived(
		facts?.count === null || facts === null ? null : m['shell.facts.rows']({ count: facts.count })
	);
	const integrityLabel = $derived.by(() => {
		if (chain === 'verified') return m['shell.facts.integrity.verified']();
		if (chain === 'broken') return m['shell.facts.integrity.broken']();
		return m['shell.facts.integrity.unverified']();
	});
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

<svelte:window onkeydown={onKeydown} onfocus={() => void refreshQueue()} />

<div
	data-slot="app-shell"
	data-sidebar={collapsed ? 'collapsed' : 'open'}
	class="grid h-dvh w-full overflow-hidden"
	style="grid-template-columns: {collapsed ? 60 : 272}px minmax(0, 1fr)"
>
	<aside
		data-slot="sidebar"
		class="flex min-h-0 flex-col border-r border-sidebar-border bg-sidebar"
		aria-label={m['shell.sidebar']()}
	>
		<!-- The top strip is the window's drag region; under the macOS overlay
		     titlebar it is tall enough to clear the traffic lights. -->
		<div
			data-slot="sidebar-top"
			class="flex shrink-0 items-center justify-end px-2"
			style="min-height: var(--titlebar-inset)"
			data-tauri-drag-region
		></div>

		<div class="flex items-center gap-3 px-3 pt-2 pb-3" data-tauri-drag-region>
			<span
				class="grid size-9 shrink-0 place-items-center rounded-xl bg-[#1d2b3a] text-base font-extrabold text-white"
				aria-hidden="true"
			>
				S
			</span>
			{#if !collapsed}
				<div class="min-w-0" data-tauri-drag-region>
					<p class="truncate text-sm font-semibold tracking-tight" data-testid="shell-brand">
						{m['shell.brand']()}
					</p>
					<p class="truncate text-xs text-muted-foreground" data-testid="shell-kernel">
						{#if facts?.version}
							{m['shell.kernel']({ version: facts.version })}
						{:else}
							{m['shell.kernel.unknown']()}
						{/if}
					</p>
				</div>
			{/if}
		</div>

		<div class="px-2 pb-2">
			<Button
				variant="outline"
				size={collapsed ? 'icon-sm' : 'sm'}
				class="w-full justify-start gap-2"
				data-testid="shell-new-chat"
				aria-label={m['shell.new_chat']()}
				onclick={newChat}
			>
				<PlusIcon aria-hidden="true" />
				{#if !collapsed}<span>{m['shell.new_chat']()}</span>{/if}
			</Button>
		</div>

		<nav aria-label={m['shell.nav']()} class="px-2">
			<ul class="grid gap-0.5">
				{#each NAV as item (item.id)}
					{@const active = isActive(item.route)}
					<li>
						<a
							href={resolve(item.route)}
							data-testid="nav-{item.id}"
							aria-current={active ? 'page' : undefined}
							class="relative flex h-9 items-center gap-3 rounded-lg px-2.5 text-sm font-medium transition-colors
								{active
								? 'bg-sidebar-active text-foreground'
								: 'text-muted-foreground hover:bg-sidebar-hover hover:text-foreground'}"
						>
							<item.Icon class="size-4 shrink-0" aria-hidden="true" />
							<span class={collapsed ? 'sr-only' : 'flex-1 truncate'}>{m[item.key]()}</span>
							{#if item.id === 'review' && queueCount !== null && queueCount > 0}
								<!-- The count rides the entry only while the queue holds
								     rows: an empty queue costs the UI nothing. The state
								     token is the stable hook; the count joins the
								     accessible name. -->
								<span
									data-testid="review-badge"
									data-queue="nonempty"
									class="grid min-w-5 place-items-center rounded-full bg-primary px-1.5 text-xs font-semibold text-primary-foreground tabular-nums
										{collapsed ? 'absolute translate-x-3 -translate-y-3' : ''}"
									aria-label={m['review.badge']({ count: queueCount })}
								>
									<span aria-hidden="true">{queueCount}</span>
								</span>
							{/if}
						</a>
					</li>
				{/each}
				<li>
					<button
						type="button"
						data-testid="nav-settings"
						class="flex h-9 w-full items-center gap-3 rounded-lg px-2.5 text-sm font-medium text-muted-foreground transition-colors hover:bg-sidebar-hover hover:text-foreground"
						aria-haspopup="dialog"
						aria-label={m['nav.settings']()}
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
						<SettingsIcon class="size-4 shrink-0" aria-hidden="true" />
						<span class={collapsed ? 'sr-only' : 'flex-1 truncate text-left'}>
							{m['nav.settings']()}
						</span>
					</button>
				</li>
			</ul>
		</nav>

		<div class="flex-1"></div>

		<div
			class="border-t border-sidebar-border px-3 py-3 text-xs text-muted-foreground"
			data-testid="shell-facts"
			data-integrity={chain}
		>
			<p class="flex items-center gap-2 {collapsed ? 'justify-center' : ''}">
				<span
					class="size-2 shrink-0 rounded-full {chain === 'verified'
						? 'bg-ok'
						: chain === 'broken'
							? 'bg-destructive'
							: 'bg-muted-foreground/50'}"
					aria-hidden="true"
				></span>
				<span class={collapsed ? 'sr-only' : 'leading-snug'}>
					{#if rowsLabel !== null}{rowsLabel} · {/if}{integrityLabel}
				</span>
			</p>
		</div>

		<div class="flex items-center gap-1 px-2 pb-2 {collapsed ? 'flex-col' : ''}">
			<Button
				variant="ghost"
				size="icon-sm"
				aria-label={m['palette.open']()}
				data-testid="shell-palette"
				onclick={() => (paletteOpen = true)}
			>
				<CommandIcon aria-hidden="true" />
			</Button>
			<Button
				variant="ghost"
				size="icon-sm"
				aria-label={collapsed ? m['shell.sidebar.expand']() : m['shell.sidebar.collapse']()}
				aria-expanded={!collapsed}
				data-testid="shell-sidebar-toggle"
				onclick={toggleSidebar}
			>
				<PanelLeftIcon aria-hidden="true" />
			</Button>
		</div>
	</aside>

	<div class="flex min-h-0 min-w-0 flex-col">
		<!-- The main column's own top strip: a drag region under the overlay
		     titlebar, empty otherwise. -->
		<div
			class="shrink-0"
			style="min-height: var(--titlebar-inset)"
			data-tauri-drag-region
		></div>
		<main class="min-h-0 flex-1 overflow-y-auto">
			<div class="mx-auto flex min-h-full w-full max-w-3xl flex-col px-6 pt-2 pb-6">
				{@render children()}
			</div>
		</main>
		<footer
			data-slot="status-bar"
			data-testid="shell-status"
			data-wire={wireFault ?? 'healthy'}
			class="flex shrink-0 items-center gap-2 border-t border-border px-4 py-2 text-xs text-muted-foreground"
		>
			<span
				class="size-2 shrink-0 rounded-full {wireFault === null ? 'bg-ok' : 'bg-destructive'}"
				aria-hidden="true"
			></span>
			<span class="truncate">
				{#if wireFault === null}
					{m['shell.status.healthy']()}
				{:else}
					{wireFaultMessage(wireFault)}
				{/if}
				{#if statusContext !== null}
					· <span data-testid="shell-status-context">{statusContext}</span>
				{/if}
				{#if facts?.model}
					· <span data-testid="shell-status-model">{m['shell.status.model']({ model: facts.model })}</span>
				{/if}
			</span>
		</footer>
	</div>
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
