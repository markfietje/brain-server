#!/usr/bin/env bash
# Sign the release binaries with minisign (detached .minisig per binary).
#
# Usage: BRAIN_MINISIGN_KEY=~/.minisign/brain.key scripts/release-sign.sh
#
# Companion to install-service.sh's verification step: an operator who sets
# BRAIN_RELEASE_PUBKEY refuses any artifact without a valid signature, so the
# update path is pinned to the holder of the signing key (P2-10 posture).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEY="${BRAIN_MINISIGN_KEY:-$HOME/.minisign/brain-server.key}"

command -v minisign >/dev/null 2>&1 || { echo "minisign not found" >&2; exit 1; }
[[ -f "$KEY" ]] || { echo "signing key not found: $KEY" >&2; exit 1; }

BINS=(brain-server brain mcp bench brain-connector-stub)
for bin in "${BINS[@]}"; do
	src="$REPO/target/release/$bin"
	[[ -x "$src" ]] || { echo "missing $src (build first)" >&2; exit 1; }
	minisign -Sm "$src" -s "$KEY"
	echo "signed: $src.minisig"
done
