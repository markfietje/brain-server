<script lang="ts">
	// The wizard renderer (D1): a branched one-question-at-a-time
	// conversation over the typed wire — NEVER a chatbot, never free text.
	// ONE wire read carries the whole catalog (the route serves ids WITH
	// validated pack bodies); branching happens CLIENT-SIDE mirroring the
	// kernel's tested next-map semantics; ambiguous or unanswered → the
	// abstain card (never invented); progress saves session-locally and
	// resumes; the answers export is the evidence artifact with the
	// local-only telemetry.
	import { _ } from 'svelte-i18n';
	import { fetchWizardPacks } from '$lib/api/client';
	import { parsePack, type ResolvedPack } from '$lib/wizard/pack';
	import { branchOf, classifyAnswer, firstStep } from '$lib/wizard/branch';
	import { buildAnswersExport } from '$lib/wizard/answers';
	import { saveProgress, restoreProgress, clearProgress } from '$lib/wizard/session';
	import { beginFlow, recordAnswer, recordError, recordAbandon } from '$lib/telemetry/buffer';
	import QuestionCard from '$lib/wizard/QuestionCard.svelte';
	import AbstainCard from '$lib/wizard/AbstainCard.svelte';
	import { Button } from '$lib/components/ui/button';
	import { Alert } from '$lib/components/ui/alert';
	import { Progress } from '$lib/components/ui/progress';
	import { Card } from '$lib/components/ui/card';

	interface CatalogEntry {
		id: string;
		question_count: number;
		pack: unknown;
	}

	type Mode =
		| { kind: 'loading' }
		| { kind: 'wire-error' }
		| { kind: 'picking'; packs: CatalogEntry[] }
		| { kind: 'flow'; pack: ResolvedPack; currentId: string; answers: Record<string, unknown> }
		| { kind: 'abstain'; pack: ResolvedPack; questionId: string }
		| { kind: 'done'; pack: ResolvedPack; answers: Record<string, unknown> };

	let mode: Mode = $state({ kind: 'loading' });
	let catalog: CatalogEntry[] = $state([]);
	let exportUrl: string | null = $state(null);

	async function loadCatalog(): Promise<void> {
		mode = { kind: 'loading' };
		try {
			const data = await fetchWizardPacks();
			const entries: CatalogEntry[] = [];
			for (const entry of data['packs']) {
				// The wire shape comes from the generated contract; the parse on
				// open is the renderer's own total gate on the pack body.
				if (
					entry &&
					typeof entry === 'object' &&
					typeof (entry as Record<string, unknown>)['id'] === 'string' &&
					typeof (entry as Record<string, unknown>)['question_count'] === 'number'
				) {
					entries.push(entry as CatalogEntry);
				}
			}
			catalog = entries;
			mode = { kind: 'picking', packs: entries };
		} catch {
			mode = { kind: 'wire-error' };
		}
	}

	function openPack(id: string): void {
		const entry = catalog.find((p) => p.id === id);
		if (!entry) return;
		const parsed = parsePack(entry.pack);
		if (!parsed.ok) {
			// The kernel validated it; the client refuses to render anything
			// it cannot itself prove. Honest refusal, never a guess.
			mode = { kind: 'wire-error' };
			return;
		}
		beginFlow();
		const restored = restoreProgress(id);
		if (restored && parsed.pack.byId.has(restored.currentQuestionId)) {
			mode = {
				kind: 'flow',
				pack: parsed.pack,
				currentId: restored.currentQuestionId,
				answers: { ...restored.answers }
			};
		} else {
			const first = firstStep(parsed.pack);
			mode =
				first.kind === 'question'
					? { kind: 'flow', pack: parsed.pack, currentId: first.id, answers: {} }
					: { kind: 'done', pack: parsed.pack, answers: {} };
		}
	}

	function submitAnswer(given: unknown): void {
		if (mode.kind !== 'flow') return;
		const { pack, currentId, answers } = mode;
		const q = pack.byId.get(currentId);
		if (!q) return;
		const key = classifyAnswer(q, given);
		if (key === null) {
			// Out of vocabulary: REFUSED — error recurrence measured (D8),
			// then the abstain card. Nothing is invented, nothing guessed.
			recordError(q.id);
			recordAbandon();
			clearProgress(pack.pack);
			mode = { kind: 'abstain', pack, questionId: q.id };
			return;
		}
		recordAnswer(q.id);
		const nextAnswers = { ...answers, [q.id]: given };
		const step = branchOf(pack, q, key);
		if (step.kind === 'end') {
			clearProgress(pack.pack);
			mode = { kind: 'done', pack, answers: nextAnswers };
			return;
		}
		saveProgress(pack, step.id, nextAnswers);
		mode = { kind: 'flow', pack, currentId: step.id, answers: nextAnswers };
	}

	function restart(): void {
		if (mode.kind !== 'done' && mode.kind !== 'abstain') return;
		clearProgress(mode.pack.pack);
		const first = firstStep(mode.pack);
		beginFlow();
		mode =
			first.kind === 'question'
				? { kind: 'flow', pack: mode.pack, currentId: first.id, answers: {} }
				: { kind: 'done', pack: mode.pack, answers: {} };
	}

	function exportAnswers(): void {
		if (mode.kind !== 'done') return;
		const json = buildAnswersExport(mode.pack, mode.answers);
		const blob = new Blob([json], { type: 'application/json' });
		if (exportUrl !== null) URL.revokeObjectURL(exportUrl);
		exportUrl = URL.createObjectURL(blob);
		const a = document.createElement('a');
		a.href = exportUrl;
		a.download = `wizard-answers-${mode.pack.pack}.json`;
		a.click();
	}

	// Narrowing helpers (plain functions — the svelte2tsx $derived
	// transformation narrows the state's INITIAL literal, so the union
	// checks happen here instead).
	function progressOf(m: Mode): number {
		if (m.kind === 'done') return 100;
		if (m.kind === 'flow') {
			const answered = Object.keys(m.answers).length;
			return Math.min(100, Math.round((answered / Math.max(1, m.pack.questions.length)) * 100));
		}
		return 0;
	}
	const progressPct = $derived(progressOf(mode));

	$effect(() => {
		// Mount: one wire read carries the whole catalog (bodies included).
		void loadCatalog();
		return () => {
			if (exportUrl !== null) URL.revokeObjectURL(exportUrl);
		};
	});
