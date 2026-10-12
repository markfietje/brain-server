#!/usr/bin/env bash
# Release-notes gate: everything release.yml refuses to publish for, runnable
# LOCALLY in under a second.
#
# Usage: scripts/release-notes-check.sh [version]
#   version defaults to the `version` in Cargo.toml (the release being cut).
#
# Why this exists: every one of these four checks lived ONLY inside
# `.github/workflows/release.yml`, so the ONLY place they could ever fail was
# a full public matrix plus four cross-builds — 30-45 minutes of runners to
# learn from a `grep` that costs nothing. That is not hypothetical: the
# v1.29.13 release was refused twice at exactly this step, for two different
# reasons (an empty bullet block because the categories were h3 headings
# rather than bold headings, and audit IDs `K4-05`/`K4-12` in the intro
# prose). Both were visible locally in seconds.
#
# ONE implementation, two callers: release.yml runs this as its first
# notes-step action, and `tests/release_notes_gate.rs` runs it as a test. A
# check that exists in only one of those places is a check that fails only
# when it is most expensive — which is the defect class this file is closing.
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
fail() { echo "::error::$1" >&2; exit 1; }

if ! grep -q "^## \[$VERSION\]" CHANGELOG.md; then
  fail "CHANGELOG.md has no '## [$VERSION]' section. release.yml generates the published notes from it and refuses to publish without one."
fi

# 1. The `### Release notes` block is mandatory.
if ! awk -v ver="$VERSION" '
      /^## \[/ { if (insec) exit; insec = ($0 ~ "^## \\[" ver "\\]"); next }
      insec && /^### Release notes/ { found = 1; exit }
      END { exit found ? 0 : 1 }
    ' CHANGELOG.md; then
  fail "CHANGELOG.md section '## [$VERSION]' is missing a '### Release notes' block — refusing to publish a release without user-facing notes."
fi

# 2. Intro prose: everything between the heading and the first subsection is
#    the published body's opening paragraph.
awk -v ver="$VERSION" '
  /^## \[/ { insec = ($0 ~ "^## \\[" ver "\\]"); next }
  !insec { next }
  /^### / { exit }
  /^\*\*M[0-9]/ { exit }
  { print }
' CHANGELOG.md > /tmp/release-notes-intro.$$
if ! grep -q '[^[:space:]]' /tmp/release-notes-intro.$$; then
  rm -f /tmp/release-notes-intro.$$
  fail "CHANGELOG.md section '## [$VERSION]' carries no prose summary between the heading and '### Release notes'. The published body opens with that summary."
fi

# 3. ONLY categorized content publishes, in either shipped shape: a bullet
#    leading with its own category, or bullets grouped under a bold category
#    heading. Indented continuation lines travel WITH their bullet. The
#    `notes && /^### / { exit }` is the subtlety that bit v1.29.13: an h3
#    category heading TERMINATES the block, so `### Security fixes` silently
#    published zero bullets.
awk -v ver="$VERSION" '
  /^## \[/ { insec = ($0 ~ "^## \\[" ver "\\]"); next }
  !insec { next }
  /^### Release notes/ { notes = 1; next }
  notes && /^### / { exit }
  notes && /^\*\*(Improvements|New|Changed|Bug fixes|Security fixes)\*\*/ {
    if (printed) print ""
    print; cat = 1; printed = 1; next
  }
  notes && /^- \*\*(Improvements|New|Changed|Bug fixes|Security fixes):/ {
    print; cat = 1; printed = 1; next
  }
  notes && cat && /^[ \t]+[^ \t]/ { print; printed = 1; next }
  notes && NF && !/^- / && !/^ / { cat = 0 }
  notes && /^- / { if (cat) print; else print "UNCATEGORIZED " $0 }
' CHANGELOG.md > /tmp/release-notes-bullets.$$

bad=$(grep -c '^UNCATEGORIZED ' /tmp/release-notes-bullets.$$ || true)
if [ "$bad" -gt 0 ]; then
  grep '^UNCATEGORIZED ' /tmp/release-notes-bullets.$$ | sed 's/^UNCATEGORIZED /::error::  /' >&2
  rm -f /tmp/release-notes-intro.$$ /tmp/release-notes-bullets.$$
  fail "CHANGELOG.md '### Release notes' for $VERSION carries $bad uncategorized bullet(s). Put them under a **Improvements**/**Bug fixes**/**Security fixes** sub-heading (bold, NOT an h3) or lead each bullet with its category."
fi

# 4. Audit/finding IDs never publish — the user-facing text names behaviour,
#    the Engineering record keeps the IDs.
ids=$(grep -nE '\b([A-Z]{1,2}[0-9]+-[0-9]+|[A-Z]-[0-9]+)\b' \
      /tmp/release-notes-intro.$$ /tmp/release-notes-bullets.$$ || true)
if [ -n "$ids" ]; then
  printf '%s\n' "$ids" | sed 's/^/::error::  /' >&2
  rm -f /tmp/release-notes-intro.$$ /tmp/release-notes-bullets.$$
  fail "audit/finding IDs must not appear in the published notes — put them in the Engineering record and name proof commits in the bullets."
fi

bullets=$(grep -c '^- ' /tmp/release-notes-bullets.$$ || true)
if [ "$bullets" -lt 1 ]; then
  rm -f /tmp/release-notes-intro.$$ /tmp/release-notes-bullets.$$
  fail "CHANGELOG.md '### Release notes' block for $VERSION produced no bullets. A category heading written as '### X' instead of '**X**' ends the block there and publishes an empty body — write real notes, then re-tag."
fi

rm -f /tmp/release-notes-intro.$$ /tmp/release-notes-bullets.$$
echo "OK  release notes for $VERSION: intro present, $bullets categorized bullet(s), no audit IDs"
