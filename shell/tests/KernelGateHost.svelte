<script lang="ts">
	// Test-only host: KernelGate needs a children SNIPPET, which only a Svelte
	// template can create. The sentinel is the anti-vacuity witness — a gate
	// that renders the app on a refusal would show it.
	import KernelGate from '../src/lib/components/KernelGate.svelte';
</script>

<KernelGate>
	<p data-testid="host-content">gate opened</p>
</KernelGate>
