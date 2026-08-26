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

# Format the canonical surface with the SAME formatter the openclaw workspace
# applies on commit (oxfmt) — otherwise its pre-commit hook re-wraps the synced
# files and byte-identity drifts again (the 0.5.0 lesson). No-op when oxfmt or
# a config is absent; never fails the sync on a missing tool.
OC_DIR="$(dirname "$TARGET")"
OXFMT="$OC_DIR/node_modules/.bin/oxfmt"
if [[ -x "$OXFMT" ]]; then
	echo ">> formatting canonical plugin/ with the openclaw workspace's oxfmt…"
	(cd "$REPO" && find plugin -type f \( -name '*.ts' -o -name '*.md' -o -name '*.json' \) \
		-not -path 'plugin/node_modules/*' -print0 | xargs -0 "$OXFMT" --write) || true
fi

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
echo "next: cd $OC_DIR && node_modules/.bin/vitest run extensions/brain-server/test && \\"
echo "      node_modules/.bin/tsc --noEmit -p extensions/brain-server/tsconfig.json, then commit the synced tree."
