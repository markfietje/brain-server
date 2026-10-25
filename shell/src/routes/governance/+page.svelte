<script lang="ts">
	// The governance surface: what the kernel ENFORCES, read and displayed —
	// the registered domains with their counts, the role bundles with their
	// scopes and verbs, and the retention policy. Three reads with three
	// scopes (domains and roles are Read, retention is Admin), so each
	// section renders its own answer; a Read bearer sees two live sections
	// and one named refusal, never a page that collapsed onto its weakest
	// read. The decision-run ledger is linked, not folded in: it is an
	// operator surface with its own page.
	import { onMount } from 'svelte';
	import { m } from '$paraglide/messages.js';
	import { resolve } from '$app/paths';
	import type { WireFault } from '#lib/api/client.js';
	import {
		fetchDomains,
		fetchRetention,
		fetchRoles,
		type DomainRow,
		type GovernanceRead,
		type RetentionPolicy,
		type RoleRow
	} from '#lib/api/governance.js';
	import { Alert } from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Card } from '#lib/components/ui/card/index.js';

	let domains = $state<GovernanceRead<readonly DomainRow[]> | null>(null);
	let roles = $state<GovernanceRead<readonly RoleRow[]> | null>(null);
	let retention = $state<GovernanceRead<RetentionPolicy> | null>(null);
	let generation = 0;

	async function load(): Promise<void> {
		const mine = ++generation;
		domains = null;
		roles = null;
		retention = null;
		const [d, r, t] = await Promise.all([fetchDomains(), fetchRoles(), fetchRetention()]);
		if (mine !== generation) return;
		domains = d;
		roles = r;
		retention = t;
	}

	onMount(() => {
		void load();
	});

	function refusalMessage(status: number | null, fault: WireFault): string {
		if (status === 403 || fault === 'unauthorized') return m['governance.refused.admin']();
		return m['governance.refused.other']({ status: status ?? '—' });
	}

	const anyOffline = $derived(
		[domains, roles, retention].some((read) => read !== null && read.kind === 'offline')
	);
</script>

