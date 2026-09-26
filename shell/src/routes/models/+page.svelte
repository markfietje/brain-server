<script lang="ts">
	// R36 M6-S2 — the Model Registry view: the bounded listing and the single-row
	// read, on one surface. `$lib/model-registry.ts` is the only runtime boundary
	// between the wire and this renderer, and it is the data contract in full.
	//
	// FIVE LAWS this surface is built to hold. None of them is a preference.
	//
	//  1. KERNEL AUTHORIZATION IS THE ONLY AUTHORITY. There is no client-side
	//     role, capability, or permission preflight anywhere in this file. A 403
	//     is rendered as a FORBIDDEN state and is never collapsed into an empty
	//     list; the list is dual-gated (Admin + DPO) while the single-row read is
	//     Read, so the two are read independently and one never blanks the other.
	//  2. A DIGEST IS A PIN, NOT A SIGNATURE. The row digest is displayed
	//     verbatim, labelled, and accompanied — in its own words — by
	//     `models.digest.pinNote`. Nothing here computes, truncates, re-cases, or
	//     repairs a digest, and no function in this file could.
	//  3. NO approve/reject CONTROL AND NO DIRECT STATUS WRITE. The only
	//     affordances are lifecycle PROPOSALS, and only the ones the kernel's own
	//     transition table allows from the row's current status
	//     (`legalLifecycleActions`). From `retired` there are none.
	//  4. DISPLAY/SUBMISSION SEPARATION. Every string on its way to the DOM goes
	//     through `displayText`. Every string on its way back to the kernel is
	//     built by `lifecycleProposalContent` from the values AS RECEIVED. The
	//     display projection is never fed back into an outbound payload.
	//  5. AN EVALUATION JOIN IS DATA. `evaluationRefs` renders as a bounded,
	//     non-authoritative list. It never influences a status, never gates a
	//     control, and the surface never claims that it makes `evaluated`
	//     reachable.
	//
	// No raw-HTML directive, no console, no token in any URL, no prompt rendering.
	import { tick } from 'svelte';
	import { _ } from 'svelte-i18n';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { client } from '$lib/api/client';
	import {
		classifyModelRegistryStatus,
		displayText,
		legalLifecycleActions,
		lifecycleProposalContent,
		modelRegistryListQuery,
		parseModelRef,
		parseModelRegistryDetail,
		parseModelRegistryList,
		type LifecycleAction,
		type ModelKind,
		type ModelRegistryDetail,
		type ModelRegistryListQueryInput,
		type ModelRegistryStatus,
		type ModelStatus,
		type ModelSummary
	} from '$lib/model-registry';
	import { Alert } from '$lib/components/ui/alert';
	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Card } from '$lib/components/ui/card';

	/** The refusal classifications the parser names, minus its `ok`. */
	type WireState =
		| 'unauthorized'
		| 'forbidden'
		| 'not_found'
		| 'conflict'
		| 'invalid'
		| 'server'
		| 'unknown';
	type ReadState = 'loading' | 'ready' | 'empty' | 'ref_malformed' | 'malformed' | 'network' | WireState;
	type ProposalState = 'idle' | 'loading' | 'pending' | 'malformed' | 'network' | WireState;

	/**
	 * The ONLY filters this view ever asks for. The server supports `limit`,
	 * `status`, and `kind`; the shell invents no filter, no cursor, and no page
	 * offset, and offers no control that could produce one. `modelRegistryListQuery`
	 * stays the authority on that set: it refuses anything outside the server's
	 * bounds, and a refusal here becomes a 400 state rather than a request the
	 * operator never made.
	 *
	 * (The value is an OBJECT because the typed client's query surface is an
	 * object; the parser's own query STRING is still what this set is measured
	 * against, and it is the parser that gates the read.)
	 */
	const LIST_FILTERS: ModelRegistryListQueryInput = { limit: 20 };

	/** The absence marker for a value the row genuinely does not carry. */
	const ABSENT = '—';

	let listState = $state<ReadState>('loading');
	let listStatus = $state<ModelRegistryStatus | null>(null);
	let rows = $state<ModelSummary[]>([]);

	// The single-row read is selected by the `ref` query segment, so the row a
	// person is looking at is a link they can share, re-open, and re-read.
	let refParam = $state<string | null>(page.url.searchParams.get('ref'));
	let parsedRef = $state<{ id: string; version: string } | null>(null);
	let detail = $state<ModelRegistryDetail | null>(null);
	let detailState = $state<ReadState>('loading');
	let detailStatus = $state<ModelRegistryStatus | null>(null);
	let evaluationOpen = $state(false);

	let proposalState = $state<ProposalState>('idle');
	let proposalStatus = $state<ModelRegistryStatus | null>(null);
	let proposalId = $state<number | null>(null);
	let headingElement: HTMLHeadingElement | undefined = $state();

	// The request guard: a response from a superseded read never lands, and an
	// abandoned surface cannot write into a later one.
	let generation = 0;
	function beginRequest(): number {
		generation += 1;
		return generation;
	}
	function isCurrent(token: number): boolean {
		return token === generation;
	}

	function statusLabel(status: ModelStatus): string {
		return $_(`models.status.${status}`);
	}

	function kindLabel(kind: ModelKind): string {
		return $_(`models.kind.${kind}`);
	}

	/** Display projection only. Never applied to a value that is sent back. */
	function show(value: string | null): string {
		return value === null || value === '' ? ABSENT : displayText(value);
	}

	function stamp(seconds: number): string {
		return new Date(seconds * 1000).toISOString();
	}

	/**
	 * Classify WITHOUT consulting a client-side role. The kernel already
	 * answered; this only names what it said, so that a denial can never be
	 * re-rendered as an absence.
	 */
	function wireStateFor(current: ModelRegistryStatus): WireState {
		switch (current.kind) {
			case 'unauthorized':
				return 'unauthorized';
			case 'forbidden':
				return 'forbidden';
			case 'not_found':
				return 'not_found';
			case 'conflict':
				return 'conflict';
			case 'invalid':
				return 'invalid';
			case 'server':
				return 'server';
			default:
				return 'unknown';
		}
	}

	function errorMessage(current: ModelRegistryStatus | null): string {
		if (current === null) return $_('models.error.heading');
		switch (current.kind) {
			case 'unauthorized':
				return $_('models.error.401');
			case 'forbidden':
				return $_('models.error.403');
			case 'not_found':
				return $_('models.error.404');
			case 'conflict':
				return $_('models.error.409');
			case 'invalid':
				// 400 and 422 share the parser's `invalid` kind but name DIFFERENT
				// refusals; the operator is told which one happened.
				return current.status === 422 ? $_('models.error.422') : $_('models.error.400');
			case 'server':
				return $_('models.error.500');
			default:
				return $_('models.error.heading');
		}
	}

	function resetProposal(): void {
		proposalState = 'idle';
		proposalStatus = null;
		proposalId = null;
	}

	async function focusHeading(token: number): Promise<void> {
		await tick();
		if (isCurrent(token)) headingElement?.focus();
	}

	async function loadList(token: number): Promise<void> {
		if (modelRegistryListQuery(LIST_FILTERS) === null) {
			listState = 'invalid';
			listStatus = classifyModelRegistryStatus(400);
			return;
		}
		listState = 'loading';
		listStatus = null;
		rows = [];
		try {
			const result = await client.GET('/workflow/model-registry', {
				params: { query: LIST_FILTERS }
			});
			if (!isCurrent(token)) return;
			if (result.response && result.response.status !== 200) {
				listStatus = classifyModelRegistryStatus(result.response.status);
				listState = wireStateFor(listStatus);
				return;
			}
			if (result.error !== undefined || result.data === undefined) {
				listState = 'malformed';
				return;
			}
			const parsed = parseModelRegistryList(result.data);
			if (!parsed.ok) {
				// An unverifiable listing is REFUSED, never rendered as an empty
				// one: a silent empty list reads as "the registry is empty".
				listState = 'malformed';
				return;
			}
			rows = parsed.value.rows;
			listState = parsed.value.rows.length === 0 ? 'empty' : 'ready';
		} catch {
			if (!isCurrent(token)) return;
			listState = 'network';
		}
	}

	function retryList(): void {
		void loadList(beginRequest());
	}

	async function loadDetail(
		ref: { id: string; version: string },
		token: number
	): Promise<void> {
		try {
			const result = await client.GET('/workflow/model-registry/{model_ref}', {
				params: { path: { model_ref: `${ref.id}@${ref.version}` } }
			});
			if (!isCurrent(token)) return;
			if (result.response && result.response.status !== 200) {
				detailStatus = classifyModelRegistryStatus(result.response.status);
				detailState = wireStateFor(detailStatus);
				return;
			}
			if (result.error !== undefined || result.data === undefined) {
				detailState = 'malformed';
				return;
			}
			const parsed = parseModelRegistryDetail(result.data);
			if (!parsed.ok) {
				detailState = 'malformed';
				return;
			}
			detail = parsed.value;
			detailState = 'ready';
		} catch {
			if (!isCurrent(token)) return;
			detailState = 'network';
		}
	}

	function reloadDetail(): void {
		const ref = parsedRef;
		if (ref === null) return;
		const token = beginRequest();
		detail = null;
		detailState = 'loading';
		detailStatus = null;
		evaluationOpen = false;
		resetProposal();
		void loadDetail(ref, token).then(() => focusHeading(token));
	}

	function toggleEvaluation(): void {
		evaluationOpen = !evaluationOpen;
	}

	/**
	 * The ONLY write this surface performs, and it is a proposal.
	 *
	 * The payload is `lifecycleProposalContent(action, row)` — the VERBATIM wire
	 * values as the detail read returned them, invisible characters and all.
	 * Nothing that was rendered is copied back into it: no sanitize, no
	 * re-serialize, no recompute, no truncation, no repair. The kernel re-validates
	 * the row, the digest, and the transition before anything is created.
	 */
	async function propose(action: LifecycleAction): Promise<void> {
		const row = detail;
		if (row === null || proposalState === 'loading') return;
		// The transition table is the kernel's law, read from the parser — the
		// control is only ever rendered for a legal action, and this keeps an
		// illegal one unreachable even if the markup were ever edited. It is a
		// TRANSITION check, not an authorization preflight: the kernel remains
		// the only authority on whether this principal may propose at all.
		if (!legalLifecycleActions(row.status).includes(action)) return;
		proposalState = 'loading';
		proposalStatus = null;
		proposalId = null;
		const content = lifecycleProposalContent(action, row);
		try {
			const result = await client.POST('/ingest/proposal', {
				body: { kind: 'registry_lifecycle', content }
			});
			if (result.response && result.response.status !== 200) {
				proposalStatus = classifyModelRegistryStatus(result.response.status);
				proposalState = wireStateFor(proposalStatus);
				return;
			}
			if (result.error !== undefined || result.data === undefined) {
				proposalState = 'malformed';
				return;
			}
			// The receipt is PENDING ONLY. A response claiming any other
			// disposition is not a receipt this surface can show, because a
			// proposal creates nothing until a human approves it.
			if (result.data.status !== 'pending') {
				proposalState = 'malformed';
				return;
			}
			proposalId = typeof result.data.id === 'number' ? result.data.id : null;
			proposalState = 'pending';
		} catch {
			proposalState = 'network';
		}
	}

	$effect(() => {
		const raw = page.url.searchParams.get('ref');
		const token = beginRequest();
		refParam = raw;
		parsedRef = null;
		detail = null;
		detailState = 'loading';
		detailStatus = null;
		evaluationOpen = false;
		resetProposal();
		if (raw === null) {
			// No `ref`: the listing surface. The two reads are deliberately
			// independent — a principal who cannot LIST can still READ a row.
			void loadList(token);
		} else {
			// A malformed ref is refused here, before any wire is touched: the
			// kernel is never asked a question the shell already knows is bad.
			const decoded = parseModelRef(raw);
			if (decoded === null) {
				detailState = 'ref_malformed';
				void focusHeading(token);
			} else {
				parsedRef = decoded;
				void loadDetail(decoded, token).then(() => focusHeading(token));
			}
		}
		return () => {
			beginRequest();
		};
	});
