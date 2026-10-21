<script lang="ts">
	// III-6 — the human review queue, DISPLAYED and nothing more.
	//
	// This surface shows what is waiting for a human and what the kernel
	// already put on the row. It never decides: there is no approve, no
	// reject, no edit, and no code path that posts a decision. The kernel's
	// write verbs stay behind the `approve`/`reject` roles.
	//
	// The four states are DISTINCT and never share a rendering:
	//   live    — the kernel answered with a queue
	//   empty   — the kernel said the queue is empty (a FACT)
	//   offline — the kernel could not be reached; the last-known METADATA
	//             is shown with its age (a NAMED state, never an empty queue)
	//   refused — the kernel refused (a REFUSAL with its reason, never an
	//             empty queue)
	import { onMount } from 'svelte';
	import { m } from '$paraglide/messages.js';
	import { wireFaultMessage } from '#lib/api/wire-fault.js';
	import type { WireFault } from '#lib/api/client.js';
	import { fetchPendingQueue } from '#lib/queue/client.js';
	import {
		queueAgeAmount,
		queueAgeMinutes,
		queueAgeUnit,
		queueScoreLabel,
		queueTimeLabel,
		readQueueCache,
		toDisplayRow,
		toQueueMetadata,
		writeQueueCache,
		type QueueCache,
		type QueueDisplayRow
	} from '#lib/queue/model.js';
	import { Alert } from '#lib/components/ui/alert/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';

	type View =
		| { kind: 'loading' }
		| { kind: 'live'; rows: readonly QueueDisplayRow[] }
		| { kind: 'empty' }
		| { kind: 'offline'; cached: QueueCache | null; rows: readonly QueueDisplayRow[] }
		| { kind: 'refused'; status: number | null; fault: WireFault }
		| { kind: 'malformed' };

	let view = $state<View>({ kind: 'loading' });
	let generation = 0;

	function ageLabel(cachedAt: number): string {
		const minutes = queueAgeMinutes(cachedAt, Math.floor(Date.now() / 1000));
		if (minutes < 1) return m['review.age.now']();
		const amount = queueAgeAmount(minutes);
		const unit = queueAgeUnit(minutes);
		if (unit === 'hours') return m['review.age.hours']({ count: amount });
		if (unit === 'days') return m['review.age.days']({ count: amount });
		return m['review.age.minutes']({ count: amount });
	}

	function refusalMessage(status: number | null, fault: WireFault): string {
		if (status === 401) return m['review.refused.unauthorized']();
		if (status === 403) return m['review.refused.forbidden']();
		if (status !== null) return m['review.refused.status']({ status: String(status) });
		return wireFaultMessage(fault);
	}

	async function load(): Promise<void> {
		const current = ++generation;
		view = { kind: 'loading' };
		const read = await fetchPendingQueue();
		if (current !== generation) return;
		if (read.kind === 'ok') {
			if (read.rows.length === 0) {
				// The kernel SAID the queue is empty. That is a fact, and it is
				// not the same fact as "the kernel could not be reached".
				view = { kind: 'empty' };
				writeQueueCache([], Math.floor(Date.now() / 1000));
				return;
			}
			// The cache holds the METADATA and nothing else: the projection
			// below is typed so that it cannot carry the proposal text.
			writeQueueCache(
				read.rows.map((row) => toQueueMetadata(row)),
				Math.floor(Date.now() / 1000)
			);
			view = {
				kind: 'live',
				rows: read.rows.map((row) => toDisplayRow(row, row.content, row.contentDigest))
			};
			return;
		}
		if (read.kind === 'offline') {
			// The OFFLINE state is NAMED and carries the age of what it shows.
			// It is never an empty queue: the distinction between "nothing is
			// waiting" and "we could not ask" is the whole point.
			const cached = readQueueCache();
			view = {
				kind: 'offline',
				cached,
				rows: (cached?.rows ?? []).map((row) => toDisplayRow(row, null, null))
			};
			return;
		}
		if (read.kind === 'refused') {
			view = { kind: 'refused', status: read.status, fault: read.fault };
			return;
		}
		view = { kind: 'malformed' };
	}

	onMount(() => {
		void load();
		return () => {
			generation += 1;
		};
	});
</script>

