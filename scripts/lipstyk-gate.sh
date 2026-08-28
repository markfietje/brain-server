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

set -eu

cd "$(git rev-parse --show-toplevel)"

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
if [ -n "$(git ls-files --others --exclude-standard -- src client plugin)" ]; then
    git add -N -- src client plugin >/dev/null 2>&1 || true
fi

CHANGED=$(git diff --name-only "$BASE" -- src client plugin)
if [ -z "$CHANGED" ]; then
    if [ "$MODE" = hook ]; then
        soft "nothing to lint under src/ client/ plugin/ vs $BASE"
    fi
    echo "lipstyk-gate: REFUSING to pass vacuously — no changed lines under src/ client/ plugin/ vs $BASE." >&2
    echo "  (already pushed? pass a real base: scripts/lipstyk-gate.sh HEAD~N  |  v<last-release-tag>)" >&2
    exit 1
fi

echo "lipstyk-gate: base=$BASE changed: $(echo "$CHANGED" | tr '\n' ' ')"
exec lipstyk --diff "$BASE" --exclude-tests src client plugin
