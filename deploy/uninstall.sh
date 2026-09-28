#!/usr/bin/env bash
# brain-server — Linux uninstall.
#
# Removes the SERVICE, never the STATE. An operator running this to "clean up"
# must never lose a customer's database, and a maintenance script that deletes
# the data directory is a data-loss incident wearing a friendly name.
#
# To remove the data as well, that is a DELIBERATE act, and it is spelled out
# here rather than done implicitly.
set -euo pipefail

DATA=/var/lib/brain-server
UNIT=/etc/systemd/system/brain-server.service

[ "$(id -u)" -eq 0 ] || { echo "uninstall.sh must run as root" >&2; exit 1; }

printf 'brain-server uninstall\n\n'

# ── stop the service ──────────────────────────────────────────────────────────
if systemctl is-active --quiet brain-server.service; then
  echo "stopping brain-server (SIGTERM -> drain -> wal_checkpoint(TRUNCATE))"
  systemctl stop brain-server.service
fi
systemctl disable brain-server.service >/dev/null 2>&1 || true

# ── remove the unit and the binaries ──────────────────────────────────────────
rm -f "$UNIT"
rm -f /usr/local/bin/brain-server /usr/local/bin/brain
rm -f /usr/local/bin/brain-clean-cycle-check /usr/local/bin/brain-shutdown-stamp
systemctl daemon-reload

# ── the data directory is NOT removed ─────────────────────────────────────────
cat <<EOF
removed the service.

  the DATA at $DATA is INTACT and was not touched.
  brain-server cannot start after this without a reinstall, but the store,
  its audit chain and its keys are all still there.

  If you intend to remove the data as well, that is a DELIBERATE act, and it
  is spelled out here rather than done implicitly:

      1. brain shred --db $DATA/brain.db   # asserts byte-level erasure
      2. remove the directory by hand, having read that line and decided

  The first line is not optional ceremony: it is the difference between
  asserting the bytes are gone and hoping they are. The second is deliberately
  NOT written out as a command — an operator who has to type the destructive one
  is an operator who has decided to.
EOF
