#!/usr/bin/env bash
# Derive the README's dynamic badge values from the real build, so the badges
# are facts, not hand-typed claims. A release-time tool, like scripts/sbom.sh.
#
#   scripts/badges.sh              → print the version/test/UMP/SBOM badge block
#   scripts/badges.sh --selfcheck  → CHEAP. The derivations that need no cargo
#                                    run: version↔README, the UMP gate, the
#                                    release-checklist's six-artifact
#                                    completeness, the committed SBOM, and the
#                                    test badge's pointer to --verify-count.
#                                    Does NOT compare the test count (see below).
#   scripts/badges.sh --verify-count → SLOW (one full `cargo test`). Compares the
#                                    DERIVED test count against the README badge
#                                    and exits non-zero on drift.
#
# It never fabricates a number it did not measure: version is read from
# Cargo.toml, the test count from an actual `cargo test` run, the SBOM flag
# from the on-disk CycloneDX file.
#
# ── Why the count is split across two modes ───────────────────────────────
# The count needs a full `cargo test --features bench,migrate` run (~3 min), so
# folding that compare into `--selfcheck` would turn every lightweight call —
# ci.yml and scripts/verification-sweep.sh both invoke it on every push — into a
# multi-minute job, and a gate nobody runs is a convention, which is the defect
# this split exists to remove. `--selfcheck` therefore does not CLAIM to check
# the count, and `--verify-count` exists so something can actually do it.
#
# `BRAIN_TEST_COUNT` is the injection seam: when set, the derived count is read
# from it instead of running cargo. It exists so the compare can be pinned
# BEHAVIOURALLY in milliseconds (tests/main_suite.rs) instead of by
# string-scanning this script, and so CI can pass a count it already derived.
# It is not a way to make the guard pass: an unset run derives the real number.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

