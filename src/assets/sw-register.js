/* External service-worker registration (moved out of dist/index.html's inline
 * <script> — the served CSP has no 'unsafe-inline', so the inline registration
 * could never run; this external file registers deterministically).
 */
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("/app/sw.js").catch(function (e) {
    console.error("[brain-sw] registration failed:", e);
  });
}
