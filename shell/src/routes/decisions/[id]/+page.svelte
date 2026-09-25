<script lang="ts">
	import { tick } from 'svelte';
	import { _ } from 'svelte-i18n';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { captureNextRawText, client } from '$lib/api/client';
	import {
		classifyDecisionStatus,
		createDecisionRequestGuard,
		parseDecisionRouteId,
		parseDecisionRunDetail,
		parseReplayAgreement,
		rawCaptureWithinLimit,
		type DecisionMode,
		type DecisionRunDetail,
		type DecisionStatus,
		type ReplayAgreement
	} from '$lib/decision-run';
	import { sanitizeTemplateText, stripInvisible } from '$lib/sanitize';
	import { Alert } from '$lib/components/ui/alert';
	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Card } from '$lib/components/ui/card';
	import { Input } from '$lib/components/ui/input';
	import { Label } from '$lib/components/ui/label';
	import { Textarea } from '$lib/components/ui/textarea';

	type DetailState =
		| 'loading'
		| 'ready'
		| 'invalid'
		| 'unauthorized'
		| 'forbidden'
		| 'not_found'
		| 'conflict'
		| 'bad_request'
		| 'malformed'
		| 'network'
		| 'server'
		| 'unknown';
	type ReplayState =
		| 'idle'
		| 'loading'
		| 'ready'
		| 'invalid'
		| 'unauthorized'
		| 'forbidden'
		| 'not_found'
		| 'conflict'
		| 'bad_request'
		| 'malformed'
		| 'network'
		| 'server'
		| 'unknown';
	type QuestionKind = 'choice' | 'score' | 'noul';
	type ReplayRequest = {
		config: Record<string, unknown>;
		rules_config: Record<string, unknown>;
		mode: DecisionMode;
		request_id: string;
		question_id?: string;
		question_kind?: QuestionKind;
		question_ids: string[];
		query: string;
	};
	type GeneratedReplayRequest = Omit<ReplayRequest, 'config' | 'rules_config'> & {
		config: Record<string, never>;
		rules_config: Record<string, never>;
	};
	const REPLAY_DOCUMENT_MAX_CHARS = 262_144;

	let routeId = $state<number | null>(null);
	let view = $state<DecisionRunDetail | null>(null);
	let rawText = $state<string | null>(null);
	let rawOmitted = $state(false);
	let viewState = $state<DetailState>('loading');
	let status = $state<DecisionStatus | null>(null);
	let expandedStages = $state<Record<number, boolean>>({});
	let replayConfig = $state('');
	let replayRules = $state('');
	let replayRequestId = $state('');
	let replayQuestionIds = $state('');
	let replayQuestionId = $state('');
	let replayQuestionKind = $state('');
	let replayQuery = $state('');
	let replayState = $state<ReplayState>('idle');
	let replayStatus = $state<DecisionStatus | null>(null);
	let replayFormError = $state(false);
	let replayReport = $state<ReplayAgreement | null>(null);
	let headingElement: HTMLHeadingElement | undefined = $state();
	const requestGuard = createDecisionRequestGuard();

	function isRecord(value: unknown): value is Record<string, unknown> {
		return typeof value === 'object' && value !== null && !Array.isArray(value);
	}

	function statusMessage(current: DecisionStatus | null, fallback: string): string {
		if (current === null) return fallback;
		switch (current.kind) {
			case 'unauthorized':
				return $_('decisions.detail.unauthorized');
			case 'forbidden':
				return $_('decisions.detail.forbidden');
			case 'not_found':
				return $_('decisions.detail.not_found');
			case 'conflict':
				return $_('decisions.detail.conflict');
			case 'bad_request':
				return $_('decisions.detail.bad_request');
			case 'server':
				return $_('decisions.detail.server');
			default:
				return $_('decisions.detail.unknown');
		}
	}

	function replayStatusMessage(current: DecisionStatus | null): string {
		if (current === null) return $_('decisions.replay.unknown');
		switch (current.kind) {
			case 'unauthorized':
				return $_('decisions.replay.401');
			case 'forbidden':
				return $_('decisions.replay.403');
			case 'not_found':
				return $_('decisions.replay.404');
			case 'conflict':
				return $_('decisions.replay.409');
			case 'bad_request':
				return $_('decisions.replay.400');
			case 'server':
				return $_('decisions.replay.server');
			default:
				return $_('decisions.replay.unknown');
		}
	}

	function detailStateForStatus(current: DecisionStatus): DetailState {
		switch (current.kind) {
			case 'unauthorized':
				return 'unauthorized';
			case 'forbidden':
				return 'forbidden';
			case 'not_found':
				return 'not_found';
			case 'conflict':
				return 'conflict';
			case 'bad_request':
				return 'bad_request';
			case 'server':
				return 'server';
			default:
				return 'unknown';
		}
	}

	function replayStateForStatus(current: DecisionStatus): ReplayState {
		switch (current.kind) {
			case 'unauthorized':
				return 'unauthorized';
			case 'forbidden':
				return 'forbidden';
			case 'not_found':
				return 'not_found';
			case 'conflict':
				return 'conflict';
			case 'bad_request':
				return 'bad_request';
			case 'server':
				return 'server';
			default:
				return 'unknown';
		}
	}

	function modeLabel(mode: DecisionMode): string {
		return mode === 'deterministic'
			? $_('decisions.mode.deterministic')
			: $_('decisions.mode.exploratory');
	}

	function display(value: string | number | null): string {
		return value === null || value === '' ? '—' : String(value);
	}

	function rawPreview(): string {
		return rawText === null ? '' : sanitizeTemplateText(rawText);
	}

	function startedAtLabel(nanoseconds: number): string {
		const milliseconds = nanoseconds / 1_000_000;
		if (!Number.isFinite(milliseconds) || Math.abs(milliseconds) > 8.64e15) {
			return String(nanoseconds);
		}
		try {
			return new Date(milliseconds).toISOString();
		} catch {
			return String(nanoseconds);
		}
	}

	async function focusDetailHeading(generation: number): Promise<void> {
		await tick();
		if (requestGuard.isCurrent(generation)) headingElement?.focus();
	}

	function toggleStage(index: number): void {
		expandedStages = { ...expandedStages, [index]: !expandedStages[index] };
	}

	function stageExpanded(index: number): boolean {
		return expandedStages[index] === true;
	}

	function clearReplayBundle(): void {
		replayConfig = '';
		replayRules = '';
		replayRequestId = '';
		replayQuestionIds = '';
		replayQuestionId = '';
		replayQuestionKind = '';
		replayQuery = '';
	}

	function hasControlOrInvisible(value: string): boolean {
		for (const character of value) {
			const codePoint = character.codePointAt(0);
			if (codePoint !== undefined && (codePoint <= 0x1f || codePoint === 0x7f)) return true;
		}
		return stripInvisible(value) !== value;
	}

	function parseJsonObject(value: string): Record<string, unknown> | null {
		if (value.trim().length === 0 || value.length > REPLAY_DOCUMENT_MAX_CHARS) return null;
		try {
			const parsed: unknown = JSON.parse(value);
			return isRecord(parsed) ? parsed : null;
		} catch {
			return null;
		}
	}

	function replayReady(): boolean {
		return (
			view !== null &&
			replayConfig.trim().length > 0 &&
			replayConfig.length <= REPLAY_DOCUMENT_MAX_CHARS &&
			replayRules.trim().length > 0 &&
			replayRules.length <= REPLAY_DOCUMENT_MAX_CHARS &&
			replayRequestId.trim().length > 0 &&
			replayQuestionIds.trim().length > 0 &&
			replayQuery.length > 0 &&
			replayState !== 'loading'
		);
	}

	function parseQuestionKind(value: string): QuestionKind | null {
		if (value === 'choice' || value === 'score' || value === 'noul') return value;
		return null;
	}

	function buildReplayRequest(): ReplayRequest | null {
		if (view === null) return null;
		const config = parseJsonObject(replayConfig);
		const rules = parseJsonObject(replayRules);
		const requestId = replayRequestId.trim();
		const query = replayQuery;
		const questionIds = replayQuestionIds
			.split(',')
			.map((value) => value.trim())
			.filter((value) => value.length > 0);
		const questionId = replayQuestionId.trim();
		const questionKind = parseQuestionKind(replayQuestionKind);
		if (
			config === null ||
			rules === null ||
			replayConfig.length > REPLAY_DOCUMENT_MAX_CHARS ||
			replayRules.length > REPLAY_DOCUMENT_MAX_CHARS ||
			requestId.length === 0 ||
			requestId.length > 256 ||
			hasControlOrInvisible(requestId) ||
			query.length === 0 ||
			query.length > 4096 ||
			hasControlOrInvisible(query) ||
			questionIds.length === 0 ||
			questionIds.length > 64 ||
			questionIds.some((value) => value.length > 256 || hasControlOrInvisible(value)) ||
			questionId.length > 256 ||
			hasControlOrInvisible(questionId) ||
			(questionId.length === 0) !== (questionKind === null) ||
			(replayQuestionKind.length > 0 && questionKind === null)
		) {
			return null;
		}
		const body: ReplayRequest = {
			config,
			rules_config: rules,
			mode: view.mode,
			request_id: requestId,
			question_ids: questionIds,
			query
		};
		if (questionId.length > 0 && questionKind !== null) {
			body.question_id = questionId;
			body.question_kind = questionKind;
		}
		return body;
	}

	async function loadDetail(id: number, generation: number): Promise<void> {
		viewState = 'loading';
		status = null;
		view = null;
		rawText = null;
		rawOmitted = false;
		try {
			captureNextRawText((text) => {
				if (!requestGuard.isCurrent(generation)) return;
				if (rawCaptureWithinLimit(text)) {
					rawText = text;
					rawOmitted = false;
				} else {
					rawText = null;
					rawOmitted = true;
				}
			});
			const result = await client.GET('/workflow/decision-runs/{id}', {
				params: { path: { id } }
			});
			if (!requestGuard.isCurrent(generation)) return;
			if (result.response && result.response.status !== 200) {
				rawText = null;
				status = classifyDecisionStatus(result.response.status);
				viewState = detailStateForStatus(status);
				return;
			}
			if (result.error !== undefined || result.data === undefined) {
				viewState = 'malformed';
				return;
			}
			const parsed = parseDecisionRunDetail(result.data);
			if (!parsed.ok) {
				viewState = 'malformed';
				return;
			}
			view = parsed.value;
			viewState = 'ready';
		} catch {
			if (!requestGuard.isCurrent(generation)) return;
			viewState = 'network';
		}
	}

	// The generated OpenAPI type erases free-form JSON as Record<string, never>.
	// The objects above were parsed from bounded operator input; this round-trip
	// keeps that single generated-client seam typed without editing schema.d.ts.
	function generatedJsonObject(value: Record<string, unknown>): Record<string, never> {
		const serialized = JSON.stringify(value);
		return serialized === undefined ? {} : JSON.parse(serialized);
	}

	function generatedReplayBody(body: ReplayRequest): GeneratedReplayRequest {
		return {
			...body,
			config: generatedJsonObject(body.config),
			rules_config: generatedJsonObject(body.rules_config)
		};
	}

	async function submitReplay(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		const id = routeId;
		if (view === null || id === null || replayState === 'loading') return;
		const body = buildReplayRequest();
		if (body === null) {
			replayFormError = true;
			replayState = 'invalid';
			return;
		}
		replayFormError = false;
		replayState = 'loading';
		replayStatus = null;
		replayReport = null;
		try {
			const result = await client.POST('/workflow/decision-runs/{id}/replay-diff', {
				params: { path: { id } },
				body: generatedReplayBody(body)
			});
			if (result.response && result.response.status !== 200) {
				replayStatus = classifyDecisionStatus(result.response.status);
				replayState = replayStateForStatus(replayStatus);
				return;
			}
			if (result.error !== undefined || result.data === undefined) {
				replayState = 'malformed';
				return;
			}
			const parsed = parseReplayAgreement(result.data);
			if (!parsed.ok || parsed.value.traceId !== id) {
				replayState = 'malformed';
				return;
			}
			replayReport = parsed.value;
			replayState = 'ready';
		} catch {
			replayState = 'network';
		} finally {
			clearReplayBundle();
		}
	}

	function exportRaw(): void {
		if (routeId === null || rawText === null) return;
		const blob = new Blob([rawText], { type: 'application/json' });
		const url = URL.createObjectURL(blob);
		const anchor = document.createElement('a');
		anchor.href = url;
		anchor.download = `decision-${routeId}.json`;
		anchor.click();
		URL.revokeObjectURL(url);
	}

	$effect(() => {
		const raw = page.params['id'];
		const parsedId = parseDecisionRouteId(raw);
		const generation = requestGuard.begin();
		routeId = parsedId;
		expandedStages = {};
		clearReplayBundle();
		replayState = 'idle';
		replayStatus = null;
		replayReport = null;
		replayFormError = false;
		if (parsedId === null) {
			view = null;
			viewState = 'invalid';
			rawText = null;
			void focusDetailHeading(generation);
			return;
		}
		void loadDetail(parsedId, generation).then(() => focusDetailHeading(generation));
		return () => {
			clearReplayBundle();
			requestGuard.begin();
		};
	});
