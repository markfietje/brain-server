#!/usr/bin/env bash
# brain-server — the operator's morning check (E3).
#
# Run this BEFORE opening the console. It answers one question: did the
# system come back CORRECT, not merely back up.
#
#   1. is the store structurally sound (PRAGMA integrity_check)?
#   2. is the audit chain intact?
#   3. was the LAST shutdown clean, or was the process killed?
#
# Exit 0 = PASS. Non-zero = FAIL, with the reason on stdout.
set -uo pipefail

BIN="${BRAIN_BIN:-/usr/local/bin/brain}"
DB="${BRAIN_DB_PATH:-/var/lib/brain-server/brain.db}"
STAMP="${BRAIN_STAMP:-/var/lib/brain-server/.shutdown-clean}"

pass() { printf '  [ok]   %s\n' "$1"; }
fail() { printf '  [FAIL] %s\n' "$1"; FAILED=1; }
FAILED=0

printf 'brain-server clean-cycle check — %s\n' "$(date -Is)"
printf '  db:   %s\n' "$DB"
printf '  bin:  %s\n\n' "$BIN"

[ -f "$DB" ] || { fail "no database at $DB — nothing to check"; exit 1; }

# (1) structural integrity
if command -v sqlite3 >/dev/null 2>&1; then
  RES=$(sqlite3 "$DB" 'PRAGMA integrity_check;' 2>&1)
  if [ "$RES" = "ok" ]; then
    pass "integrity_check: ok"
  else
    fail "integrity_check: $RES"
  fi
  # (E4) the storage invariant. A local block filesystem reaches 'wal'; a
  # network filesystem silently does not, and the server now refuses to start
  # on one. Reporting it here tells you BEFORE you restart the service.
  MODE=$(sqlite3 "$DB" 'PRAGMA journal_mode;' 2>&1)
  if [ "$MODE" = "wal" ] || [ "$MODE" = "memory" ]; then
    pass "journal_mode: $MODE"
  else
    fail "journal_mode: $MODE — NOT wal. This volume cannot do write-ahead logging. Move the data to a local block filesystem (ext4/xfs); a network filesystem cannot provide the advisory locking and shared memory WAL requires (sqlite.org/lockingv3.html §6.0)."
  fi
else
  printf '  [skip] sqlite3 not installed — integrity and journal mode not checked\n'
fi

# (2) the audit chain, verified through the server's own verifier
if [ -x "$BIN" ]; then
  if "$BIN" anchor --db "$DB" >/tmp/.brain-anchor.$$ 2>&1; then
    pass "anchor recorded (record it OFF-HOST — paper, password manager, second machine)"
  else
    fail "anchor failed: $(tail -1 /tmp/.brain-anchor.$$)"
  fi
  rm -f /tmp/.brain-anchor.$$
else
  printf '  [skip] %s not found — audit chain not verified\n' "$BIN"
fi

# (3) the last shutdown. This is the signal that turns a three-week-later
# mystery into a message at 08:00.
if [ -f "$STAMP" ]; then
  pass "last shutdown was CLEAN ($(cat "$STAMP"))"
else
  fail "no clean-shutdown stamp at $STAMP — the previous process was KILLED, not stopped. The database recovers its WAL automatically, but expect a slower first query and investigate what killed it."
fi

printf '\n'
if [ "$FAILED" -eq 0 ]; then
  printf 'RESULT: PASS — safe to serve.\n'
  exit 0
fi
printf 'RESULT: FAIL — resolve the above before serving.\n'
exit 1
