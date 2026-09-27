#!/bin/sh
# Three-way documentation truth: router source vs openapi.yaml vs docs/.
# Exits non-zero only on a HIGH finding — a route in one source and not
# another. LOW findings are printed and do not fail the gate.
exec python3 "$(dirname "$0")/docs-truth.py" "$@"
