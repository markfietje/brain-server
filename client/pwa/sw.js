/*
 * brain-client service worker (v1.16.7 M2 "PWA").
 *
 * Deliberately narrow: it caches ONLY the static app shell (index.html + the
 * /app/assets/* wasm/js/css bundles). It NEVER caches the API (/recall,
 * /search, /audit, /dsar, ...) — memory content must not be persisted in the
 * browser cache (MASVS-STORAGE posture, same as the in-memory token rule).
 * Every non-GET and every non-shell request is left to the network untouched.
 */
const SHELL_CACHE = 'brain-shell-v1';
const SHELL_HTML = '/app/index.html';
const ASSET_PREFIX = '/app/assets/';

self.addEventListener('install', (event) => {
  self.skipWaiting();
  event.waitUntil(
    caches.open(SHELL_CACHE).then((c) => c.add(SHELL_HTML))
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys.filter((k) => k !== SHELL_CACHE).map((k) => caches.delete(k))
        )
      )
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;

  // Navigation: try the network, fall back to the cached shell for offline.
  if (req.mode === 'navigate') {
    event.respondWith(fetch(req).catch(() => caches.match(SHELL_HTML)));
    return;
  }

  // Shell assets only (wasm/js/css + the SPA html). Cache-first with a
  // background refresh so updates land on the next visit without blocking.
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  const isShellAsset = url.pathname === '/app/' || url.pathname.startsWith(ASSET_PREFIX);
  if (!isShellAsset) return; // API and anything else: untouched.

  event.respondWith(
    caches.match(req).then((hit) => {
      const refresh = fetch(req)
        .then((resp) => {
          if (resp && resp.ok) {
            const clone = resp.clone();
            caches.open(SHELL_CACHE).then((c) => c.put(req, clone));
          }
          return resp;
        })
        .catch(() => hit);
      return hit || refresh;
    })
  );
});
