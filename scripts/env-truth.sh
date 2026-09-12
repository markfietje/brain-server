#!/usr/bin/env bash
# docs-vs-code environment-variable truth gate (v1.28.87 docs-truth).
#
# Every BRAIN_* knob a reader can mistake for LIVE configuration must either
# be implemented in the tree or be explicitly owned as removed/roadmap with a
# forward Loop tracking row. Silent documented-but-unimplemented knobs are the
# failure this script exists to catch (fourth-pass S-01/S-02 class).
#
# Scope (deliberate):
#   contract docs: docs/configuration.md docs/deployment.md (what operators read)
#   live config:   deploy/tiers/*.env (what actually exports)
#   code:          src crates client tools (where a knob would be read)
# Excluded on purpose: CHANGELOG.md, docs/AGENTS_HISTORY.md, IMPLEMENTATION_PLAN_*
#   files — append-only history; past claims there are corrected by new notes,
#   never rewritten.
#
# Rules per distinct BRAIN_[A-Z0-9_]+ name found in scope:
#   1. implemented (rg hit in src|crates|client|tools) → OK.
#   2. else the docs mention MUST carry a same-line qualifier
#      (removed|roadmap|ships? in|planned|Loop|future|not yet|out of scope)
#      AND ROADMAP.md MUST track it forward (same line holds the name plus a
#      Loop|roadmap|Planned|future|v[23]\. marker — a Shipped history row does
#      NOT count) → OK with owner. Otherwise FAIL naming var + file:line.
#   3. every BRAIN_* exported by deploy/tiers/*.env MUST be implemented —
#      tier files are live config, qualifiers do not apply → FAIL if not.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail=0

implemented() { rg -q --no-messages "$1" "$REPO/src" "$REPO/crates" "$REPO/client" "$REPO/tools" 2>/dev/null; }

# Rule 3 first: tiers are live config, no qualifiers accepted.
if ls "$REPO"/deploy/tiers/*.env >/dev/null 2>&1; then
  for f in "$REPO"/deploy/tiers/*.env; do
    while IFS= read -r name; do
      [[ -z "$name" ]] && continue
      if ! implemented "$name"; then
        echo "ERR: tier file $f exports '$name' with zero code hits (src|crates|client|tools)" >&2
        fail=1
      fi
    done < <(rg -o --no-filename 'BRAIN_[A-Z0-9_]+' "$f" 2>/dev/null | sort -u || true)
  done
fi

# Rules 1+2: contract docs.
while IFS= read -r name; do
  [[ -z "$name" ]] && continue
  if implemented "$name"; then continue; fi
  # unimplemented: check qualifier + Loop tracking
  ok_doc=0; ok_roadmap=0; where=""
  while IFS= read -r loc; do
    where="${where} [${loc}]"
    line="$(printf '%s' "$loc" | sed 's/^[^:]*:[0-9][0-9]*://')"
    if printf '%s' "$line" | rg -qi 'removed|roadmap|ships? in|planned|Loop|future|not yet|out of scope'; then
      ok_doc=1
    fi
  done < <(rg -n --no-heading "$name" "$REPO/docs/configuration.md" "$REPO/docs/deployment.md" 2>/dev/null || true)
  while IFS= read -r rline; do
    if printf '%s' "$rline" | rg -qi 'Loop|roadmap|planned|future|v[23]\.'; then
      if ! printf '%s' "$rline" | rg -qi 'shipped'; then ok_roadmap=1; fi
    fi
  done < <(rg -n --no-heading "$name" "$REPO/ROADMAP.md" 2>/dev/null || true)
  if [[ "$ok_doc" == "1" && "$ok_roadmap" == "1" ]]; then continue; fi
  echo "ERR: '$name' documented but unimplemented (zero code hits). mentions:${where:- none}" >&2
  [[ "$ok_doc" == "0" ]] && echo "     missing: same-line removed/roadmap qualifier in docs" >&2
  [[ "$ok_roadmap" == "0" ]] && echo "     missing: forward Loop tracking row in ROADMAP.md" >&2
  fail=1
done < <(rg -o --no-filename 'BRAIN_[A-Z0-9_]+' "$REPO/docs/configuration.md" "$REPO/docs/deployment.md" 2>/dev/null | sort -u || true)

if [[ "$fail" == "1" ]]; then exit 1; fi
echo "OK  env truth clean (tiers live, docs qualified + Loop-tracked)"
