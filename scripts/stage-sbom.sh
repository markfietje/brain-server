#!/usr/bin/env bash
# Stage exactly the release-matched SBOM into dist/ (fail-closed).
#
# Usage: scripts/stage-sbom.sh <version> [tree-root] [dist-dir]
#   → copies sbom/brain-server-<version>.cdx.json to dist/, nothing else.
#
# Refuses (nonzero exit, nothing staged) when the exact file is absent or its
# CycloneDX metadata.component.version disagrees with the requested version,
# so a release can never silently publish historical SBOMs in its place.
set -euo pipefail

VERSION="${1:?usage: stage-sbom.sh <version> [tree-root] [dist-dir]}"
REPO="${2:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
DIST="${3:-$REPO/dist}"

SBOM="$REPO/sbom/brain-server-${VERSION}.cdx.json"
[[ -f "$SBOM" ]] || { echo "ERR: required SBOM missing: $SBOM" >&2; exit 1; }

ACTUAL="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["metadata"]["component"]["version"])' "$SBOM")"
[[ "$ACTUAL" == "$VERSION" ]] || {
  echo "ERR: SBOM version mismatch: $SBOM reports '$ACTUAL', want '$VERSION'" >&2
  exit 1
}

mkdir -p "$DIST"
cp -f "$SBOM" "$DIST/"
echo "OK  staged $(basename "$SBOM")"
