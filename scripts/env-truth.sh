#!/usr/bin/env bash
# docs-vs-code environment-variable truth gate (v1.28.87 docs-truth;
# v1.28.91 S7-05: the bare-substring matcher is now a CODE-SHAPE match).
#
# Every BRAIN_* knob a reader can mistake for LIVE configuration must either
# be implemented in the tree or be explicitly owned as removed/roadmap with a
# forward Loop tracking row. Silent documented-but-unimplemented knobs are the
# failure this script exists to catch (fourth-pass S-01/S-02 class).
#
# S7-05 closure: `implemented()` no longer accepts a bare substring — a
# doc comment, log line, or test fixture string naming the var counted as
# "implemented" under the old matcher (demonstrated red-first: a knob whose
# only in-scope occurrence was a comment passed the old gate). The same
# coarseness class the repo replaced elsewhere (sql-inventory lesson).
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
#   1. implemented (code-shape hit, or a pinned call-site inventory entry,
#      or a declared non-knob) → OK.
#   2. else the docs mention MUST carry a same-line qualifier
#      (removed|roadmap|ships? in|planned|Loop|future|not yet|out of scope)
#      AND ROADMAP.md MUST track it forward (same line holds the name plus a
#      Loop|roadmap|Planned|future|v[23]\. marker — a Shipped history row does
#      NOT count) → OK with owner. Otherwise FAIL naming var + file:line.
#   3. every BRAIN_* exported by deploy/tiers/*.env MUST be implemented —
#      tier files are live config, qualifiers do not apply → FAIL if not.
#
# --selfcheck builds throwaway fixture trees (clean + hostile) and runs this
# script against them; the hostile tree is the S7-05 red proof kept permanent.
set -euo pipefail
REPO="${BRAIN_ENV_TRUTH_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
fail=0

# Arm 2 — pinned call-site inventory: vars whose consumption shape the regex
# below cannot see. Each entry NAME|evidence-with-file:line; a hit is printed,
# never silent. Keep this list SHORT — a var belongs here only when its name
# is derived at runtime or consumed by a pinned external process.
PINNED_CALLSITES=(
  "BRAIN_CASE_STATUS_KEY|secrets-ladder derive: src/secrets.rs format!(\"BRAIN_{NAME}_KEY\") <- crate::secrets::resolve(\"case_status\") at src/workflow/case_status.rs:97"
  "BRAIN_CASE_STATUS_KEY_FILE|secrets-ladder derive: src/secrets.rs format!(\"BRAIN_{NAME}_KEY_FILE\") <- the same resolve call"
  "BRAIN_SERVER_AUTH_TOKEN|external consumer: openclaw-host env substitution for the plugin authToken (docs/deployment.md:629); writer src/bin/brain.rs:3918"
)

# Arm 3 — declared non-knobs: names the contract docs themselves state are
# NOT configuration. Each entry NAME|evidence. Not "removed/roadmap" — the
# name was never a knob, so a forward-tracking row would be fiction.
DECLINED_NON_KNOBS=(
  "BRAIN_MODEL_PROFILE|docs/configuration.md:55 declares it not a config key; appears only inside a re-embed hint string (src/server/bootstrap.rs:212)"
)

# Arm 1: the code shape. Implemented only when the NAME sits on an
# env-read/write line. Declared ceilings (both fail toward scrutiny, never
# toward silence): a #[cfg(test)] fixture writing the var still counts, and
# a multi-line env::var( call whose name sits on a later line is missed.
implemented() {
  local name="$1" entry
  if rg -q --no-messages -e "env::(var|var_os|set_var|remove_var)\([^)]*${name}" \
      "$REPO/src" "$REPO/crates" "$REPO/client" "$REPO/tools" 2>/dev/null; then
    return 0
  fi
  for entry in "${PINNED_CALLSITES[@]}"; do
    if [[ "${entry%%|*}" == "$name" ]]; then
      echo "note: '$name' implemented via pinned call-site: ${entry#*|}" >&2
      return 0
    fi
  done
  for entry in "${DECLINED_NON_KNOBS[@]}"; do
    if [[ "${entry%%|*}" == "$name" ]]; then
      echo "note: '$name' declared non-knob: ${entry#*|}" >&2
      return 0
    fi
  done
  return 1
}

