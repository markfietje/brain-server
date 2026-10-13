<script lang="ts">
	import '../app.css';
	import { bootstrapToken } from '#lib/api/bootstrap.js';
	import { applyPlatformAttr } from '#lib/platform.js';
	import AppShell from '#lib/components/AppShell.svelte';

	// Native platform feel: <html data-platform> keys the design tokens, and
	// form controls stay NATIVE (the OS webview draws its own popups/combos).
	// data-tauri gates the Overlay-titlebar inset (traffic-light clearance).
	applyPlatformAttr();
	if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
		document.documentElement.dataset.tauri = 'true';
	}

	// D7: the runtime token handoff (Tauri command → memory only).
	void bootstrapToken();

	let { children } = $props();
</script>

<AppShell>
	{@render children()}
</AppShell>
