import { cleanup } from '@testing-library/svelte';
import { afterEach } from 'vitest';
import { addMessages, init } from 'svelte-i18n';
import en from '../src/lib/i18n/en.json';

afterEach(() => {
	cleanup();
});

// The component tests render isolated from +layout.svelte, so the i18n
// runtime is initialized here (en; the parity test covers all five).
addMessages('en', en);
init({ fallbackLocale: 'en', initialLocale: 'en' });
