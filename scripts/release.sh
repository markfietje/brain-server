#!/usr/bin/env bash
# Release helper for brain-server.
# Usage: ./scripts/release.sh vX.Y.Z   (e.g. ./scripts/release.sh v1.13.0)
#
# What it does:
#   1. sanity checks (clean tree, tag not taken, main in sync with origin)
#   2. warns if Cargo.toml / CHANGELOG.md don't match the version
#   3. creates an annotated tag and pushes it
#   4. the GitHub Actions "release" workflow then builds the binaries and
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
