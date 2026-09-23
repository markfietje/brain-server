<script lang="ts">
	// The search surface (M2-S2): the legacy flat-query GET /search over the
	// typed wire. Typed result cards, honest empty/error states — nothing
	// invented, nothing decorated. Every rendered string passes the
	// sanitizer (D9).
	import { _ } from 'svelte-i18n';
	import { client } from '$lib/api/client';
	import { sanitizeTemplateText } from '$lib/sanitize';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Card } from '$lib/components/ui/card';
	import { Alert } from '$lib/components/ui/alert';
	import { Badge } from '$lib/components/ui/badge';

	type SearchResultItem = {
		id: number;
		similarity?: number;
		title?: string | null;
		snippet?: string;
		source?: string | null;
		source_uri?: string | null;
	};

	let query = $state('');
	let busy = $state(false);
	let attempted = $state(false);
	let failed = $state(false);
	let results = $state<SearchResultItem[]>([]);

	async function run(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		const q = query.trim();
		if (q.length === 0 || busy) return;
		busy = true;
		attempted = true;
		failed = false;
		results = [];
		try {
			const { data, error } = await client.GET('/search', {
				params: { query: { q, k: 10 } }
			});
			if (error !== undefined || data === undefined) {
				failed = true;
			} else {
				results = (data['results'] ?? []) as SearchResultItem[];
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
		<h1 class="mb-1">{$_('search.title')}</h1>
		<p class="text-muted-foreground">{$_('search.subtitle')}</p>
	</div>

	<form onsubmit={run} class="flex gap-2">
		<Input
			type="search"
			placeholder={$_('search.placeholder')}
			bind:value={query}
			aria-label={$_('search.placeholder')}
			data-testid="search-input"
		/>
		<Button type="submit" disabled={busy}>{$_('search.button')}</Button>
	</form>

	{#if failed}
		<Alert variant="destructive" role="alert">{$_('search.error')}</Alert>
	{:else if attempted && !busy && results.length === 0}
		<p class="text-muted-foreground" role="status">{$_('search.empty')}</p>
	{:else if results.length > 0}
		<p class="text-sm text-muted-foreground" role="status">
			{$_('search.results', { values: { count: results.length } })}
		</p>
		<div class="grid gap-3">
			{#each results as r (r.id)}
				<Card class="gap-2 p-4" data-testid="search-result">
					<div class="flex items-baseline justify-between gap-3">
						<h2 class="text-sm font-semibold">
							{sanitizeTemplateText(r.title ?? `${r.id}`)}
						</h2>
						<div class="flex shrink-0 items-center gap-2">
							{#if r.source}
								<Badge variant="secondary">{sanitizeTemplateText(r.source)}</Badge>
							{/if}
							{#if typeof r.similarity === 'number'}
								<span class="text-xs tabular-nums text-muted-foreground">
									{r.similarity.toFixed(3)}
								</span>
							{/if}
						</div>
					</div>
					{#if r.snippet}
						<p class="text-sm">{sanitizeTemplateText(r.snippet)}</p>
					{/if}
					{#if r.source_uri}
						<p class="truncate text-xs text-muted-foreground" title={r.source_uri}>
							{sanitizeTemplateText(r.source_uri)}
						</p>
					{/if}
				</Card>
			{/each}
		</div>
	{/if}
</div>
