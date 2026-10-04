#!/usr/bin/env bash
# R65 verification sweep — the lanes that never ran.
# Sequential on purpose: parallel cargo on one target dir serialises on the
# cargo lock anyway, and separate target dirs would force full rebuilds.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
export RUSTFLAGS="-D warnings"
LOG=target/r65-verify.log
: > "$LOG"
fail=0

run() {
  local name="$1"; shift
  echo "### $name" >> "$LOG"
  if "$@" >> "$LOG" 2>&1; then
    echo "PASS  $name" >> "$LOG"
  else
    echo "FAIL  $name" >> "$LOG"
    fail=1
  fi
}

run "all-targets"            cargo test --all-targets
run "clippy-otel"            cargo clippy --all-targets --features otel -- -D warnings
for f in loom rerank-tier neural-embed injection-classifier compliance-pack multivec; do
  run "lane-$f" cargo clippy --all-targets --features "$f" -- -D warnings
done
run "test-crates"            cargo test --manifest-path crates/Cargo.toml --all-targets
run "test-harness"           cargo test --manifest-path tools/steward-harness/Cargo.toml
run "audit"                  cargo audit
run "docs-truth"             bash scripts/docs-truth.sh
run "env-truth"              bash scripts/env-truth.sh --selfcheck
run "badges"                 bash scripts/badges.sh --selfcheck
run "lipstyk"                scripts/lipstyk-gate.sh

echo "SWEEP_EXIT=$fail" >> "$LOG"
echo "SWEEP_EXIT=$fail"