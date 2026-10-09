#!/usr/bin/env bash
# Verify the merged release dist carries exactly its tag-matched SBOM.
#
# Usage: scripts/check-dist-sbom.sh <version> [dist-dir]
#   → asserts dist/brain-server-<version>.cdx.json exists, reports the
#     requested version inside its CycloneDX metadata, and that no other
#     SBOM sits beside it. Fail-closed on all three (nonzero exit,
#     nothing published by the caller).
#
# Kept as a file (not inline workflow Python) because run-block quoting
# layers do not survive the transport intact; a script file executes
# byte-identical everywhere.
set -euo pipefail

VERSION="${1:?usage: check-dist-sbom.sh <version> [dist-dir]}"
DIST="${2:-dist}"

SBOM="$DIST/brain-server-${VERSION}.cdx.json"
[[ -f "$SBOM" ]] || { echo "ERR: required SBOM missing: $SBOM" >&2; exit 1; }

ACTUAL="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["metadata"]["component"]["version"])' "$SBOM")"
[[ "$ACTUAL" == "$VERSION" ]] || {
  echo "ERR: SBOM version mismatch: $SBOM reports '$ACTUAL', want '$VERSION'" >&2
  exit 1
}

EXTRA="$(ls "$DIST"/*.cdx.json 2>/dev/null | grep -v -F "$SBOM" || true)"
[[ -z "$EXTRA" ]] || {
  echo "ERR: historical SBOMs in $DIST (only $SBOM ships): $EXTRA" >&2
  exit 1
}

echo "OK  dist carries exactly $(basename "$SBOM")"