selfcheck() {
  # deliberate global: the EXIT trap fires after the function's scope ends
  root="$(mktemp -d "${TMPDIR:-/tmp}/env-truth-selfcheck.XXXXXX")"
  trap 'rm -rf "$root"' EXIT

  # Clean fixture: shape hit + pinned inventory + declared non-knob + rule-2
  # qualified/roadmap-owned knob + live tier export → must PASS.
  mkdir -p "$root/clean/src" "$root/clean/docs" "$root/clean/deploy/tiers"
  printf 'fn main() { let t = std::env::var("BRAIN_FIXTURE_LIVE"); }\n' > "$root/clean/src/live.rs"
  printf 'BRAIN_FIXTURE_LIVE=1\n' > "$root/clean/deploy/tiers/t1.env"
  printf '%s\n' \
    '`BRAIN_FIXTURE_LIVE` — live knob.' \
    '`BRAIN_FIXTURE_ROADMAP` — planned for the Loop line.' \
    '`BRAIN_SERVER_AUTH_TOKEN` — external consumer.' \
    '`BRAIN_MODEL_PROFILE` — inventory-managed.' \
    > "$root/clean/docs/configuration.md"
  : > "$root/clean/docs/deployment.md"
  printf '%s\n' "- \`BRAIN_FIXTURE_ROADMAP\` — Loop line tracks this planned knob" > "$root/clean/ROADMAP.md"
  if BRAIN_ENV_TRUTH_REPO="$root/clean" "$0" >/dev/null 2>"$root/clean.err"; then
    echo "selfcheck: clean fixture passes (shape + inventory + non-knob + rule-2 + tier)"
  else
    echo "selfcheck: FAIL — clean fixture rejected:" >&2
    cat "$root/clean.err" >&2
    return 1
  fi

  # Hostile fixture — the S7-05 red proof, permanent: a knob documented as
  # live whose only in-scope occurrence is a COMMENT must FAIL (the old
  # substring matcher passed exactly this tree), and an unimplemented tier
  # export must be named.
  mkdir -p "$root/hostile/src" "$root/hostile/docs" "$root/hostile/deploy/tiers"
  printf '// BRAIN_FIXTURE_COMMENT is mentioned only in this comment.\n' > "$root/hostile/src/comment_only.rs"
  printf 'fn main() { let t = std::env::var("BRAIN_FIXTURE_LIVE"); }\n' > "$root/hostile/src/live.rs"
  printf 'BRAIN_FIXTURE_TIERMISS=1\nBRAIN_FIXTURE_LIVE=1\n' > "$root/hostile/deploy/tiers/t1.env"
  printf '%s\n' \
    'comment-only: `BRAIN_FIXTURE_COMMENT`' \
    'tier: `BRAIN_FIXTURE_TIERMISS`' \
    > "$root/hostile/docs/configuration.md"
  : > "$root/hostile/docs/deployment.md"
  : > "$root/hostile/ROADMAP.md"
  if BRAIN_ENV_TRUTH_REPO="$root/hostile" "$0" >/dev/null 2>"$root/hostile.err"; then
    echo "selfcheck: FAIL — hostile fixture passed (a comment counted as implemented)" >&2
    return 1
  fi
  grep -q "BRAIN_FIXTURE_COMMENT" "$root/hostile.err" || {
    echo "selfcheck: FAIL — comment-only var not named in the refusal" >&2
    return 1
  }
  grep -q "BRAIN_FIXTURE_TIERMISS" "$root/hostile.err" || {
    echo "selfcheck: FAIL — unimplemented tier export not named in the refusal" >&2
    return 1
  }
  echo "selfcheck: hostile fixture rejected (comment-only + tier-miss both named)"
  echo "selfcheck: OK"
}

if [[ "${1:-}" == "--selfcheck" ]]; then selfcheck; exit $?; fi

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
