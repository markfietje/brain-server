#!/usr/bin/env bash
# The release WITNESS's verdict — one implementation, two callers.
#
# Usage:
#   scripts/release-watch.sh <repo-slug> <sha> <tag> [attempts] [interval_seconds]
#
#   Prints on stdout:   GREEN | RED <conclusions> | UNKNOWN
#   Exits:              0 GREEN | 1 RED | 2 UNKNOWN | 3 the witness could not run
#
#   The callers are `scripts/release.sh` (which watches the public runs after
#   the tag push) and `tests/release_witness_gate.rs` (which drives it against
#   fixtures). A check that exists in only one of them fails only in the more
#   expensive one — and this one lives at the moment a release is being cut.
#
# ── THE LAW ────────────────────────────────────────────────────────────────
# **An EMPTY run list is UNKNOWN, never GREEN.**
#
# The witness this replaces read the runs with an inner `while` that could only
# clear its "done" flag for a run it SAW:
#
#     DONE=1
#     while IFS=$'\t' read -r ST CO; do
#       [[ "$ST" != "completed" ]] && DONE=0
#       [[ -n "$CO" && "$CO" != "success" ]] && VERDICT="$VERDICT $CO"
#     done < <(gh run list ... 2>/dev/null)
#     [[ "$DONE" == 1 ]] && break
#
# So when `gh run list` returned NOTHING — a transient API failure, a rate
# limit, a query that matched no rows — the loop body never ran, DONE stayed
# 1, and the script printed "all public runs for $TAG are green".
#
# It did exactly that on the v1.29.26 release: the witness reported green
# while BOTH public runs were still `in_progress`, roughly twenty minutes
# before the matrix landed. Nothing was mis-published — the ENFORCEMENT is
# release.yml's own fail-closed check on the ci.yml verdict, not this
# helper's — but this helper is the operator's WITNESS, and a witness that
# cries wolf on success teaches you to ignore the one signal the release line
# has. The release.sh header already records that lesson for the loop
# BESIDE this one (it queried the tag OBJECT rather than the COMMIT and
# waited its full ten minutes on releases that had published); this is the
# same class of defect, in the other loop.
#
# So: no runs observed is UNKNOWN, and UNKNOWN keeps the watch alive. The
# only GREEN is every observed run completed, with a `success` conclusion.
set -uo pipefail

SLUG="${1:-}"
SHA="${2:-}"
TAG="${3:-}"
ATTEMPTS="${4:-150}"
INTERVAL="${5:-30}"

if [[ -z "$SLUG" || -z "$SHA" || -z "$TAG" ]]; then
  echo "usage: $0 <repo-slug> <sha> <tag> [attempts] [interval_seconds]" >&2
  exit 3
fi
if ! command -v gh >/dev/null 2>&1; then
  # No `gh` is no verdict. Failing closed here is the whole point: a witness
  # that cannot ask must not answer.
  echo "the release witness could not run: 'gh' is not on PATH" >&2
  exit 3
fi

# One poll: the runs this tag's SHA registered, as "<status>\t<conclusion>".
# Scoped to the TAG, never to the commit alone — a tag push registers under
# the tag name, and without that scope a stale RED from an earlier attempt
# on the same commit would be read as this release's verdict.
observed() {
  gh run list --repo "$SLUG" --commit "$SHA" \
    --json status,conclusion,headBranch \
    --jq ".[] | select(.headBranch == \"$TAG\") | [.status, (.conclusion // \"\")] | @tsv" \
    2>/dev/null || true
}

for i in $(seq 1 "$ATTEMPTS"); do
  raw="$(observed)"

  if [[ -z "${raw//[[:space:]]/}" ]]; then
    # NOTHING OBSERVED. Not a verdict, and never a green one.
    echo ">> the witness observed no runs for $TAG — unknown, still watching" >&2
  else
    bad=""
    pending=0
    while IFS=$'\t' read -r st co; do
      [[ -z "${st//[[:space:]]/}" ]] && continue
      st="${st//$'\r'/}"
      co="${co//$'\r'/}"
      if [[ "$st" != "completed" ]]; then
        pending=1
        continue
      fi
      # A completed run with NO conclusion is not a success. Fail closed.
      if [[ "$co" != "success" ]]; then
        bad="$bad ${co:-none}"
      fi
    done <<< "$raw"

    if [[ -n "$bad" ]]; then
      # A run that concluded badly decides the release, whether or not another
      # run is still going.
      echo "RED$bad"
      exit 1
    fi
    if [[ "$pending" == 1 ]]; then
      echo ">> the runs for $TAG are still in progress — unknown, still watching" >&2
    else
      echo "GREEN"
      exit 0
    fi
  fi

  [[ "$i" -lt "$ATTEMPTS" ]] && sleep "$INTERVAL"
done

echo "UNKNOWN"
exit 2
