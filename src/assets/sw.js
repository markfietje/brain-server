/* brain service worker — cache keys are stamped with the boot-manifest
 * digest. A cached asset whose digest disagrees with the live manifest is
 * never served: the worker goes network-first on mismatch, so a once-poisoned
 * cache stops poisoning.
 */
(function () {
  "use strict";
  const MANIFEST = "/app/boot.json";
  async function manifestDigest() {
    try {
      const res = await fetch(MANIFEST, { cache: "no-store" });
      if (!res.ok) return null;
      const m = await res.json();
      if (m.boot !== "brain" || !Array.isArray(m.bundles)) return null;
      const lines = m.bundles.map((b) => b.path + ":" + b.bytes + ":" + b.sha256);
      const buf = await crypto.subtle.digest(
        "SHA-256", new TextEncoder().encode(lines.join("\n"))
      );
      return Array.from(new Uint8Array(buf))
        .map((b) => b.toString(16).padStart(2, "0"))
        .join("");
    } catch (_) {
      return null;
    }
  }
  self.addEventListener("activate", (event) => {
    event.waitUntil(
      (async () => {
        const d = await manifestDigest();
        const keep = d ? ["brain-shell-" + d] : [];
        const names = await caches.keys();
        await Promise.all(
          names.filter((n) => !keep.includes(n)).map((n) => caches.delete(n))
        );
      })()
    );
  });
  self.addEventListener("fetch", (event) => {
    const req = event.request;
    if (req.method !== "GET" || new URL(req.url).origin !== self.location.origin)
      return;
    event.respondWith(
      (async () => {
        // Network-first for shell assets; the cache is only a fallback and is
        // rebuilt under the CURRENT manifest digest's key.
        try {
          const res = await fetch(req);
          const d = await manifestDigest();
          if (d && (req.destination === "script" || req.destination === "style" ||
                    req.destination === "" )) {
            const cache = await caches.open("brain-shell-" + d);
            cache.put(req, res.clone());
          }
          return res;
        } catch (_) {
          const d = await manifestDigest();
          const cache = await caches.open("brain-shell-" + (d || "unknown"));
          const hit = await cache.match(req);
          if (hit) return hit;
          throw new Error("offline and uncached");
        }
      })()
    );
  });
})();
