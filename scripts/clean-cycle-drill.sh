#!/usr/bin/env bash
# Deploy the REAL R48 unit + scripts to the MiniPC and run the clean-cycle
# drill (E1). Read-only against the live install: the drill runs its own
# instance on port 8766 with its own data dir, and the live one is never
# touched.
set -uo pipefail

STAMP=$(date -u +%Y%m%dT%H%M%SZ)
LOG=/tmp/clean-cycle-drill-$STAMP.log
D=/home/mark/brain-demo
B=/home/mark/brain-server/target/release/brain-server

say() { printf '\n=== %s ===\n' "$1"; }
run() { printf '$ %s\n' "$*"; "$@" 2>&1; }

exec > >(tee -a "$LOG") 2>&1
say "R48 clean-cycle DRILL  $STAMP"
echo "log: $LOG"

# ── 0. preconditions ─────────────────────────────────────────────────────────
say "0. preconditions"
run uname -srm
findmnt -no FSTYPE,SOURCE /home/mark || true
test -x "$B" && echo "binary: present" || { echo "MISSING $B"; exit 1; }

# ── 1. seed ──────────────────────────────────────────────────────────────────
say "1. seed the demo store"
curl -sf -X POST http://127.0.0.1:8766/ingest/memory -H 'content-type: application/json' \
  -d '{"content":"R48 drill: the City Treasurer processes real property tax payments every third working day.","domain":"global"}' \
  && echo "  seeded" || echo "  (service not running; will start then seed)"

# ── 2. the fingerprint BEFORE ─────────────────────────────────────────────────
say "2. state fingerprint BEFORE the cycle"
run "$B" --version
/home/mark/brain-server/target/release/brain anchor --db $D/active/brain.db 2>&1 | head -1 | tee /tmp/r48-anchor-before.txt
cat /tmp/r48-anchor-before.txt

# ── 3. graceful stop (what an office does at 18:00) ───────────────────────────
say "3. GRACEFUL STOP  (SIGTERM -> drain -> wal_checkpoint(TRUNCATE))"
PID=$(pgrep -f "$B" | head -1)
echo "pid=$PID"
S=$(date +%s%N)
kill -TERM "$PID"
while kill -0 "$PID" 2>/dev/null; do sleep 0.02; done
E=$(date +%s%N)
echo "MEASURED SIGTERM->exit: $(( (E-S)/1000000 )) ms"
echo "live instance still up: $(pgrep -c -f /home/mark/.local/bin/brain-server)"

# ── 4. prove it is cold ───────────────────────────────────────────────────────
say "4. COLD — nothing must be listening on 8766"
echo "listeners on 8766: $(ss -tln 2>/dev/null | grep -c 8766)"

# ── 5. the morning boot ───────────────────────────────────────────────────────
say "5. COLD START  (what happens at 08:00)"
S=$(date +%s%N)
tmux kill-session -t braindemo-run 2>/dev/null
tmux new-session -d -s braindemo-run "$D/start.sh > $D/logs/server.log 2>&1"
until curl -sf -o /dev/null http://127.0.0.1:8766/health; do sleep 0.01; done
E=$(date +%s%N)
echo "MEASURED boot->serving: $(( (E-S)/1000000 )) ms"
run curl -s http://127.0.0.1:8766/health

# ── 6. the fingerprint AFTER — must be byte-identical ─────────────────────────
say "6. state fingerprint AFTER the cycle"
/home/mark/brain-server/target/release/brain anchor --db $D/active/brain.db 2>&1 | head -1 | tee /tmp/r48-anchor-after.txt
cat /tmp/r48-anchor-after.txt

say "7. THE VERDICT"
if diff -q /tmp/r48-anchor-before.txt /tmp/r48-anchor-after.txt >/dev/null; then
  echo "PASS — the fingerprint is BYTE-IDENTICAL across the cycle."
else
  echo "FAIL — the fingerprint MOVED:"; diff /tmp/r48-anchor-before.txt /tmp/r48-anchor-after.txt
fi
say "8. post-cycle health"
run curl -s http://127.0.0.1:8766/audit/verify
run curl -s -X POST http://127.0.0.1:8766/recall -H 'content-type: application/json' \
  -d '{"query":"City Treasurer tax payments","domain":"global"}' | head -c 200
echo
echo "DRILL COMPLETE — $LOG"