{#snippet queueRows(rows: readonly QueueDisplayRow[])}
	<ul class="grid gap-3" data-testid="review-list">
		{#each rows as row (row.id)}
			<li class="grid gap-2 rounded-lg border p-4" data-testid="review-list-row">
				<div class="flex flex-wrap items-center gap-2">
					<h2 class="text-base font-semibold" data-testid="review-row-title">
						{m['review.row.title']({ id: String(row.id) })}
					</h2>
					{#if row.screenVerdict === 'quarantine'}
						<Badge variant="outline" data-testid="review-row-quarantine">
							{m['review.row.quarantine']()}
						</Badge>
					{:else}
						<Badge variant="secondary" data-testid="review-row-clean">
							{m['review.row.clean']()}
						</Badge>
					{/if}
				</div>
				<dl class="grid gap-x-3 gap-y-1 text-sm sm:grid-cols-[10rem_1fr]">
					<dt class="text-muted-foreground">{m['review.row.kind']()}</dt>
					<dd data-testid="review-row-kind">{row.kind}</dd>
					<dt class="text-muted-foreground">{m['review.row.domain']()}</dt>
					<dd data-testid="review-row-domain">{row.domain}</dd>
					<dt class="text-muted-foreground">{m['review.row.owner']()}</dt>
					<dd data-testid="review-row-owner">{row.owner ?? m['review.value.none']()}</dd>
					<dt class="text-muted-foreground">{m['review.row.created']()}</dt>
					<dd data-testid="review-row-created">{queueTimeLabel(row.createdAt)}</dd>
					<dt class="text-muted-foreground">{m['review.row.edited']()}</dt>
					<dd data-testid="review-row-edited">
						{row.editedAt === null ? m['review.value.none']() : queueTimeLabel(row.editedAt)}
					</dd>
					<dt class="text-muted-foreground">{m['review.row.decided']()}</dt>
					<dd data-testid="review-row-decided">
						{row.decidedAt === null ? m['review.value.none']() : queueTimeLabel(row.decidedAt)}
					</dd>
					<dt class="text-muted-foreground">{m['review.row.expires']()}</dt>
					<dd data-testid="review-row-expires">{queueTimeLabel(row.expiresAt)}</dd>
					<dt class="text-muted-foreground">{m['review.row.scores']()}</dt>
					<dd data-testid="review-row-scores">
						<span data-testid="review-row-novelty">
							{m['review.row.novelty']()}: {queueScoreLabel(row.novelty)}
						</span>
						<span class="ms-3" data-testid="review-row-salience">
							{m['review.row.salience']()}: {queueScoreLabel(row.salience)}
						</span>
						<span class="ms-3" data-testid="review-row-conflict">
							{m['review.row.conflict']()}: {row.conflictWith ?? m['review.value.none']()}
						</span>
						<span class="ms-3" data-testid="review-row-authority">
							{m['review.row.authority']()}: {row.authority ?? m['review.value.none']()}
						</span>
					</dd>
				</dl>
				<div class="grid gap-1">
					<p class="text-xs font-medium text-muted-foreground">{m['review.row.content']()}</p>
					{#if row.content === null}
						<!-- The cached copy holds METADATA only, so the proposal
						     text reads as UNAVAILABLE, never as absent. -->
						<p class="text-sm" data-testid="review-row-content">
							{m['review.row.content_unavailable']()}
						</p>
					{:else}
						<p class="text-sm whitespace-pre-wrap" data-testid="review-row-content">{row.content}</p>
					{/if}
				</div>
				{#if row.contentDigest !== null}
					<div class="grid gap-1">
						<p class="text-xs font-medium text-muted-foreground">{m['review.row.digest']()}</p>
						<p class="break-all font-mono text-xs" data-testid="review-row-digest">
							{row.contentDigest}
						</p>
					</div>
				{/if}
			</li>
		{/each}
	</ul>
{/snippet}

<div class="grid gap-5" data-testid="review-surface">
	<div>
		<h1>{m['review.title']()}</h1>
		<p class="text-muted-foreground">{m['review.subtitle']()}</p>
	</div>

	<p class="text-xs text-muted-foreground" data-testid="review-disclaimer">
		{m['review.disclaimer']()}
	</p>

	{#if view.kind === 'loading'}
		<p aria-busy="true" data-testid="review-loading">{m['review.loading']()}</p>
	{:else if view.kind === 'refused'}
		<Alert variant="destructive" role="alert" data-testid="review-refused">
			<p class="font-medium">{m['review.refused.title']()}</p>
			<p data-testid="review-refused-reason">{refusalMessage(view.status, view.fault)}</p>
		</Alert>
		<div>
			<Button variant="outline" size="sm" onclick={() => void load()} data-testid="review-retry">
				{m['review.retry']()}
			</Button>
		</div>
	{:else if view.kind === 'malformed'}
		<Alert variant="destructive" role="alert" data-testid="review-malformed">
			{m['review.malformed']()}
		</Alert>
		<div>
			<Button variant="outline" size="sm" onclick={() => void load()} data-testid="review-retry">
				{m['review.retry']()}
			</Button>
		</div>
	{:else if view.kind === 'empty'}
		<p role="status" data-testid="review-empty">{m['review.empty']()}</p>
	{:else if view.kind === 'offline'}
		<Alert role="status" data-testid="review-offline">
			<p class="font-medium">{m['review.offline.title']()}</p>
			{#if view.cached === null}
				<p data-testid="review-offline-nocache">{m['review.offline.nocache']()}</p>
			{:else}
				<p data-testid="review-offline-age">
					{m['review.offline.age']({
						when: queueTimeLabel(view.cached.cachedAt),
						age: ageLabel(view.cached.cachedAt)
					})}
				</p>
				<p data-testid="review-offline-metadata-only">{m['review.offline.metadata_only']()}</p>
			{/if}
		</Alert>
		{#if view.rows.length > 0}
			<p role="status" data-testid="review-offline-count">
				{m['review.count']({ count: view.rows.length })}
			</p>
			{@render queueRows(view.rows)}
		{/if}
	{:else}
		<p role="status" data-testid="review-count">{m['review.count']({ count: view.rows.length })}</p>
		{@render queueRows(view.rows)}
	{/if}
</div>
