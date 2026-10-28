/**
 * A reactive `$app/state` page mock. The real `page` is a `$state` object
 * the router mutates on navigation, and a component's `$derived` follows it
 * only because of that reactivity — a plain object mocks the mount-time
 * read, not the write-then-re-read a filter affordance promises. Mutate
 * `pageState.url` and deriveds in the component under test recompute, same
 * as the router's own navigation.
 */
import { SvelteURL } from 'svelte/reactivity';

// The declared type stays `URL` — SvelteURL IS a URL, and the tests assign
// plain URLs through the same $state proxy on navigation.
export const pageState: { url: URL } = $state({
	url: new SvelteURL('http://127.0.0.1/audit')
});
