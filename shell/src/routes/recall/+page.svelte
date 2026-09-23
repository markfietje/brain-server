<script lang="ts">
	// The recall surface (M2-S2): the structured POST /recall over the typed
	// wire. Typed hit cards, the honest low_confidence abstention state, and
	// a per-query trace deep-link rendered ONLY when the kernel itself
	// returned a trace_id (read-event auditing on; the shell cannot request
	// a trace — the openapi QueryDoc does not document the `trace` field,
	// R24 prereg FINDING 2, and the kernel contract is byte-frozen). The
	// trace TARGET page performs the real typed read; the R25 replay UI
	// stays deferred.
	import { _ } from 'svelte-i18n';
	import { resolve } from '$app/paths';
	import { client } from '$lib/api/client';
	import { sanitizeTemplateText } from '$lib/sanitize';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Card } from '$lib/components/ui/card';
	import { Alert } from '$lib/components/ui/alert';
	import { Badge } from '$lib/components/ui/badge';

	type RecallHitItem = {
		id: number;
		title?: string | null;
		snippet?: string;
		score?: number;
		domain?: string | null;
		source?: string | null;
		conflict?: boolean | null;
		ingest_kind?: string | null;
	};

	let query = $state('');
	let busy = $state(false);
	let attempted = $state(false);
	let failed = $state(false);
	let abstained = $state(false);
	let traceId = $state<number | null>(null);
	let hits = $state<RecallHitItem[]>([]);

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
</script>

<div class="grid gap-4">
	<div>
		<h1 class="mb-1">{$_('recall.title')}</h1>
		<p class="text-muted-foreground">{$_('recall.subtitle')}</p>
	</div>

	<form onsubmit={run} class="flex gap-2">
		<Input
			type="search"
			placeholder={$_('recall.placeholder')}
			bind:value={query}
			aria-label={$_('recall.placeholder')}
			data-testid="recall-input"
		/>
		<Button type="submit" disabled={busy}>{$_('recall.button')}</Button>
	</form>

	{#if failed}
		<Alert variant="destructive" role="alert">{$_('recall.error')}</Alert>
	{:else if abstained}
		<Alert role="status" class="border-l-warn-border border-warn-border bg-warn-bg border-l-4">
			{$_('recall.abstain')}
		</Alert>
	{:else if attempted && !busy && hits.length === 0}
		<p class="text-muted-foreground" role="status">{$_('recall.empty')}</p>
	{:else if hits.length > 0}
		<div class="flex items-center justify-between gap-3">
			<p class="text-sm text-muted-foreground" role="status">
				{$_('recall.results', { values: { count: hits.length } })}
			</p>
			{#if traceId !== null}
				<a
					href={resolve(`/recall/${traceId}/trace`)}
					class="text-sm font-medium text-brand underline-offset-4 hover:underline"
					data-testid="trace-link"
				>
					{$_('recall.trace_link')}
				</a>
			{/if}
		</div>
		<div class="grid gap-3">
			{#each hits as hit (hit.id)}
				<Card class="gap-2 p-4" data-testid="recall-hit">
					<div class="flex items-baseline justify-between gap-3">
						<h2 class="text-sm font-semibold">
							{sanitizeTemplateText(hit.title ?? `${hit.id}`)}
						</h2>
						<div class="flex shrink-0 items-center gap-2">
							{#if hit.conflict}
								<Badge variant="destructive">{$_('recall.conflict')}</Badge>
							{/if}
							{#if hit.source}
								<Badge variant="secondary">{sanitizeTemplateText(hit.source)}</Badge>
							{/if}
							{#if typeof hit.score === 'number'}
								<span class="text-xs tabular-nums text-muted-foreground">
									{hit.score.toFixed(3)}
								</span>
							{/if}
						</div>
					</div>
					{#if hit.snippet}
						<p class="text-sm">{sanitizeTemplateText(hit.snippet)}</p>
					{/if}
				</Card>
			{/each}
		</div>
	{/if}
</div>
