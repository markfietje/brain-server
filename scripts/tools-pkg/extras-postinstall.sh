#!/bin/bash
# The BENCH and CONNECTORS components' postinstall — the same preserve law,
# the same library, one line of difference: no component-specific behavior.
# The payload IS the list.
set -u

. "$(dirname "$0")/lib-postinstall.sh"

install_modes

for tool_path in "$ANCHOR"/*; do
    name=$(basename "$tool_path")
    [ "$name" = "stewardmem-detect" ] && continue
    install_tool_link "$name"
done

print_hand_off
exit 0
