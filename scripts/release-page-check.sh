#!/usr/bin/env bash
# The page-shape gate — the release's PUBLISHED assets are the ~9-asset set
# EXACTLY, and an EXTRA asset is a red (the register row 127's owed guard,
# made real: "the honest guard is a check on the RELEASE's published assets —
# 'does this release carry a manifest?' — beside the release's other
# fail-closed gates").
#
# Usage:
#   scripts/release-page-check.sh <tag>                    # the live release
#   scripts/release-page-check.sh <tag> --fixture <file>   # a name-per-line file
#                                                          # (the pin's seam)
#
# ONE implementation, TWO callers:
#   * `scripts/release.sh`, which must not call a release green while the
#     page is the wrong shape — the check runs after the chain check, so a
#     green release means the endpoint answered AND the page is the right
#     shape;
#   * `tests/release_page_gate.rs`, which drives this script against
#     fixtures. A check that exists in only one of them fails only in the
#     more expensive one.
#
# THE SET (the Tools round's trim — the darwin binaries live ONLY inside the
# stapled `StewardMem-Tools.dmg`; the Linux set is ONE tarball): the page
# carries the app DMG, the updater chain (the artefact + its signature +
# latest.json), the Tools DMG, the Linux tarball, SHA256SUMS, PLATFORMS.txt
# and the SBOM — nothing else. A page carrying MORE is the bloat growing
# back; a page carrying LESS is a promise the updater discovers broken at
# the user's machine.
#
# Exit codes: 0 green · 1 red (the page is the wrong shape, or the release
# is absent or still a draft — "no published release" is the fail-closed
# reading of a draft) · 2 could not run (a missing tool is never read as a
# red page).
set -euo pipefail

TAG="${1:-}"
if [[ -z "$TAG" || "$TAG" != v* ]]; then
  echo "usage: $0 vX.Y.Z [--fixture <file>]" >&2
  exit 2
fi
VERSION="${TAG#v}"
FIXTURE=""
if [[ "${2:-}" == "--fixture" ]]; then
  FIXTURE="${3:-}"
  [[ -n "$FIXTURE" && -f "$FIXTURE" ]] || { echo "usage: $0 <tag> --fixture <file>" >&2; exit 2; }
elif [[ -n "${2:-}" ]]; then
  echo "usage: $0 <tag> [--fixture <file>]" >&2
  exit 2
fi

REPO_SLUG="${RELEASE_PAGE_REPO:-stewardmem/stewardmem}"

# The missing-tool check runs BEFORE anything that needs the other tools
# (even `mktemp`): "the check could not run" and "the page is red" are
# different facts, and the first must be reported as its own exit code.
if [[ -z "$FIXTURE" ]]; then
  command -v gh >/dev/null 2>&1 || {
    echo "could not run: gh is not on PATH — the page check cannot ask the release" >&2
    exit 2
  }
fi

# The trimmed set. The SBOM's name carries the version; everything else is
# literal. THE ORDER DOES NOT MATTER — the page is a SET.
expected=(
  "StewardMem-CLI-linux.tar.gz"
  "SHA256SUMS"
  "PLATFORMS.txt"
  "brain-server-${VERSION}.cdx.json"
  "latest.json"
  "StewardMem.app.tar.gz"
  "StewardMem.app.tar.gz.sig"
  "StewardMem-Tools.dmg"
  "StewardMem.dmg"
)

names_file="$(mktemp)"
trap 'rm -f "$names_file"' EXIT

if [[ -n "$FIXTURE" ]]; then
  cp "$FIXTURE" "$names_file"
else
  # `releases/tags/<tag>` addresses PUBLISHED releases only: a draft is not
  # found, which is exactly the fail-closed reading (a draft is not a page).
  if ! gh api "repos/${REPO_SLUG}/releases/tags/${TAG}" --jq '.assets[].name' > "$names_file" 2>/dev/null; then
    echo "PAGE RED: no published release for ${TAG} on ${REPO_SLUG} — the page has no shape at all" >&2
    exit 1
  fi
fi

missing=()
extra=()
declare -A seen=()
while IFS= read -r name; do
  [[ -z "$name" ]] && continue
  found=0
  for want in "${expected[@]}"; do
    if [[ "$name" == "$want" ]]; then found=1; break; fi
  done
  if [[ "$found" == "1" ]]; then
    seen["$name"]=1
  else
    extra+=("$name")
  fi
done < "$names_file"

for want in "${expected[@]}"; do
  [[ -n "${seen[$want]:-}" ]] || missing+=("$want")
done

red=0
if [[ ${#missing[@]} -gt 0 ]]; then
  for name in "${missing[@]}"; do echo "::error::missing asset: $name" >&2; done
  red=1
fi
if [[ ${#extra[@]} -gt 0 ]]; then
  for name in "${extra[@]}"; do echo "::error::extra asset: $name — the page carries the trimmed set EXACTLY, and the bloat cannot return" >&2; done
  red=1
fi
if [[ "$red" == "1" ]]; then
  echo "PAGE RED: ${#missing[@]} missing, ${#extra[@]} extra (expected ${#expected[@]} assets)" >&2
  exit 1
fi
echo "PAGE OK — ${#expected[@]} assets, exactly the trimmed set for ${TAG}"
