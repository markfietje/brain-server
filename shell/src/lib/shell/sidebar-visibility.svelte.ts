/**
 * The proof surfaces' sidebar visibility — the ChatGPT-minimal default
 * (the operator's call, 2026-10-30): the first GUI shows the CONVERSATION
 * only; Memory / Review / Audit / Governance are the PROOF surfaces
 * (proof of memory, of retrieval, of deletion — the product's legal spine)
 * and are revealed on demand: right-click the sidebar → the NATIVE macOS
 * context menu (Tauri CheckMenuItems) toggles each one, and the choice
 * persists (localStorage). They can be hidden again the same way.
 *
 * The browser build (no Tauri) renders the same store through a minimal
 * custom context menu — one store, two menus, one truth.
 */

const KEY = 'stewardmem.sidebar.surfaces';

export interface SurfaceVisibility {
	readonly memory: boolean;
	readonly review: boolean;
	readonly audit: boolean;
	readonly governance: boolean;
}

/** The first-run posture: the conversation, nothing else. */
const DEFAULTS: SurfaceVisibility = { memory: false, review: false, audit: false, governance: false };

function restore(): SurfaceVisibility {
	if (typeof window === 'undefined') return { ...DEFAULTS };
	try {
		const raw = window.localStorage.getItem(KEY);
		if (raw === null) return { ...DEFAULTS };
		const parsed = JSON.parse(raw) as Partial<SurfaceVisibility>;
		return {
			memory: parsed.memory === true,
			review: parsed.review === true,
			audit: parsed.audit === true,
			governance: parsed.governance === true
		};
	} catch {
		return { ...DEFAULTS };
	}
}

class SidebarVisibility {
	memory = $state(DEFAULTS.memory);
	review = $state(DEFAULTS.review);
	audit = $state(DEFAULTS.audit);
	governance = $state(DEFAULTS.governance);

	constructor() {
		const restored = restore();
		this.memory = restored.memory;
		this.review = restored.review;
		this.audit = restored.audit;
		this.governance = restored.governance;
	}

	toggle(surface: keyof SurfaceVisibility): void {
		this[surface] = !this[surface];
		try {
			window.localStorage.setItem(
				KEY,
				JSON.stringify({
					memory: this.memory,
					review: this.review,
					audit: this.audit,
					governance: this.governance
				})
			);
		} catch {
			// A storage-refusing context keeps the choice for the session only.
		}
	}
}

export const sidebarVisibility = new SidebarVisibility();
