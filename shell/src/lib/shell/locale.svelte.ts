/**
 * The session locale — ONE source for every reader (the M3-a law).
 *
 * The settings dialog's Locale section writes it and the frame's relative
 * session times read it; both now go through this module, so the picker and
 * the clock cannot disagree. The choice persists for the session only
 * (sessionStorage, like the sidebar's collapsed state), and a fresh launch
 * falls back to the browser's own preference, narrowed to a locale the
 * catalog set actually carries.
 *
 * The INITIAL value is computed at module load, deliberately NOT lazily on
 * first read: a read happens during render (the frame's relative times), and
 * Svelte 5 refuses a state write inside a render — the old frame code
 * initialised in a mount effect, which is the same "before first paint"
 * moment as a module body.
 */

import { assertIsLocale, setLocale } from '$paraglide/runtime.js';

/** The picker's rows — the five catalogs the parity pin holds shut. */
export const LOCALES = [
	{ code: 'en', name: 'English' },
	{ code: 'de', name: 'Deutsch' },
	{ code: 'fr', name: 'Français' },
	{ code: 'es', name: 'Español' },
	{ code: 'nl', name: 'Nederlands' }
] as const;

const LOCALE_KEY = 'shell.locale';

/** Session storage wins; otherwise the browser's own preference. Private
 * mode: nothing persisted, the fallbacks decide. */
function initialLocale(): string {
	let stored: string | null = null;
	try {
		stored = sessionStorage.getItem(LOCALE_KEY);
	} catch {
		// private mode: nothing persisted
	}
	const browser = typeof navigator === 'undefined' ? '' : navigator.language.slice(0, 2);
	const code = stored && LOCALES.some((l) => l.code === stored)
		? stored
		: LOCALES.some((l) => l.code === browser)
			? browser
			: 'en';
	return code;
}

const initial = initialLocale();
let currentLocale = $state(initial);

// Paraglide types the locale as a literal union; assert narrows the runtime
// string the fallbacks produced without widening the catalog.
setLocale(assertIsLocale(initial));

/** The current locale's code. */
export function currentLocaleCode(): string {
	return currentLocale;
}

/** The picker's act: set the runtime locale and persist the choice for this
 * session only. */
export function chooseLocale(code: string): void {
	currentLocale = code;
	// Paraglide types the locale as a literal union; assert narrows the
	// runtime string the picker produced without widening the catalog.
	setLocale(assertIsLocale(code));
	try {
		sessionStorage.setItem(LOCALE_KEY, code);
	} catch {
		// private mode: the choice stays live for this session only
	}
}
