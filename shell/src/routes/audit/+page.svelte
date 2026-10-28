<script lang="ts">
	// The audit surface: the kernel's hash chain, read and DISPLAYED.
	//
	// Two reads, both Admin in the kernel's own table. The chain verdict is
	// the kernel recomputing every domain's chain; the events are one page of
	// rows, newest first, with `Load more` paging further back. Rows carry
	// hashes only (the kernel's contract), so a target is shown as a prefix
	// and never as content.
	//
	// The states are DISTINCT and never share a rendering: live, empty (the
	// kernel said so), offline (the wire failed), refused (the kernel said
	// no, with its reason), malformed (an answer this surface cannot prove).
	// The chain line is the same discipline in one phrase: "verified" only
	// when the kernel answered `ok: true`; a read that did not happen says
	// "not verified", never "verified" on a guess.
	import { onMount } from 'svelte';
	import { m } from '$paraglide/messages.js';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { SvelteURLSearchParams } from 'svelte/reactivity';
	import type { WireFault } from '#lib/api/client.js';
	import {
		AUDIT_PAGE,
		fetchAuditEvents,
		verifyAuditChain,
		type AuditEvent,
		type ChainRead
	} from '#lib/api/audit.js';
	import { Alert } from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';

	type View =
		| { kind: 'loading' }
		| { kind: 'live'; events: readonly AuditEvent[]; exhausted: boolean }
		| { kind: 'empty' }
		| { kind: 'offline' }
		| { kind: 'refused'; status: number | null; fault: WireFault }
		| { kind: 'malformed' };

	let view = $state<View>({ kind: 'loading' });
	let chain = $state<ChainRead | null>(null);
	let loadingMore = $state(false);
	let generation = 0;

	// The principal filter (M3-g, plan 27): the URL is the source, so a link
	// from the governance surface (or a bookmark) lands on the principal's
	// audited footprint directly — the retired GUI's `/audit?principal=`
	// promise, finally built. The chip names the filter and links back to the
	// unfiltered chain; the filter reads rows, the verdict read beside it is
	// untouched by it.
	const principalFilter = $derived(page.url.searchParams.get('principal')?.trim() || null);

	// The since filter (the audit-filter round): the retired GUI's OTHER URL
	// promise, the last arm of `/audit?since&:principal` to ship. Same law as
	// the principal filter — the URL is the source — plus the one affordance
	// a moment needs: a datetime input whose Apply WRITES the URL and then
	// re-reads, because a filter that silently changed the rows would be
	// indistinguishable from a smaller chain. The input speaks the local wall
	// clock; the wire speaks ISO-8601 — the conversion happens once, here,
	// and an empty input clears the bound.
	const sinceFilter = $derived(page.url.searchParams.get('since')?.trim() || null);
	let sinceInput = $state('');
	// The route's own path, resolved once: the chips' clear links ride it, and
	// the rule-clean shape for a resolved href is the identifier itself.
	const auditPath = resolve('/audit');

	async function applySince(): Promise<void> {
		const raw = sinceInput.trim();
		const params = new SvelteURLSearchParams(page.url.searchParams.toString());
		if (raw !== '') {
			const parsed = new Date(raw);
			if (Number.isNaN(parsed.getTime())) return;
			params.set('since', parsed.toISOString());
		} else {
			params.delete('since');
		}
		const qs = params.toString();
		// The Apply navigates with the query the operator asked for; resolve()
		// names routes, not their query state — the one navigation this
		// surface makes that resolve() cannot express.
		// eslint-disable-next-line svelte/no-navigation-without-resolve
		await goto(qs === '' ? auditPath : `${auditPath}?${qs}`);
		await load();
	}

	function refusalMessage(status: number | null, fault: WireFault): string {
		if (status === 401 || fault === 'unauthorized' || fault === 'no_credential') {
			return m['audit.refused.unauthorized']();
		}
		if (status === 403) return m['audit.refused.forbidden']();
		return m['audit.refused.other']({ status: status ?? '—' });
	}

	async function load(): Promise<void> {
		const mine = ++generation;
		view = { kind: 'loading' };
		const [events, verdict] = await Promise.all([
			fetchAuditEvents(0, principalFilter ?? undefined, sinceFilter ?? undefined),
			verifyAuditChain()
		]);
		if (mine !== generation) return;
		chain = verdict;
		if (events.kind === 'ok') {
			view =
				events.events.length === 0
					? { kind: 'empty' }
					: { kind: 'live', events: events.events, exhausted: events.events.length < AUDIT_PAGE };
		} else if (events.kind === 'offline') {
			view = { kind: 'offline' };
		} else if (events.kind === 'refused') {
			view = { kind: 'refused', status: events.status, fault: events.fault };
		} else {
			view = { kind: 'malformed' };
		}
	}

	async function loadMore(): Promise<void> {
		if (view.kind !== 'live' || loadingMore) return;
		loadingMore = true;
		const before = view.events;
		try {
			const nextPage = await fetchAuditEvents(
				before.length,
				principalFilter ?? undefined,
				sinceFilter ?? undefined
			);
			if (view.kind !== 'live') return;
			if (nextPage.kind === 'ok') {
				view = {
					kind: 'live',
					events: [...before, ...nextPage.events],
					exhausted: nextPage.events.length < AUDIT_PAGE
				};
			}
			// A failed further page leaves what is on screen on screen; the
			// next click asks again. The first page's state owns the surface.
		} finally {
			loadingMore = false;
		}
	}

	onMount(() => {
		void load();
	});

	function timeLabel(ts: string): string {
		const parsed = Date.parse(ts);
		if (Number.isNaN(parsed)) return ts;
		return new Date(parsed).toLocaleString();
	}

	/** A hash prefix a reader can match by eye; the full hash is in the title. */
	function hashPrefix(hash: string): string {
		return hash.length > 10 ? `${hash.slice(0, 4)}…${hash.slice(-2)}` : hash;
	}
