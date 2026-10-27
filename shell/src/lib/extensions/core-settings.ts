/**
 * coreSettings — the settings dialog's sections, contributed IN ORDER: the
 * kernel-origin override, the session locale, the kernel token (Tauri only)
 * and the software update (Tauri only). The four were the dialog's inline
 * markup; the dialog now renders `{#each settingsSections}` and the frame
 * carries none of their state — the sections own it (the locale through the
 * shared `#lib/shell/locale` module, whose reader is the frame's clock).
 */

import OriginSection from '#lib/components/settings/origin-section.svelte';
import LocaleSection from '#lib/components/settings/locale-section.svelte';
import TokenSection from '#lib/components/settings/token-section.svelte';
import UpdateSection from '#lib/components/settings/update-section.svelte';
import type { ExtensionManifest } from './types.js';

export const coreSettings: ExtensionManifest = {
	id: 'coreSettings',
	version: '0.1.1',
	contributes: {
		settingsSections: [
			{ id: 'origin', component: OriginSection },
			{ id: 'locale', component: LocaleSection },
			{ id: 'token', component: TokenSection },
			{ id: 'update', component: UpdateSection }
		]
	}
};
