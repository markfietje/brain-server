#!/usr/bin/env bash
# Sync private `main` to the public mirror, correctly.
#
#   scripts/sync-mirror.sh            → sync main, then report the diff
#   scripts/sync-mirror.sh --check    → report only; never push
#
# The mirror is NOT byte-identical to private, by ONE file:
# `.github/dependabot.yml` exists on private (per-ecosystem PR caps and
# grouping) and is DELETED on public (no Dependabot on the release mirror —
# version updates belong where `main` actually moves).
#
# That asymmetry is the reason this script exists. Done by hand it is tribal
# knowledge, and it has already gone wrong: a plain merge of public/main takes
# the file from private and Dependabot wakes up on the mirror, and because
# ci.yml fires on a bare `pull_request:` with no path filter each version PR
# queues the full 20-job Rust matrix. That is how 14 PRs put ~400 jobs in a
# queue that did not drain for nine hours.
#
# The sequence below is the safe order and it is not interchangeable:
#   1. merge the mirror's HEAD into local main, resolving dependabot.yml to
#      PRIVATE's version — that makes the mirror an ancestor, so step 2 is a
#      fast-forward rather than a force;
#   2. push main to the mirror;
#   3. re-apply the deletion as a MIRROR-ONLY commit.
# Steps 2 and 3 must be separate pushes. Doing them in one means the mirror
# briefly has the Dependabot config, and Dependabot wakes on it immediately.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

MIRROR_FILE=".github/dependabot.yml"
CHECK_ONLY=0
[[ "${1:-}" == "--check" ]] && CHECK_ONLY=1

say() { printf '  %s\n' "$*"; }

# Step 3 builds the mirror-only commit on a DETACHED HEAD, so an interruption
# anywhere after that point used to strand the repo off `main` — and, worse,
# strand it exactly when the mirror is still carrying $MIRROR_FILE. On
# 2026-10-13 a run died between the step-2 push and the step-3 push: Dependabot
# woke on the mirror, opened a PR, and the local checkout stayed detached.
# Restore the starting ref on the way out, whatever happened.
ORIGINAL_HEAD="$(git rev-parse HEAD)"
ORIGINAL_REF="$(git symbolic-ref -q HEAD || echo '')"
restore_head() {
  local rc=$?
  if [[ -z "$(git symbolic-ref -q HEAD || echo '')" && -n "$ORIGINAL_REF" ]]; then
    git symbolic-ref HEAD "$ORIGINAL_REF" >/dev/null 2>&1 || true
    git reset --hard "$ORIGINAL_HEAD" >/dev/null 2>&1 || true
    echo "WARN: restored HEAD to $ORIGINAL_REF — this run did not finish" >&2
  fi
  return $rc
}
trap restore_head EXIT

if ! git diff --quiet || [[ -n "$(git status --porcelain)" ]]; then
  echo "ERR: working tree is dirty — commit or stash before syncing the mirror." >&2
  exit 1
fi

# `public` is a REMOTE NAME, not a ref — `git fetch origin public` asks origin
# for a branch called "public" and exits 128. Fetch each remote by name.
git fetch origin --quiet
git fetch public --quiet

if [[ $CHECK_ONLY -eq 1 ]]; then
  echo "mirror sync status:"
  if git merge-base --is-ancestor public/main HEAD 2>/dev/null; then
    say "public/main is an ancestor of main — push would be a fast-forward"
  else
    say "public/main is NOT an ancestor — the merge in step 1 is required"
  fi
  ahead=$(git rev-list --count public/main..main 2>/dev/null || echo 0)
  say "main is $ahead commit(s) ahead of the mirror"
  if git cat-file -e "public/main:$MIRROR_FILE" 2>/dev/null; then
    # Fail-closed. This used to be a WARNING and still exit 0, which is how a
    # mirror carrying Dependabot survived a --check. `--check` exists to be
    # trusted; a warning nobody is required to read is not a check.
    echo "ERR: $MIRROR_FILE is PRESENT on the mirror — Dependabot is awake." >&2
    echo "     Re-run without --check to re-apply the deletion." >&2
    exit 1
  fi
  say "ok: $MIRROR_FILE is absent on the mirror"
  exit 0
fi

if ! git merge-base --is-ancestor public/main HEAD 2>/dev/null; then
  say "step 1: merging the mirror's HEAD, keeping private's $MIRROR_FILE"
  git merge public/main --no-edit
  # The merge takes the mirror's DELETION by default; private must keep it.
  if [[ ! -f "$MIRROR_FILE" ]]; then
    git checkout HEAD^2 -- "$MIRROR_FILE" 2>/dev/null \
      || git checkout HEAD~1 -- "$MIRROR_FILE"
    git add "$MIRROR_FILE"
    git commit --amend --no-edit
  fi
  [[ -f "$MIRROR_FILE" ]] || { echo "ERR: could not restore $MIRROR_FILE" >&2; exit 1; }
fi
say "step 1 done — public/main is an ancestor, so the push is a fast-forward"

say "step 2: pushing main to the mirror"
git push public main

# Step 3 — the mirror-only deletion.
#
# BUILT DETACHED, ON PURPOSE. The first version of this script ran `git rm` +
# `git commit` on local main and pushed that to public. Because the commit
# then sat on local main, the NEXT private push carried it — and its deletion
# — into brain-server-private's history, where dependabot.yml silently
# disappeared. It stayed missing for two syncs before --check caught it.
#
# A mirror-only artifact must never be an ancestor of private main, or it is
# no longer mirror-only. So: detach, commit, push THAT sha to public, return.
# Local main is not advanced and never holds the deletion.
say "step 3: re-applying the mirror-only $MIRROR_FILE deletion"
LOCAL_HEAD="$(git rev-parse HEAD)"
git checkout --detach "$LOCAL_HEAD" >/dev/null 2>&1
git rm -q "$MIRROR_FILE"
git commit -q -m "dependabot: still no version updates on the release mirror

Unchanged in substance: version updates belong on the private repo, where
main moves and dependabot.yml carries per-ecosystem caps and grouping. This
mirror re-applies the deletion on every sync — see scripts/sync-mirror.sh."
MIRROR_SHA="$(git rev-parse HEAD)"
# Fast-forward on the mirror: public/main == LOCAL_HEAD by step 1.
git push public "$MIRROR_SHA":main >/dev/null
git checkout main >/dev/null 2>&1
git reset --hard "$LOCAL_HEAD" >/dev/null
[[ -f "$MIRROR_FILE" ]] || { echo "ERR: local $MIRROR_FILE did not survive step 3" >&2; exit 1; }

# Verify the MIRROR, not the local tree. The check above can only prove this
# checkout is intact; the whole point of step 3 is what landed on the remote, and
# that is the thing that has silently failed before. Read the ref back from the
# remote so a push that appeared to succeed but moved nothing is caught here.
git fetch public --quiet
if [[ "$(git rev-parse --short public/main)" != "$(git rev-parse --short "$MIRROR_SHA")" ]]; then
  echo "ERR: mirror is at $(git rev-parse --short public/main), expected $MIRROR_SHA" >&2
  exit 1
fi
if git cat-file -e "public/main:$MIRROR_FILE" 2>/dev/null; then
  echo "ERR: $MIRROR_FILE is STILL on the mirror after step 3 — Dependabot is awake" >&2
  exit 1
fi
say "ok: mirror verified at $MIRROR_SHA with $MIRROR_FILE absent"

echo
echo "mirror synced. verify with:  scripts/sync-mirror.sh --check"