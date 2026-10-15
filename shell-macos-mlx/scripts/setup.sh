#!/usr/bin/env bash
# Put MLX's metallib where the test process can find it.
#
# MLX's last-resort loader is `load_library_from_path(device, METAL_PATH)`
# where METAL_PATH is the BARE RELATIVE filename "default.metallib"
# (mlx/backend/metal/device.cpp). Everything before it is bundle-discovery,
# which does not enumerate the SwiftPM resource bundle inside an xctest
# process — so the first four strategies miss and only the relative path
# works. `swift test` runs with CWD at the package root, hence the link HERE.
#
# Symptom when this is missing:
#   MLX error: Failed to load the default metallib. library not found
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
SRC=$(find .build -name default.metallib -path "*mlx-swift_Cmlx.bundle*" | head -1)
if [[ -z "$SRC" ]]; then
  echo "setup: no metallib in .build yet — run 'swift build --build-tests' first" >&2
  exit 1
fi
ln -sf "$SRC" default.metallib
echo "setup: default.metallib -> $SRC"
