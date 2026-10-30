<script lang="ts">
	// Test-only host: the settings PAGE rendered INSIDE the frame — the real
	// integration shape. The R129 re-read law needs the FRAME (the credential
	// epoch's glance-read effects live in AppShell), and the page is the
	// route content the frame hosts after the 2026-10-30 settings-page round.
	import AppShell from '../src/lib/components/AppShell.svelte';
	import SettingsPage from '../src/routes/settings/+page.svelte';
</script>

<AppShell>
	<SettingsPage />
</AppShell>
