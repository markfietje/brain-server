<script lang="ts">
	import '../app.css';
	import { addMessages, init, getLocaleFromNavigator } from 'svelte-i18n';
	import en from '$lib/i18n/en.json';
	import de from '$lib/i18n/de.json';
	import fr from '$lib/i18n/fr.json';
	import es from '$lib/i18n/es.json';
	import nl from '$lib/i18n/nl.json';
	import { bootstrapToken } from '$lib/api/bootstrap';

	// The Dioxus parity locale set (D9). The catalogs' keys parity is a red
	// test, not a hope.
	addMessages('en', en);
	addMessages('de', de);
	addMessages('fr', fr);
	addMessages('es', es);
	addMessages('nl', nl);
	init({ fallbackLocale: 'en', initialLocale: getLocaleFromNavigator() ?? 'en' });

	// D7: the runtime token handoff (Tauri command → memory only).
	void bootstrapToken();

	let { children } = $props();
</script>

{@render children()}
