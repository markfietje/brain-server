<script lang="ts">
	// ONE question, rendered as typed controls through @sjsf (the pinned
	// field skin). The question's CLOSED vocabulary becomes the schema's
	// oneOf options — the form cannot express an out-of-vocabulary answer;
	// the branch engine refuses anything that slips through anyway. All
	// rendered template text passes the sanitizer (D9).
	import { createForm, BasicForm, type Schema } from '@sjsf/form';
	import { resolver } from '@sjsf/form/resolvers/basic';
	import { translation } from '@sjsf/form/translations/en';
	import { createFormMerger } from '@sjsf/form/mergers/modern';
	import { createFormIdBuilder } from '@sjsf/form/id-builders/modern';
	import { createFormValidator } from '@sjsf/ajv8-validator';
	import { theme } from '@sjsf/basic-theme';
	import '@sjsf/basic-theme/css/basic.css';
	import type { WizardQuestion } from '$lib/wizard/pack';
	import { sanitizeTemplateText } from '$lib/sanitize';
	import { recordTouch } from '$lib/telemetry/buffer';
	import { _ } from 'svelte-i18n';

	interface Props {
		question: WizardQuestion;
		onanswer: (given: unknown) => void;
	}
	let { question, onanswer }: Props = $props();

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

	function schemaFor(q: WizardQuestion): Schema {
		// The const carries the TYPED answer value — the kernel's
		// classify_answer reads choice labels as strings, score levels as
		// integers, and noul as booleans; the form's value must match so the
		// branch engine can classify it without coercion.
		const options = q.answerKeys.map((key) => ({
			const: q.qtype === 'score' ? Number(key) : q.qtype === 'noul' ? key === 'true' : key,
			title: titleOf(key)
		}));
		// The closed vocabulary as titled oneOf options — the schema object
		// is structurally loose at this construction boundary (JSON Schema),
		// so the composed definition casts to the library's Schema type.
		// The ROOT type follows the answer's kind: choice = string label,
		// score = integer level, noul = boolean.
		const def =
			q.qtype === 'score'
				? { type: 'integer', oneOf: options }
				: q.qtype === 'noul'
					? { type: 'boolean', oneOf: options }
					: { type: 'string', oneOf: options };
		return def as unknown as Schema;
	}

	const form = createForm<unknown>({
		theme,
		schema: schemaFor(question),
		resolver,
		translation,
		merger: createFormMerger,
		validator: createFormValidator,
		idBuilder: createFormIdBuilder,
		onSubmit: (value) => {
			onanswer(value);
		}
	});

	const formId = $derived(`wizard-q-${question.id}`);
	$effect(() => {
		recordTouch(question.id);
	});

	// The single continue path: requestSubmit runs the form's own validation
	// pipeline; only a VALID (in-vocabulary) value reaches onSubmit →
	// onanswer. The abstain path lives in the page, for refusals/absence.
	function requestContinue(event: Event): void {
		event.preventDefault();
		const el = document.getElementById(formId);
		if (el instanceof HTMLFormElement) el.requestSubmit();
	}
</script>

<section aria-labelledby="{formId}-heading">
	<h2 id="{formId}-heading">{sanitizeTemplateText(question.instructions)}</h2>
	<BasicForm id={formId} {form} novalidate />
	<button type="button" onclick={requestContinue}>{$_('flow.continue')}</button>
</section>
