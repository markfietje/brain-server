#!/usr/bin/env bash
# Release helper for brain-server.
# Usage: ./scripts/release.sh vX.Y.Z   (e.g. ./scripts/release.sh v1.29.3)
#
# What it does:
#   1. sanity checks (clean tree, tag not taken, main in sync with origin,
#      public remote actually pushable, CHANGELOG section exists — the
#      release-notes generator refuses to publish without one)
#   2. warns if Cargo.toml doesn't match the version
#   3. creates an annotated tag and pushes it to `public` — under the
#      2026-10-06 billing law (see below) that tag push IS the public CI
#      trigger
#   4. watches the public runs for the tagged SHA and exits non-zero on a
#      not-green verdict (the AGENTS.md witness duty — enforcement is NOT
#      here)
#   5. the GitHub Actions "release" workflow builds the binaries, REFUSES
#      to publish unless the ci.yml run for the tagged SHA is green
#      (fail-closed), and creates the GitHub release with notes generated
#      from CHANGELOG.md.
#
# TWO REMOTES, DELIBERATELY:
#   origin (brain-server-private)  — main. Private working history.
#   public (brain-server)          — release TAGS only. This is where releases
#                                     and their binaries are meant to live, and
#                                     where `release.yml` must fire.
# `main` is NOT pushed to `public`, and must not be: a tag carries its commit's
# whole ancestry, but a branch carries everything after it too. Publishing only
# the tag ships the released snapshot; publishing the branch would ship the
# rounds that follow, which include work that is deliberately not public.
#
# The consequence to know about: `public/main` therefore LAGS `origin/main` by
# design, and it must not be treated as drift to be corrected.
#
# CI AND THE BILLING LAW (2026-10-06): private-repo Actions are DISABLED —
# the free 2,000 min/month died in six days. There is no pre-tag CI run left
# to wait on: the TAG PUSH is the CI trigger. A v* tag on `public` runs the
# full ci.yml matrix there (public-repo Actions are free), and release.yml's
# publication step fail-closes unless that matrix is green for the tagged
# SHA — red or absent ⇒ binaries build but NOTHING publishes.
# Tests-before-shipping survives the private-CI removal intact: the gate
# moved out of a helper's wait loop into the workflow that does the
# shipping. The pre-tag discipline is the LOCAL gate suite (AGENTS.md → CI
# dry-run): cut the tag only from a tree that already passed it. This
# script's remaining CI duty is the witness — watch the public runs, report
# red loudly, exit non-zero on a not-green verdict — and `git tag && git
# push --tags` bypassing this script still cannot publish, because the
# enforcement lives in release.yml (S7-10), not here.
set -euo pipefail

TAG="${1:-}"
if [[ -z "$TAG" || "$TAG" != v* ]]; then
  echo "usage: $0 vX.Y.Z   (e.g. ./scripts/release.sh v1.29.3)" >&2
  exit 1
fi

# 1a. Working tree clean?
if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: working tree is not clean. Commit or stash before releasing." >&2
  exit 1
fi

# 1b. Tag already exists (local, or already PUBLISHED)?
# The remote half MUST ask `public`, not `origin`: releases live on `public`,
# and `origin` has never held a single tag. Checking `origin` therefore made
# this guard permanently vacuous — it could never catch a duplicate, so a
# re-run would sail past it. It is the same bug as the push at the bottom.
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "error: tag $TAG already exists locally." >&2
  exit 1
fi
if git ls-remote --tags public "refs/tags/$TAG" 2>/dev/null | grep -q "$TAG"; then
  echo "error: tag $TAG is already published on public." >&2
  exit 1
fi

# 1c. `public` must actually be pushable BEFORE the tag is created. The push
# URL was once set to the literal string `DISABLED` (the private-only era);
# it is enabled again. The guard stays as a cheap tripwire so a re-disabled
# URL fails HERE instead of after the tag object exists and the matrix has
# run — which would leave a half-finished release and a local tag that 1b
# then refuses to recreate. Fail here instead, for free.
PUBLIC_PUSH_URL="$(git remote get-url --push public 2>/dev/null || true)"
if [[ -z "$PUBLIC_PUSH_URL" || "$PUBLIC_PUSH_URL" == "DISABLED" ]]; then
  echo "error: remote 'public' is not pushable (push URL = '${PUBLIC_PUSH_URL:-<none>}')." >&2
  echo "       Releases are published to the PUBLIC repo — the URLs at the bottom" >&2
  echo "       of this script name it. Re-enable the push URL, or push the tag" >&2
  echo "       yourself at your own judgement. Refusing to tag." >&2
  exit 1
fi

# 1d. Tag must point at a commit already published on origin/main.
# `origin`, not `public` — see the two-remote note at the top. This is the
# gate that keeps a release reproducible from private history without
# publishing that history.
HEAD_SHA="$(git rev-parse HEAD)"
ORIGIN_SHA="$(git rev-parse origin/main 2>/dev/null || echo missing)"
if [[ "$HEAD_SHA" != "$ORIGIN_SHA" ]]; then
  echo "error: local main is not in sync with origin/main. Run 'git push origin main' first." >&2
  exit 1
