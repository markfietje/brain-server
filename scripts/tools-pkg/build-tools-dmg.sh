#!/usr/bin/env bash
# The Tools DMG builder — the stapled image that carries every installable
# binary, the selective pkg, the escape hatch, and the sums that cover all
# of them.
#
#   scripts/tools-pkg/build-tools-dmg.sh --version <semver> \
#       --staged-dir <dir with the SIGNED release assets> \
#       --pkg <the signed+stapled pkg> \
#       --out <output dmg path> [--arch arm64]
#
# THE LAWS (Tools doc §1 + §4, the execution prompt §2):
# - Same-build-pass: the binaries are the STAGED signed bytes — the builder
#   refuses a staged dir missing any release asset, and never rebuilds
#   beside the release.
# - The SHA256SUMS covers EVERY file in the image (an uncovered file = red):
#   the builder generates it and then re-checks it against the assembled
#   volume — the check is the builder's own gate, not a hope.
# - The provenance: brain-server and brain answer --version with the
#   version arg (the staged set's hash chain to the release is the rest of
#   the record — the README carries it).
# - The staple: the DMG is signed with the Developer ID APPLICATION
#   identity, notarized, stapled — the offline verdict for everything
#   inside, forever.
# - D6: the operator lane. No key ever rides CI.

set -euo pipefail

VERSION=""
STAGED_DIR=""
PKG=""
OUT=""
ARCH="arm64"
while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="$2"; shift 2 ;;
        --staged-dir) STAGED_DIR="$2"; shift 2 ;;
        --pkg) PKG="$2"; shift 2 ;;
        --out) OUT="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done
[ -n "$VERSION" ] && [ -n "$STAGED_DIR" ] && [ -n "$PKG" ] && [ -n "$OUT" ] || {
    echo "usage: build-tools-dmg.sh --version <semver> --staged-dir <dir> --pkg <pkg> --out <dmg> [--arch arm64]" >&2
    exit 2
}

REPO_ROOT=$(cd "$(dirname "$0")/../.." && pwd)
PKG_DIR="$REPO_ROOT/scripts/tools-pkg"
APP_IDENTITY="Developer ID Application: Mark Fietje (JHB4UNPL3G)"
VOL="StewardMem Tools"
WORK=$(mktemp -d /tmp/stewardmem-tools-dmg.XXXXXX)
trap 'rm -rf "$WORK"' EXIT

# ── 1 · the volume ──────────────────────────────────────────────────────────
VOL_DIR="$WORK/volume"
mkdir -p "$VOL_DIR/binaries"

echo "== staging the SIGNED release binaries (arch $ARCH) =="
for tool in brain-server brain mcp bench brain-connector-stub brain-connector-gh brain-connector-crm; do
    [ -f "$STAGED_DIR/$tool-darwin-$ARCH" ] || { echo "the staged dir is missing $tool-darwin-$ARCH — refusing to rebuild beside the release" >&2; exit 1; }
    cp "$STAGED_DIR/$tool-darwin-$ARCH" "$VOL_DIR/binaries/$tool"
done
chmod 755 "$VOL_DIR"/binaries/*

# The provenance gate: the binaries that answer --version must name the
# release this build assembles from.
echo "== the provenance gate =="
for tool in brain-server brain; do
    observed=$("$VOL_DIR/binaries/$tool" --version 2>/dev/null | awk '{print $NF}')
    [ "$observed" = "$VERSION" ] || { echo "$tool answers version '$observed', expected '$VERSION' — the staged set is not this release" >&2; exit 1; }
    echo "   $tool --version → $observed"
done

echo "== the pkg + the escape hatch =="
[ -f "$PKG" ] || { echo "the pkg is missing: $PKG" >&2; exit 1; }
xcrun stapler validate "$PKG"   # the DMG carries a stapled pkg or it carries nothing
cp "$PKG" "$VOL_DIR/Install StewardMem Tools.pkg"
cp "$PKG_DIR/uninstall-stewardmem-tools.sh" "$VOL_DIR/"
chmod 755 "$VOL_DIR/uninstall-stewardmem-tools.sh"
cp "$PKG_DIR/README.md" "$VOL_DIR/README.md"

# ── 2 · the sums cover EVERY file (generated, then re-checked) ─────────────
echo "== SHA256SUMS over the whole volume =="
( cd "$VOL_DIR" && find . -type f ! -name SHA256SUMS -exec shasum -a 256 {} \; | sed 's|\./||' > SHA256SUMS )
COUNT=$(wc -l < "$VOL_DIR/SHA256SUMS" | tr -d ' ')
echo "   covering $COUNT files"
( cd "$VOL_DIR" && shasum -a 256 --check SHA256SUMS > /dev/null ) \
    || { echo "the sums do not check against the assembled volume — refusing" >&2; exit 1; }
# An uncovered file = red: re-walk the volume and confirm every file is IN the sums.
while IFS= read -r file; do
    grep -q "  $file\$" "$VOL_DIR/SHA256SUMS" || { echo "UNCOVERED FILE: $file — the sums must cover everything" >&2; exit 1; }
done < <( cd "$VOL_DIR" && find . -type f ! -name SHA256SUMS | sed 's|^\./||' )

# ── 3 · the image: sign → notarize → staple ────────────────────────────────
echo "== hdiutil =="
rm -f "$OUT"
hdiutil create -volname "$VOL" -srcfolder "$VOL_DIR" -format UDZO -o "$OUT" -quiet

echo "== codesign (the DMG signs with the Application identity) =="
codesign --force --timestamp --sign "$APP_IDENTITY" "$OUT"
codesign --verify --strict "$OUT"

echo "== notarytool =="
xcrun notarytool submit "$OUT" --keychain-profile stewardmem --wait

echo "== stapler =="
xcrun stapler staple "$OUT"
xcrun stapler validate "$OUT"

echo "== the receipt =="
echo "the staple check (offline): $(xcrun stapler validate "$OUT" 2>&1 | tail -1)"
echo "staged, signed, notarized, stapled, sums covering $COUNT files: $OUT"
