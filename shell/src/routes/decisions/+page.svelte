<script lang="ts">
	import { onMount } from 'svelte';
	import { _ } from 'svelte-i18n';
	import { resolve } from '$app/paths';
	import { client } from '$lib/api/client';
	import {
		classifyDecisionStatus,
		createDecisionRequestGuard,
		decisionRunListQuery,
		parseDecisionRunList,
		type DecisionRunSummary,
		type DecisionStatus
	} from '$lib/decision-run';
	import { Alert } from '$lib/components/ui/alert';
	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';

	type ListState =
		| 'loading'
		| 'ready'
		| 'empty'
		| 'unauthorized'
		| 'forbidden'
		| 'not_found'
		| 'conflict'
		| 'bad_request'
		| 'not_found'
		| 'malformed'
		| 'network'
		| 'server'
		| 'unknown';

	let limitInput = $state('20');
	let runIdInput = $state('');
	let rows = $state<DecisionRunSummary[]>([]);
	let viewState = $state<ListState>('loading');
	let status = $state<DecisionStatus | null>(null);
	const requestGuard = createDecisionRequestGuard();

	function statusMessage(current: DecisionStatus | null): string {
		if (current === null) return $_('decisions.list.unknown');
		switch (current.kind) {
			case 'unauthorized':
				return $_('decisions.list.unauthorized');
			case 'forbidden':
				return $_('decisions.list.forbidden');
			case 'not_found':
				return $_('decisions.list.not_found');
			case 'conflict':
				return $_('decisions.list.conflict');
			case 'bad_request':
				return $_('decisions.list.bad_request');
			case 'server':
				return $_('decisions.list.server');

			default:
				return $_('decisions.list.unknown');
		}
	}

	function modeLabel(mode: DecisionRunSummary['mode']): string {
		return mode === 'deterministic'
			? $_('decisions.mode.deterministic')
			: $_('decisions.mode.exploratory');
	}

	function createdLabel(seconds: number): string {
		return new Date(seconds * 1000).toISOString();
	}

	async function load(): Promise<void> {
		const generation = requestGuard.begin();
		const rawRunId = runIdInput.trim();
		if (rawRunId.length > 0 && !/^(0|[1-9][0-9]*)$/.test(rawRunId)) {
			viewState = 'bad_request';
			status = null;
			rows = [];
			return;
		}
		const runId = rawRunId.length === 0 ? null : Number(rawRunId);
		const query = decisionRunListQuery(Number(limitInput), runId);
		if (query === null) {
			viewState = 'bad_request';
			status = null;
			rows = [];
			return;
		}

		viewState = 'loading';
		status = null;
		rows = [];
		try {
			const result = await client.GET('/workflow/decision-runs', {
				params: { query }
			});
			if (!requestGuard.isCurrent(generation)) return;
			if (result.response && result.response.status !== 200) {
				status = classifyDecisionStatus(result.response.status);
				if (status.kind === 'bad_request') {
					viewState = 'bad_request';
				} else if (status.kind === 'unknown') {
					viewState = 'unknown';
				} else {
					viewState = status.kind;
				}
				return;
			}
			if (result.error !== undefined || result.data === undefined) {
				viewState = 'malformed';
				return;
			}
			const parsed = parseDecisionRunList(result.data);
			if (!parsed.ok) {
				viewState = 'malformed';
				return;
			}
			rows = parsed.value.rows;
			viewState = rows.length === 0 ? 'empty' : 'ready';
		} catch {
			if (!requestGuard.isCurrent(generation)) return;
			viewState = 'network';
		}
	}

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		void load();
	}

	onMount(() => {
		void load();
		return () => requestGuard.begin();
	});
</script>