</script>

<div class="grid gap-5" data-testid="audit-surface">
	<div>
		<h1 class="mb-1">{m['audit.title']()}</h1>
		<p class="text-muted-foreground">{m['audit.subtitle']()}</p>
	</div>

	<div class="flex flex-wrap items-center gap-3 text-sm" data-testid="audit-chain" data-chain={chain === null ? 'unread' : chain.kind === 'ok' ? (chain.verified ? 'verified' : 'broken') : 'unverified'}>
		{#if chain !== null && chain.kind === 'ok' && chain.verified}
			<Badge variant="outline" class="rounded-full border-brand/60 px-2.5 py-1 text-brand">
				{m['audit.chain.verified']()} ✓
			</Badge>
		{:else if chain !== null && chain.kind === 'ok'}
			<Badge variant="destructive" class="rounded-full px-2.5 py-1">{m['audit.chain.broken']()}</Badge>
		{:else}
			<Badge variant="outline" class="rounded-full px-2.5 py-1">{m['audit.chain.unverified']()}</Badge>
		{/if}
		<span class="font-mono text-xs text-muted-foreground">{m['audit.chain.detail']()}</span>
		{#if view.kind === 'live'}
			<span class="text-xs text-muted-foreground" data-testid="audit-count">
				{m['audit.events.loaded']({ count: view.events.length })}
			</span>
		{/if}
		{#if chain !== null && chain.kind === 'refused'}
			<span class="text-xs text-muted-foreground">{m['audit.chain.refused']()}</span>
		{/if}
	</div>

	{#if principalFilter !== null}
		<div class="flex items-center gap-2 text-sm" data-testid="audit-principal-filter">
			<Badge variant="secondary" class="rounded-full px-2.5 py-1 font-mono">
				{m['audit.principal.filtered']({ name: principalFilter })}
			</Badge>
			<a
				href={auditPath}
				class="text-xs text-muted-foreground underline-offset-4 hover:underline"
				data-testid="audit-principal-clear"
			>
				{m['audit.principal.clear']()}
			</a>
		</div>
	{/if}

	{#if sinceFilter !== null}
		<div class="flex items-center gap-2 text-sm" data-testid="audit-since-filter">
			<Badge variant="secondary" class="rounded-full px-2.5 py-1 font-mono">
				{m['audit.since.filtered']({ ts: sinceFilter })}
			</Badge>
		<a
			href={auditPath}
			class="text-xs text-muted-foreground underline-offset-4 hover:underline"
			data-sveltekit-reload
			data-testid="audit-since-clear"
			>
				{m['audit.since.clear']()}
			</a>
		</div>
	{/if}

	<div class="flex flex-wrap items-end gap-2 text-sm" data-testid="audit-since-row">
		<div class="flex flex-col gap-1">
			<label for="audit-since-input" class="text-xs text-muted-foreground">
				{m['audit.since.label']()}
			</label>
			<input
				id="audit-since-input"
				type="datetime-local"
				bind:value={sinceInput}
				class="rounded-md border border-border bg-surface px-2 py-1 font-mono text-xs"
				data-testid="audit-since-input"
			/>
		</div>
		<Button variant="outline" size="sm" onclick={() => void applySince()} data-testid="audit-since-apply">
			{m['audit.since.apply']()}
		</Button>
	</div>

	{#if view.kind === 'loading'}
		<p aria-busy="true" data-testid="audit-loading">{m['audit.loading']()}</p>
	{:else if view.kind === 'refused'}
		<Alert variant="destructive" role="alert" data-testid="audit-refused">
			<p class="font-semibold">{m['audit.refused.title']()}</p>
			<p data-testid="audit-refused-reason">{refusalMessage(view.status, view.fault)}</p>
			<Button variant="outline" size="sm" class="mt-2" onclick={() => void load()} data-testid="audit-retry">
				{m['audit.retry']()}
			</Button>
		</Alert>
	{:else if view.kind === 'malformed'}
		<Alert variant="destructive" role="alert" data-testid="audit-malformed">
			<p>{m['audit.malformed']()}</p>
			<Button variant="outline" size="sm" class="mt-2" onclick={() => void load()} data-testid="audit-retry">
				{m['audit.retry']()}
			</Button>
		</Alert>
	{:else if view.kind === 'offline'}
		<Alert role="status" data-testid="audit-offline">
			<p>{m['audit.offline']()}</p>
			<Button variant="outline" size="sm" class="mt-2" onclick={() => void load()} data-testid="audit-retry">
				{m['audit.retry']()}
			</Button>
		</Alert>
	{:else if view.kind === 'empty'}
		<p role="status" data-testid="audit-empty">{m['audit.empty']()}</p>
	{:else}
		<div class="overflow-x-auto rounded-xl border border-border">
			<table class="w-full text-sm" data-testid="audit-table">
				<thead class="bg-surface-2 text-left text-xs text-muted-foreground">
					<tr>
						<th scope="col" class="px-3 py-2 font-medium">{m['audit.col.time']()}</th>
						<th scope="col" class="px-3 py-2 font-medium">{m['audit.col.kind']()}</th>
						<th scope="col" class="px-3 py-2 font-medium">{m['audit.col.actor']()}</th>
						<th scope="col" class="px-3 py-2 font-medium">{m['audit.col.status']()}</th>
						<th scope="col" class="px-3 py-2 font-medium">{m['audit.col.target']()}</th>
						<th scope="col" class="px-3 py-2 font-medium">{m['audit.col.domain']()}</th>
					</tr>
				</thead>
				<tbody>
					{#each view.events as event (`${event.domain}:${event.id}`)}
						<tr class="border-t border-border" data-testid="audit-row" data-status={event.status}>
							<td class="px-3 py-2 whitespace-nowrap font-mono text-xs tabular-nums">{timeLabel(event.ts)}</td>
							<td class="px-3 py-2 font-mono text-xs">{event.kind}</td>
							<td class="max-w-40 truncate px-3 py-2 text-xs">{event.actor}</td>
							<td class="px-3 py-2 text-xs">
								<span class={event.status === 'ok' ? 'text-brand' : 'text-destructive'}>{event.status}</span>
							</td>
							<td class="px-3 py-2 font-mono text-xs" title={event.targetHash}>{hashPrefix(event.targetHash)}</td>
							<td class="px-3 py-2 font-mono text-xs">{event.domain}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
		{#if !view.exhausted}
			<div>
				<Button variant="outline" size="sm" disabled={loadingMore} onclick={() => void loadMore()} data-testid="audit-load-more">
					{m['audit.load_more']()}
				</Button>
			</div>
		{/if}
	{/if}
</div>
