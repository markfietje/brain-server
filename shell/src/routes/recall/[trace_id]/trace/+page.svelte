<script lang="ts">
	// The trace deep-link TARGET (M2-S2): performs the REAL typed read of
	// GET /recall/{trace_id}/trace and renders the stored trace JSON
	// verbatim (read-only). This is a raw-trace viewer, not a stub of the
	// replay UI — the R25 slice owns the replay semantics on top of the
	// committed fixture (tests/fixtures/recall-trace.json). The stored
	// query_hash is a bounded hash — never raw query text (the kernel's law).
	import { _ } from 'svelte-i18n';
	import { page } from '$app/state';
	import { client } from '$lib/api/client';
	import { Button } from '$lib/components/ui/button';
	import { Alert } from '$lib/components/ui/alert';

	let traceJson = $state<string | null>(null);
	let notFound = $state(false);
	let failed = $state(false);
	let loading = $state(true);

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
			traceJson = null;
			try {
				const { data, error, response } = await client.GET('/recall/{trace_id}/trace', {
					params: { path: { trace_id: id } }
				});
				if (response?.status === 404) {
					notFound = true;
				} else if (error !== undefined || data === undefined) {
					failed = true;
				} else {
					traceJson = JSON.stringify(data, null, 2);
				}
			} catch {
				failed = true;
			} finally {
				loading = false;
			}
		})();
	});
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
		<Alert role="status">{$_('recall.trace.not_found')}</Alert>
	{:else if failed}
		<Alert variant="destructive" role="alert">{$_('recall.trace.error')}</Alert>
	{:else if traceJson !== null}
		<pre
			data-testid="trace-json"
			class="overflow-x-auto rounded-lg border p-4 text-xs leading-relaxed"
			role="figure"
			aria-label={$_('recall.trace.title')}>{traceJson}</pre
		>
	{/if}
</div>