<div class="grid gap-5" data-testid="decision-list">
	<div>
		<h1>{$_('decisions.list.title')}</h1>
		<p class="text-muted-foreground">{$_('decisions.list.subtitle')}</p>
	</div>

	<form class="grid gap-3 sm:grid-cols-[10rem_14rem_auto] sm:items-end" onsubmit={submit}>
		<div class="grid gap-1.5">
			<Label for="decision-limit">{$_('decisions.list.limit.label')}</Label>
			<Input
				id="decision-limit"
				type="number"
				min="1"
				max="50"
				step="1"
				bind:value={limitInput}
				aria-invalid={viewState === 'bad_request'}
				aria-describedby="decision-limit-help"
				data-testid="decision-limit"
			/>
		</div>
		<div class="grid gap-1.5">
			<Label for="decision-run-id">{$_('decisions.list.run_id.label')}</Label>
			<Input
				id="decision-run-id"
				type="text"
				inputmode="numeric"
				bind:value={runIdInput}
				aria-invalid={viewState === 'bad_request'}
				aria-describedby="decision-run-id-help"
				data-testid="decision-run-id"
			/>
		</div>
		<Button type="submit" disabled={viewState === 'loading'} data-testid="decision-load">
			{$_('decisions.list.load')}
		</Button>
		<p id="decision-limit-help" class="sr-only">{$_('decisions.list.limit.label')}</p>
		<p id="decision-run-id-help" class="sr-only">{$_('decisions.list.run_id.label')}</p>
	</form>

	{#if viewState === 'loading'}
		<p aria-busy="true" data-testid="decision-list-loading">{$_('decisions.list.loading')}</p>
	{:else if viewState === 'empty'}
		<p role="status" data-testid="decision-list-empty">{$_('decisions.list.empty')}</p>
	{:else if viewState === 'malformed'}
		<Alert variant="destructive" role="alert" data-testid="decision-list-malformed">
			{$_('decisions.list.malformed')}
		</Alert>
	{:else if viewState === 'network'}
		<Alert variant="destructive" role="alert" data-testid="decision-list-network">
			{$_('decisions.list.network')}
		</Alert>
	{:else if viewState !== 'ready'}
		<Alert
			variant={viewState === 'bad_request' ? 'default' : 'destructive'}
			role={viewState === 'bad_request' ? 'status' : 'alert'}
			data-testid={viewState === 'bad_request' ? 'decision-list-bad-request' : 'decision-list-error'}
		>
			{statusMessage(status)}
		</Alert>
		{#if viewState !== 'bad_request'}
			<div>
				<Button variant="outline" size="sm" onclick={() => void load()}>
					{$_('decisions.list.retry')}
				</Button>
			</div>
		{/if}
	{:else}
		<p role="status" data-testid="decision-list-count">
			{$_('decisions.list.count', { values: { count: rows.length } })}
		</p>
		{#if rows.some((row) => row.mode === 'exploratory')}
			<Alert role="status" data-testid="decision-list-nonpromotable">
				{$_('decisions.list.nonpromotable')}
			</Alert>
		{/if}
		<div class="overflow-x-auto rounded-lg border">
			<table class="w-full text-sm" data-testid="decision-list-table">
				<caption class="sr-only">{$_('decisions.list.title')}</caption>
				<thead>
					<tr class="border-b text-xs text-muted-foreground">
						<th scope="col" class="px-3 py-2 text-start">{$_('decisions.list.id')}</th>
						<th scope="col" class="px-3 py-2 text-start">{$_('decisions.list.run_id')}</th>
						<th scope="col" class="px-3 py-2 text-start">{$_('decisions.list.pipeline')}</th>
						<th scope="col" class="px-3 py-2 text-start">Mode</th>
						<th scope="col" class="px-3 py-2 text-start">{$_('decisions.list.config_hash')}</th>
						<th scope="col" class="px-3 py-2 text-start">{$_('decisions.list.created')}</th>
						<th scope="col" class="px-3 py-2 text-start">{$_('decisions.list.stages')}</th>
					</tr>
				</thead>
				<tbody>
					{#each rows as row (row.id)}
						<tr class="border-b last:border-b-0" data-testid="decision-list-row">
							<td class="px-3 py-2 font-mono tabular-nums">
								<a href={resolve(`/decisions/${row.id}`)} data-testid="decision-list-link">
									{row.id}
								</a>
							</td>
							<td class="px-3 py-2 font-mono tabular-nums">{row.runId}</td>
							<td class="px-3 py-2 font-mono">{row.pipelineVersion}</td>
							<td class="px-3 py-2">
								<Badge variant={row.mode === 'exploratory' ? 'outline' : 'secondary'}>
									{modeLabel(row.mode)}
								</Badge>
								{#if row.mode === 'exploratory'}
									<span class="sr-only">{$_('decisions.list.nonpromotable')}</span>
								{/if}
							</td>
							<td class="max-w-[16rem] break-all px-3 py-2 font-mono text-xs">{row.configHash}</td>
							<td class="px-3 py-2 text-xs">{createdLabel(row.createdAt)}</td>
							<td class="px-3 py-2 tabular-nums">{row.stageCount}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</div>