<div class="grid gap-6" data-testid="governance-surface">
	<div>
		<h1 class="mb-1">{m['governance.title']()}</h1>
		<p class="text-muted-foreground">{m['governance.subtitle']()}</p>
	</div>

	{#if anyOffline}
		<Alert role="status" data-testid="governance-offline">
			<p>{m['governance.offline']()}</p>
			<Button variant="outline" size="sm" class="mt-2" onclick={() => void load()} data-testid="governance-retry">
				{m['governance.retry']()}
			</Button>
		</Alert>
	{/if}

	<section class="grid gap-3" aria-labelledby="governance-domains">
		<h2 id="governance-domains" class="text-base font-semibold">{m['governance.domains']()}</h2>
		{#if domains === null}
			<p aria-busy="true" class="text-sm text-muted-foreground">{m['governance.loading']()}</p>
		{:else if domains.kind === 'ok'}
			{#if domains.value.length === 0}
				<p class="text-sm text-muted-foreground" data-testid="governance-domains-empty">
					{m['governance.domains.empty']()}
				</p>
			{:else}
				<ul class="grid gap-2">
					{#each domains.value as domain (domain.name)}
						<li>
							<Card class="flex flex-row flex-wrap items-center gap-3 rounded-xl p-4" data-testid="governance-domain-row">
								<span class="font-mono text-sm font-semibold">{domain.name}</span>
								<span class="flex flex-wrap gap-2 text-xs text-muted-foreground">
									<span class="tabular-nums">{m['governance.domain.entries']({ count: domain.entries })}</span>
									<span class="tabular-nums">{m['governance.domain.entities']({ count: domain.entities })}</span>
									<span class="tabular-nums">{m['governance.domain.relations']({ count: domain.relations })}</span>
								</span>
								{#if domain.multiDb}
									<Badge variant="outline" class="rounded-full">{m['governance.domain.own_db']()}</Badge>
								{/if}
							</Card>
						</li>
					{/each}
				</ul>
			{/if}
		{:else if domains.kind === 'refused'}
			<p class="text-sm text-destructive" role="alert" data-testid="governance-domains-refused">
				{refusalMessage(domains.status, domains.fault)}
			</p>
		{:else if domains.kind === 'malformed'}
			<p class="text-sm text-destructive" role="alert">{m['governance.malformed']()}</p>
		{/if}
	</section>

	<section class="grid gap-3" aria-labelledby="governance-roles">
		<h2 id="governance-roles" class="text-base font-semibold">{m['governance.roles']()}</h2>
		{#if roles === null}
			<p aria-busy="true" class="text-sm text-muted-foreground">{m['governance.loading']()}</p>
		{:else if roles.kind === 'ok'}
			{#if roles.value.length === 0}
				<p class="text-sm text-muted-foreground" data-testid="governance-roles-empty">
					{m['governance.roles.empty']()}
				</p>
			{:else}
				<ul class="grid gap-2">
					{#each roles.value as role (role.name)}
						<li>
							<Card class="grid gap-2 rounded-xl p-4" data-testid="governance-role-row">
								<div class="flex items-baseline justify-between gap-3">
									<span class="font-mono text-sm font-semibold">{role.name}</span>
								</div>
								{#if role.description.length > 0}
									<p class="text-sm text-muted-foreground">{role.description}</p>
								{/if}
								<div class="flex flex-wrap items-center gap-1.5 text-xs">
									<span class="text-muted-foreground">{m['governance.role.scopes']()}:</span>
									{#each role.scopes as scope (scope)}
										<Badge variant="secondary" class="rounded-full font-mono">{scope}</Badge>
									{/each}
									<span class="ms-2 text-muted-foreground">{m['governance.role.can']()}:</span>
									{#each role.can as verb (verb)}
										<Badge variant="outline" class="rounded-full font-mono">{verb}</Badge>
									{/each}
								</div>
							</Card>
						</li>
					{/each}
				</ul>
			{/if}
		{:else if roles.kind === 'refused'}
			<p class="text-sm text-destructive" role="alert" data-testid="governance-roles-refused">
				{refusalMessage(roles.status, roles.fault)}
			</p>
		{:else if roles.kind === 'malformed'}
			<p class="text-sm text-destructive" role="alert">{m['governance.malformed']()}</p>
		{/if}
	</section>

	<section class="grid gap-3" aria-labelledby="governance-retention">
		<h2 id="governance-retention" class="text-base font-semibold">{m['governance.retention']()}</h2>
		{#if retention === null}
			<p aria-busy="true" class="text-sm text-muted-foreground">{m['governance.loading']()}</p>
		{:else if retention.kind === 'ok'}
			<Card class="grid gap-2 rounded-xl p-4" data-testid="governance-retention-card" data-enabled={retention.value.enabled}>
				<p class="text-sm">
					{retention.value.enabled
						? m['governance.retention.enabled']()
						: m['governance.retention.disabled']()}
				</p>
				{#if Object.keys(retention.value.policy).length === 0}
					<p class="text-xs text-muted-foreground">{m['governance.retention.none']()}</p>
				{:else}
					<ul class="flex flex-wrap gap-2">
						{#each Object.entries(retention.value.policy) as [kind, days] (kind)}
							<li>
								<Badge variant="outline" class="rounded-full font-mono tabular-nums">
									{m['governance.retention.policy']({ kind, days })}
								</Badge>
							</li>
						{/each}
					</ul>
				{/if}
			</Card>
		{:else if retention.kind === 'refused'}
			<p class="text-sm text-muted-foreground" role="status" data-testid="governance-retention-refused">
				{refusalMessage(retention.status, retention.fault)}
			</p>
		{:else if retention.kind === 'malformed'}
			<p class="text-sm text-destructive" role="alert">{m['governance.malformed']()}</p>
		{/if}
	</section>

	<section class="grid gap-1">
		<a
			href={resolve('/decisions')}
			class="w-fit text-sm font-medium text-brand underline-offset-4 hover:underline"
			data-testid="governance-decisions-link"
		>
			{m['governance.decisions']()} →
		</a>
		<p class="text-xs text-muted-foreground">{m['governance.decisions.hint']()}</p>
	</section>
</div>
