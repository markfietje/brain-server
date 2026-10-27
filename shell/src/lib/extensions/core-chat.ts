/**
 * coreChat — the conversation surface (the app's home) and its chrome.
 *
 * Contributes the sidebar's first entry, the chat empty state's three acting
 * chips (the M3-b three-chip law — the chips are the onboarding, each does
 * the thing it names), and the FIRST member of the status context chain:
 * the surface-set sentence from the frame's store ("3 memories", "4
 * citations"), which is what the status bar's middle slot has shown since
 * the frame existed. The member ignores the kernel facts on purpose — the
 * chain's signature is `facts → string | null` for the members that will
 * derive from them; this one bridges the store.
 */

import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import MessageCircleIcon from '@lucide/svelte/icons/message-circle';
import { currentStatusContext } from '#lib/shell/frame.svelte.js';
import type { ExtensionManifest } from './types.js';

export const coreChat: ExtensionManifest = {
	id: 'coreChat',
	version: '0.1.1',
	contributes: {
		navEntries: [
			{ id: 'chat', route: '/', labelKey: 'nav.chat', Icon: MessageCircleIcon, order: 1 }
		],
		emptyChips: [
			{
				id: 'ask',
				labelKey: 'chat.empty.chip.ask',
				run: () => {
					// The composer's id — the same identity its label binds to —
					// not a component-bound ref: the ref is the mount effect's
					// channel and is not populated under jsdom, and a chip whose
					// act only works where the ref binds would be unpinnable
					// where pins run (the M3-b record).
					document.getElementById('chat-composer')?.focus();
				}
			},
			{
				id: 'trace',
				labelKey: 'chat.empty.chip.trace',
				run: () => void goto(resolve('/recall'))
			},
			{
				id: 'review',
				labelKey: 'chat.empty.chip.review',
				run: () => void goto(resolve('/review'))
			}
		],
		statusContext: [
			{
				order: 1,
				derive: () => currentStatusContext()
			}
		]
	}
};
