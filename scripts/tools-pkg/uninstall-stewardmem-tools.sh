#!/bin/bash
# The no-app escape hatch — the residue law REVERSED, our artifacts only.
#
# What it removes (and nothing else):
#   - symlinks in ~/.local/bin that point INTO the StewardMem tools anchor
#     (ours by construction — a link into our anchor is a link we created);
#   - the anchor itself: ~/Library/Application Support/StewardMem/tools.
#
# What it NEVER touches:
#   - the operator's service: the D1 check runs FIRST, and a referenced
#     path that is ours is REFUSED removal by name — an uninstall that
#     breaks a running memory service is not an escape hatch, it is a
#     wrecking bar;
#   - real files in ~/.local/bin (the user's own binaries);
#   - anything outside the user's home. No sudo, no admin.
#
# Scriptable: bash uninstall-stewardmem-tools.sh [--dry-run]

set -u

DRY_RUN=0
[ "${1:-}" = "--dry-run" ] && DRY_RUN=1

ANCHOR_REL="Library/Application Support/StewardMem/tools"
ANCHOR="$HOME/$ANCHOR_REL"
BIN_DIR="$HOME/.local/bin"

if [ ! -d "$ANCHOR" ]; then
    echo "nothing to uninstall: $ANCHOR does not exist — the Tools pkg is not installed"
    exit 0
fi

# The D1 guard, through the detection core's own binary (the newest one in
# the anchor — the tools it describes are being removed, so this runs BEFORE
# any removal).
referenced=""
detector=$(ls -t "$ANCHOR"/*/stewardmem-detect 2>/dev/null | head -n 1)
if [ -n "$detector" ]; then
    referenced=$("$detector" --fields 2>/dev/null | sed -n 's/^existing.binaryPath=//p' | head -n 1)
fi

removed=""
kept_ours=""
for link in "$BIN_DIR"/*; do
    [ -L "$link" ] || continue
    target=$(readlink "$link")
    case "$target" in
        *"/$ANCHOR_REL/"*)
            if [ -n "$referenced" ] && [ "$link" = "$referenced" ]; then
                kept_ours="$kept_ours $link"
                continue
            fi
            if [ "$DRY_RUN" = "1" ]; then
                echo "would remove: $link -> $target"
            else
                rm "$link" && removed="$removed $link"
            fi
            ;;
    esac
done

# The fail-closed guard: if the running service still executes from OUR
# anchor, the anchor stays — removing it would leave the kept link dangling
# and the service broken. An escape hatch that wrecks the service it sits
# beside is not an escape hatch. Migrate or stop the service first.
if [ -n "$kept_ours" ]; then
    echo "REFUSED:$kept_ours — the running service still references this path."
    echo "REFUSED: the anchor was NOT removed. Stop the service or migrate it"
    echo "REFUSED: (see the README), then re-run this script."
    exit 0
fi

if [ "$DRY_RUN" = "1" ]; then
    echo "would remove the anchor: $ANCHOR"
    exit 0
fi

rm -rf "$ANCHOR"
# The empty parents of OUR anchor go too — rmdir removes an empty dir only,
# so a parent the user actually uses is untouched by construction.
rmdir "$HOME/Library/Application Support/StewardMem" 2>/dev/null || true

echo "REMOVED:$removed"
echo "REMOVED: $ANCHOR (all installed versions)"
echo "Your own files in $BIN_DIR were never touched."