</script>

<div class="grid gap-4">
	<!-- The header doubles as the window drag region under the macOS
	     Overlay titlebar (inert outside Tauri). -->
	<div data-tauri-drag-region>
		<h1 data-tauri-drag-region>{$_('app.title')}</h1>
		<p class="text-muted-foreground" data-tauri-drag-region>{$_('app.subtitle')}</p>
	</div>

	{#if mode.kind === 'loading'}
		<p aria-busy="true">…</p>
	{:else if mode.kind === 'wire-error'}
		<Alert variant="destructive">{$_('wire.error')}</Alert>
	{:else if mode.kind === 'picking'}
		<Card class="p-5" aria-labelledby="pick-heading">
			<h2 id="pick-heading" class="text-lg font-semibold tracking-tight">{$_('pick.heading')}</h2>
			<div class="mt-3 grid gap-2.5">
				{#each mode.packs as pack (pack.id)}
					<Button
						variant="secondary"
						class="w-full justify-between text-left"
						onclick={() => openPack(pack.id)}
						data-pack={pack.id}
					>
						{pack.id} — {$_('pack.count', { values: { count: pack.question_count } })}
					</Button>
				{/each}
			</div>
		</Card>
	{:else if mode.kind === 'flow'}
		<div class="my-3">
			<Progress
				value={progressPct}
				max={100}
				aria-label={$_('flow.progress')}
				aria-valuemin={0}
				aria-valuemax={100}
				aria-valuenow={progressPct}
			/>
		</div>
		{#key mode.currentId}
			{@const q = mode.pack.byId.get(mode.currentId)}
			{#if q}
				<QuestionCard question={q} onanswer={submitAnswer} />
			{/if}
		{/key}
	{:else if mode.kind === 'abstain'}
		<AbstainCard questionId={mode.questionId} />
		<Button variant="outline" class="mt-2 w-full" onclick={restart}>{$_('flow.restart')}</Button>
	{:else if mode.kind === 'done'}
		<p role="status">{$_('flow.done')}</p>
		<div class="mt-3 grid gap-2.5">
			<Button onclick={exportAnswers} data-testid="export">{$_('flow.export')}</Button>
			<Button variant="outline" onclick={restart}>{$_('flow.restart')}</Button>
		</div>
	{/if}
</div>
