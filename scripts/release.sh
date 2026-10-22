#!/usr/bin/env bash
# Release helper for brain-server.
# Usage: ./scripts/release.sh vX.Y.Z   (e.g. ./scripts/release.sh v1.29.3)
#
# What it does:
#   1. sanity checks (clean tree, tag not taken, main in sync with origin,
#      public remote actually pushable; then the schema-drift drill — 1f,
#      added by the 1.29.22 stale-client repair: regenerate the typed client
#      and byte-compare it against the contract, refusing on mismatch or on
#      an inability to run; then the CHANGELOG section check, which the
#      release-notes generator refuses to publish without)
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
#   origin (brain-server-private)  — main, and the TAGS that gate shipping.
#                                     Private working history.
#   public (brain-server)          — main is pushed here too (restored
#                                     2026-10-13), plus every release TAG.
#                                     This is where releases and their binaries
#                                     live, and where `release.yml` must fire.
#                                     It is NOT a byte-identical mirror: see
#                                     scripts/sync-mirror.sh for the one file
#                                     that differs by design.
# `main` IS pushed to `public` again (restored 2026-10-13). The tag-only rule
# below was the 2026-10-06 posture, written on the assumption that this branch
# carries rounds that must not be published. That assumption does not hold for
# this project right now — there is no shipped product, no landing page, no
# company, no public surface to protect — and the rule was costing the one
# thing CI was being kept alive for: per-merge verification. Pushing `main`
# restores it, because public-repo Actions are free where private ones are not.
#
# Publishing unreleased work on the branch is therefore ACCEPTED here. The
# release boundary is still the TAG, and it is the tag that gates shipping:
# `release.yml` fail-closes unless `ci.yml` was green for the tagged SHA, so
# a green `main` push publishes nothing at all. `main` is the review surface;
# the tag is the release surface. Do not conflate them.
#
# Revisit if that ever stops being true: if this project acquires a public
# surface or work that genuinely must not be published, restore tag-only.
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

# 1f. The typed client matches the contract being shipped (the schema-drift
# drill). This is a LOCAL gate and not a CI gate, and the reason is the
# 1.29.22 repair (36195da9): the shell lane (`shell.yml`) carries the drift
# gate, but it triggers on main-push / pull-request / workflow_dispatch and
# NEVER on a tag push, private-repo Actions are throttled to zero here, and
# the release verdict release.yml fail-closes on is the ci.yml matrix — so
# the gate was silent across every event a release actually travels.
# `openapi.yaml` moved in release commits while the shell tree sat untouched
# and no trigger ever ran the lane; the drift had been in the tree since
# 520a1dea, and the release shipped a client whose documented shapes were
# stale ones. A stale client does not fail a build — it FAILS OPEN, which is
# the worse failure. So: regenerate locally, byte-compare, refuse on both
# sides — a MISMATCH refuses, and an INABILITY to check refuses too. A gate
# that cannot run must be louder than a green one, not quieter.
if [[ ! -f openapi.yaml ]]; then
  echo "error: openapi.yaml is missing — the drift gate has no contract to check against (a kernel-only tree is not a releasable tree). Refusing to tag." >&2
  exit 1
fi
if [[ ! -f shell/src/lib/api/schema.d.ts ]]; then
  echo "error: shell/src/lib/api/schema.d.ts is missing. Regenerate it (cd shell && pnpm gen:api), commit it, then re-run. Refusing to tag." >&2
  exit 1
fi
if ! command -v pnpm >/dev/null 2>&1; then
  echo "error: pnpm is unavailable, so the drift gate cannot run. Refusing to tag rather than shipping a client whose shapes are unverified." >&2
  echo "       Install pnpm (corepack enable), run the regen by hand (cd shell && pnpm gen:api)," >&2
  echo "       commit it, and re-run this script. Refusing is the fail-closed direction:" >&2
  echo "       a release whose gate did not run is a release that may be lying." >&2
  exit 1
