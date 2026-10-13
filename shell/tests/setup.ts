import { cleanup } from '@testing-library/svelte';
import { afterEach } from 'vitest';

afterEach(() => {
	cleanup();
});

// jsdom implements neither of the two browser APIs the ⌘K palette touches.
// Both shims keep the palette test honest about ITS subject (open, filter,
// navigate), not about jsdom's missing viewport. Without them the suite
// passes but Vitest reports an unhandled rejection and fails the run —
// which is exactly what happened when the palette moved to Zag.

// scrollIntoView: called when the machine moves the active option.
if (!Element.prototype.scrollIntoView) {
	Element.prototype.scrollIntoView = () => {};
}

// ResizeObserver: @zag-js/dialog and @zag-js/combobox both observe their
// content to position the layer. A no-op observer is the correct stand-in —
// the tests assert behaviour, never geometry.
if (!globalThis.ResizeObserver) {
	globalThis.ResizeObserver = class {
		observe() {}
		unobserve() {}
		disconnect() {}
	} as unknown as typeof ResizeObserver;
}

// No i18n runtime init is needed: Paraglide compiles every locale into
// `src/paraglide` ahead of time, so there is nothing to register at runtime.
// The catalogs' five-way key parity is still a red test (i18n-parity.test.ts).