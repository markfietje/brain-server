<script lang="ts">
	// ONE question, rendered with the control its answer kind deserves:
	//   noul   → a macOS-style SWITCH (binary: the two criteria texts flank it)
	//   choice + score → a SEGMENTED CONTROL (small closed option sets)
	//   >5 options → a native select fallback (long lists stay lists)
	// All controls are real, accessible widgets (radio groups / role=switch);
	// the committed value is TYPED like the kernel's classify_answer reads it
	// (choice = label string, score = integer, noul = boolean) — never text.
	// Every rendered string passes the sanitizer (D9).
	import type { WizardQuestion } from '$lib/wizard/pack';
	import { sanitizeTemplateText } from '$lib/sanitize';
	import { recordTouch } from '$lib/telemetry/buffer';
	import { _ } from 'svelte-i18n';

	interface Props {
		question: WizardQuestion;
		onanswer: (given: unknown) => void;
	}
	let { question, onanswer }: Props = $props();

	const SEGMENT_MAX = 5;
	const useSegmented = $derived(
		question.qtype !== 'noul' && question.answerKeys.length <= SEGMENT_MAX
	);
	const useSwitch = $derived(question.qtype === 'noul');

	const titleOf = (key: string): string => {
		if (question.qtype === 'choice' || question.qtype === 'noul') {
			const criteria =
				question.criteria !== undefined &&
				question.criteria !== null &&
				typeof question.criteria === 'object' &&
				!Array.isArray(question.criteria)
					? (question.criteria as Record<string, unknown>)
					: {};
			const desc = criteria[key];
			return sanitizeTemplateText(typeof desc === 'string' && desc.length > 0 ? desc : key);
		}
		// score: "level i: <label>"
		const levels = Array.isArray(question.criteria) ? question.criteria : [];
		const label = levels[Number(key)];
		return sanitizeTemplateText(
			`level ${key}: ${typeof label === 'string' ? label : key}`
		);
	};

	/** The short segment label: the option's name (capitalized key, or the
	 * score level); the description rides as the segment's helper line. */
	const segmentLabel = (key: string): string => {
		if (question.qtype === 'score') {
			const levels = Array.isArray(question.criteria) ? question.criteria : [];
			const label = levels[Number(key)];
			return sanitizeTemplateText(typeof label === 'string' ? label : key);
		}
		const key_ = key.replace(/_/g, ' ');
		return sanitizeTemplateText(key_.charAt(0).toUpperCase() + key_.slice(1));
	};

	const segmentDesc = (key: string): string | null => {
		if (question.qtype !== 'choice') return null;
		const criteria =
			question.criteria !== null && typeof question.criteria === 'object'
				? (question.criteria as Record<string, unknown>)
				: {};
		const desc = criteria[key];
		return typeof desc === 'string' && desc.length > 0 ? sanitizeTemplateText(desc) : null;
	};

	let pending: string | null = $state(null); // the vocabulary key, undecided = null

	const formId = $derived(`wizard-q-${question.id}`);
	$effect(() => {
		recordTouch(question.id);
	});

	const typedValue = $derived(
		pending === null
			? null
			: question.qtype === 'score'
				? Number(pending)
				: question.qtype === 'noul'
					? pending === 'true'
					: pending
	);

	function commit(): void {
		if (typedValue === null) return;
		onanswer(typedValue);
	}

	// ── noul switch ──
	const noulTexts = $derived.by(() => {
		const criteria =
			question.criteria !== undefined && question.criteria !== null && typeof question.criteria === 'object'
				? (question.criteria as Record<string, unknown>)
				: {};
		const labelFor = (k: 'false' | 'true') => {
			const t = criteria[k];
			return sanitizeTemplateText(typeof t === 'string' && t.length > 0 ? t : k === 'true' ? 'Yes' : 'No');
		};
		return { false: labelFor('false'), true: labelFor('true') };
	});
</script>

<section aria-labelledby="{formId}-heading">
	<h2 id="{formId}-heading">{sanitizeTemplateText(question.instructions)}</h2>

	{#if useSwitch}
		<div class="switchrow">
			<button
				type="button"
				class="sidelabel"
				class:chosen={pending === 'false'}
				onclick={() => (pending = 'false')}
			>
				{noulTexts.false}
			</button>
			<button
				type="button"
				role="switch"
				aria-checked={pending === 'true'}
				aria-label={sanitizeTemplateText(question.instructions)}
				class="switch"
				class:on={pending === 'true'}
				class:off={pending === 'false'}
				onclick={() => (pending = pending === 'true' ? 'false' : 'true')}
			>
				<span class="thumb"></span>
			</button>
			<button
				type="button"
				class="sidelabel"
				class:chosen={pending === 'true'}
				onclick={() => (pending = 'true')}
			>
				{noulTexts.true}
			</button>
		</div>
	{:else if useSegmented}
		<fieldset class="segments" role="radiogroup" aria-labelledby="{formId}-heading">
			<legend class="sr-only">{sanitizeTemplateText(question.instructions)}</legend>
			{#each question.answerKeys as key (key)}
				<label class="segment" class:checked={pending === key}>
					<input
						type="radio"
						name={formId}
						value={key}
						checked={pending === key}
						onchange={() => (pending = key)}
					/>
					<span class="segment-label">{segmentLabel(key)}</span>
					{#if segmentDesc(key)}
						<span class="segment-desc">{segmentDesc(key)}</span>
					{/if}
				</label>
			{/each}
		</fieldset>
	{:else}
		<!-- Long option lists stay a list: the styled native select. -->
		<select
			id="{formId}-select"
			onchange={(e) => (pending = e.currentTarget.value)}
			aria-label={sanitizeTemplateText(question.instructions)}
		>
			<option value="" disabled selected>—</option>
			{#each question.answerKeys as key (key)}
				<option value={key}>{titleOf(key)}</option>
			{/each}
		</select>
	{/if}

	<button type="button" class="continue" disabled={pending === null} onclick={commit}>
		{$_('flow.continue')}
	</button>
</section>
