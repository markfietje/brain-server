#!/usr/bin/env bash
# v1.20.10 "Proof": assemble the Cyber Resilience Act (CRA) evidentiary bundle.
#
# Usage: scripts/cra-kit.sh
#   → writes dist/cra-kit/{SBOM, SECURITY.md, SUPPORT.md, docs, CRA_MANIFEST.json}
#
# The CRA evidentiary bar (2026 in force): an SBOM, a documented vulnerability
# reporting path, and a security-support statement. brain-server already ships
# the SBOM (scripts/sbom.sh → CycloneDX JSON); this kit assembles it with the
# repo's reporting/support docs into one hashed bundle an auditor can pull
# without spelunking the tree.
#
# Idempotent: re-running rebuilds the bundle from the same sources, so the
# manifest hashes are stable for unchanged content. No new deps (shasum only).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1)"
OUT="$REPO/dist/cra-kit"

rm -rf "$OUT"
mkdir -p "$OUT"

# SHA-256 helper, portable across macOS (shasum) and Linux (sha256sum).
HASH=()
if command -v sha256sum >/dev/null 2>&1; then
  HASH=(sha256sum)
else
  HASH=(shasum -a 256)
fi

# 1. CycloneDX SBOM. Generate if the versioned SBOM is absent (release-time
#    tool; fails loud with install instructions when cargo-cyclonedx is missing).
SBOM_SRC="$REPO/sbom/brain-server-${VERSION}.cdx.json"
if [[ ! -f "$SBOM_SRC" ]]; then
  "$REPO/scripts/sbom.sh"
fi
[[ -f "$SBOM_SRC" ]] || { echo "ERR: no SBOM at $SBOM_SRC" >&2; exit 1; }

# 2. Static repo artifacts (the reporting + support docs the CRA wants).
cp "$SBOM_SRC" "$OUT/brain-server-${VERSION}.cdx.json"
cp "$REPO/SECURITY.md" "$OUT/SECURITY.md"
cp "$REPO/SUPPORT.md" "$OUT/SUPPORT.md"
cp "$REPO/docs/deployment.md" "$OUT/deployment.md"
cp "$REPO/COMPLIANCE.md" "$OUT/COMPLIANCE.md"

# 3. Manifest listing each artifact + its SHA-256 (the evidentiary index).
MANIFEST="$OUT/CRA_MANIFEST.json"
{
  printf '{\n  "kit": "cra",\n  "version": "%s",\n  "artifacts": {\n' "$VERSION"
  first=1
  for f in brain-server-${VERSION}.cdx.json SECURITY.md SUPPORT.md deployment.md COMPLIANCE.md; do
    h=$("${HASH[@]}" "$OUT/$f" | awk '{print $1}')
    [[ $first -eq 0 ]] && printf ',\n'
    printf '    "%s": "sha256:%s"' "$f" "$h"
    first=0
  done
  printf '\n  }\n}\n'
} > "$MANIFEST"

echo "OK  wrote $OUT/CRA_MANIFEST.json"
