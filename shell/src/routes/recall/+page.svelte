<script lang="ts">
	// The memory surface: the structured POST /recall over the typed wire,
	// rendered as the cards the kernel's own hit shape supports — score and
	// its band, memory kind and origin, the validity window, the quarantine
	// and untrusted marks. A per-query trace deep-link renders ONLY when the
	// kernel itself returned a trace_id (the shell cannot request a trace —
	// the openapi QueryDoc does not document the `trace` field, and the
	// kernel contract is byte-frozen); the trace TARGET page performs the
	// real typed read.
	//
	// The kind filter is CLIENT-SIDE over the hits of the last query: the
	// QueryDoc carries no memory-kind filter (measured — its `source` and
	// `sources` fields filter the INGEST kind, not the node kind), so the
	// chips narrow what came back and never pretend to have asked the kernel
	// for less.
	//
	// "Add memory" files a PROPOSAL. It never writes a memory: the candidate
	// waits in Review until a human approves it, which is the product's law
	// and the reason the button goes to /ingest/proposal and nowhere else.
	import { onDestroy } from 'svelte';
	import { m } from '$paraglide/messages.js';
	import { resolve } from '$app/paths';
	import SearchIcon from '@lucide/svelte/icons/search';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import { client } from '#lib/api/client.js';
	import { fetchKernelFacts } from '#lib/api/kernel-facts.js';
	import { proposeMemory, PROPOSAL_CONTENT_MAX, type ProposalKind, type ProposeOutcome } from '#lib/api/propose.js';
	import { confidenceWord, validityOf } from '#lib/chat/provenance.js';
	import type { Passage } from '#lib/api/recall.js';
	import { sanitizeTemplateText } from '#lib/sanitize.js';
	import {
		currentKernelFacts,
		recordKernelFacts,
		setStatusContext
	} from '#lib/shell/frame.svelte.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { Dialog } from '#lib/components/ui/dialog/index.js';
	import { Card } from '#lib/components/ui/card/index.js';
	import { Alert } from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';

	type RecallHitItem = {
		id: number;
		title?: string | null;
		content?: string;
		snippet?: string;
		score?: number;
		domain?: string | null;
		source?: string | null;
		conflict?: boolean | null;
		ingest_kind?: string | null;
		memory_kind?: string | null;
		origin?: string | null;
		flagged?: boolean;
		authority?: number | null;
		evidence?: { valid_from?: string | null; valid_to?: string | null } | null;
	};

	type KindFilter = 'all' | 'facts' | 'procedures';

	let query = $state('');
	let busy = $state(false);
	let attempted = $state(false);
	let failed = $state(false);
	let abstained = $state(false);
	let traceId = $state<number | null>(null);
	let hits = $state<RecallHitItem[]>([]);
	let filter = $state<KindFilter>('all');

	const FILTERS: readonly { id: KindFilter; key: 'memory.filter.all' | 'memory.filter.facts' | 'memory.filter.procedures' }[] = [
		{ id: 'all', key: 'memory.filter.all' },
		{ id: 'facts', key: 'memory.filter.facts' },
		{ id: 'procedures', key: 'memory.filter.procedures' }
	];

	function matchesFilter(hit: RecallHitItem): boolean {
		if (filter === 'all') return true;
		const kind = hit.memory_kind ?? null;
		if (filter === 'facts') return kind === 'fact';
		return kind === 'procedure' || kind === 'step';
	}

	const shown = $derived(hits.filter(matchesFilter));

	// The store's count for the empty state. The frame reads it at mount; a
	// surface that mounts before the frame's read answered (or without the
	// frame at all) asks once itself, and records the same answer.
	const facts = $derived(currentKernelFacts());
	$effect(() => {
		if (currentKernelFacts() !== null) return;
		void fetchKernelFacts().then((read) => {
			if (read.kind === 'ok') recordKernelFacts(read.facts);
		});
	});

	// The status bar's middle slot states how many memories are on screen;
	// it is cleared on the way out so the next surface does not inherit it.
	$effect(() => {
		setStatusContext(attempted && !busy && hits.length > 0 ? m['memory.shown']({ count: shown.length }) : null);
	});
	onDestroy(() => setStatusContext(null));

	async function run(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		const q = query.trim();
		if (q.length === 0 || busy) return;
		busy = true;
		attempted = true;
		failed = false;
		abstained = false;
		traceId = null;
		hits = [];
		try {
			const { data, error } = await client.POST('/recall', {
				body: { v: 1, query: q, limit: 10 }
			});
			if (error !== undefined || data === undefined) {
				failed = true;
			} else {
				hits = (data['hits'] ?? []) as RecallHitItem[];
				abstained = data['decision'] === 'low_confidence' && hits.length === 0;
				const t = data['trace_id'];
				traceId = typeof t === 'number' ? t : null;
			}
		} catch {
			failed = true;
		} finally {
			busy = false;
		}
	}

	/** The kernel's hit, in the shape the chat's provenance helpers read. */
	function asPassage(hit: RecallHitItem): Passage {
		return {
			id: hit.id,
			title: hit.title ?? null,
			text: hit.content ?? hit.snippet ?? '',
			snippet: hit.snippet ?? null,
			domain: hit.domain ?? null,
			score: hit.score ?? 0,
			valid_from: hit.evidence?.valid_from ?? null,
			valid_to: hit.evidence?.valid_to ?? null,
			flagged: hit.flagged === true,
			ingest_kind: hit.ingest_kind ?? null,
			memory_kind: hit.memory_kind ?? null,
			authority: hit.authority ?? null
		};
	}

	function scoreLabel(score: number): string {
		const band = confidenceWord(score);
		return m['memory.badge.score']({
			score: score.toFixed(3),
			word: band.kind === 'word' ? band.word : '—'
		});
	}

	/** The validity line only when the kernel stated a window; silence otherwise. */
	function validityLabel(hit: RecallHitItem): string | null {
		const passage = asPassage(hit);
		if (passage.valid_from === null && passage.valid_to === null) return null;
		return validityOf(passage, Date.now()).summary;
	}

	/** The write-side taint label: anything the kernel did not attribute to a person. */
	function isUntrusted(hit: RecallHitItem): boolean {
		return hit.origin === 'model' || hit.origin === 'agent' || hit.origin === 'imported';
	}

	function kindLabel(hit: RecallHitItem): string | null {
		const kind = hit.memory_kind ?? null;
		const origin = hit.origin ?? null;
		if (kind === null && origin === null) return null;
		if (kind !== null && origin !== null && !isUntrusted(hit)) return `${kind} · ${origin}`;
		return kind ?? origin;
	}

	// ── add memory: the proposal dialog ──────────────────────────────────
	let addOpen = $state(false);
	let draft = $state('');
	let draftKind = $state<ProposalKind>('fact');
	let filing = $state(false);
	let outcome = $state<'idle' | 'empty' | ProposeOutcome>('idle');

	function openAdd(): void {
		outcome = 'idle';
		addOpen = true;
	}

	async function file(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		const content = draft.trim();
		if (content.length === 0) {
			outcome = 'empty';
			return;
		}
		filing = true;
		try {
			outcome = await proposeMemory(content.slice(0, PROPOSAL_CONTENT_MAX), draftKind);
			if (outcome.kind === 'filed') draft = '';
		} finally {
			filing = false;
		}
	}

	function chooseKind(event: Event): void {
		const value = (event.currentTarget as HTMLSelectElement).value;
		draftKind = value === 'procedure' ? 'procedure' : 'fact';
	}
