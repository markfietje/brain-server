// The SPA posture (D4): no server-side rendering exists — the kernel is the
// only backend, reached over the typed wire. One static build serves the
// PWA and the Tauri webview.
export const ssr = false;
export const prerender = false;
