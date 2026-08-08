#!/usr/bin/env bash
# Publish the wiki/ directory to the GitHub wiki for this repo.
#
# GitHub only materializes a repo's wiki the first time a page is created in the
# browser (Wiki tab -> "Create the first page"). Until then, a push to the
# .wiki.git remote fails with "Repository not found". After you create any first
# page, run this script to push the full wiki.
#
# Usage: scripts/publish-wiki.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC_DIR="$REPO_ROOT/wiki"
WIKI_REMOTE="git@github.com:markfietje/brain-server.wiki.git"
WORK="$(mktemp -d /tmp/brain-server-wiki-publish.XXXXXX)"

trap 'rm -rf "$WORK"' EXIT

if [ ! -d "$SRC_DIR" ]; then
  echo "error: no wiki/ directory found at $SRC_DIR" >&2
  exit 1
fi

echo ">> Staging wiki into $WORK"
git init -b master "$WORK" >/dev/null
git -C "$WORK" remote add origin "$WIKI_REMOTE"

# Fetch the existing wiki (may contain a placeholder first page), then take
# ownership of every .md page from wiki/ as the source of truth.
git -C "$WORK" fetch origin >/dev/null 2>&1 || true
git -C "$WORK" checkout -q master 2>/dev/null || git -C "$WORK" checkout -q --orphan master

git -C "$WORK" rm -rf --quiet --ignore-unmatch '*.md'
# Copy every markdown page (including _Sidebar.md and _Footer.md).
cp "$SRC_DIR"/*.md "$WORK"/

git -C "$WORK" add -A

if ! git -C "$WORK" diff --cached --quiet HEAD >/dev/null 2>&1; then
  git -C "$WORK" commit -m "Publish Brain Server wiki" >/dev/null
  echo ">> Pushing to $WIKI_REMOTE"
  git -C "$WORK" push -u origin master --force-with-lease
  echo ">> Wiki published to https://github.com/markfietje/brain-server/wiki"
else
  echo ">> No changes to publish (wiki/ already matches the remote)."
fi
