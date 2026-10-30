/**
 * The distribution channel this build serves (PLAN_MAS_FORK §3): the
 * renderer asks the core at mount and gates the direct channel's
 * affordances on the answer. The store edition (`mas`) renders no updater
 * surface — the store owns updates there — and never learns an update
 * endpoint.
 *
 * The PWA posture (no Tauri runtime) is its own answer: `pwa`. No cache:
 * the answer is per-mount by design, so a pin that re-renders under a
 * different channel re-reads the truth instead of a first-mount memo.
 */

export type BuildChannel = 'direct' | 'mas' | 'pwa';

export async function buildChannel(): Promise<BuildChannel> {
	if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
		return 'pwa';
	}
	try {
		const { invoke } = await import('@tauri-apps/api/core');
		const answer = await invoke<string>('build_channel');
		return answer === 'mas' ? 'mas' : 'direct';
	} catch {
		// The core did not answer: the honest default is the direct channel
		// (the pre-fork behavior), never a silent third posture.
		return 'direct';
	}
}
