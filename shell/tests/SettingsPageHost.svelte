<script lang="ts">
	// Test-only host: the settings PAGE needs SvelteKit's route context
	// (paraglide + the api client resolve fine, but the page imports
	// $app/... nothing — it is renderable standalone). This host exists for
	// symmetry with AppShellHost and to keep one import site if the page
	// ever needs route context mocked in one place.
	import SettingsPage from '../src/routes/settings/+page.svelte';
</script>

<SettingsPage />
