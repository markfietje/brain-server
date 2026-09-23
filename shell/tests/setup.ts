import { cleanup } from '@testing-library/svelte';
import { afterEach } from 'vitest';
import { addMessages, init } from 'svelte-i18n';
import en from '../src/lib/i18n/en.json';

afterEach(() => {
	cleanup();
});

// jsdom does not implement scrollIntoView; the bits-ui command palette
// calls it when selection moves. The shim keeps the palette test honest
// about ITS subject (open + navigate), not jsdom's viewport.
if (!Element.prototype.scrollIntoView) {
	Element.prototype.scrollIntoView = () => {};
}

// The component tests render isolated from +layout.svelte, so the i18n
// runtime is initialized here (en; the parity test covers all five).
addMessages('en', en);
init({ fallbackLocale: 'en', initialLocale: 'en' });
