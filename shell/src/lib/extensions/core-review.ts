/**
 * coreReview — the review queue surface (the write gate's pending proposals;
 * the badge that rides this entry is the frame's own exception chrome, keyed
 * on this entry's id).
 */

import ClipboardCheckIcon from '@lucide/svelte/icons/clipboard-check';
import type { ExtensionManifest } from './types.js';

export const coreReview: ExtensionManifest = {
	id: 'coreReview',
	version: '0.1.1',
	contributes: {
		navEntries: [
			{ id: 'review', route: '/review', labelKey: 'nav.review', Icon: ClipboardCheckIcon, order: 3 }
		]
	}
};
