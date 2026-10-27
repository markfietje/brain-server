/**
 * coreGovernance — the governance surface (domains, roles, retention — what
 * the kernel enforces).
 */

import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
import type { ExtensionManifest } from './types.js';

export const coreGovernance: ExtensionManifest = {
	id: 'coreGovernance',
	version: '0.1.1',
	contributes: {
		navEntries: [
			{
				id: 'governance',
				route: '/governance',
				labelKey: 'nav.governance',
				Icon: ShieldCheckIcon,
				order: 5
			}
		]
	}
};
