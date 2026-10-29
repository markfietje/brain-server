#!/usr/bin/env bash
# III-7 — the release's update chain: DOES THE ENDPOINT ANSWER?
#
# Usage:
#   scripts/release-chain-check.sh [url] [attempts] [interval_seconds]
#
#   Prints on stdout:   LIVE | DEAD
#   Exits:              0 LIVE | 1 DEAD | 2 the check could not run
#
# ── WHY THIS EXISTS ─────────────────────────────────────────────────────────
#
# The endpoint the desktop application fetches is the LATEST release's copy of
# the update manifest (`releases/latest/download/latest.json`), so **the moment
# a new release is published, the manifest the previous release carried stops
# being the one that is served.** A release that ships without its chain
# therefore leaves the updater's path DEAD for everyone — and nothing in the
# release pipeline can see it, because the chain can only be produced where the
# key is (the signing lane), and the chain necessarily lands AFTER the release
# is published.
#
# **This is the check that can see it.** Everything else in the release is
# about what the release CONTAINS; this one asks the only question that
# matters to a user — *does the address the application fetches answer?* — and
# it asks the PUBLIC INTERNET, because that is where the application is.
#
# It is ONE implementation with TWO callers: `scripts/release.sh` (which cuts
# the release and must not report success while the endpoint is dead) and
# `tests/release_chain_gate.rs` (which drives it against fixtures). A check
# that exists in only one of them fails only in the more expensive one — and
# the expensive one is the moment a release is being cut.
#
# The check is BOUNDED: it retries a few times (the release's assets are
# served by GitHub, so there is a short settling delay after an upload) and
# then says DEAD. A check that waits forever is a check that hangs the
# release line.
set -uo pipefail

URL="${1:-}"
ATTEMPTS="${2:-6}"
INTERVAL="${3:-10}"

if [[ -z "$URL" ]]; then
  URL="https://github.com/stewardmem/stewardmem/releases/latest/download/latest.json"
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "the chain check could not run: 'curl' is not on PATH" >&2
  exit 2
fi

for i in $(seq 1 "$ATTEMPTS"); do
  # `-f` fails on a 404 and on a 5xx; `-L` follows GitHub's redirect to the
  # asset; `-o /dev/null` asks only whether the bytes arrive.
  if curl -fsSL --max-time 30 -o /dev/null "$URL" 2>/dev/null; then
    echo "LIVE"
    exit 0
  fi
  [[ "$i" -lt "$ATTEMPTS" ]] && sleep "$INTERVAL"
done

echo "DEAD"
exit 1
