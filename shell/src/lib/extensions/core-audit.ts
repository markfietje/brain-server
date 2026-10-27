/**
 * coreAudit — the audit surface (the hash chain, displayed).
 */

import ListIcon from '@lucide/svelte/icons/list';
import type { ExtensionManifest } from './types.js';

export const coreAudit: ExtensionManifest = {
	id: 'coreAudit',
	version: '0.1.1',
	contributes: {
		navEntries: [
			{ id: 'audit', route: '/audit', labelKey: 'nav.audit', Icon: ListIcon, order: 4 }
		]
	}
};
