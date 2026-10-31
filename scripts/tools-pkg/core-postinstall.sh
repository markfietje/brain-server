#!/bin/bash
# The CORE component's postinstall — brain-server, brain, mcp (+ the
# detection core's own binary, which stays in the anchor unlinked: it is the
# preserve law's instrument, not a user tool).
#
# pkgbuild runs this as the invoking user under
# `installer -target CurrentUserHomeDirectory`; $3 is the install destination
# (the payload anchor) and $HOME is the user's home. The laws live in
# lib-postinstall.sh (sourced beside this file by the builder).
set -u

. "$(dirname "$0")/lib-postinstall.sh"

install_modes

# Every payload binary gets its ~/.local/bin link by the preserve law — the
# payload IS the list (self-describing; no baked names to drift).
for tool_path in "$ANCHOR"/*; do
    name=$(basename "$tool_path")
    [ "$name" = "stewardmem-detect" ] && continue
    install_tool_link "$name"
done

print_hand_off
exit 0
