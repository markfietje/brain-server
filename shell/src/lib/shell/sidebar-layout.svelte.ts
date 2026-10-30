/**
 * The sidebar's layout store — the dsh AppFrame pattern (measured from
 * `~/Sites/deepseek-harness/packages/client/ui-layout`, 2026-10-30), adapted
 * to Svelte 5 runes:
 *
 * - CSS grid solves the squeeze NATIVELY: the JS store carries only the
 *   discrete decisions (the width preference, collapsed, the drag base) and
 *   the frame renders `grid-template-columns` from them. A window resize
 *   lands in the same layout pass as the frame edge — no JS-priced squeeze,
 *   no jitter (dsh's own note: "when it priced the squeeze itself, the
 *   centre column absorbed each width change whole and was corrected two
 *   frames later — visible jitter").
 * - The drag base is the width captured at drag start, frozen for the whole
 *   gesture: grabbing a clamped panel must not jump back, and dx deltas must
 *   not compound.
 * - The width preference PERSISTS (localStorage, `dynamically adjustable…
 *   changing settings in UI settings` — the operator's ask): a fresh session
 *   restores the user's width.
 * - Collapse toggles keep the preference untouched (collapse ≠ resize), so
 *   re-expanding restores the pre-collapse width — dsh's exact law.
 */

const KEY = 'stewardmem.sidebar.width';
export const SIDEBAR_MIN = 200;
export const SIDEBAR_MAX = 480;
export const SIDEBAR_DEFAULT = 260;

function clamp(px: number): number {
	return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, Math.round(px)));
}

function restore(): number {
	if (typeof window === 'undefined') return SIDEBAR_DEFAULT;
	try {
		const raw = window.localStorage.getItem(KEY);
		if (raw === null) return SIDEBAR_DEFAULT;
		const parsed = Number(raw);
		return Number.isFinite(parsed) ? clamp(parsed) : SIDEBAR_DEFAULT;
	} catch {
		return SIDEBAR_DEFAULT;
	}
}

class SidebarLayout {
	width = $state(restore());
	collapsed = $state(false);
	/** Set while a drag gesture is live: the eased track transitions pause. */
	dragging = $state(false);
	#base = 0;

	startDrag(): void {
		this.#base = this.width;
		this.dragging = true;
	}

	/** dx from the drag start (the divider moves left → the column widens). */
	dragBy(dx: number): void {
		this.width = clamp(this.#base - dx);
	}

	endDrag(): void {
		this.dragging = false;
		try {
			window.localStorage.setItem(KEY, String(this.width));
		} catch {
			// A storage-refusing context (private mode) keeps the width for
			// the session only — the drag still worked.
		}
	}

	toggle(): void {
		// Collapse flips the track only: the width preference survives.
		this.collapsed = !this.collapsed;
	}
}

export const sidebarLayout = new SidebarLayout();
