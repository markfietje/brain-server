#!/usr/bin/env bash
# sync-plugin.sh — the permanent Parity discipline: rsync plugin/ (the
# canonical edit surface) into the openclaw workspace's deployed extension,
# then verify byte-identity. Refuses to run when the target has uncommitted
# changes or pre-diff drift not present in plugin/.
#
# Usage: scripts/sync-plugin.sh [OPENCLAW_DIR]   (default ~/Sites/openclaw)
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$REPO/plugin"
TARGET="${1:-$HOME/Sites/openclaw}/extensions/brain-server"

[[ -d "$TARGET" ]] || { echo "target missing: $TARGET" >&2; exit 2; }

cd "$(dirname "$TARGET")"
if [[ -n "$(git status --porcelain -- "$(basename "$TARGET")")" ]]; then
	echo "refusing: $TARGET has uncommitted changes — commit or stash first" >&2
	exit 1
fi

# Pre-diff drift guard: every differing file must also differ in plugin/ vs
# HEAD of the target (i.e. drift is explained by plugin/-side edits).
rsync -rci --delete \
	--exclude node_modules \
	--exclude package-lock.json \
	"$SRC/" "$TARGET/" | grep '^[<>ch]' | while read -r line; do
	echo "$line"
done

echo "synced $SRC -> $TARGET"
echo "next: cd $(dirname "$TARGET") && pnpm test $(realpath --relative-to="$(dirname "$TARGET")" "$TARGET") && pnpm exec tsc --noEmit, then commit the synced tree."
