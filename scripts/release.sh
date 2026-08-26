#!/usr/bin/env bash
# Release helper for brain-server.
# Usage: ./scripts/release.sh vX.Y.Z   (e.g. ./scripts/release.sh v1.13.0)
#
# What it does:
#   1. sanity checks (clean tree, tag not taken, main in sync with origin)
#   2. BLOCKS until the CI run for that exact commit is green (fail-closed:
#      the tag itself re-runs no tests, so this wait is the only bridge
#      between "pushed main" and "shipped binaries")
#   3. warns if Cargo.toml / CHANGELOG.md don't match the version
#   4. creates an annotated tag and pushes it
#   5. the GitHub Actions "release" workflow then builds the binaries and
#      creates the GitHub release with auto-generated notes.
set -euo pipefail

TAG="${1:-}"
if [[ -z "$TAG" || "$TAG" != v* ]]; then
  echo "usage: $0 vX.Y.Z   (e.g. ./scripts/release.sh v1.13.0)" >&2
  exit 1
fi

# 1a. Working tree clean?
if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: working tree is not clean. Commit or stash before releasing." >&2
  exit 1
fi

# 1b. Tag already exists (local or remote)?
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "error: tag $TAG already exists locally." >&2
  exit 1
fi
if git ls-remote --tags origin "refs/tags/$TAG" 2>/dev/null | grep -q "$TAG"; then
  echo "error: tag $TAG already exists on origin." >&2
  exit 1
fi

# 1c. Tag must point at a commit already published on origin/main.
HEAD_SHA="$(git rev-parse HEAD)"
ORIGIN_SHA="$(git rev-parse origin/main 2>/dev/null || echo missing)"
if [[ "$HEAD_SHA" != "$ORIGIN_SHA" ]]; then
  echo "error: local main is not in sync with origin/main. Run 'git push origin main' first." >&2
  exit 1
fi

# 1d. CI must be GREEN on the exact commit being tagged. The tag re-runs
# nothing (ci.yml ignores v* tags and release.yml runs no tests), so the
# main-push CI run is the ONLY automated gate between "pushed" and
# "shipped" — tagging while it is red or unfinished would publish binaries
# that never passed the matrix. Fail-closed: no verifiable green = no tag.
if ! command -v gh >/dev/null 2>&1; then
  echo "error: gh CLI not found — refusing to tag without verifying CI green" >&2
  echo "       on the commit (fail-closed). Install + auth gh, verify the Actions" >&2
  echo "       tab yourself, or push a tag manually at your own judgement." >&2
  exit 1
fi
echo ">> waiting for CI (ci.yml) on ${HEAD_SHA:0:10} to finish…"
RUN_ID=""
for i in $(seq 1 20); do  # ≤ 10 min for the run to register after the push
  RUN_ID="$(gh run list --workflow ci.yml --commit "$HEAD_SHA" --json databaseId --limit 1 --jq '.[0].databaseId' 2>/dev/null || true)"
  [[ -n "$RUN_ID" ]] && break
  sleep 30
done
if [[ -z "$RUN_ID" ]]; then
  echo "error: no CI run found for $HEAD_SHA — was main actually pushed? Refusing to tag (fail-closed)." >&2
  exit 1
fi
RUN_URL="$(gh run view "$RUN_ID" --json url --jq .url 2>/dev/null || echo "(gh run view $RUN_ID)")"
echo ">> watching CI run #$RUN_ID — $RUN_URL"
STATUS="queued"; CONCLUSION=""
for i in $(seq 1 120); do  # ≤ 60 min for the matrix to complete
  LINE="$(gh run view "$RUN_ID" --json status,conclusion --jq '[.status, (.conclusion // "")] | @tsv' 2>/dev/null || true)"
  if [[ -n "$LINE" ]]; then
    STATUS="${LINE%%$'\t'*}"
    CONCLUSION="${LINE##*$'\t'}"
  fi
  [[ "$STATUS" == "completed" ]] && break
  sleep 30
done
if [[ "$STATUS" != "completed" ]]; then
  echo "error: CI run #$RUN_ID still '$STATUS' after 60 min — refusing to tag. Re-run release.sh once it is green." >&2
  exit 1
fi
if [[ "$CONCLUSION" != "success" ]]; then
  echo "error: CI on $HEAD_SHA concluded '$CONCLUSION' — a red matrix must never ship. Fix it, then re-run release.sh." >&2
  exit 1
fi
echo ">> CI green on ${HEAD_SHA:0:10}."

# 2. Version consistency warnings (non-fatal).
CARGO_VER="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [[ "$CARGO_VER" != "${TAG#v}" ]]; then
  echo "warning: Cargo.toml says $CARGO_VER but you are tagging ${TAG#v}."
  echo "         Bump Cargo.toml and CHANGELOG.md first if this release should match."
fi
if ! grep -q "^## \[${TAG#v}\]" CHANGELOG.md; then
  echo "warning: CHANGELOG.md has no section for ${TAG#v}. Add one if you want it in the notes."
fi

# 3. Tag and push. The GitHub Actions release workflow takes it from here.
git tag -a "$TAG" -m "Release $TAG"
git push origin "$TAG"
echo ""
echo "Tag $TAG pushed. The 'release' workflow is building binaries and creating the release."
echo "Watch it: https://github.com/markfietje/brain-server/actions"
echo "Result:   https://github.com/markfietje/brain-server/releases"
