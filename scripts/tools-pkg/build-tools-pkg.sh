#!/usr/bin/env bash
# The Tools pkg builder — the selective installer the Tools DMG carries.
#
#   scripts/tools-pkg/build-tools-pkg.sh --version <semver> \
#       --staged-dir <dir with the SIGNED release binaries> \
#       --out <output pkg path> [--no-sign]
#
# WHAT IT DOES
#   1. Builds the detector (stewardmem-detect) from THIS tree in release
#      mode and signs it with the Developer ID Application identity — the
#      pkg's machinery rides beside the release binaries it reads about.
#   2. Assembles three component payloads (core / bench / connectors) from
#      the STAGED dir — the same-build-pass law: these are the signed bytes
#      the release shipped, never a rebuild beside it.
#   3. pkgbuild per component (relative install-location — the payload lands
#      under the user's home via `installer -target
#      CurrentUserHomeDirectory`, no root), productbuild over the
#      distribution (Core pre-selected, Bench and Connectors one click
#      away).
#   4. Signs with the Developer ID INSTALLER identity (a product archive
#      refuses the Application identity), notarizes, staples.
#
# D6: this is the OPERATOR lane — it holds no key in CI and is never run
# there. --no-sign exists for the hermetic drills only.

set -euo pipefail

VERSION=""
STAGED_DIR=""
OUT=""
ARCH="arm64"
NO_SIGN=0
while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="$2"; shift 2 ;;
        --staged-dir) STAGED_DIR="$2"; shift 2 ;;
        --out) OUT="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        --no-sign) NO_SIGN=1; shift ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done
[ -n "$VERSION" ] && [ -n "$STAGED_DIR" ] && [ -n "$OUT" ] || {
    echo "usage: build-tools-pkg.sh --version <semver> --staged-dir <dir> --out <pkg> [--arch arm64] [--no-sign]" >&2
    exit 2
}

REPO_ROOT=$(cd "$(dirname "$0")/../.." && pwd)
PKG_DIR="$REPO_ROOT/scripts/tools-pkg"
WORK=$(mktemp -d /tmp/stewardmem-tools-pkg.XXXXXX)
trap 'rm -rf "$WORK"' EXIT

APP_IDENTITY="Developer ID Application: Mark Fietje (JHB4UNPL3G)"
INSTALLER_IDENTITY="Developer ID Installer: Mark Fietje (JHB4UNPL3G)"
INSTALL_LOCATION="Library/Application Support/StewardMem/tools/$VERSION"

# ── 1 · the detector ────────────────────────────────────────────────────────
echo "== building stewardmem-detect (release) =="
( cd "$REPO_ROOT/shell/src-tauri" && cargo build --release --bin stewardmem-detect )
DETECT_BIN="$REPO_ROOT/shell/src-tauri/target/release/stewardmem-detect"
[ -x "$DETECT_BIN" ] || { echo "the detector did not build" >&2; exit 1; }
if [ "$NO_SIGN" = "0" ]; then
    codesign --force --options runtime --timestamp --sign "$APP_IDENTITY" "$DETECT_BIN"
    codesign --verify --strict "$DETECT_BIN"
fi

# ── 2 · the component payloads (from the STAGED signed bytes) ───────────────
# The staged dir carries the RELEASE ASSET names (<tool>-darwin-<arch>); the
# pkg carries the TOOL names. The arch is the app's own floor (arm64) — an
# x86_64 variant is a mechanical later addition, recorded in the round doc.
echo "== assembling payloads from $STAGED_DIR (arch $ARCH) =="
for tool in brain-server brain mcp bench brain-connector-stub brain-connector-gh brain-connector-crm; do
    [ -f "$STAGED_DIR/$tool-darwin-$ARCH" ] || { echo "the staged dir is missing $tool-darwin-$ARCH — refusing to rebuild beside the release" >&2; exit 1; }
done
mkdir -p "$WORK/core" "$WORK/bench" "$WORK/connectors"
cp "$STAGED_DIR/brain-server-darwin-$ARCH" "$WORK/core/brain-server"
cp "$STAGED_DIR/brain-darwin-$ARCH" "$WORK/core/brain"
cp "$STAGED_DIR/mcp-darwin-$ARCH" "$WORK/core/mcp"
cp "$DETECT_BIN" "$WORK/core/stewardmem-detect"
cp "$STAGED_DIR/bench-darwin-$ARCH" "$WORK/bench/bench"
cp "$STAGED_DIR/brain-connector-stub-darwin-$ARCH" "$WORK/connectors/brain-connector-stub"
cp "$STAGED_DIR/brain-connector-gh-darwin-$ARCH" "$WORK/connectors/brain-connector-gh"
cp "$STAGED_DIR/brain-connector-crm-darwin-$ARCH" "$WORK/connectors/brain-connector-crm"
chmod 755 "$WORK"/core/* "$WORK"/bench/* "$WORK"/connectors/*

# The scripts: the shared library (version baked) + the per-component
# postinstall, copied into each component's scripts dir by the builder.
for component in core bench connectors; do
    mkdir -p "$WORK/$component-scripts"
    sed "s/__VERSION__/$VERSION/" "$PKG_DIR/lib-postinstall.sh" > "$WORK/$component-scripts/lib-postinstall.sh"
    case "$component" in
        core) cp "$PKG_DIR/core-postinstall.sh" "$WORK/$component-scripts/postinstall" ;;
        *)    cp "$PKG_DIR/extras-postinstall.sh" "$WORK/$component-scripts/postinstall" ;;
    esac
    chmod +x "$WORK/$component-scripts/postinstall"
done

# ── 3 · pkgbuild per component + productbuild ──────────────────────────────
echo "== pkgbuild x3 (install-location: ~/$INSTALL_LOCATION) =="
for component in core bench connectors; do
    pkgbuild \
        --root "$WORK/$component" \
        --scripts "$WORK/$component-scripts" \
        --identifier "com.stewardmem.tools.$component" \
        --version "$VERSION" \
        --install-location "$INSTALL_LOCATION" \
        "$WORK/$component.pkg"
done

echo "== productbuild (the selective distribution) =="
mkdir -p "$WORK/resources"
cp "$PKG_DIR/welcome.html" "$PKG_DIR/conclusion.html" "$WORK/resources/"
# The distribution carries the pkg-refs with the baked version — the
# builder versions the XML the same way it versions the library.
sed "s/__VERSION__/$VERSION/" "$PKG_DIR/distribution.xml" > "$WORK/distribution.xml"
SIGN_ARGS=()
if [ "$NO_SIGN" = "0" ]; then
    SIGN_ARGS=(--sign "$INSTALLER_IDENTITY")
fi
productbuild \
    --distribution "$WORK/distribution.xml" \
    --package-path "$WORK" \
    --resources "$WORK/resources" \
    "${SIGN_ARGS[@]}" \
    "$OUT"
[ "$NO_SIGN" = "0" ] || { echo "UNSIGNED (drill mode): $OUT"; exit 0; }

# ── 4 · notarize + staple ───────────────────────────────────────────────────
echo "== notarytool =="
xcrun notarytool submit "$OUT" --keychain-profile stewardmem --wait
echo "== stapler =="
xcrun stapler staple "$OUT"
echo "== the receipt =="
pkgutil --check-signature "$OUT"
xcrun stapler validate "$OUT"
echo "staged, signed, notarized, stapled: $OUT"
