/**
 * coreSettings — the settings dialog's sections, contributed IN ORDER: the
 * kernel-origin override, the session locale, the kernel token (Tauri only)
 * and the software update (Tauri only). The four were the dialog's inline
 * markup; the dialog now renders `{#each settingsSections}` and the frame
 * carries none of their state — the sections own it (the locale through the
 * shared `#lib/shell/locale` module, whose reader is the frame's clock).
 */

import LocaleSection from '#lib/components/settings/locale-section.svelte';
import UpdateSection from '#lib/components/settings/update-section.svelte';
import AdvancedSection from '#lib/components/settings/advanced-section.svelte';
import ModelSection from '#lib/components/settings/model-section.svelte';
import SignalSection from '#lib/components/settings/signal-section.svelte';
import RunModeSection from '#lib/components/settings/run-mode-section.svelte';
import type { ExtensionManifest } from './types.js';

export const coreSettings: ExtensionManifest = {
	id: 'coreSettings',
	version: '0.1.1',
	contributes: {
		settingsSections: [
			{ id: 'model', component: ModelSection, category: 'model' },
			{ id: 'locale', component: LocaleSection, category: 'general' },
			// W-2 (the wizard round): "How StewardMem runs" joins General — the
			// gate's one-time decision, re-runnable as a SECTION, not a dialog.
			// Direct channel only: the component renders nothing in the mas
			// build (its commands are compiled out there).
			{ id: 'runmode', component: RunModeSection, category: 'general' },
			{ id: 'update', component: UpdateSection, category: 'updates' },
			// The optional Signal gateway module (PLAN_MAS_FORK §1): an optional
			// connect-your-own-gateway surface — describe + detect, nothing more —
			// so it lives with the operator acts, not the onboarding steps.
			{ id: 'signal', component: SignalSection, category: 'advanced' },
			// The operator affordances FOLDED (the operator's UX call,
			// 2026-10-30): origin + token are rotation/override acts — the
			// credential bootstraps automatically, so they are not onboarding
			// steps. One collapsed disclosure instead of two top sections.
			{ id: 'advanced', component: AdvancedSection, category: 'advanced' }
		]
	}
};