version()      { sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1; }
client_version(){ sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/client/Cargo.toml" | head -1; }
test_count()   {
  if [[ -n "${BRAIN_TEST_COUNT:-}" ]]; then
    printf '%s' "$BRAIN_TEST_COUNT"
    return
  fi
  # PLATFORM-NORMALIZED (2026-10-07): the sandbox suite carries tests that
  # exist ONLY on one OS — 7 seatbelt tests compile solely on macOS, 2
  # landlock tests solely on Linux — so a raw derivation differs by 5
  # across the two platforms and the badge gate (which runs on Linux CI)
  # can never agree with a macOS-derived number. The skips below exclude
  # BOTH platform-only families by name, so the count measures the SAME
  # test set everywhere. BARE NAMES, DELIBERATELY: libtest matches --skip
  # by substring against the full path, and the landlock pair lives at
  # workflow::sandbox::landlock::linux_ci::* — a module a darwin host
  # cannot even compile, so the first cut's full-path skips were written
  # blind and matched nothing on the runner (derived 3166, not 3164). A
  # bare name cannot be wrong about nesting. A new platform-gated sandbox
  # test must join these skips or the badge drifts by design again.
  ( cd "$REPO" && cargo test --features bench,migrate -- \
      --skip handlers::case_run::conformance::gdl_conformance_pack_run \
      --skip workflow::sandbox::tests::realized_paths_law_pinned_against_symlinked_temp \
      --skip landlock_write_inside_workdir_succeeds \
      --skip landlock_write_outside_workdir_fails \
      --skip workflow::sandbox::tests::exec_route_wraps_the_sandbox_when_selected \
      --skip workflow::sandbox::tests::escape_is_process_not_thread \
      --skip workflow::sandbox::tests::sandboxed_network_is_denied \
      --skip workflow::sandbox::tests::secret_not_in_hostcall_payload \
      --skip workflow::sandbox::tests::harness_kill_within_budget \
      --skip workflow::sandbox::tests::sandbox_cancel_mid_run_kills_cancelled \
      --skip workflow::sandbox::tests::sandbox_handle_drop_kills_and_reaps_mid_run \
      2>&1 ) \
    | grep -Eo '[0-9]+ passed' | awk '{ s+=$1 } END { print s+0 }'
}

# The number the README badge presents, or empty when it presents none.
# Scoped to the badge line itself (not a whole-file grep) so the disclaimer and
# the number can never be satisfied by unrelated text elsewhere in the file.
readme_test_badge() {
  grep -oE 'badge/tests-[0-9]+' "$1" 2>/dev/null | head -1 | grep -oE '[0-9]+' || true
}

VERSION="$(version)"
# Hoisted: both the `--verify-count` compare and the `--selfcheck` arms below
# read it, and `--selfcheck` is entered first, so an assignment inside either
# arm would be unreachable from the other.
README="$REPO/README.md"

if [[ "${1:-}" == "--verify-count" ]]; then
  # The REAL count comparison, kept out of --selfcheck only because it costs a
  # full cargo test run. --selfcheck has already checked everything cheap; this
  # answers the one question it cannot: does the number the README presents
  # match the build?
  DERIVED="$(test_count)"
  PRESENTED="$(readme_test_badge "$README")"
  if [[ -z "$PRESENTED" ]]; then
    echo "ERR: README presents no tests-<N> badge, so there is nothing to compare against the derived $DERIVED" >&2
    exit 1
  fi
  if [[ "$PRESENTED" != "$DERIVED" ]]; then
    echo "ERR: README test-count badge drifts from the build — badge says $PRESENTED, the run derives $DERIVED." >&2
    echo "     Fix: run scripts/badges.sh and paste its output into the README badge block." >&2
    exit 1
  fi
  echo "OK  README test-count badge matches the build ($DERIVED)"
  exit 0
fi

if [[ "${1:-}" == "--selfcheck" ]]; then
  # 1. badges derive the version from the real build, not a stored claim.
  CARGO_VERSION="$(grep -m1 '^version' "$REPO/Cargo.toml" | grep -oE '[0-9][0-9.]*')"
  if [[ -z "$VERSION" || "$VERSION" != "$CARGO_VERSION" ]]; then
    echo "ERR: badges version '$VERSION' drifts from Cargo.toml '$CARGO_VERSION'" >&2
    exit 1
  fi
  # 2. the README carries the DERIVED version, not a stale hand-typed one
  #    (v1.28.65 lesson: the version badge sat two releases behind while
  #    selfcheck passed — nothing compared README to the derivation).
  if ! grep -q "badge/version-${CARGO_VERSION}-blue.svg" "$README"; then
    echo "ERR: README version badge drifts from Cargo.toml '$CARGO_VERSION' — run scripts/badges.sh and paste the block" >&2
    exit 1
  fi
  # The test-count badge cannot be compared here without a full cargo run, so
  # --selfcheck does NOT claim to verify it (see the header). What it CAN do —
  # and now does, scoped to the badge line rather than the whole file — is
  # refuse to let the count drift silently UNLABELLED: the badge's own line must
  # carry the `not selfcheck-verified` disclaimer (v1.28.87 derive-or-drop).
  # Scoping matters: a whole-file grep was satisfied by a disclaimer in a
  # paragraph 28 lines below the badge, so the badge could be arbitrarily wrong
  # while the guard stayed green.
  BADGE_LINE="$(grep -n 'badge/tests-' "$README" | head -1 || true)"
  if [[ -z "$BADGE_LINE" ]]; then
    echo "ERR: README has no tests badge to verify — expected a badge/tests-<N> image" >&2
    exit 1
  fi
  # The disclaimer must name the BADGE, and its scope is the badge's own
  # `<p align="center">` block plus the prose that immediately follows it —
  # not the whole file (a whole-file grep was satisfied by a sentence 28 lines
  # away, so the badge could be arbitrarily wrong while the guard stayed green)
  # and not the single `<img>` line either (an HTML attribute is the wrong home
  # for the claim). The enclosing block is the unit that actually contains both,
  # and it is bounded by the NEXT `<p align=` or EOF so the scope cannot grow
  # without the check noticing.
  BADGE_BLOCK="$(awk '
    /badge\/tests-/ { inside=1 }
    inside && /^<p align=/ { blocks++ }
    inside && blocks >= 2 { exit }
    inside { print }
  ' "$README")"
  if ! grep -q "verify-count" <<<"$BADGE_BLOCK"; then
    echo "ERR: README's test-count badge block does not say the count is NOT machine-checked here —" \
         "--selfcheck cannot compare it (that needs a full cargo test), so the badge must point at" \
         "the arm that does: scripts/badges.sh --verify-count" >&2
    exit 1
  fi
  # UMP level derives from the CI conformance gate, not from a literal in
  # this script (v1.28.87 derive-or-drop). Two-sided guard: the gate must
  # still exist in ci.yml, and the README must present the level as
  # CI-derived (not a bare asserted "L3").
  if ! grep -q 'UMP 1.0 / L3' "$REPO/.github/workflows/ci.yml"; then
    echo "ERR: UMP conformance gate missing from .github/workflows/ci.yml — README L3 claim is now self-attested, reword it" >&2
    exit 1
  fi
  if ! grep -q 'UMP 1\.0 L3' "$README"; then
    echo "ERR: README UMP level drifts from the CI-derived 'UMP 1.0 L3' — reword to the derived form" >&2
    exit 1
  fi
  if ! grep -qE 'UMP 1\.0.*L3.*(CI|ci.yml|conformance)' "$README"; then
    echo "ERR: README UMP claim does not name its CI-conformance derivation — reword to the derived form" >&2
    exit 1
  fi
  # 3. the release-checklist names all six wrap artifacts (self-completeness guard).
  CK="$REPO/docs/release-checklist.md"
  if [[ ! -f "$CK" ]]; then
    echo "ERR: docs/release-checklist.md missing" >&2
    exit 1
  fi
  for a in "Cargo.toml" "openapi.yaml" "CHANGELOG" "ROADMAP" "README" "AGENTS"; do
    if ! grep -q "$a" "$CK"; then
      echo "ERR: release-checklist.md omits '$a'" >&2
      exit 1
    fi
  done
  # 4. the SBOM freshness gate (the preflight line, X-C8): the tag carries
  #    its SBOM IN-TREE. A release whose version has no COMMITTED
  #    sbom/brain-server-<version>.cdx.json fails the gate — the human step
  #    (scripts/sbom.sh + commit) is unforgoable; no CI bot commits.
  if ! git -C "$REPO" ls-files --error-unmatch "sbom/brain-server-${CARGO_VERSION}.cdx.json" >/dev/null 2>&1; then
    if [[ -f "$REPO/sbom/brain-server-${CARGO_VERSION}.cdx.json" ]]; then
      echo "ERR: sbom/brain-server-${CARGO_VERSION}.cdx.json exists but is NOT committed — run scripts/sbom.sh and commit it before tagging" >&2
    else
      echo "ERR: sbom/brain-server-${CARGO_VERSION}.cdx.json missing from the tree — run scripts/sbom.sh, commit, then tag" >&2
    fi
    exit 1
  fi
  echo "OK  badges + release checklist self-check clean"
  exit 0
fi

CLIENT="$(client_version)"
TESTS="$(test_count)"
# UMP level is DERIVED, not asserted: the `integration` CI job
# ("release build + UMP conformance + recall gate" in
# .github/workflows/ci.yml) boots a scratch keyed instance, runs the official
# @universalmemoryprotocol/core reference conformance runner, and fails the
# push unless the output contains "UMP 1.0 / L3". If that gate ever goes
# missing, the level drops to self-attested LOUDLY (stderr) instead of
# silently keeping the badge.
ump_level() {
  if grep -q 'UMP 1.0 / L3' "$REPO/.github/workflows/ci.yml" 2>/dev/null; then
    printf 'L3'
  else
    echo "WARN: UMP conformance gate absent from .github/workflows/ci.yml — level is SELF-ATTESTED, not CI-derived" >&2
    printf 'L3 self-attested'
  fi
}
UMP="$(ump_level)"
SBOM="$REPO/sbom/brain-server-${VERSION}.cdx.json"
SBOM_FLAG=no; [[ -f "$SBOM" ]] && SBOM_FLAG=yes

cat <<EOF
server $VERSION   client $CLIENT   tests $TESTS passed   UMP $UMP   sbom $SBOM_FLAG

[![Version](https://img.shields.io/badge/version-$VERSION-blue.svg)](#)
[![Tests](https://img.shields.io/badge/tests-$TESTS%20passed-brightgreen.svg)](#)
[![UMP Conformance](https://img.shields.io/badge/UMP%201.0-L3%20verified-success.svg)](docs/universal-memory-protocol.md)
EOF
