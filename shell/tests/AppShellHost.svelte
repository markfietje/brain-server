<script lang="ts">
	// Test-only host: AppShell needs a children SNIPPET, which only a Svelte
	// template can create — this host provides one for the jsdom render.
	import AppShell from '../src/lib/components/AppShell.svelte';
</script>

<AppShell>
	<p data-testid="host-content">host content</p>
</AppShell>