fi
DRIFT_TMP="$(mktemp)"
trap 'rm -f "$DRIFT_TMP"' EXIT
if (cd shell && pnpm exec openapi-typescript ../openapi.yaml -o "$DRIFT_TMP") >/dev/null; then
  if ! cmp -s shell/src/lib/api/schema.d.ts "$DRIFT_TMP"; then
    echo "error: shell/src/lib/api/schema.d.ts is STALE against openapi.yaml — regenerating produces different bytes." >&2
    echo "       Run:   cd shell && pnpm gen:api   (then commit the regenerated file) and re-run." >&2
    echo "       This is the 1.29.22 shape and it must be refused, not re-created: the" >&2
    echo "       one lane carrying the drift gate does not fire on a tag push, so the stale" >&2
    echo "       bytes would ride to the tag with nothing between them and the release." >&2
    echo "       First diff hunk:" >&2
    diff shell/src/lib/api/schema.d.ts "$DRIFT_TMP" | head -12 >&2 || true
    exit 1
  fi
  echo ">> 1f drift gate: shell/src/lib/api/schema.d.ts matches openapi.yaml byte-for-byte."
else
  echo "error: the drift regen itself failed to run. Is shell/node_modules installed (cd shell && pnpm install)?" >&2
  echo "       Refusing to tag rather than shipping an unverified client: the byte-comparison" >&2
  echo "       below this point cannot be trusted if the regeneration never happened." >&2
  exit 1
fi

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
  # The COMMIT the tag points at, never the tag object.
  #
  # This script creates an ANNOTATED tag (`git tag -a`), and `git rev-parse
  # <annotated-tag>` returns the TAG OBJECT's id. GitHub registers a tag-push
  # run against the COMMIT, so querying by the object id matches nothing: the
  # witness waited its full 10 minutes and then reported "the tag triggered
  # nothing observable" on releases whose matrix was green and whose GitHub
  # release was published. It fired on 1.29.10 and again on 1.29.11, and both
  # times the only reason anyone knew the release was fine was a human opening
  # the run list by hand. A witness that cries wolf on success is worse than no
  # witness — it teaches you to ignore the one signal the release line has.
  SHA="$(git rev-parse "$TAG^{commit}")"
  echo ">> watching the public runs for ${TAG} (${SHA:0:10})…"
  # Every query is scoped to THIS tag's runs, not to every run that ever touched
  # the commit: a tag push registers under the tag name, so `headBranch` is what
  # separates this release from an older run on the same commit (a re-cut tag, a
  # manual `workflow_dispatch`, a backfill). Without the scope, a stale RED from
  # an earlier attempt would be read as this release's verdict — the mirror image
  # of the defect above, and just as wrong.
  RUN_SEEN=""
  for _ in $(seq 1 20); do  # ≤ 10 min for the runs to register after the push
    RUN_SEEN="$(gh run list --repo markfietje/brain-server --commit "$SHA" \
                 --json databaseId,headBranch \
                 --jq "[.[] | select(.headBranch == \"$TAG\")] | .[0].databaseId // empty" \
                 2>/dev/null || true)"
    [[ -n "$RUN_SEEN" ]] && break
    sleep 30
  done
  if [[ -z "$RUN_SEEN" ]]; then
    echo "error: no public runs registered for ${TAG} (${SHA:0:10}) within 10 min" >&2
    echo "       — the tag triggered nothing observable. Nothing was witnessed, and" >&2
    echo "       release.yml cannot have published without its own fail-closed check." >&2
    echo "       Inspect: https://github.com/markfietje/brain-server/actions" >&2
    exit 1
  fi
  # THE WITNESS. `scripts/release-watch.sh` owns the verdict, so ONE
  # implementation answers for the operator AND for the pin that drives it
  # with fixtures (`tests/release_witness_gate.rs`).
  #
  # The loop this replaces decided the verdict with an inner `while` that
  # could only clear its "done" flag for a run it SAW: when `gh run list`
  # returned nothing — a transient API failure, a rate limit, a query that
  # matched no rows — the loop body never ran, the flag stayed set, and the
  # script printed "all public runs are green". It did exactly that on the
  # v1.29.26 release, while BOTH public runs were still in progress. Nothing
  # was mis-published (the enforcement is release.yml's own fail-closed check,
  # not this helper's), but this helper is the operator's WITNESS, and a
  # witness that cries wolf on success teaches you to ignore the one signal
  # the release line has.
  #
  # So: no runs observed is UNKNOWN, and UNKNOWN keeps the watch alive.
  REPO_SLUG="markfietje/brain-server"
  WATCH_SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-watch.sh"
  VERDICT=""
  VERDICT_RC=0
  VERDICT="$(bash "$WATCH_SCRIPT" "$REPO_SLUG" "$SHA" "$TAG")" || VERDICT_RC=$?
  case "$VERDICT_RC" in
    0)
      echo ">> all public runs for $TAG are green."
      ;;
    1)
      echo "error: public runs concluded not-green (${VERDICT#RED}). Nothing published:" >&2
      echo "       release.yml refuses to ship over a red matrix. Inspect:" >&2
      echo "       https://github.com/markfietje/brain-server/actions" >&2
      exit 1
      ;;
    3)
      echo "error: the release witness could not run — ${VERDICT:-unknown reason}" >&2
      exit 1
      ;;
    *)
      echo "warning: no green verdict for $TAG within 75 min — keep watching:" >&2
      echo "         https://github.com/markfietje/brain-server/actions" >&2
      echo "         Publication stays fail-closed until they land green." >&2
      exit 0
      ;;
  esac
