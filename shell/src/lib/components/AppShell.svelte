<script lang="ts">
	// The app chrome — the frame every surface renders inside, shaped after
	// the harness it borrows from: a sidebar column (brand + kernel version,
	// "New chat", the surface list, the store's facts) beside a main column
	// (the surface, then a one-line status bar). The conversation is the home
	// surface and the first entry in the list; every other surface is one
	// click away and one ⌘K away. The settings panel stays a dialog — its
	// four sections are the registry's `settingsSections` — and the palette
	// stays in-page — no OS global shortcut.
	//
	// The frame carries NO surface literals: the sidebar's entries, the
	// palette's navigate group, the dialog's sections and the status bar's
	// context chain all render from the extension registry (Tier A, plan 26)
	// — ONE source. The frame's OWN chrome stays here: the review badge (the
	// exception control keyed on the review entry's id), the `/decisions`
	// palette item (an operator surface, deliberately not a nav entry), the
	// Settings entry (it opens frame state), the session region, and the
	// facts foot.
	//
	// Chrome states facts it has read and never guesses them: the review
	// count rides the Review entry only while the queue read returned rows,
	// the sidebar's integrity line says "not verified" until the chain read
	// answered, and the status bar's model is the one the kernel reported.
	import { m } from '$paraglide/messages.js';
	import { sidebarLayout } from '#lib/shell/sidebar-layout.svelte.js';
	import { sidebarVisibility, type SurfaceVisibility } from '#lib/shell/sidebar-visibility.svelte.js';
	import { nativeMenusAvailable, popupNativeContextMenu, SURFACES } from '#lib/shell/sidebar-context-menu.js';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import ListTreeIcon from '@lucide/svelte/icons/list-tree';
	import PanelLeftIcon from '@lucide/svelte/icons/panel-left';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import CommandIcon from '@lucide/svelte/icons/command';
	import { Button } from '#lib/components/ui/button/index.js';
	import { CommandPalette } from '#lib/components/ui/command/index.js';
	import { Dialog } from '#lib/components/ui/dialog/index.js';
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
		activate as activateSession,
		currentActiveKey,
		currentSessions,
		fresh as freshSession,
		refresh as refreshSessions,
		remove as removeSession,
		rename as renameSession
	} from '#lib/shell/sessions.svelte.js';
	import { currentLocaleCode } from '#lib/shell/locale.svelte.js';
	import {
		currentCredentialEpoch,
		currentKernelFacts,
		recordKernelFacts,
		startNewConversation
	} from '#lib/shell/frame.svelte.js';
	import { navEntries, resolveStatusContext } from '#lib/extensions/registry.js';

	let paletteOpen = $state(false);
	// The settings is a PAGE (/settings — the ChatGPT-desktop shape); the
	// frame has no settings state at all: the foot, the native ⌘,/menu
	// event, and the palette all NAVIGATE.

	let { children } = $props();

	function onKeydown(event: KeyboardEvent): void {
		if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
			event.preventDefault();
			paletteOpen = !paletteOpen;
		}
	}

	// ── the surface list ──────────────────────────────────────────────────
	//
	// The registry's navEntries (coreChat, coreMemory, coreReview, coreAudit,
	// coreGovernance) — the conversation first, then the surfaces a
	// conversation cites. `/decisions` is deliberately NOT a nav entry (the
	// census module-load guard and the registry's route union both refuse
	// one): the decision-run listing is an operator surface reached from the
	// palette and from Governance, not a daily destination.

	const activePath = $derived(page.url?.pathname ?? '/');

	/** The home entry matches exactly; every other entry owns its subtree. */
	function isActive(route: string): boolean {
		if (route === '/') return activePath === '/';
		return activePath === route || activePath.startsWith(`${route}/`);
	}

	const activeLabel = $derived.by(() => {
		const entry = navEntries.find((item) => isActive(item.route));
		return entry === undefined ? m['shell.brand']() : m[entry.labelKey]();
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

	// The sidebar's layout (the dsh AppFrame pattern): the width preference
	// persists, collapse keeps the preference, and the drag base freezes per
	// gesture. The collapsed flag remains session-scoped here (sessionStorage)
	// while the WIDTH is the persisted preference (localStorage) — collapse is
	// a session posture, the width is a preference.
	const layout = sidebarLayout;
	let collapsed = $state(false);
	// The browser fallback's context-menu position (Tauri uses the NATIVE
	// menu instead; this is only for the no-menu runtime).
	let browserMenu = $state<{ x: number; y: number } | null>(null);

	$effect(() => {
		try {
			collapsed = sessionStorage.getItem(SIDEBAR_KEY) === 'collapsed';
		} catch {
			// private mode: the sidebar opens
		}
	});

	function toggleSidebar(): void {
		collapsed = !collapsed;
		layout.toggle();
		try {
			sessionStorage.setItem(SIDEBAR_KEY, collapsed ? 'collapsed' : 'open');
		} catch {
			// private mode: the choice stays live for this session only
		}
	}

	function newChat(): void {
		startNewConversation();
		// The session store follows: a fresh key becomes active with NO write
		// (the lazy law — the row appears when the turn is said, not before).
		freshSession();
		void goto(resolve('/'));
	}

	// ── quiet scrollbars (plan 24 D4) ─────────────────────────────────────
	//
	// A scroll column draws its bar only while the pointer is inside it and
	// keeps it ~2 s after the pointer leaves (dsh's linger): the bar is a
	// pointer affordance, not furniture. The whole state is ONE custom
	// property the stylesheet reads into `scrollbar-color`; nothing listens
	// to scroll — pointer presence is the affordance, not scroll position.
	const SCROLLBAR_LINGER_MS = 2000;

	function scrollColumn() {
		let live = $state(false);
		let linger: ReturnType<typeof setTimeout> | null = null;
		const cancel = (): void => {
			if (linger !== null) {
				clearTimeout(linger);
				linger = null;
			}
		};
		return {
			get thumb(): string {
				return live ? 'var(--scrollbar-thumb-live)' : 'transparent';
			},
			enter(): void {
				cancel();
				live = true;
			},
			leave(): void {
				cancel();
				linger = setTimeout(() => {
					live = false;
					linger = null;
				}, SCROLLBAR_LINGER_MS);
			},
			dispose: cancel
		};
	}

	const sessionScroll = scrollColumn();
	const mainScroll = scrollColumn();

	$effect(() => () => {
		sessionScroll.dispose();
		mainScroll.dispose();
	});

	// ── the session region (plan 23 §2 D5) ────────────────────────────
	//
	// The dsh-shaped list between the nav and the facts foot: one row per
	// stored session — label (or the kernel's own shape) plus a RELATIVE time
	// — click opens the transcript in the frame, rename is an inline input,
	// delete is a confirmed dialog. Chrome-by-exception: ZERO sessions render
	// NOTHING (an empty rail is noise), and a failed read is not an empty
	// list — the region renders nothing for a refusal too, while the status
	// bar carries the wire's own sentence (the fault is the one interrupt,
	// plan 21 §1's law).
	const sessionRows = $derived(currentSessions());
	const activeSessionKey = $derived(currentActiveKey());

	/** Relative time, LOCALE-AWARE: `Intl.RelativeTimeFormat` over the
	 * paraglide locale (the shared session locale — the settings picker
	 * writes the same source), never hand-rolled English (the i18n pin reds
	 * those). */
	function relativeTime(unixSeconds: number): string {
		const rtf = new Intl.RelativeTimeFormat(currentLocaleCode(), { numeric: 'auto' });
		const deltaSeconds = unixSeconds - Math.floor(Date.now() / 1000);
		const minutes = Math.round(deltaSeconds / 60);
		if (Math.abs(minutes) < 60) return rtf.format(minutes, 'minute');
		const hours = Math.round(deltaSeconds / 3600);
		if (Math.abs(hours) < 24) return rtf.format(hours, 'hour');
		const days = Math.round(deltaSeconds / 86400);
		if (Math.abs(days) < 30) return rtf.format(days, 'day');
		return rtf.format(Math.round(deltaSeconds / (86400 * 30)), 'month');
	}

	/** The row's name: the renamed label if the user gave one, else the
	 * kernel-agnostic "session" sentence — the SNIPPET-first design lives
	 * with the stored turns (open() maps them), not a guess on the row. */
	function rowName(label: string): string {
		const trimmed = label.trim();
		if (trimmed.length > 0) return trimmed;
		return m['chat.sessions.unlabelled']();
	}

	let renamingKey = $state<string | null>(null);
	let renameDraft = $state('');

	function startRename(key: string, label: string): void {
		renamingKey = key;
		renameDraft = label;
	}

	async function commitRename(key: string): Promise<void> {
		const draft = renameDraft;
		renamingKey = null;
		if (draft.trim().length === 0) return;
		await renameSession(key, draft);
	}

	let deleteDialogOpen = $state(false);
	let deleteTarget = $state<string | null>(null);

	function askDelete(key: string): void {
		deleteTarget = key;
		deleteDialogOpen = true;
	}

	/** The name the confirmation dialog speaks about — the row's own name,
	 * read from the list the frame already holds. */
	const deleteTargetName = $derived.by(() => {
		const row = sessionRows.find((r) => r.session_key === deleteTarget);
		return row === undefined ? m['chat.sessions.unlabelled']() : rowName(row.label);
	});

	async function confirmDelete(): Promise<void> {
		const key = deleteTarget;
		deleteTarget = null;
		if (key === null) return;
		await removeSession(key);
		// The list just changed shape: the confirmed delete IS a glance.
		void refreshSessions();
	}

	const PALETTE_GROUPS = $derived([
		{
			heading: m['palette.navigate'](),
			items: [
				...navEntries.map((item) => ({
					value: item.route,
					label: m[item.labelKey](),
					Icon: item.Icon,
					onSelect: () => runItem(() => goto(resolve(item.route)))
				})),
				{
					// The frame's own operator entry: `/decisions` is reachable
					// from the palette BY DESIGN while never being a nav entry.
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
					// `more` keeps settings: the entry NAVIGATES (the settings
					// is a page — the ChatGPT-desktop shape).
					value: 'settings',
					label: m['nav.settings'](),
					Icon: SettingsIcon,
					onSelect: () => runItem(() => goto(resolve('/settings')))
				}
			]
		}
	]);

	// Reached from the foot, the native ⌘,/menu event, and the palette —
	// all three NAVIGATE (the settings is a PAGE now, the ChatGPT-desktop
	// shape; the operator's call, 2026-10-30).
	$effect(() => {
		if (!(typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window)) return;
		let unlisten: (() => void) | undefined;
		void (async () => {
			const { listen } = await import('@tauri-apps/api/event');
			const un = await listen('settings://open', () => {
				goto(resolve('/settings'));
			});
			unlisten = un;
		})();
		return () => {
			unlisten?.();
		};
	});

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
		// The session region's FIRST glance: the list is read at mount like
		// every other chrome fact (the queue's own law — mount, focus, palette
		// open; the after-final trigger is the page's `noteFinal()`).
		void refreshSessions();
	});

	$effect(() => {
		if (paletteOpen) {
			void refreshQueue();
			void refreshSessions();
		}
	});

	const facts = $derived(currentKernelFacts());
	// The status bar's middle slot: the registry's context chain — pure
	// members, facts in, `string | null` out, first non-null wins. The
	// testid does not change; the chain is what fills it now.
	const statusContext = $derived(resolveStatusContext(facts));
	const wireFault = $derived(currentWireFault());

	const rowsLabel = $derived(
		facts?.count === null || facts === null ? null : m['shell.facts.rows']({ count: facts.count })
	);
	const integrityLabel = $derived.by(() => {
		if (chain === 'verified') return m['shell.facts.integrity.verified']();
		if (chain === 'broken') return m['shell.facts.integrity.broken']();
		return m['shell.facts.integrity.unverified']();
	});

	// A credential change (the token section notes it on the frame's epoch)
	// is the one moment the frame's facts can go from refused to answered
	// (or back), so the three glance reads re-run on it rather than waiting
	// for the next launch. Epoch 0 is the pre-mount state — the mount effect
	// above already runs the reads.
	const credentialEpoch = $derived(currentCredentialEpoch());
	$effect(() => {
		if (credentialEpoch > 0) {
			void refreshQueue();
			void refreshFacts();
			void refreshChain();
		}
	});
