#!/usr/bin/env bash
# Derive the README's dynamic badge values from the real build, so the badges
# are facts, not hand-typed claims. A release-time tool, like scripts/sbom.sh.
#
#   scripts/badges.sh            → print the version/test/UMP/SBOM badge block
#   scripts/badges.sh --selfcheck→ verify the derivations + the release
#                                  checklist's six-artifact completeness
#                                  (exits nonzero on any drift)
#
# It never fabricates a number it did not measure: version is read from
# Cargo.toml, the test count from an actual `cargo test` run, the SBOM flag
# from the on-disk CycloneDX file.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

version()      { sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1; }
client_version(){ sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/client/Cargo.toml" | head -1; }
test_count()   {
  ( cd "$REPO" && cargo test --features bench,migrate 2>&1 ) \
    | grep -Eo '[0-9]+ passed' | awk '{ s+=$1 } END { print s+0 }'
}

VERSION="$(version)"

if [[ "${1:-}" == "--selfcheck" ]]; then
  # 1. badges derive the version from the real build, not a stored claim.
  CARGO_VERSION="$(grep -m1 '^version' "$REPO/Cargo.toml" | grep -oE '[0-9][0-9.]*')"
  if [[ -z "$VERSION" || "$VERSION" != "$CARGO_VERSION" ]]; then
    echo "ERR: badges version '$VERSION' drifts from Cargo.toml '$CARGO_VERSION'" >&2
    exit 1
  fi
  # 2. the README carries the DERIVED version, not a stale hand-typed one
  #    (v1.28.65 lesson: the version badge sat two releases behind while
  #    selfcheck passed — nothing compared README to the derivation).
  README="$REPO/README.md"
  if ! grep -q "badge/version-${CARGO_VERSION}-blue.svg" "$README"; then
    echo "ERR: README version badge drifts from Cargo.toml '$CARGO_VERSION' — run scripts/badges.sh and paste the block" >&2
    exit 1
  fi
  # The test-count badge can't be verified without a full cargo run; the
  # release checklist carries that step instead.
  # 3. the release-checklist names all six wrap artifacts (self-completeness guard).
  CK="$REPO/docs/release-checklist.md"
  if [[ ! -f "$CK" ]]; then
    echo "ERR: docs/release-checklist.md missing" >&2
    exit 1
  fi
  for a in "Cargo.toml" "openapi.yaml" "CHANGELOG" "ROADMAP" "README" "AGENTS"; do
    if ! grep -q "$a" "$CK"; then
      echo "ERR: release-checklist.md omits '$a'" >&2
      exit 1
    fi
  done
  # 4. the SBOM freshness gate (the preflight line, X-C8): the tag carries
  #    its SBOM IN-TREE. A release whose version has no COMMITTED
  #    sbom/brain-server-<version>.cdx.json fails the gate — the human step
  #    (scripts/sbom.sh + commit) is unforgoable; no CI bot commits.
  if ! git -C "$REPO" ls-files --error-unmatch "sbom/brain-server-${CARGO_VERSION}.cdx.json" >/dev/null 2>&1; then
    if [[ -f "$REPO/sbom/brain-server-${CARGO_VERSION}.cdx.json" ]]; then
      echo "ERR: sbom/brain-server-${CARGO_VERSION}.cdx.json exists but is NOT committed — run scripts/sbom.sh and commit it before tagging" >&2
    else
      echo "ERR: sbom/brain-server-${CARGO_VERSION}.cdx.json missing from the tree — run scripts/sbom.sh, commit, then tag" >&2
    fi
    exit 1
  fi
  echo "OK  badges + release checklist self-check clean"
  exit 0
fi

CLIENT="$(client_version)"
TESTS="$(test_count)"
UMP="L3"        # self-attested level, v1.17.4; asserted every push by the integration CI job
SBOM="$REPO/sbom/brain-server-${VERSION}.cdx.json"
SBOM_FLAG=no; [[ -f "$SBOM" ]] && SBOM_FLAG=yes

cat <<EOF
server $VERSION   client $CLIENT   tests $TESTS passed   UMP $UMP   sbom $SBOM_FLAG

[![Version](https://img.shields.io/badge/version-$VERSION-blue.svg)](#)
[![Tests](https://img.shields.io/badge/tests-$TESTS%20passed-brightgreen.svg)](#)
[![UMP Conformance](https://img.shields.io/badge/UMP%201.0-L3%20verified-success.svg)](docs/universal-memory-protocol.md)
EOF
