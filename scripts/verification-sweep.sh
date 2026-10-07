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
# Audit EVERY workspace lockfile on disk, not just the root's. Bare `cargo
# audit` covers the root workspace only, which is exactly why the
# RUSTSEC-2026-0285 rustls finding (0.23.43 → 0.23.45) landed in the tools/*
# trees: the root lockfile never saw it. CI already loops over every lockfile
# (.github/workflows/ci.yml, "Audit every tracked workspace lockfile"); this
# local gate now mirrors it rather than being the weaker of the two.
#
# On-disk, not `git ls-files`: that is one MORE than CI audits (fuzz/Cargo.lock
# is gitignored via fuzz/.gitignore), so coverage errs high rather than leaving
# a hole, and the loop cannot miss a lockfile that a checkout would carry.
audit_every_lockfile() {
  local found=0 lock n=0
  while IFS= read -r lock; do
    n=$((n + 1))
    if ! cargo audit --file "$lock"; then found=1; fi
  done < <(find . -name Cargo.lock -not -path '*/target/*')
  echo "audited $n lockfile(s)"
  return "$found"
}
run "audit"                  audit_every_lockfile
# Lock FRESHNESS, not just vulnerabilities: a lockfile can be committed stale
# against its own manifest, and every bare cargo invocation (CI lane, local
# clippy) then re-locks SILENTLY — green over dependency versions nobody
# committed. The probe MUST be the full-form `cargo metadata --locked`: the
# --no-deps form passes vacuously on exactly the stale locks this lane exists
# to catch, because it never resolves the requirement graph against the lock.
#
# TRACKED lockfiles only (git ls-files), unlike the audit lane's on-disk find:
# freshness is a property of what a checkout builds, and the one on-disk
# exception (fuzz/Cargo.lock, gitignored) is a local build artifact no CI
# checkout ever sees — flagging it would make this lane permanently red over
# a file the repository does not ship.
lock_freshness_every_lockfile() {
  local found=0 lock dir n=0
  while IFS= read -r lock; do
    n=$((n + 1))
    dir=$(dirname "$lock")
    if ! cargo metadata --locked --format-version 1 --manifest-path "$dir/Cargo.toml" >/dev/null 2>&1; then
      echo "STALE  $lock (cargo metadata --locked refused)"
      found=1
    fi
  done < <(git ls-files -z -- '*Cargo.lock' | while IFS= read -r -d '' f; do echo "$f"; done)
  echo "lock-freshness checked $n tracked lockfile(s)"
  return "$found"
}
run "lock-freshness"         lock_freshness_every_lockfile
run "docs-truth"             bash scripts/docs-truth.sh
run "env-truth"              bash scripts/env-truth.sh --selfcheck
# The gateway-token truth lane: the openclaw gateway must authenticate with
# the AGENT token, never the operator token (the twokeys boundary only binds
# traffic to a revocable principal if the process carries the agent token).
# Digest-only output — no token material on any lane. Machine-local: on a
# checkout without an openclaw gateway env it SKIPS loudly (exit 0) — absence
# of the setup is not a violation, and CI must stay green without the setup.
run "secrets-truth"          bash scripts/secrets-truth.sh --selfcheck
run "badges"                 bash scripts/badges.sh --selfcheck
run "lipstyk"                scripts/lipstyk-gate.sh

echo "SWEEP_EXIT=$fail" >> "$LOG"
echo "SWEEP_EXIT=$fail"