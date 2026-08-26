#!/usr/bin/env bash
# repo-brief.sh — one-shot ground truth for AI coding agents (and humans).
#
# Answers, in under a second, the questions every session-start re-verify
# list asks: versions, HEAD, dirty state, structural inventories, guard
# presence. Output is FACTS FROM THE TREE, never from docs. Feed it to a
# fresh agent instead of letting it guess (stale cites are how docs drift).
set -euo pipefail
REPO="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO"

ver() { sed -n 's/^version = "\(.*\)"/\1/p' "$1" 2>/dev/null | head -1; }

echo "== brain-server repo brief ($(date -u +%FT%TZ)) =="
echo "server:  $(ver Cargo.toml)"
echo "client:  $(ver client/Cargo.toml)"
echo "plugin:  $(sed -n 's/^  "version": "\(.*\)",/\1/p' plugin/package.json 2>/dev/null || true)"
echo "HEAD:    $(git log --oneline -1 | head -1)"
n_dirty=$(git status --porcelain | wc -l | tr -d ' ')
echo "dirty:   ${n_dirty} path(s)"
[ "$n_dirty" != "0" ] && git status --porcelain | sed 's/^/  /'

echo "--- structural inventory (src/) ---"
m=src/main.rs
if [ -f "$m" ]; then
  total=$(wc -l < "$m" | tr -d ' ')
  testln=$(awk '/^#\[cfg\(test\)\]/{found=NR} END{print found}' "$m")
  routes=$(grep -c '\.route(' "$m")
  echo "main.rs: ${total} lines | test-region from L${testln:-?} | .route( sites: ${routes}"
fi
echo "env vars:      $(grep -rhoE 'BRAIN_[A-Z0-9_]+' src --include='*.rs' | sort -u | wc -l | tr -d ' ')"
echo "unique paths:  $(tr '\n' ' ' < "$m" 2>/dev/null | grep -oE '"(/[a-zA-Z0-9{}_.:/-]+)"' | sort -u | wc -l | tr -d ' ')"
echo "cli commands:  $(sed -n 's/^        name: "\([a-z-]*\)",/\1/p' src/bin/brain.rs 2>/dev/null | wc -l | tr -d ' ')"

echo "--- guards present ---"
for g in docs_truth dup_guard spire_inventory reg_watch sql_inventory_baseline; do
  hit=$(grep -rl "$g" src --include='*.rs' 2>/dev/null || true | head -1)
  printf '%-24s %s\n' "$g" "${hit:-ABSENT}"
done

echo "--- living-doc staleness quick probe ---"
stale=0
while read -r pat; do
  hits=$(grep -rl "$pat" docs/*.md *.md 2>/dev/null | grep -cvE 'CHANGELOG|AGENTS_HISTORY|IMPLEMENTATION_|EXECUTION_|ROADMAP' || true)
  [ "${hits:-0}" != "0" ] && { echo "  STALE MARKER '$pat' in $hits file(s)"; stale=1; }
done <<'MARKERS'
BRAIN_PORT
CHECKPOINT_EVERY
TLS_ENABLED
no erase command
MARKERS
[ "$stale" = "0" ] && echo "  clean (no known stale markers in living docs)"
echo "== end brief =="
