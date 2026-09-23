<script lang="ts">
	// The Overview (M2-S1): the kernel's OWN numbers over the typed wire —
	// /health (the public probe shape), /version, /stats. No cached values,
	// no opinions: when the wire fails the honest error state shows.
	import { _ } from 'svelte-i18n';
	import { client } from '$lib/api/client';
	import { Card } from '$lib/components/ui/card';
	import { Alert } from '$lib/components/ui/alert';
	import { Button } from '$lib/components/ui/button';

	interface Health {
		status: string;
		version: string;
	}
	interface Stats {
		count?: number;
		embeddings?: number;
		entities?: number;
		relationships?: number;
		model?: string;
		version?: string;
	}

	let health = $state<Health | null>(null);
	let version = $state<string | null>(null);
	let stats = $state<Stats | null>(null);
	let failed = $state(false);
	let loading = $state(true);

	async function load(): Promise<void> {
		loading = true;
		failed = false;
		try {
			const [h, v, s] = await Promise.all([
				client.GET('/health'),
				// /version is text/plain — openapi-fetch's runtime default is
				// json, so the text parse is explicit here.
				client.GET('/version', { parseAs: 'text' }),
				client.GET('/stats')
			]);
			// openapi-fetch resolves {data,error} per call; /version is text/plain.
			const healthData = h.data && !h.error ? (h.data as Health) : null;
			const versionData =
				v.data !== undefined && !v.error && typeof v.data === 'string' ? v.data : null;
			const statsData = s.data && !s.error ? (s.data as Stats) : null;
			if (!healthData && !versionData && !statsData) {
				failed = true;
			}
			health = healthData;
			version = versionData;
			stats = statsData;
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load();
	});
</script>

<div class="grid gap-4">
	<div>
		<h1 class="mb-1">{$_('overview.title')}</h1>
		<p class="text-muted-foreground">{$_('overview.subtitle')}</p>
	</div>

	{#if failed}
		<Alert variant="destructive">
			{$_('overview.error')}
			<Button variant="outline" size="sm" class="mt-3" onclick={load}>
				{$_('common.retry')}
			</Button>
		</Alert>
	{:else if loading}
		<p aria-busy="true">…</p>
	{:else}
		<div class="grid gap-3 sm:grid-cols-3">
			<Card class="gap-1 p-4" data-testid="stat-health">
				<p class="text-xs text-muted-foreground">{$_('overview.health')}</p>
				<p class="text-lg font-semibold">{health?.status ?? '—'}</p>
				{#if health?.version}
					<p class="text-xs text-muted-foreground">kernel {health.version}</p>
				{/if}
			</Card>
			<Card class="gap-1 p-4" data-testid="stat-version">
				<p class="text-xs text-muted-foreground">{$_('overview.version')}</p>
				<p class="text-lg font-semibold">{version ?? health?.version ?? '—'}</p>
			</Card>
			<Card class="gap-1 p-4" data-testid="stat-model">
				<p class="text-xs text-muted-foreground">{$_('overview.stats.model')}</p>
				<p class="truncate text-sm font-semibold" title={stats?.model}>
					{stats?.model ?? '—'}
				</p>
			</Card>
		</div>
		<div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
			<Card class="gap-1 p-4" data-testid="stat-count">
				<p class="text-xs text-muted-foreground">{$_('overview.stats.count')}</p>
				<p class="text-xl font-semibold tabular-nums">{stats?.count ?? '—'}</p>
			</Card>
			<Card class="gap-1 p-4" data-testid="stat-embeddings">
				<p class="text-xs text-muted-foreground">{$_('overview.stats.embeddings')}</p>
				<p class="text-xl font-semibold tabular-nums">{stats?.embeddings ?? '—'}</p>
			</Card>
			<Card class="gap-1 p-4" data-testid="stat-entities">
				<p class="text-xs text-muted-foreground">{$_('overview.stats.entities')}</p>
				<p class="text-xl font-semibold tabular-nums">{stats?.entities ?? '—'}</p>
			</Card>
			<Card class="gap-1 p-4" data-testid="stat-relationships">
				<p class="text-xs text-muted-foreground">{$_('overview.stats.relationships')}</p>
				<p class="text-xl font-semibold tabular-nums">{stats?.relationships ?? '—'}</p>
			</Card>
		</div>
	{/if}
</div>
