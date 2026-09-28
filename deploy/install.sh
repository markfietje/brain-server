#!/usr/bin/env bash
# brain-server — Linux install (E2).
#
# Installs the binaries, the systemd unit, and the service user. It REFUSES to
# touch an existing data directory: an upgrade that silently overwrites a
# customer's database is unrecoverable, and "the installer overwrote it" is
# not a post-mortem anyone wants to write.
#
#   sudo ./deploy/install.sh [--prefix /usr/local] [--data /var/lib/brain-server]
set -euo pipefail

PREFIX=/usr/local
DATA=/var/lib/brain-server
BIN="$PREFIX/bin/brain-server"
UNIT=/etc/systemd/system/brain-server.service
SVC_USER=brain
SVC_GROUP=brain

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) PREFIX="$2"; shift 2 ;;
    --data)   DATA="$2";   shift 2 ;;
    -h|--help)
      sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[ "$(id -u)" -eq 0 ] || { echo "install.sh must run as root" >&2; exit 1; }
BIN="$PREFIX/bin/brain-server"

printf 'brain-server install\n  prefix: %s\n  data:   %s\n\n' "$PREFIX" "$DATA"

# ── refuse to clobber (E2) ───────────────────────────────────────────────────
if [ -e "$DATA/brain.db" ] && [ "${BRAIN_FORCE:-0}" != "1" ]; then
  echo "REFUSING: $DATA/brain.db already exists." >&2
  echo "This installer will not overwrite an existing store." >&2
  echo "An upgrade should:" >&2
  echo "  1. systemctl stop brain-server" >&2
  echo "  2. cp -a $DATA $DATA.bak.\$(date +%Y%m%d%H%M%S)" >&2
  echo "  3. re-run this script (binaries and unit are still installed)" >&2
  echo "  4. systemctl start brain-server" >&2
  echo "To override deliberately: BRAIN_FORCE=1 $0" >&2
  exit 1
fi

# ── service user ─────────────────────────────────────────────────────────────
if ! getent group "$SVC_GROUP" >/dev/null; then
  groupadd --system "$SVC_GROUP"
fi
if ! id -u "$SVC_USER" >/dev/null 2>&1; then
  useradd --system --gid "$SVC_GROUP" --home-dir "$DATA" \
          --shell /usr/sbin/nologin "$SVC_USER"
fi

install -d -m 0750 -o "$SVC_USER" -g "$SVC_GROUP" "$DATA" "$DATA/keys"
install -d -m 0755 "$PREFIX/bin"

# ── binaries ─────────────────────────────────────────────────────────────────
for b in brain-server brain; do
  src="$(dirname "$0")/../target/release/$b"
  if [ ! -x "$src" ]; then
    echo "missing $src — run: cargo build --release --bin brain-server --bin brain" >&2
    exit 1
  fi
  # Stop by ABSOLUTE BINARY PATH, never by a database path or a port.
  # BRAIN_DB_PATH lives in the ENVIRONMENT, not in argv, so `pkill -f` on it
  # matches nothing at all — and a stop that reports success while the process
  # is still running is worse than no stop at all.
  if pgrep -f "$src" >/dev/null 2>&1; then
    echo "stopping the running $b (SIGTERM, drain, then wal_checkpoint)"
    pkill -TERM -f "$src"
    for _ in $(seq 1 60); do pgrep -f "$src" >/dev/null 2>&1 || break; sleep 1; done
  fi
  install -m 0755 "$src" "$PREFIX/bin/$b"
done

# ── the clean-shutdown stamp (E3) ────────────────────────────────────────────
cat > "$PREFIX/bin/brain-shutdown-stamp" <<'EOS'
#!/usr/bin/env sh
# Run by systemd ExecStopPost. Its ABSENCE after a stop means the process was
# killed rather than stopped, which the morning check reports (E3).
date -Is > /var/lib/brain-server/.shutdown-clean 2>/dev/null || true
EOS
chmod 0755 "$PREFIX/bin/brain-shutdown-stamp"
install -m 0755 "$(dirname "$0")/clean-cycle-check.sh" "$PREFIX/bin/brain-clean-cycle-check"

# ── the unit ──────────────────────────────────────────────────────────────────
install -m 0644 "$(dirname "$0")/systemd/brain-server.service" "$UNIT"
# Keep the data path and the prefix the operator actually chose.
sed -i "s#/var/lib/brain-server#$DATA#g; s#/usr/local/bin#$PREFIX/bin#g" "$UNIT"

systemctl daemon-reload
systemctl enable brain-server.service >/dev/null 2>&1 || true

cat <<EOF

installed.

  unit:     $UNIT
  data:     $DATA  (owner $SVC_USER, mode 0750)
  check:    $PREFIX/bin/brain-clean-cycle-check
  start:    systemctl start brain-server
  logs:     journalctl -u brain-server -f

The data volume MUST be a local block filesystem (ext4/xfs). A network
filesystem cannot provide the advisory locking and shared memory SQLite's WAL
requires; the server refuses to start on one and names the cause.
EOF