</script>

<div class="grid gap-5" data-testid="decision-detail">
	<div class="flex items-start justify-between gap-3">
		<div>
			<h1 bind:this={headingElement} tabindex="-1">
				{$_('decisions.detail.title', { values: { id: page.params['id'] ?? '' } })}
			</h1>
			<p class="text-muted-foreground">{$_('decisions.detail.subtitle')}</p>
		</div>
		<Button variant="ghost" size="sm" href={resolve('/decisions')} data-testid="decision-back">
			{$_('decisions.detail.back')}
		</Button>
	</div>

	{#if viewState === 'loading'}
		<p aria-busy="true" data-testid="decision-detail-loading">{$_('decisions.detail.loading')}</p>
	{:else if viewState === 'invalid'}
		<Alert role="status" data-testid="decision-detail-invalid">{$_('decisions.detail.invalid')}</Alert>
	{:else if viewState === 'not_found'}
		<Alert role="status" data-testid="decision-detail-not-found">{$_('decisions.detail.not_found')}</Alert>
	{:else if viewState === 'malformed'}
		<Alert variant="destructive" role="alert" data-testid="decision-detail-malformed">
			{$_('decisions.detail.malformed')}
		</Alert>
	{:else if viewState === 'network'}
		<Alert variant="destructive" role="alert" data-testid="decision-detail-network">
			{$_('decisions.detail.network')}
		</Alert>
	{:else if viewState !== 'ready'}
		<Alert variant="destructive" role="alert" data-testid="decision-detail-error">
			{statusMessage(status, $_('decisions.detail.unknown'))}
		</Alert>
	{:else if view !== null}
		<div class="grid gap-4" data-testid="decision-detail-ready">
			{#if view.mode === 'exploratory'}
				<Alert role="status" data-testid="decision-detail-nonpromotable">
					{$_('decisions.list.nonpromotable')}
				</Alert>
			{/if}

			<Card class="gap-3 p-4">
				<h2 class="font-semibold">{$_('decisions.detail.identity')}</h2>
				<dl class="grid gap-2 text-sm sm:grid-cols-[12rem_1fr]">
					<dt class="text-muted-foreground">{$_('decisions.detail.mode')}</dt>
					<dd><Badge variant={view.mode === 'exploratory' ? 'outline' : 'secondary'}>{modeLabel(view.mode)}</Badge></dd>
					<dt class="text-muted-foreground">{$_('decisions.detail.action')}</dt>
					<dd data-testid="decision-outcome">{view.outcome.action}</dd>
					<dt class="text-muted-foreground">{$_('decisions.detail.config_hash')}</dt>
					<dd class="break-all font-mono text-xs">{view.configHash}</dd>
					<dt class="text-muted-foreground">{$_('decisions.detail.input_digest')}</dt>
					<dd class="break-all font-mono text-xs">{view.inputDigest}</dd>
					<dt class="text-muted-foreground">Run ID</dt>
					<dd class="font-mono">{view.runId}</dd>
					<dt class="text-muted-foreground">Pipeline</dt>
					<dd class="font-mono">{view.pipelineVersion}</dd>
				</dl>
				{#if view.outcome.escalation !== null}
					<div class="rounded border border-warn-border bg-warn-bg p-3 text-sm" data-testid="decision-escalation">
						<strong>{$_('decisions.detail.escalation')}:</strong>
						{view.outcome.escalation.stage} · {view.outcome.escalation.reason}
						<div>{view.outcome.escalation.detail}</div>
					</div>
				{/if}
			</Card>

			<Card class="gap-3 p-4">
				<h2 class="font-semibold">{$_('decisions.detail.models')}</h2>
				{#if view.modelRefs.length === 0}
					<p role="status">—</p>
				{:else}
					<div class="overflow-x-auto">
						<table class="w-full text-sm" data-testid="decision-model-table">
							<caption class="sr-only">{$_('decisions.detail.models')}</caption>
							<thead><tr class="border-b text-xs text-muted-foreground">
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.model_id')}</th>
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.model_version')}</th>
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.weights_digest')}</th>
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.registry')}</th>
							</tr></thead>
							<tbody>
								{#each view.modelRefs as model, index (`${model.id}-${index}`)}
									<tr class="border-b last:border-b-0">
										<td class="px-2 py-2 font-mono">{model.id}</td>
										<td class="px-2 py-2 font-mono">{model.version}</td>
										<td class="break-all px-2 py-2 font-mono text-xs">{display(model.weightsDigest)}</td>
										<td class="px-2 py-2 font-mono text-xs">{model.registryId === null ? '—' : `${model.registryId}@${model.registryVersion ?? ''}`}</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</Card>

			<Card class="gap-3 p-4">
				<h2 class="font-semibold">{$_('decisions.detail.context')}</h2>
				{#if view.contextRefs.length === 0}
					<p role="status">—</p>
				{:else}
					<div class="overflow-x-auto">
						<table class="w-full text-sm" data-testid="decision-context-table">
							<caption class="sr-only">{$_('decisions.detail.context')}</caption>
							<thead><tr class="border-b text-xs text-muted-foreground">
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.evidence_id')}</th>
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.content_digest')}</th>
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.tier')}</th>
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.ranks')}</th>
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.score')}</th>
								<th scope="col" class="px-2 py-2 text-start">{$_('decisions.detail.flags')}</th>
							</tr></thead>
							<tbody>
								{#each view.contextRefs as context, index (`${context.evidenceId}-${index}`)}
									<tr class="border-b last:border-b-0">
										<td class="px-2 py-2 font-mono">{context.evidenceId}</td>
										<td class="max-w-[18rem] break-all px-2 py-2 font-mono text-xs">{context.contentDigest}</td>
										<td class="px-2 py-2">{context.tier}</td>
										<td class="px-2 py-2 font-mono text-xs">{context.vectorRank ?? '—'} / {context.ftsRank ?? '—'} / {context.graphRank ?? '—'}</td>
										<td class="px-2 py-2 font-mono text-xs">{context.fusedScore ?? '—'}</td>
										<td class="px-2 py-2 text-xs">{context.flagged ? 'flagged' : '—'} / {context.untrusted ? 'untrusted' : '—'}</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</Card>

			<Card class="gap-3 p-4">
				<h2 class="font-semibold">{$_('decisions.detail.retrieval')}</h2>
				<dl class="grid gap-2 text-sm sm:grid-cols-[12rem_1fr]">
					<dt class="text-muted-foreground">{$_('decisions.detail.rrf_k')}</dt><dd>{view.retrievalParams.rrfK}</dd>
					<dt class="text-muted-foreground">{$_('decisions.detail.limit')}</dt><dd>{view.retrievalParams.limit}</dd>
					<dt class="text-muted-foreground">{$_('decisions.detail.leg')}</dt><dd>{view.retrievalParams.leg}</dd>
					<dt class="text-muted-foreground">{$_('decisions.detail.kernel')}</dt><dd>{view.envFingerprint.kernelVersion}</dd>
					<dt class="text-muted-foreground">{$_('decisions.detail.features')}</dt><dd>{view.envFingerprint.featureFlags.length === 0 ? '—' : view.envFingerprint.featureFlags.join(', ')}</dd>
				</dl>
			</Card>

			<section class="grid gap-3" aria-labelledby="decision-stages-heading">
				<h2 id="decision-stages-heading" class="font-semibold">{$_('decisions.detail.stages')}</h2>
				{#if view.stages.length === 0}
					<p role="status">{$_('decisions.detail.no_stages')}</p>
				{:else}
					{#each view.stages as stage, index (stage.stage)}
						<Card class="gap-2 p-3" data-testid="decision-stage">
							<Button
								variant="ghost"
								class="w-full justify-between"
								aria-expanded={stageExpanded(index)}
								aria-controls={`decision-stage-panel-${index}`}
								onclick={() => toggleStage(index)}
							>
								<span>{stage.stage}</span>
								<span class="text-xs text-muted-foreground">{stage.algorithm}</span>
							</Button>
							{#if stageExpanded(index)}
								<div id={`decision-stage-panel-${index}`} class="grid gap-2 border-t pt-3 text-sm">
									<div>{$_('decisions.detail.outputs')}:</div>
									<pre class="max-h-64 overflow-auto rounded border bg-muted p-3 text-xs" data-testid="decision-stage-output">{sanitizeTemplateText(stage.outputPreview ?? '—')}</pre>
									<dl class="grid gap-1 sm:grid-cols-[12rem_1fr]" data-testid="decision-stage-timing">
										<dt class="text-muted-foreground">{$_('decisions.detail.config_hash')}</dt><dd class="break-all font-mono text-xs">{stage.configHash}</dd>
										<dt class="text-muted-foreground">Input digest</dt><dd class="break-all font-mono text-xs">{stage.inputsDigest}</dd>
										<dt class="text-muted-foreground">Output digest</dt><dd class="break-all font-mono text-xs">{stage.outputsDigest}</dd>
										<dt class="text-muted-foreground">{$_('decisions.detail.tiers_seen')}</dt><dd>{stage.trustTiersSeen.join(', ') || '—'}</dd>
										<dt class="text-muted-foreground">{$_('decisions.detail.started')}</dt><dd>{startedAtLabel(stage.startedAt)}</dd>
										<dt class="text-muted-foreground">{$_('decisions.detail.duration')}</dt><dd>{stage.durationNs}</dd>
									</dl>
								</div>
							{/if}
						</Card>
					{/each}
				{/if}
			</section>

			<Card class="gap-3 p-4">
				<div class="flex items-center justify-between gap-3">
					<h2 class="font-semibold">{$_('decisions.detail.raw')}</h2>
					{#if rawText !== null}
						<Button variant="outline" size="sm" onclick={exportRaw} data-testid="decision-raw-download">
							{$_('decisions.detail.download')}
						</Button>
					{/if}
				</div>
				{#if rawOmitted}
					<p role="status" data-testid="decision-raw-omitted">{$_('decisions.detail.raw_omitted')}</p>
				{:else if rawText === null}
					<p role="status" data-testid="decision-raw-unavailable">{$_('decisions.detail.no_raw')}</p>
				{:else}
					<pre class="max-h-96 overflow-auto rounded border bg-muted p-3 text-xs" data-testid="decision-raw-json" aria-label={$_('decisions.detail.raw')}>{rawPreview()}</pre>
				{/if}
			</Card>

			<Card class="gap-4 p-4" data-testid="decision-replay">
				<h2 class="font-semibold">{$_('decisions.replay.title')}</h2>
				<p class="text-sm text-muted-foreground">{$_('decisions.replay.notice')}</p>
				<form class="grid gap-3" onsubmit={submitReplay}>
					<div class="grid gap-1.5">
						<Label for="replay-config">{$_('decisions.replay.config')}</Label>
						<Textarea id="replay-config" bind:value={replayConfig} rows={4} maxlength={REPLAY_DOCUMENT_MAX_CHARS} aria-invalid={replayState === 'invalid'} data-testid="replay-config" />
					</div>
					<div class="grid gap-1.5">
						<Label for="replay-rules">{$_('decisions.replay.rules')}</Label>
						<Textarea id="replay-rules" bind:value={replayRules} rows={4} maxlength={REPLAY_DOCUMENT_MAX_CHARS} aria-invalid={replayState === 'invalid'} data-testid="replay-rules" />
					</div>
					<div class="grid gap-1.5">
						<Label for="replay-request-id">{$_('decisions.replay.request_id')}</Label>
						<Input id="replay-request-id" bind:value={replayRequestId} aria-invalid={replayState === 'invalid'} data-testid="replay-request-id" />
					</div>
					<div class="grid gap-1.5">
						<Label for="replay-question-ids">{$_('decisions.replay.question_ids')}</Label>
						<Input id="replay-question-ids" bind:value={replayQuestionIds} aria-invalid={replayState === 'invalid'} data-testid="replay-question-ids" />
					</div>
					<div class="grid gap-1.5 sm:grid-cols-2">
						<div class="grid gap-1.5">
							<Label for="replay-question-id">{$_('decisions.replay.question_id')}</Label>
							<Input id="replay-question-id" bind:value={replayQuestionId} data-testid="replay-question-id" />
						</div>
						<div class="grid gap-1.5">
							<Label for="replay-question-kind">{$_('decisions.replay.question_kind')}</Label>
							<select id="replay-question-kind" bind:value={replayQuestionKind} data-testid="replay-question-kind">
								<option value="">—</option>
								<option value="choice">choice</option>
								<option value="score">score</option>
								<option value="noul">noul</option>
							</select>
						</div>
					</div>
					<div class="grid gap-1.5">
						<Label for="replay-query">{$_('decisions.replay.query')}</Label>
						<Textarea id="replay-query" bind:value={replayQuery} rows={3} aria-invalid={replayState === 'invalid'} data-testid="replay-query" />
					</div>
					<div class="flex items-center gap-3 text-sm">
						<span class="text-muted-foreground">{$_('decisions.replay.mode')}</span>
						<output data-testid="replay-mode">{modeLabel(view.mode)}</output>
					</div>
					{#if replayFormError || replayState === 'invalid'}
						<p role="alert" class="text-sm text-destructive" data-testid="replay-invalid">{$_('decisions.replay.invalid')}</p>
					{/if}
					<Button type="submit" disabled={!replayReady()} data-testid="replay-submit">
						{$_('decisions.replay.submit')}
					</Button>
					<p class="text-xs text-muted-foreground">{$_('decisions.replay.required')}</p>
				</form>

				{#if replayState === 'loading'}
					<p aria-busy="true" data-testid="replay-loading">{$_('decisions.replay.loading')}</p>
				{:else if replayState === 'network'}
					<Alert variant="destructive" role="alert" data-testid="replay-network">{$_('decisions.replay.network')}</Alert>
				{:else if replayState === 'malformed'}
					<Alert variant="destructive" role="alert" data-testid="replay-malformed">{$_('decisions.replay.malformed')}</Alert>
				{:else if replayState === 'ready' && replayReport !== null}
					<div class="grid gap-3" data-testid="decision-replay-report">
						<p class="text-sm font-semibold">{$_('decisions.replay.success')}</p>
						<dl class="grid gap-1 text-sm sm:grid-cols-[14rem_1fr]">
							<dt class="text-muted-foreground">{$_('decisions.replay.config_agreement')}</dt><dd>{String(replayReport.configHashMatch)}</dd>
							<dt class="text-muted-foreground">{$_('decisions.replay.input_agreement')}</dt><dd>{String(replayReport.inputDigestMatch)}</dd>
							<dt class="text-muted-foreground">{$_('decisions.replay.all_agreement')}</dt><dd>{String(replayReport.allMatch)}</dd>
						</dl>
						<div class="overflow-x-auto">
							<table class="w-full text-sm" data-testid="replay-stage-table">
								<caption class="sr-only">{$_('decisions.replay.success')}</caption>
								<thead><tr class="border-b text-xs text-muted-foreground">
									<th scope="col" class="px-2 py-2 text-start">{$_('decisions.replay.stage')}</th>
									<th scope="col" class="px-2 py-2 text-start">{$_('decisions.replay.match')}</th>
									<th scope="col" class="px-2 py-2 text-start">{$_('decisions.replay.stored_digest')}</th>
									<th scope="col" class="px-2 py-2 text-start">{$_('decisions.replay.replayed_digest')}</th>
								</tr></thead>
								<tbody>
									{#each replayReport.stages as stage, index (`${stage.stage}-${index}`)}
										<tr class="border-b last:border-b-0">
											<td class="px-2 py-2 font-mono">{stage.stage}</td>
											<td class="px-2 py-2">{String(stage.match)}</td>
											<td class="max-w-[16rem] break-all px-2 py-2 font-mono text-xs">{stage.storedOutputsDigest}</td>
											<td class="max-w-[16rem] break-all px-2 py-2 font-mono text-xs">{stage.replayedOutputsDigest}</td>
										</tr>
									{/each}
								</tbody>
							</table>
						</div>
					</div>
				{:else if replayStatus !== null}
					<Alert variant="destructive" role="alert" data-testid="replay-error">{replayStatusMessage(replayStatus)}</Alert>
				{/if}
			</Card>
		</div>
	{/if}
</div>
