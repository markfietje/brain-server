#!/usr/bin/env bash
# Generate a CycloneDX SBOM for a brain-server release (EU CRA / OWASP A03:2025).
#
# Usage: scripts/sbom.sh
#   → writes sbom/brain-server-<version>.cdx.json
#
# Requires `cargo-cyclonedx` (a release-time tool, NOT a runtime dependency):
#   cargo install cargo-cyclonedx
# Fails loud with install instructions if it is absent.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1)"
OUT="$REPO/sbom/brain-server-${VERSION}.cdx.json"

if ! cargo cyclonedx --version >/dev/null 2>&1; then
  cat >&2 <<EOF
ERR: 'cargo cyclonedx' is not installed (it is a release-time tool, not a
     runtime dependency). Install it and re-run:
       cargo install cargo-cyclonedx
EOF
  exit 1
fi

# Generate JSON CycloneDX for this package. cargo-cyclonedx drops a
# brain-server.cdx.json in the repo root (output-pattern = package).
( cd "$REPO" && cargo cyclonedx -f json --override-outputFilename brain-server >/dev/null )

SRC="$REPO/brain-server.cdx.json"
if [[ ! -f "$SRC" ]]; then
  SRC="$(ls "$REPO"/*.cdx.json 2>/dev/null | head -1 || true)"
fi
[[ -f "$SRC" ]] || { echo "ERR: cargo cyclonedx produced no .cdx.json" >&2; exit 1; }

mkdir -p "$(dirname "$OUT")"
mv -f "$SRC" "$OUT"
echo "OK  wrote $OUT"
