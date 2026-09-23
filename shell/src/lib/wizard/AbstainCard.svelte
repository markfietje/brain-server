<script lang="ts">
	// The abstain/HITL card — the kernel contract rendered honestly: an
	// ambiguous or unanswered question STOPS the flow, never invents (D1;
	// the Art. 14-engineering posture — a human stays in command).
	import { _ } from 'svelte-i18n';

	interface Props {
		questionId: string;
	}
	let { questionId }: Props = $props();
</script>

<div class="abstain" role="status" data-testid="abstain-card">
	<strong>{$_('abstain.title')}</strong>
	<p>{$_('abstain.body')}</p>
	<p><code>{questionId}</code></p>
</div>
