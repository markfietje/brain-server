<script lang="ts">
	// The abstain/HITL card — the kernel contract rendered honestly: an
	// ambiguous or unanswered question STOPS the flow, never invents (D1;
	// the Art. 14-engineering posture — a human stays in command).
	import { _ } from 'svelte-i18n';
	import { Alert } from '$lib/components/ui/alert';
	import { AlertTitle } from '$lib/components/ui/alert';
	import { AlertDescription } from '$lib/components/ui/alert';

	interface Props {
		questionId: string;
	}
	let { questionId }: Props = $props();
</script>

<Alert
	role="status"
	data-testid="abstain-card"
	class="border-l-warn-border border-warn-border bg-warn-bg border-l-4"
>
	<AlertTitle class="text-base">{$_('abstain.title')}</AlertTitle>
	<AlertDescription>
		<p>{$_('abstain.body')}</p>
		<p><code class="mt-1 inline-block rounded-md bg-surface-2 px-1.5 py-0.5 text-sm">{questionId}</code></p>
	</AlertDescription>
</Alert>
