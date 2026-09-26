#!/bin/sh
# R36 — self-healing E2E entrypoint.
#
# WHY THIS EXISTS. Playwright's `webServer` PRE-PROBES its port and aborts with
#     Error: http://localhost:4173 is already used ...
# BEFORE it ever invokes `webServer.command`. Measured, not assumed: with the
# reclaim placed INSIDE that command, it never ran — no reclaim output and no
# `pnpm build` output appeared, because the command is never reached.
#
# An interrupted run causes this. `webServer.command` is
# `pnpm build && pnpm exec vite preview ...`, so the preview is a GRANDCHILD of
# the process Playwright supervises; interrupting kills the supervised process
# and leaves the preview holding the port.
#
# So the reclaim has to happen BEFORE Playwright starts, which is exactly what
# this wrapper does. It is the only place available without editing a package
# manifest (§10 forbids those) — `globalSetup` runs after `webServer` (measured:
# the launch order is webServer, then globalSetup).
#
# The reclaim itself is narrow: it reclaims ONLY an orphaned `vite preview` and
# REFUSES anything else on the port, so it can never kill something of yours
# that merely landed there.
#
# Usage — the same environment the round documents, plus --workers:
#   CI=true E2E_KERNEL_PORT=8799 sh scripts/e2e.sh --workers=1
set -e
cd "$(dirname "$0")/.."

PORT=4173
node scripts/reclaim-port.mjs "$PORT"

: "${CI:=true}"
: "${E2E_KERNEL_PORT:=8799}"
export CI E2E_KERNEL_PORT

exec pnpm test:e2e "$@"