fi

# 1e. The green-CI gate lives in the PUBLIC release workflow, not here
# (2026-10-06: private-repo CI is disabled for billing; public Actions are
# free). The TAG PUSH itself triggers the full ci.yml matrix on public —
# `main` is deliberately never pushed there — and release.yml's publication
# step fail-closes on that very run: red OR absent ci.yml for the tagged
# SHA ⇒ binaries build but NOTHING publishes. The pre-tag discipline is the
# LOCAL gate suite (fmt, clippy, tests, badges) on the tree being tagged.

# 2. Version consistency: Cargo mismatch is a warning (the tag defines the
# release; a deliberate mismatch has no downstream failure). A missing
# CHANGELOG section is FATAL here: release.yml's notes generator refuses to
# publish without one, and finding that out after the matrix and four
# cross-builds is an hour of runners spent learning what grep knew for free.
CARGO_VER="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [[ "$CARGO_VER" != "${TAG#v}" ]]; then
  echo "warning: Cargo.toml says $CARGO_VER but you are tagging ${TAG#v}."
  echo "         Bump Cargo.toml and CHANGELOG.md first if this release should match."
fi
if ! grep -q "^## \[${TAG#v}\]" CHANGELOG.md; then
  echo "error: CHANGELOG.md has no '## [${TAG#v}]' section. release.yml generates the" >&2
  echo "       published notes from it and REFUSES to publish without one — add the" >&2
  echo "       section (with its ### Release notes block), then re-run. Refusing to" >&2
  echo "       spend the build queue on a release that cannot publish." >&2
  exit 1
fi

# 3. Tag and push. `public`, NOT `origin`: the release workflow, the
# binaries and the GitHub release all live on the public repo, and pushing the
# tag to `origin` produced neither — which is why three version bumps' worth
# of releases (v1.29.0/1/2) sat tagged on this machine and nowhere else.
# Only the tag goes: it carries the released snapshot's ancestry, not the
# rounds after it.
git tag -a "$TAG" -m "Release $TAG"
git push public "$TAG"
echo ""
echo "Tag $TAG pushed. The tag triggers BOTH the full CI matrix and the release"
echo "workflow on public; release.yml publishes binaries + notes ONLY if that"
echo "matrix is green (its own fail-closed check watches the ci.yml run for"
echo "this SHA)."
if command -v gh >/dev/null 2>&1; then
  SHA="$(git rev-parse "$TAG")"
  echo ">> watching the public runs for ${TAG}…"
  # Registration first: an EMPTY query is not a green verdict. If the tag
  # triggered nothing (workflow disabled, YAML error, race), claiming green
  # here would be exactly the vacuous pass this repo keeps killing.
  RUN_SEEN=""
  for _ in $(seq 1 20); do  # ≤ 10 min for the runs to register after the push
    RUN_SEEN="$(gh run list --repo markfietje/brain-server --commit "$SHA" \
                 --limit 1 --json databaseId --jq '.[0].databaseId' 2>/dev/null || true)"
    [[ -n "$RUN_SEEN" ]] && break
    sleep 30
  done
  if [[ -z "$RUN_SEEN" ]]; then
    echo "error: no public runs registered for ${SHA:0:10} within 10 min — the tag" >&2
    echo "       triggered nothing observable. Nothing was witnessed, and release.yml" >&2
    echo "       cannot have published without its own fail-closed check. Inspect:" >&2
    echo "       https://github.com/markfietje/brain-server/actions" >&2
    exit 1
  fi
  DONE=0; VERDICT=""
  for _ in $(seq 1 150); do  # ≤ 75 min for the matrix + release assembly
    DONE=1; VERDICT=""
    while IFS=$'\t' read -r ST CO; do
      [[ "$ST" != "completed" ]] && DONE=0
      [[ -n "$CO" && "$CO" != "success" ]] && VERDICT="$VERDICT $CO"
    done < <(gh run list --repo markfietje/brain-server --commit "$SHA" \
               --json status,conclusion --jq '.[] | [.status, (.conclusion // "")] | @tsv' 2>/dev/null)
    [[ "$DONE" == 1 ]] && break
    sleep 30
  done
  if [[ -n "$VERDICT" ]]; then
    echo "error: public runs concluded not-green ($VERDICT). Nothing published:" >&2
    echo "       release.yml refuses to ship over a red matrix. Inspect:" >&2
    echo "       https://github.com/markfietje/brain-server/actions" >&2
    exit 1
  fi
  if [[ "$DONE" != 1 ]]; then
    echo "warning: public runs still in progress after 75 min — keep watching:" >&2
    echo "         https://github.com/markfietje/brain-server/actions" >&2
    echo "         Publication stays fail-closed until they land green." >&2
    exit 0
  fi
  echo ">> all public runs for $TAG are green."
fi
echo "Watch it: https://github.com/markfietje/brain-server/actions"
echo "Result:   https://github.com/markfietje/brain-server/releases"
