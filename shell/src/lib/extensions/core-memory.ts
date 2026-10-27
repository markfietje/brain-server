/**
 * coreMemory — the recall surface (the sidebar's "Memory" entry; the route
 * name stayed `/recall` because the Dioxus counterpart carries the same path
 * and the trace detail nests under it).
 */

import LayoutGridIcon from '@lucide/svelte/icons/layout-grid';
import type { ExtensionManifest } from './types.js';

export const coreMemory: ExtensionManifest = {
	id: 'coreMemory',
	version: '0.1.1',
	contributes: {
		navEntries: [
			{ id: 'memory', route: '/recall', labelKey: 'nav.memory', Icon: LayoutGridIcon, order: 2 }
		]
	}
};