</script>

<svelte:window onkeydown={onKeydown} onfocus={() => { void refreshQueue(); void refreshSessions(); }} />

<div
	data-slot="app-shell"
	data-sidebar={collapsed ? 'collapsed' : 'open'}
	class="grid h-dvh w-full overflow-hidden"
	class:data-dragging={layout.dragging}
	style="grid-template-columns: {collapsed ? 60 : layout.width}px 5px minmax(0, 1fr)"
>
	<aside
		data-slot="sidebar"
		class="flex min-h-0 flex-col border-r border-sidebar-border bg-sidebar"
		aria-label={m['shell.sidebar']()}
	>
		<!-- The top strip is the window's drag region; under the macOS overlay
		     titlebar it is tall enough to clear the traffic lights. -->
		<div
			class="flex shrink-0 items-center justify-end px-2"
			style="min-height: var(--titlebar-inset)"
			data-tauri-drag-region
		></div>
		<div
			class="flex items-center gap-3 px-3 pt-2 pb-3"
			style="min-height: var(--titlebar-inset)"
			data-tauri-drag-region
		>
			<span
				class="grid size-9 shrink-0 place-items-center rounded-xl bg-[#1d2b3a] text-base font-extrabold text-white"
				aria-hidden="true"
			>
				S
			</span>
			{#if !collapsed}
				<div class="min-w-0" data-tauri-drag-region>
					<p class="truncate text-sm font-medium tracking-tight" data-testid="shell-brand">
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

		<nav
			aria-label={m['shell.nav']()}
			class="px-2"
			oncontextmenu={(event) => {
				// THE PROOF SURFACES' REVEAL (the operator's ask): right-click
				// the sidebar → the NATIVE macOS context menu (Tauri
				// CheckMenuItems) toggles Memory/Review/Audit/Governance;
				// the first GUI shows only the conversation. The browser
				// build gets the same menu as a minimal custom popup.
				event.preventDefault();
				if (nativeMenusAvailable()) {
					void popupNativeContextMenu(event.clientX, event.clientY);
				} else {
					browserMenu = { x: event.clientX, y: event.clientY };
				}
			}}
		>
			<ul class="grid gap-0.5">
				{#each navEntries as item (item.id)}
					{@const active = isActive(item.route)}
					{@const surface = item.id as keyof SurfaceVisibility}
					{#if item.id === 'chat' || sidebarVisibility[surface]}
						<li>
							<a
								href={resolve(item.route)}
								data-testid="nav-{item.id}"
								aria-current={active ? 'page' : undefined}
								class="relative flex h-9 items-center gap-3 rounded-lg px-2.5 text-sm transition-colors
									{active
									? 'bg-sidebar-active text-foreground'
									: 'text-muted-foreground hover:bg-sidebar-hover hover:text-foreground'}"
							>
								<item.Icon class="size-4 shrink-0" aria-hidden="true" />
								<span class={collapsed ? 'sr-only' : 'flex-1 truncate'}>{m[item.labelKey]()}</span>
								{#if item.id === 'review' && queueCount !== null && queueCount > 0}
									<!-- The count rides the entry only while the queue holds
									     rows: an empty queue costs the UI nothing. The state
										   token is the stable hook; the count joins the
										   accessible name. -->
									<span
										data-testid="review-badge"
										data-queue="nonempty"
										class="grid min-w-5 place-items-center rounded-full bg-primary px-1.5 text-xs font-medium text-primary-foreground tabular-nums
											{collapsed ? 'absolute translate-x-3 -translate-y-3' : ''}"
										aria-label={m['review.badge']({ count: queueCount })}
									>
										<span aria-hidden="true">{queueCount}</span>
									</span>
								{/if}
							</a>
						</li>
					{/if}
				{/each}
				<!-- The Settings entry is NOT a nav item (the operator's call:
				     not under Governance; it is the app's preferences). It
				     moved to the sidebar FOOT below — the dsh pattern's
				     `sidebar.settings` seat — and the desktop adds the native
				     StewardMem → Settings… ⌘, item (both open one surface). -->
			</ul>
		</nav>

		<!-- The session region (plan 23 §2 D5) — the frame's one empty slot,
		     between the nav and the facts foot. Chrome-by-exception: ZERO rows
		     (and any state that is not a live list) render NOTHING here — an
		     empty rail is noise, and a refusal is not an empty list. The rail
		     (collapsed) keeps every name in the accessible tree via sr-only
		     rows, the R141 law. The pointer pair below only flips the column's
		     scrollbar affordance (plan 24 D4) — no behaviour rides on it and
		     nothing a keyboard or screen-reader user needs, so the static
		     element stays static. -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="min-h-0 flex-1 overflow-y-auto"
			data-testid="session-region"
			data-slot="scroll-column"
			style:--scrollbar-thumb={sessionScroll.thumb}
			onpointerenter={sessionScroll.enter}
			onpointerleave={sessionScroll.leave}
		>
			{#if sessionRows.length > 0}
				<p class="px-3 pb-1 pt-2 text-xs font-medium text-muted-foreground {collapsed ? 'sr-only' : ''}">
					{m['chat.sessions.heading']()}
				</p>
				<ul class="grid gap-0.5 px-2">
					{#each sessionRows as row (row.session_key)}
						{@const active = row.session_key === activeSessionKey}
						{@const name = rowName(row.label)}
						<li class="group relative" data-testid="session-item">
							{#if renamingKey === row.session_key}
								<!-- The inline rename: an input with a named label,
								     Enter commits, Escape cancels. -->
								<input
									class="h-8 w-full rounded-md border border-brand bg-background px-2 text-sm outline-none"
									bind:value={renameDraft}
									maxlength={80}
									aria-label={m['chat.sessions.rename.label']({ name })}
									data-testid="session-rename-input"
									onkeydown={(event) => {
										if (event.key === 'Enter') {
											event.preventDefault();
											void commitRename(row.session_key);
										} else if (event.key === 'Escape') {
											event.preventDefault();
											renamingKey = null;
										}
									}}
								/>
							{:else}
								<button
									type="button"
									data-testid="session-row"
									data-active={active ? 'true' : 'false'}
									aria-current={active ? 'true' : undefined}
									class="flex h-9 w-full items-center gap-2 rounded-lg px-2.5 text-left text-sm text-muted-foreground transition-colors hover:bg-sidebar-hover hover:text-foreground {active
									? 'bg-sidebar-active text-foreground'
									: ''}"
									onclick={() => activateSession(row.session_key)}
								>
									<span class="min-w-0 flex-1 truncate {collapsed ? 'sr-only' : ''}">{name}</span>
									{#if !collapsed}
										<span
											class="shrink-0 text-xs text-muted-foreground tabular-nums"
											data-testid="session-relative-time"
										>
											{relativeTime(row.last_turn_at)}
										</span>
									{/if}
								</button>
								<!-- The row's two affordances, visible on hover/focus
								     (and always in the tab order), collapsed-rail safe:
								     each carries its own accessible name. -->
								<span class="absolute right-1 top-1 flex {collapsed ? 'sr-only' : ''}">
									<button
										type="button"
										data-testid="session-rename"
										aria-label={m['chat.sessions.rename']({ name })}
										class="grid size-6 place-items-center rounded-md text-muted-foreground opacity-60 transition-opacity hover:opacity-100 focus-visible:opacity-100 hover:text-foreground"
										onclick={() => startRename(row.session_key, row.label)}
									>
										<PencilIcon class="size-3.5" aria-hidden="true" />
									</button>
									<button
										type="button"
										data-testid="session-delete"
										aria-label={m['chat.sessions.delete']({ name })}
										class="grid size-6 place-items-center rounded-md text-muted-foreground opacity-60 transition-opacity hover:opacity-100 focus-visible:opacity-100 hover:text-destructive"
										onclick={() => askDelete(row.session_key)}
									>
										<Trash2Icon class="size-3.5" aria-hidden="true" />
										</button>
									</span>
									{/if}
								</li>
							{/each}
				</ul>
			{/if}
		</div>

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
					{#if rowsLabel !== null}{`${rowsLabel} · `}{/if}{integrityLabel}
				</span>
			</p>
		</div>

		<!-- The settings FOOT (the dsh `sidebar.settings` seat): bottom-pinned,
		     separated from the nav list — Settings is the app's preferences,
		     not a surface. The desktop ALSO carries the native StewardMem →
		     Settings… ⌘, item; both open this one surface. -->
		<div class="mt-auto border-t border-sidebar-border px-2 pt-2 {collapsed ? 'pb-2' : ''}">
			<button
				type="button"
				data-testid="nav-settings"
				class="flex h-9 w-full items-center gap-3 rounded-lg px-2.5 text-sm text-muted-foreground transition-colors hover:bg-sidebar-hover hover:text-foreground"
				aria-label={m['nav.settings']()}
				onclick={() => goto(resolve('/settings'))}
			>
				<SettingsIcon class="size-4 shrink-0" aria-hidden="true" />
				<span class={collapsed ? 'sr-only' : 'flex-1 truncate text-left'}>
					{m['nav.settings']()}
				</span>
			</button>
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

	<!-- The resize divider (the dsh AppFrame gesture): 5px hit strip, with
	     a wider invisible hit area via ::before (the CSS keeps the visible
	     edge hairline-thin). The drag base freezes at gesture start; the
	     pointer is captured so leaving the strip mid-drag keeps the gesture
	     alive; text selection is suppressed while dragging. Collapsed hides
	     the divider (a 60px rail has nothing to resize). -->
	{#if !collapsed}
		<div
			role="separator"
			aria-orientation="vertical"
			aria-label={m['shell.sidebar.resize']()}
			data-testid="sidebar-resizer"
			class="sidebar-resizer relative z-10 cursor-col-resize"
			onpointerdown={(event) => {
				(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
				layout.startDrag();
			}}
			onpointermove={(event) => {
				if (layout.dragging) layout.dragBy(event.movementX);
			}}
			onpointerup={(event) => {
				(event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
				layout.endDrag();
			}}
			onlostpointercapture={() => layout.endDrag()}
			ondblclick={() => {
				layout.startDrag();
				layout.dragBy(layout.width - 260);
				layout.endDrag();
			}}
		></div>
	{/if}

	<div class="flex min-h-0 min-w-0 flex-col">
		<!-- The main column's own top strip: a drag region under the overlay
		     titlebar, empty otherwise. -->
		<div
			class="shrink-0"
			style="min-height: var(--titlebar-inset)"
			data-tauri-drag-region
		></div>
		<main
			class="min-h-0 flex-1 overflow-y-auto"
			data-slot="scroll-column"
			style:--scrollbar-thumb={mainScroll.thumb}
			onpointerenter={mainScroll.enter}
			onpointerleave={mainScroll.leave}
		>
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

	<!-- The delete confirmation: erasure is DESTRUCTIVE (the kernel hard-deletes
	 the turns and the row in one transaction), so the act is named twice —
	 the dialog says what will be lost and the buttons carry their verbs. -->
<Dialog
	bind:open={deleteDialogOpen}
	title={m['chat.sessions.delete.title']()}
	description={m['chat.sessions.delete.description']({ name: deleteTargetName })}
	closeLabel={m['common.cancel']()}
>
	<div class="flex justify-end gap-2 py-2">
		<Button
			variant="outline"
			size="sm"
			data-testid="session-delete-cancel"
			onclick={() => (deleteDialogOpen = false)}
		>
			{m['common.cancel']()}
		</Button>
		<Button
			variant="destructive"
			size="sm"
			data-testid="session-delete-confirm"
			onclick={() => {
				deleteDialogOpen = false;
				void confirmDelete();
			}}
		>
			{m['chat.sessions.delete.confirm']()}
		</Button>
	</div>
</Dialog>

<!-- The browser fallback for the sidebar's right-click (Tauri uses the
     NATIVE macOS context menu instead). Same items, same store; dismissed
     by any click or Escape. Rendered through a fixed mask so the dismissal
     needs no per-item wiring. -->
{#if browserMenu !== null}
	<div
		class="fixed inset-0 z-50"
		role="presentation"
		onclick={() => (browserMenu = null)}
		onkeydown={(e) => {
			if (e.key === 'Escape') browserMenu = null;
		}}
	>
		<div
			class="absolute rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-md"
			style="left: {browserMenu.x}px; top: {browserMenu.y}px"
		>
			{#each SURFACES as surface (surface.key)}
				<button
					type="button"
					class="flex w-full items-center justify-between gap-6 rounded-md px-2 py-1.5 text-left text-sm hover:bg-sidebar-hover"
					onclick={() => {
						sidebarVisibility.toggle(surface.key);
						browserMenu = null;
					}}
				>
					{m[surface.labelKey as 'nav.memory']()}
					{#if sidebarVisibility[surface.key]}<span aria-hidden="true">✓</span>{/if}
				</button>
			{/each}
		</div>
	</div>
{/if}