</script>

<div class="grid gap-5" data-testid="models-view">
	{#if refParam === null}
		<div class="grid gap-5" data-testid="models-list">
			<div>
				<h1>{$_('models.title')}</h1>
				<p class="text-muted-foreground">{$_('models.heading')}</p>
			</div>

			{#if listState === 'loading'}
				<p aria-busy="true" data-testid="models-list-loading">{$_('models.loading')}</p>
			{:else if listState === 'empty'}
				<p role="status" data-testid="models-list-empty">{$_('models.empty')}</p>
			{:else if listState === 'malformed'}
				<Alert variant="destructive" role="alert" data-testid="models-list-malformed">
					{$_('models.error.heading')}
				</Alert>
			{:else if listState === 'network'}
				<Alert variant="destructive" role="alert" data-testid="models-list-network">
					{$_('models.error.network')}
				</Alert>
				<div>
					<Button variant="outline" size="sm" onclick={retryList} data-testid="models-list-retry">
						{$_('common.retry')}
					</Button>
				</div>
			{:else if listState !== 'ready'}
				<!-- A refusal is never an empty registry: 401/403/409/422 each say
				     what the kernel answered, and no retry is offered for a denial. -->
				<Alert variant="destructive" role="alert" data-testid="models-list-error">
					{errorMessage(listStatus)}
				</Alert>
				{#if listStatus !== null && listStatus.retryable}
					<div>
						<Button variant="outline" size="sm" onclick={retryList} data-testid="models-list-retry">
							{$_('common.retry')}
						</Button>
					</div>
				{/if}
			{:else}
				<div class="overflow-x-auto rounded-lg border">
					<table class="w-full text-sm" data-testid="models-list-table">
						<caption class="sr-only">{$_('models.table.caption')}</caption>
						<thead>
							<tr class="border-b text-xs text-muted-foreground">
								<th scope="col" class="px-3 py-2 text-start">{$_('models.identity.id')}</th>
								<th scope="col" class="px-3 py-2 text-start">{$_('models.identity.version')}</th>
								<th scope="col" class="px-3 py-2 text-start">{$_('models.identity.kind')}</th>
								<th scope="col" class="px-3 py-2 text-start">{$_('models.identity.name')}</th>
								<th scope="col" class="px-3 py-2 text-start">{$_('models.digest.artifact')}</th>
								<th scope="col" class="px-3 py-2 text-start">{$_('models.digest.config')}</th>
								<th scope="col" class="px-3 py-2 text-start">{$_('models.identity.proposedBy')}</th>
								<th scope="col" class="px-3 py-2 text-start">{$_('models.identity.created')}</th>
								<th scope="col" class="px-3 py-2 text-start">{$_('models.identity.updated')}</th>
							</tr>
						</thead>
						<tbody>
							{#each rows as row (row.id + '@' + row.version)}
								<tr class="border-b last:border-b-0" data-testid="models-list-row">
									<td class="px-3 py-2 font-mono">
										<a
											href={resolve(`/models?ref=${encodeURIComponent(row.id + '@' + row.version)}`)}
											data-testid="models-list-link">{show(row.id)}</a
										>
										<!-- Status is TEXT, never a colour: the word is the
										     conveyance, and the badge is decoration around it. -->
										<span class="mt-1 block">
											<Badge variant="outline" data-testid="models-list-status">
												{statusLabel(row.status)}
											</Badge>
										</span>
									</td>
									<td class="px-3 py-2 font-mono">{show(row.version)}</td>
									<td class="px-3 py-2">{kindLabel(row.kind)}</td>
									<td class="max-w-[18rem] break-words px-3 py-2">{show(row.name)}</td>
									<!-- The listing carries the artifact digest as a PRESENCE
									     BOOLEAN only; its value lives on the single-row read. -->
									<td class="px-3 py-2 text-xs" data-testid="models-list-artifact">
										{row.artifactDigestPresent
											? $_('models.digest.presence.present')
											: $_('models.digest.presence.absent')}
									</td>
									<td class="max-w-[12rem] break-all px-3 py-2 font-mono text-xs">
										{show(row.configDigest)}
									</td>
									<td class="px-3 py-2 font-mono text-xs">{show(row.proposedBy)}</td>
									<td class="px-3 py-2 text-xs">{stamp(row.createdAt)}</td>
									<td class="px-3 py-2 text-xs">{stamp(row.updatedAt)}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</div>
	{:else}
		<div class="grid gap-5" data-testid="models-detail">
			<div class="flex items-start justify-between gap-3">
				<div>
					<h1 bind:this={headingElement} tabindex="-1">
						{#if detail === null}
							{$_('models.title')}
						{:else}
							{show(detail.id)}@{show(detail.version)}
						{/if}
					</h1>
					<p class="text-muted-foreground">{$_('models.heading')}</p>
				</div>
				<Button variant="ghost" size="sm" href={resolve('/models')} data-testid="models-detail-back">
					{$_('common.back')}
				</Button>
			</div>

			{#if detailState === 'loading'}
				<p aria-busy="true" data-testid="models-detail-loading">{$_('models.loading')}</p>
			{:else if detailState === 'ref_malformed'}
				<Alert role="status" data-testid="models-detail-ref-malformed">
					{$_('models.ref.malformed')}
				</Alert>
			{:else if detailState === 'malformed'}
				<Alert variant="destructive" role="alert" data-testid="models-detail-malformed">
					{$_('models.error.heading')}
				</Alert>
			{:else if detailState === 'network'}
				<Alert variant="destructive" role="alert" data-testid="models-detail-network">
					{$_('models.error.network')}
				</Alert>
				<div>
					<Button variant="outline" size="sm" onclick={reloadDetail} data-testid="models-detail-retry">
						{$_('common.retry')}
					</Button>
				</div>
			{:else if detailState !== 'ready' || detail === null}
				<Alert variant="destructive" role="alert" data-testid="models-detail-error">
					{errorMessage(detailStatus)}
				</Alert>
				{#if detailStatus !== null && detailStatus.retryable}
					<div>
						<Button variant="outline" size="sm" onclick={reloadDetail} data-testid="models-detail-retry">
							{$_('common.retry')}
						</Button>
					</div>
				{/if}
			{:else}
				<div class="grid gap-4" data-testid="models-detail-ready">
					<p class="text-sm">
						<Badge variant="outline" data-testid="models-detail-status">{statusLabel(detail.status)}</Badge>
					</p>

					<Card class="gap-3 p-4">
						<h2 class="font-semibold">{$_('models.heading')}</h2>
						<dl
							class="grid gap-2 text-sm sm:grid-cols-[12rem_1fr]"
							data-testid="models-detail-identity"
						>
							<dt class="text-muted-foreground">{$_('models.identity.id')}</dt>
							<dd class="font-mono">{show(detail.id)}</dd>
							<dt class="text-muted-foreground">{$_('models.identity.version')}</dt>
							<dd class="font-mono">{show(detail.version)}</dd>
							<dt class="text-muted-foreground">{$_('models.identity.kind')}</dt>
							<dd>{kindLabel(detail.kind)}</dd>
							<dt class="text-muted-foreground">{$_('models.identity.name')}</dt>
							<dd data-testid="models-detail-name">{show(detail.name)}</dd>
							<dt class="text-muted-foreground">{$_('models.identity.proposedBy')}</dt>
							<dd class="font-mono">{show(detail.proposedBy)}</dd>
							<dt class="text-muted-foreground">{$_('models.identity.created')}</dt>
							<dd>{stamp(detail.createdAt)}</dd>
							<dt class="text-muted-foreground">{$_('models.identity.updated')}</dt>
							<dd>{stamp(detail.updatedAt)}</dd>
						</dl>
						<!-- Each digest tier is its OWN labelled row. They are never
						     merged, never substituted for one another, and an absent
						     tier renders as absent — never as an invented value. -->
						<dl class="grid gap-2 text-sm sm:grid-cols-[12rem_1fr]" data-testid="models-detail-digests">
							<dt class="text-muted-foreground">{$_('models.digest.artifact')}</dt>
							<dd class="break-all font-mono text-xs" data-testid="models-detail-digest-artifact">
								{show(detail.artifactDigest)}
							</dd>
							<dt class="text-muted-foreground">{$_('models.digest.config')}</dt>
							<dd class="break-all font-mono text-xs" data-testid="models-detail-digest-config">
								{show(detail.configDigest)}
							</dd>
							<dt class="text-muted-foreground">{$_('models.digest.calibration')}</dt>
							<dd class="break-all font-mono text-xs" data-testid="models-detail-digest-calibration">
								{show(detail.calibrationRef)}
							</dd>
							<dt class="text-muted-foreground">{$_('models.digest.row')}</dt>
							<dd class="break-all font-mono text-xs" data-testid="models-detail-digest-row">
								{show(detail.rowDigest)}
							</dd>
						</dl>
						<p class="text-sm text-muted-foreground" data-testid="models-digest-pin-note">
							{$_('models.digest.pinNote')}
						</p>
					</Card>

					<!-- The evaluation join is DATA. It is disclosed, bounded, and
					     carries no control: nothing here can move a status, and
					     nothing claims it makes `evaluated` reachable. -->
					<section
						class="grid gap-2"
						aria-labelledby="models-evaluation-toggle"
						data-testid="models-detail-evaluation"
					>
						<Button
							id="models-evaluation-toggle"
							variant="ghost"
							class="w-fit justify-start"
							aria-expanded={evaluationOpen}
							aria-controls="models-evaluation-panel"
							onclick={toggleEvaluation}
							data-testid="models-detail-evaluation-toggle"
						>
							{$_('models.evaluation.heading')}
						</Button>
						<p class="text-sm text-muted-foreground" data-testid="models-detail-evaluation-note">
							{$_('models.evaluation.nonAuthoritative')}
						</p>
						<ul id="models-evaluation-panel" hidden={!evaluationOpen} class="grid gap-1 text-sm">
							{#each detail.evaluationRefs as ref, index (`${ref}-${index}`)}
								<li class="font-mono text-xs" data-testid="models-detail-evaluation-item">
									{displayText(ref)}
								</li>
							{/each}
						</ul>
					</section>

					<Card class="gap-3 p-4" data-testid="models-detail-actions">
						{#if legalLifecycleActions(detail.status).length === 0}
							<p role="status" data-testid="models-action-none">{$_('models.action.none')}</p>
						{:else}
							<div class="flex flex-wrap gap-2">
								{#each legalLifecycleActions(detail.status) as action (action)}
									<Button
										variant="outline"
										disabled={proposalState === 'loading' || proposalState === 'conflict'}
										onclick={() => void propose(action)}
										data-testid={`models-action-${action}`}>{$_(`models.action.${action}`)}</Button
									>
								{/each}
							</div>
						{/if}
						<p class="text-sm text-muted-foreground" data-testid="models-proposal-remedy">
							{$_('models.proposal.remedy')}
						</p>

						{#if proposalState === 'loading'}
							<p aria-busy="true" data-testid="models-proposal-loading">{$_('models.loading')}</p>
						{:else if proposalState === 'pending'}
							<p role="status" data-testid="models-proposal-pending">
								{$_('models.proposal.pending')}
								{#if proposalId !== null}
									<span class="font-mono" data-testid="models-proposal-id">{proposalId}</span>
								{/if}
							</p>
						{:else if proposalState === 'conflict'}
							<!-- The row changed: the pinned bytes are stale, so the only
							     honest affordance is to re-read. A bare retry would
							     re-send a digest the kernel has already moved past. -->
							<Alert role="alert" data-testid="models-proposal-conflict">
								{$_('models.conflict.reload')}
							</Alert>
							<div>
								<Button variant="outline" size="sm" onclick={reloadDetail} data-testid="models-detail-reload">
									{$_('models.conflict.reload')}
								</Button>
							</div>
						{:else if proposalState === 'network'}
							<Alert variant="destructive" role="alert" data-testid="models-proposal-network">
								{$_('models.error.network')}
							</Alert>
						{:else if proposalState !== 'idle'}
							<Alert variant="destructive" role="alert" data-testid="models-proposal-error">
								{errorMessage(proposalStatus)}
							</Alert>
						{/if}
					</Card>
				</div>
			{/if}
		</div>
	{/if}
</div>