fi
echo "Watch it: https://github.com/markfietje/brain-server/actions"
echo "Result:   https://github.com/markfietje/brain-server/releases"

# ── THE RELEASE'S UPDATE CHAIN ─────────────────────────────────────────────
#
# The endpoint the desktop application fetches is the LATEST release's copy of
# the update manifest, so the moment a new release is published, the manifest
# the previous release carried stops being the one that is served. **A release
# that ships without its chain leaves the updater's path DEAD for everyone** —
# and nothing else in the release pipeline can see it, because the chain can
# only be produced where the key is (the signing lane), and the chain
# necessarily lands AFTER the release is published.
#
# **So the tool that cuts the release is the tool that publishes the chain.**
# The passphrase comes from the LOGIN KEYCHAIN — never typed, never written
# down, never in this repository — and the signing lane refuses to run in the
# public Actions, so the key stays where it is.
#
# The chain's own checks are the signing lane's: the pairing (the key that
# signs IS the key the application ships), the round trip (the artefact's
# signature verifies against the application's PINNED public key), the
# manifest's binding, and the three-asset gate. This step adds the LAST one —
# the only question only the public internet can answer: **DOES THE ENDPOINT
# ANSWER?**
echo
echo "=== the release's update chain ==="
CHAIN_NOTES="$(awk -v ver="${TAG#v}" '
  /^## \[/ {
    if (insec) exit
    insec = ($0 ~ "^## \\[" ver "\\]")
    if (insec) {
      split($0, parts, "—")
      title = parts[3]
      gsub(/^[ \t]+|[ \t]+$/, "", title)
      print title
      exit
    }
  }
' CHANGELOG.md)"
[[ -n "$CHAIN_NOTES" ]] || CHAIN_NOTES="Release ${TAG}."

# 1. The sidecar must be staged for THIS release's commit: the manifest
#    writer refuses a staged record that names another commit, and the record
#    is what says which kernel the bundle carries.
echo "-- staging the sidecar for $(git rev-parse --short HEAD)…"
if ! node shell/scripts/sidecar.mjs stage; then
  echo "error: the sidecar could not be staged for ${TAG} — the chain needs it." >&2
  echo "       The release is published; the chain is not. The updater's endpoint" >&2
  echo "       will answer 'not found' until it is." >&2
  exit 1
fi

# 2. The signing lane: build the updater artefact with the bundler, sign it
#    with the key the application ships, verify the round trip against the
#    application's PINNED key, write the manifest, and publish all three.
echo "-- signing and publishing the chain…"
if ! scripts/release-app-sign.sh --tag "$TAG" --notes "$CHAIN_NOTES"; then
  echo "error: the chain did not publish for ${TAG}." >&2
  echo "       The release is published; the chain is not. The updater's endpoint" >&2
  echo "       will answer 'not found' until it is." >&2
  exit 1
fi

# 3. THE LAST CHECK: does the address the application fetches answer? This is
#    the one only the public internet can answer, and it is the one that
#    decides whether the update path works at all.
CHAIN_CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-chain-check.sh"
if bash "$CHAIN_CHECK" > /dev/null 2>&1; then
  echo ">> the updater's endpoint answers: the chain is LIVE for ${TAG}."
else
  echo "error: the updater's endpoint does NOT answer for ${TAG}." >&2
  echo "       The release is published, but the address the application fetches" >&2
  echo "       is dead — so the updater cannot update. Inspect:" >&2
  echo "       https://github.com/markfietje/brain-server/releases/latest" >&2
  exit 1
fi
echo "Chain:   https://github.com/markfietje/brain-server/releases/latest/download/latest.json"