</script>

<div class="grid gap-5" data-testid="memory-surface">
	<div>
		<h1 class="mb-1">{m['memory.title']()}</h1>
		<p class="text-muted-foreground">{m['memory.subtitle']()}</p>
	</div>

	<div class="flex flex-wrap items-center gap-3">
		<form onsubmit={run} class="flex min-w-56 flex-1 items-center gap-2">
			<div class="relative flex-1">
				<SearchIcon
					class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
					aria-hidden="true"
				/>
				<Input
					type="search"
					class="rounded-full pl-9"
					placeholder={m['memory.search.placeholder']()}
					bind:value={query}
					aria-label={m['memory.search.label']()}
					data-testid="recall-input"
				/>
			</div>
			<Button type="submit" variant="secondary" size="sm" disabled={busy} class="rounded-full">
				{m['recall.button']()}
			</Button>
		</form>

		<div
			class="flex items-center gap-1 rounded-full border border-border bg-surface-2 p-1"
			role="group"
			aria-label={m['memory.filter.label']()}
			data-testid="memory-filter"
		>
			{#each FILTERS as option (option.id)}
				<button
					type="button"
					class="rounded-full px-3 py-1 text-sm font-medium transition-colors
						{filter === option.id
						? 'bg-surface text-foreground shadow-xs'
						: 'text-muted-foreground hover:text-foreground'}"
					aria-pressed={filter === option.id}
					data-testid="memory-filter-{option.id}"
					onclick={() => (filter = option.id)}
				>
					{m[option.key]()}
				</button>
			{/each}
		</div>

		<Button size="sm" class="rounded-full" data-testid="memory-add" onclick={openAdd}>
			<PlusIcon aria-hidden="true" />
			{m['memory.add']()}
		</Button>
	</div>

	{#if failed}
		<Alert variant="destructive" role="alert">{m['recall.error']()}</Alert>
	{:else if abstained}
		<Alert role="status" class="border-l-warn-border border-warn-border bg-warn-bg border-l-4">
			{m['recall.abstain']()}
		</Alert>
	{:else if attempted && !busy && hits.length === 0}
		<p class="text-muted-foreground" role="status">{m['recall.empty']()}</p>
	{:else if hits.length > 0}
		<div class="flex items-center justify-between gap-3">
			<p class="text-sm text-muted-foreground" role="status">
				{m['recall.results']({ count: hits.length })}
			</p>
			{#if traceId !== null}
				<a
					href={resolve('/recall/[trace_id]/trace', { trace_id: String(traceId) })}
					class="text-sm font-medium text-brand underline-offset-4 hover:underline"
					data-testid="trace-link"
				>
					{m['recall.trace_link']()}
				</a>
			{/if}
		</div>
		{#if shown.length === 0}
			<p class="text-sm text-muted-foreground" data-testid="memory-filter-empty">
				{m['memory.filter.empty']()}
			</p>
		{/if}
		<div class="grid gap-3">
			{#each shown as hit (hit.id)}
				{@const validity = validityLabel(hit)}
				{@const kind = kindLabel(hit)}
				<Card class="gap-2.5 rounded-xl p-5" data-testid="recall-hit">
					<div class="flex items-baseline justify-between gap-3">
						<h2 class="text-base font-semibold">
							{sanitizeTemplateText(hit.title ?? `${hit.id}`)}
						</h2>
						<span class="shrink-0 font-mono text-xs text-muted-foreground">
							#{hit.id}{#if hit.domain} · {sanitizeTemplateText(hit.domain)}{/if}
						</span>
					</div>
					{#if hit.snippet}
						<p class="line-clamp-2 text-sm text-muted-foreground">{sanitizeTemplateText(hit.snippet)}</p>
					{/if}
					<div class="flex flex-wrap items-center gap-2 pt-1">
						{#if typeof hit.score === 'number'}
							<Badge
								variant="outline"
								class="rounded-full px-2.5 py-1 font-mono tabular-nums {hit.score >= 0.7
									? 'border-brand/50 bg-brand/10 text-brand'
									: ''}"
							>
								{scoreLabel(hit.score)}
							</Badge>
						{/if}
						{#if kind !== null}
							<Badge variant="outline" class="rounded-full px-2.5 py-1 font-mono">
								{sanitizeTemplateText(kind)}
							</Badge>
						{/if}
						{#if validity !== null}
							<Badge variant="outline" class="rounded-full px-2.5 py-1 font-mono">{validity}</Badge>
						{/if}
						{#if isUntrusted(hit)}
							<Badge variant="outline" class="rounded-full px-2.5 py-1 font-mono" data-testid="memory-untrusted">
								{m['memory.badge.untrusted']()}
							</Badge>
						{/if}
						{#if hit.flagged}
							<Badge variant="destructive" class="rounded-full px-2.5 py-1 font-mono">
								{m['memory.badge.quarantined']()}
							</Badge>
						{/if}
						{#if hit.conflict}
							<Badge variant="destructive" class="rounded-full px-2.5 py-1 font-mono">
								{m['recall.conflict']()}
							</Badge>
						{/if}
						{#if hit.source}
							<Badge variant="secondary" class="rounded-full px-2.5 py-1 font-mono">
								{sanitizeTemplateText(hit.source)}
							</Badge>
						{/if}
					</div>
				</Card>
			{/each}
		</div>
	{:else if !attempted}
		<div class="grid gap-1 py-6 text-sm text-muted-foreground" data-testid="memory-empty">
			{#if facts?.count !== null && facts?.count !== undefined}
				<p data-testid="memory-count">{m['memory.count']({ count: facts.count })}</p>
			{/if}
		</div>
	{/if}
</div>

<Dialog
	bind:open={addOpen}
	title={m['memory.add.title']()}
	description={m['memory.add.description']()}
	closeLabel={m['memory.add.cancel']()}
>
	<form class="grid gap-4 py-2" onsubmit={file} data-testid="memory-add-form">
		<div class="grid gap-2">
			<Label for="memory-add-content">{m['memory.add.content']()}</Label>
			<Textarea
				id="memory-add-content"
				bind:value={draft}
				rows={5}
				maxlength={PROPOSAL_CONTENT_MAX}
				data-testid="memory-add-content"
			/>
		</div>
		<div class="grid gap-2">
			<Label for="memory-add-kind">{m['memory.add.kind']()}</Label>
			<!-- The native select (the open list stays the platform's own
			     popup — the native-first control law). -->
			<select id="memory-add-kind" value={draftKind} onchange={chooseKind} data-testid="memory-add-kind">
				<option value="fact">{m['memory.add.kind.fact']()}</option>
				<option value="procedure">{m['memory.add.kind.procedure']()}</option>
			</select>
		</div>
		<div class="flex items-center justify-end gap-2">
			<Button type="button" variant="ghost" onclick={() => (addOpen = false)}>
				{m['memory.add.cancel']()}
			</Button>
			<Button type="submit" disabled={filing} data-testid="memory-add-submit">
				{m['memory.add.submit']()}
			</Button>
		</div>
		{#if outcome === 'empty'}
			<p class="text-sm text-destructive" role="alert" data-outcome="empty">{m['memory.add.empty']()}</p>
		{:else if outcome !== 'idle'}
			{#if outcome.kind === 'filed'}
				<p class="text-sm text-brand" role="status" data-outcome="filed" data-testid="memory-add-filed">
					{m['memory.add.filed']({ id: outcome.id })}
				</p>
			{:else if outcome.kind === 'refused'}
				<p class="text-sm text-destructive" role="alert" data-outcome="refused">
					{m['memory.add.refused']({ status: outcome.status ?? '—' })}
				</p>
			{:else if outcome.kind === 'offline'}
				<p class="text-sm text-destructive" role="alert" data-outcome="offline">
					{m['memory.add.offline']()}
				</p>
			{:else}
				<p class="text-sm text-destructive" role="alert" data-outcome="malformed">
					{m['memory.add.malformed']()}
				</p>
			{/if}
		{/if}
	</form>
</Dialog>
