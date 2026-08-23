#!/usr/bin/env bash
# v1.28.20 Cockpit M1 — desktop bundle wrapper (operator step, not CI).
# Wraps the documented `dx` command set with deploy-web.sh's fail-on-error
# discipline. The dx CLI must be installed (cargo install dioxus-cli).
#
# Usage: scripts/build-desktop.sh [macos|nsis|appimage|all]
set -euo pipefail
cd "$(dirname "$0")/../client"

command -v dx >/dev/null 2>&1 || { echo "dx CLI not found — cargo install dioxus-cli"; exit 1; }

target="${1:-macos}"
case "$target" in
  macos)   package_types='"macos" "dmg"' ;;
  nsis)    package_types='"nsis"' ;;
  appimage) package_types='"appimage"' ;;
  all)     package_types='"macos" "dmg" "nsis" "appimage"' ;;
  *) echo "unknown target: $target (macos|nsis|appimage|all)"; exit 2 ;;
esac

# shellcheck disable=SC2086
eval dx bundle --desktop --package-types $package_types
echo "desktop bundle complete ($target)"
