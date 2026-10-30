<script lang="ts">
	// Test-only host: the frame WITH the settings page as its route content —
	// the real integration shape for pins that need both (the 16px law needs
	// the composer AND the settings page's controls in one tree).
	import AppShell from '../src/lib/components/AppShell.svelte';
	import SettingsPage from '../src/routes/settings/+page.svelte';
</script>

<AppShell>
	<SettingsPage />
</AppShell>
