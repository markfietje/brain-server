// Boot-manifest loader vectors (node:test + node:crypto, Node >= 20).
// Ports the contract of the deleted client/src/plugins/boot.rs pure module:
// the manifest the server publishes must parse, must refuse traversal /
// non-hex digests / oversized shapes, and — since the Anchor release — must
// carry a valid Ed25519 signature over the canonical bundle list that the
// embedded /app/boot.js loader verifies with WebCrypto.
import test from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";

function goodManifest() {
  return {
    boot: "brain",
    bundles: [{ path: "pkg/app.js", bytes: 10, sha256: "a".repeat(64) }],
  };
}

// The canonical message both sides sign/verify: "path:bytes:sha256" lines.
function canonical(bundles) {
  return bundles.map((b) => `${b.path}:${b.bytes}:${b.sha256}`).join("\n");
}

function validate(v) {
  if (v.boot !== "brain") return "not a brain boot manifest";
  if (!Array.isArray(v.bundles)) return "missing bundles array";
  if (v.bundles.length > 256) return "bundle count unbounded";
  for (const b of v.bundles) {
    if (typeof b.path !== "string" || !b.path.startsWith("pkg/") ||
        b.path.slice(4).includes("/") || b.path.includes("..")) {
      return `bundle path escapes pkg/: ${b.path}`;
    }
    if (typeof b.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(b.sha256)) {
      return `non-hex digest: ${b.sha256}`;
    }
  }
  return null;
}

test("good manifest validates", () => {
  assert.equal(validate(goodManifest()), null);
});

test("wrong boot label refused", () => {
  assert.ok(validate({ boot: "other", bundles: [] }));
  assert.ok(validate({}));
});

test("traversal path refused", () => {
  const m = goodManifest();
  m.bundles[0].path = "pkg/../secrets";
  assert.ok(validate(m));
});

test("non-hex digest refused", () => {
  const m = goodManifest();
  m.bundles[0].sha256 = "z".repeat(64);
  assert.ok(validate(m));
});

test(">256 bundles refused", () => {
  const m = goodManifest();
  m.bundles = Array.from({ length: 257 }, (_, i) => ({
    path: `pkg/b${i}.js`,
    bytes: 1,
    sha256: "0".repeat(64),
  }));
  assert.ok(validate(m));
});

test("ed25519 signature over canonical bundle list verifies (and tamper fails)", () => {
  const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
  const m = goodManifest();
  const msg = Buffer.from(canonical(m.bundles), "utf8");
  const sig = crypto.sign(null, msg, privateKey);
  assert.equal(
    crypto.verify(null, msg, publicKey, sig),
    true,
    "signature over canonical list verifies"
  );
  // Tampered bundle list fails verification.
  const tampered = goodManifest();
  tampered.bundles[0].bytes = 11;
  assert.equal(
    crypto.verify(
      null,
      Buffer.from(canonical(tampered.bundles), "utf8"),
      publicKey,
      sig
    ),
    false,
    "any bundle drift invalidates the signature"
  );
});
