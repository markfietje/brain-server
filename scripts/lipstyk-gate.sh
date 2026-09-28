#!/bin/sh
# lipstyk-gate.sh — the local lipstyk diff-watchdog, trap-proof.
#
# CI's lipstyk job diffs a push against that push's BASE commit; the naive
# local invocation has two failure modes that make a "clean" run lie:
#
#   1. MOVING BASE — `lipstyk --diff "$(git rev-parse origin/main)"` goes
#      VACUOUS once you push: origin/main == HEAD, the diff is empty, exit
#      is 0, and nothing was scanned. A pass after a push proves nothing
#      (the v1.28.49 escape: the watchdog only fired in CI).
#   2. INVISIBLE NEW FILES — untracked files appear in no `git diff`, so a
#      brand-new module is never scored until it is staged or committed.
#
# This wrapper closes both, fail-closed:
#   - marks untracked files intent-to-add (`git add -N`) so new modules are
#     diffable WITHOUT staging their content (reversible with `git reset`);
#   - resolves a base that cannot move: explicit arg > pre-push merge-base
#     (exactly what CI will diff against) > HEAD~1 (post-push recovery for
#     a single-commit push; pass the old remote tip or the last release
#     tag for multi-commit pushes, e.g. `lipstyk-gate.sh v1.28.48`);
#   - REFUSES to pass vacuously: an empty changed-line set under the
#     scanned trees is a hard failure, not a green light.
#
# `--hook` (for .git/hooks/pre-push): same enforcement, two softer edges —
#   nothing to lint → pass with a note (a docs-only push is honest pass,
#   not a lie), and a missing lipstyk binary → pass with a note (the CI
#   watchdog is the canonical backstop; a tool-less machine must not have
#   every push bricked). Real findings still block, here and in CI.
#
# SCOPE — `crates` was added to the scanned trees. It was missing, and the
# omission was invisible in the worst way: a round that adds a whole crate
# under `crates/` looks exactly like a round that adds no Rust at all to the
# linted trees. lipstyk is PATH-based and does not care that `crates/` is a
# separate cargo workspace node — the exclusion was this script's, not the
# tool's. `fuzz/` and the three `tools/*` nodes are still unscanned and are
# the same omission; they are called out rather than silently carried.

set -eu

cd "$(git rev-parse --show-toplevel)"

# The scanned trees, defined ONCE and used in all three places below. A path
# list duplicated across the intent-to-add, the vacuity check, and the scan
# itself is precisely how a scope drifts so that one of the three silently
# stops covering — the class of failure this repo keeps paying for, and the
# reason the list is a variable rather than a literal repeated three times.
SCAN_PATHS="src client plugin crates"

MODE=normal
if [ "${1:-}" = "--hook" ]; then
    MODE=hook
    shift
fi

soft() { # hook-mode note: pass with a reason
    echo "lipstyk-gate: $1"
    exit 0
}

if ! command -v lipstyk >/dev/null 2>&1; then
    if [ "$MODE" = hook ]; then
        soft "lipstyk not installed — skipping (the CI watchdog still enforces)"
    fi
    echo "lipstyk-gate: lipstyk binary not found on PATH" >&2
    exit 1
fi

BASE=${1:-}
if [ -z "$BASE" ]; then
    if git rev-parse --verify -q '@{u}' >/dev/null 2>&1; then
        MB=$(git merge-base '@{u}' HEAD)
        if [ "$MB" != "$(git rev-parse HEAD)" ]; then
            BASE=$MB
        fi
    fi
    if [ -z "$BASE" ]; then
        BASE=HEAD~1
        echo "lipstyk-gate: no unpushed work — diffing $BASE (pass the old remote tip or release tag for multi-commit pushes)" >&2
    fi
fi

# New files must be visible to git diff (intent-to-add: an index entry that
# carries no content; tracked files are untouched).
if [ -n "$(git ls-files --others --exclude-standard -- $SCAN_PATHS)" ]; then
    git add -N -- $SCAN_PATHS >/dev/null 2>&1 || true
fi

CHANGED=$(git diff --name-only "$BASE" -- $SCAN_PATHS)
if [ -z "$CHANGED" ]; then
    if [ "$MODE" = hook ]; then
        soft "nothing to lint under $SCAN_PATHS vs $BASE"
    fi
    echo "lipstyk-gate: REFUSING to pass vacuously — no changed lines under $SCAN_PATHS vs $BASE." >&2
    echo "  (already pushed? pass a real base: scripts/lipstyk-gate.sh HEAD~N  |  v<last-release-tag>)" >&2
    exit 1
fi

echo "lipstyk-gate: base=$BASE changed: $(echo "$CHANGED" | tr '\n' ' ')"
exec lipstyk --diff "$BASE" --exclude-tests $SCAN_PATHS
