/**
 * The sidebar's right-click context menu: SHOW/HIDE the proof surfaces.
 *
 * Desktop (Tauri): a NATIVE macOS context menu — `Menu.popup()` with
 * `CheckMenuItem` per surface, the checkmark riding the visibility store,
 * and each item's `action` toggling it (verified against the current
 * Tauri v2 docs via Context7, 2026-10-30: the action callback fires
 * client-side, no Rust round-trip). This is exactly the operator's ask:
 * the proof surfaces are not in the first GUI; right-click makes them
 * visible; right-click hides them again.
 *
 * Browser: no native menus — a minimal custom DOM menu drives the same
 * store (same items, same checkmarks, positioned at the cursor,
 * dismissed on click-away/Escape).
 */
import { sidebarVisibility, type SurfaceVisibility } from '#lib/shell/sidebar-visibility.svelte.js';

const SURFACES: ReadonlyArray<{ key: keyof SurfaceVisibility; labelKey: string }> = [
	{ key: 'memory', labelKey: 'nav.memory' },
	{ key: 'review', labelKey: 'nav.review' },
	{ key: 'audit', labelKey: 'nav.audit' },
	{ key: 'governance', labelKey: 'nav.governance' }
];

/** Whether this runtime carries the native menu API. */
export function nativeMenusAvailable(): boolean {
	return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** The native popup (Tauri only). Resolves when the menu is dismissed. */
export async function popupNativeContextMenu(x: number, y: number): Promise<void> {
	const [{ Menu, CheckMenuItem }, { PhysicalPosition }] = await Promise.all([
		import('@tauri-apps/api/menu'),
		import('@tauri-apps/api/dpi')
	]);
	const items = await Promise.all(
		SURFACES.map(async ({ key, labelKey }) => {
			const { m } = await import('$paraglide/messages.js');
			return CheckMenuItem.new({
				id: key,
				text: m[labelKey as 'nav.memory'](),
				checked: sidebarVisibility[key],
				action: () => sidebarVisibility.toggle(key)
			});
		})
	);
	const menu = await Menu.new({ items });
	await menu.popup(new PhysicalPosition(x, y));
}

export { SURFACES };
