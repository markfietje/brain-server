<script lang="ts">
	// The recall-trace REPLAY view (M2-S3): the structured, Dioxus-parity
	// rendering of GET /recall/{trace_id}/trace — the stored decision path
	// (decision, actor, query hash, applied scope, domains searched, per-hit
	// injection details). The wire shape is free-form JSON, so the page
	// parses it CLOSED via parseRecallTrace (src/lib/trace.ts): a field is
	// rendered from an exactly-typed, invisibility-stripped value or it is
	// the honest `—` absence — nothing invented. The EXPORT is the evidence
	// door (Art. 22/ADMT transparency: render + verbatim export only, no
	// legal claims): it downloads the EXACT response bytes as
	// trace-{id}.json (the captured raw text; the typed parse's stringify is
	// the disclosed fallback). The raw form (data-testid="trace-json") stays
	// alongside. The stored query_hash is a hash — never raw query text (the
	// kernel's law); the kernel additionally sanitizes stored-trace strings
	// server-side — the client-side strip is defense in depth.
	import { _ } from 'svelte-i18n';
	import { page } from '$app/state';
	import { client, captureNextRawText } from '$lib/api/client';
	import { parseRecallTrace, type RecallTraceView } from '$lib/trace';
	import { sanitizeTemplateText } from '$lib/sanitize';
	import { Button } from '$lib/components/ui/button';
	import { Alert } from '$lib/components/ui/alert';

	let view = $state<RecallTraceView | null>(null);
	let rawText = $state<string | null>(null);
	let rawJson = $state<unknown>(null);
	let notFound = $state(false);
	let failed = $state(false);
	let loading = $state(true);

	function exportTrace(): void {
		if (view === null || rawJson === null) return;
		// The byte law: the captured wire text verbatim; the fallback is the
		// same response re-serialized (everything preserved, disclosed) —
		// never a view-shaped reconstruction.
		const bytes = rawText ?? JSON.stringify(rawJson);
		const blob = new Blob([bytes], { type: 'application/json' });
		const url = URL.createObjectURL(blob);
		const a = document.createElement('a');
		a.href = url;
		a.download = `trace-${Number(page.params['trace_id'])}.json`;
		a.click();
		URL.revokeObjectURL(url);
	}

	$effect(() => {
		const raw = page.params['trace_id'];
		const id = Number(raw);
		if (raw === undefined || !Number.isInteger(id) || id < 0) {
			notFound = true;
			loading = false;
			return;
		}
		void (async () => {
			loading = true;
			notFound = false;
			failed = false;
			view = null;
			rawText = null;
			rawJson = null;
			try {
				captureNextRawText((text) => {
					rawText = text;
				});
				const { data, error, response } = await client.GET('/recall/{trace_id}/trace', {
					params: { path: { trace_id: id } }
				});
				if (response?.status === 404) {
					notFound = true;
				} else if (error !== undefined || data === undefined) {
					failed = true;
				} else {
					rawJson = data;
					view = parseRecallTrace(data);
				}
			} catch {
				failed = true;
			} finally {
				loading = false;
			}
		})();
	});

	function orDash(value: string | null): string {
		return value === null || value.length === 0 ? '—' : value;
	}

	function listOrDash(value: string[] | null): string {
		return value === null ? '—' : sanitizeTemplateText(value.join(', '));
	}
</script>

<div class="grid gap-4">
	<div class="flex items-center justify-between gap-3">
		<div>
			<h1 class="mb-1">{$_('recall.trace.title')}</h1>
			<p class="text-muted-foreground">
				{$_('recall.trace.subtitle', { values: { id: page.params['trace_id'] ?? '' } })}
			</p>
		</div>
		<Button variant="ghost" size="sm" onclick={() => history.back()}>
			{$_('common.back')}
		</Button>
	</div>

	{#if loading}
		<p aria-busy="true">…</p>
	{:else if notFound}
		<Alert role="status" data-testid="trace-not-found">{$_('recall.trace.not_found')}</Alert>
	{:else if failed}
		<Alert variant="destructive" role="alert">{$_('recall.trace.error')}</Alert>
	{:else if view !== null}
		<div class="grid gap-4" data-testid="trace-view">
			<div class="rounded-lg border p-4">
				<dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
					<dt class="text-muted-foreground">{$_('recall.trace.fields.query_hash')}</dt>
					<dd class="font-mono break-all" data-testid="trace-field-query-hash">
						{orDash(view.query_hash)}
					</dd>
					<dt class="text-muted-foreground">{$_('recall.trace.fields.decision')}</dt>
					<dd class="font-mono" data-testid="trace-field-decision">{orDash(view.decision)}</dd>
					<dt class="text-muted-foreground">{$_('recall.trace.fields.actor')}</dt>
					<dd class="font-mono" data-testid="trace-field-actor">{orDash(view.actor)}</dd>
					<dt class="text-muted-foreground">{$_('recall.trace.fields.scope')}</dt>
					<dd data-testid="trace-field-scope">{listOrDash(view.scope)}</dd>
					<dt class="text-muted-foreground">{$_('recall.trace.fields.domains')}</dt>
					<dd data-testid="trace-field-domains">{listOrDash(view.domains_searched)}</dd>
				</dl>
			</div>

			{#if view.hits !== null && view.hits.length > 0}
				<div class="overflow-x-auto rounded-lg border" data-testid="trace-hits-table">
					<table class="w-full text-sm">
						<thead>
							<tr class="border-b text-xs text-muted-foreground">
								<th class="px-3 py-2 text-start">id</th>
								<th class="px-3 py-2 text-start">score</th>
								<th class="px-3 py-2 text-start">source</th>
								<th class="px-3 py-2 text-start">relevance</th>
								<th class="px-3 py-2 text-start">assertion_kind</th>
								<th class="px-3 py-2 text-start">decayed</th>
							</tr>
						</thead>
						<tbody>
							{#each view.hits as hit, i (i)}
								<tr class="border-b last:border-b-0" data-testid="trace-hit-row">
									<td class="px-3 py-2 font-mono tabular-nums">{orDash(hit.id)}</td>
									<td class="px-3 py-2 font-mono tabular-nums">{orDash(hit.score)}</td>
									<td class="px-3 py-2">{orDash(hit.source)}</td>
									<td class="px-3 py-2">{orDash(hit.relevance)}</td>
									<td class="px-3 py-2">{orDash(hit.assertion_kind)}</td>
									<td class="px-3 py-2">
										{hit.decayed === true ? $_('recall.trace.decayed_marker') : '—'}
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}

			<div class="flex items-center gap-3">
				<Button variant="outline" size="sm" onclick={exportTrace} data-testid="trace-export">
					{$_('recall.trace.export')}
				</Button>
				<pre
					data-testid="trace-json"
					class="min-w-0 flex-1 overflow-x-auto rounded-lg border p-4 text-xs leading-relaxed"
					role="figure"
					aria-label={$_('recall.trace.title')}>{rawJson === null
						? ''
						: JSON.stringify(rawJson, null, 2)}</pre
				>
			</div>
		</div>
	{/if}
</div>
