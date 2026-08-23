/* brain boot loader — the fail-closed fetch-and-refuse driver.
 * Fetches /app/boot.json, verifies the Ed25519 signature over the canonical
 * bundle list against /app/boot.pub (WebCrypto, all current browsers), then
 * fetches every listed bundle and compares SHA-256. ANY failure refuses the
 * page: no fallback, no partial load. Zero dependencies.
 */
(function () {
  "use strict";
  function refuse(why) {
    try { window.stop(); } catch (_) {}
    document.documentElement.textContent = "brain client refused to load: " + why;
    console.error("[brain-boot] refuse:", why);
  }
  function hex(buf) {
    return Array.from(new Uint8Array(buf)).map(function (b) {
      return b.toString(16).padStart(2, "0");
    }).join("");
  }
  // Canonical message the server signed: one "path:bytes:sha256" line per
  // bundle, joined with \n — trivially reproducible on both sides.
  function canonical(bundles) {
    return bundles.map(function (b) {
      return b.path + ":" + b.bytes + ":" + b.sha256;
    }).join("\n");
  }
  async function main() {
    var res = await fetch("/app/boot.json", { cache: "no-store" });
    if (!res.ok) return refuse("boot manifest unavailable");
    var m = await res.json();
    if (m.boot !== "brain" || !Array.isArray(m.bundles)) return refuse("bad manifest");
    if (!m.sig || !m.kid) return refuse("unsigned manifest");
    var pubRes = await fetch("/app/boot.pub", { cache: "no-store" });
    if (!pubRes.ok) return refuse("signing key unavailable");
    var pubRaw = new Uint8Array(await pubRes.arrayBuffer());
    var key = await crypto.subtle.importKey(
      "raw", pubRaw, { name: "Ed25519" }, false, ["verify"]
    );
    var sigBytes = new Uint8Array(
      m.sig.match(/.{2}/g).map(function (h) { return parseInt(h, 16); })
    );
    var okSig = await crypto.subtle.verify(
      "Ed25519", key, sigBytes, new TextEncoder().encode(canonical(m.bundles))
    );
    if (!okSig) return refuse("manifest signature invalid");
    for (var i = 0; i < m.bundles.length; i++) {
      var b = m.bundles[i];
      if (!/^pkg\/[\w.-]+$/.test(b.path)) return refuse("bundle path escapes pkg/: " + b.path);
      var bRes = await fetch("/app/" + b.path, { cache: "no-store" });
      if (!res.ok) return refuse("bundle unavailable: " + b.path);
      var bytes = await bRes.arrayBuffer();
      if (bytes.byteLength !== b.bytes) return refuse("bundle size mismatch: " + b.path);
      var digest = hex(await crypto.subtle.digest("SHA-256", bytes));
      if (digest !== b.sha256) return refuse("bundle digest mismatch: " + b.path);
    }
    window.__BRAIN_BOOT_VERIFIED__ = true;
  }
  main().catch(function (e) { refuse(String(e)); });
})();
