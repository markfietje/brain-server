#!/usr/bin/env bash
# Commit the two loose changes from the R48/R50 planning session.
# SAFE BY CONSTRUCTION: it shows you what it is about to do, runs the test
# suite before touching the kernel, and refuses to commit if the suite is red.
#
#   bash commit-loose-changes.sh
#
# Two repos:
#   kernel  brain-server  — the openapi.yaml duplicate-key fix
#   spine   brain-steward-ip — the R50 plan amendment, the R49 skip record,
#                               and the R50 execution prompt
set -uo pipefail

K=/Users/mark/Sites/brain-server
S=/Users/mark/Sites/brain-steward-ip
say() { printf '\n\033[1m== %s\033[0m\n' "$1"; }

say "1. kernel — what is uncommitted?"
git -C "$K" --no-pager diff --stat
git -C "$K" status --short

say "2. kernel — verify the fix is the ORPHAN duplicate key, nothing else"
if git -C "$K" --no-pager diff --name-only | grep -qvx 'openapi.yaml'; then
  echo "ABORT: openapi.yaml is not the only changed file. Review manually:"
  git -C "$K" --no-pager diff --name-only
  exit 1
fi
# the old tree had the key twice in a row; the new one must have it once
DUP=$(grep -c '^  /webhooks/delivery/{kind}:$' "$K/openapi.yaml")
if [ "$DUP" -ne 1 ]; then
  echo "ABORT: expected exactly 1 '/webhooks/delivery/{kind}:' key, found $DUP"
  exit 1
fi
echo "ok: one key, one deletion"

say "3. kernel — the suite, BEFORE committing"
cd "$K" || exit 1
if ! TMPDIR=/tmp cargo test --offline --locked --all-targets 2>&1 | tail -20; then
  echo "ABORT: the suite did not pass; nothing committed."
  exit 1
fi
# cargo test exits non-zero on failure even through a pipe, so re-check explicitly
if ! TMPDIR=/tmp cargo test --offline --locked --all-targets >/dev/null 2>&1; then
  echo "ABORT: suite FAILED; nothing committed."
  exit 1
fi
echo "suite green"

say "4. kernel — commit the openapi fix"
git -C "$K" add openapi.yaml
git -C "$K" -c commit.gpgsign=false commit -q -F - <<'MSG'
fix(openapi): a duplicate map key — two identical path keys, one with no body

/webhooks/delivery/{kind}: appeared TWICE in a row, the first an orphan
with no operation body and the second carrying the real `post`. A strict
YAML linter rejects it, and a naive key count reads 214 where the true
unique count is 213 — the "a number asserted rather than measured" class
this repo keeps catching.

Semantically a no-op: YAML already discarded the orphan, so the parsed
document is unchanged. Removed for the ambiguity, which is real for a
human reader and for any tool that counts keys.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
git -C "$K" --no-pager log -n1 --format='committed: %h %s'

say "5. spine — what is uncommitted?"
git -C "$S" status --short

say "6. spine — commit the R50 amendment, the R49 skip record, the R50 prompt"
git -C "$S" add -A
git -C "$S" -c commit.gpgsign=false commit -q -F - <<'MSG'
plans(R50): the Create loop prompt — and the schema stamp the plan got wrong

R48 landed (4049064). R49 is SKIPPED by operator decision and recorded in
R49_SKIP_RECORD_2026-09-28.md. R50 is next.

THE FINDING THAT MATTERS: the schema stamp is 1.32.19, NOT 1.32.18.

  SCHEMA_VERSION_V1_32_18 EXISTS — it is the v1.32.18 "Releases" stamp
  shipped by R43 (src/storage_layout.rs:230-234)
  LATEST_KNOWN_SCHEMA = SCHEMA_VERSION_V1_32_18  (storage_layout.rs:254)
  latest_stamp_matches_migration (:711-746) asserts the migration's
  schema_version literal EQUALS LATEST_KNOWN_SCHEMA

The plan says "a stamp bump to 1.32.18" in SEVEN places (§0 pre-flight
comment, §0 verified block, §1 produces, §3 migration.rs, §3
storage_layout.rs, §6 rehearsal, §7 commit 2). Following any of them
literally is a duplicate-const compile error; repointing the const while
the migration stamps something else trips the lockstep pin — a release
that REFUSES ITS OWN DATABASE. The plan carries an amendment block
saying so, and the prompt's §2.1 leads with it.

THREE MORE CORRECTIONS, all measured at 4049064:

  * R46 HAS LANDED. The plan's most load-bearing claim — "R46 has not
    landed" — is false, in the PERMISSIVE direction. The hard prerequisite
    is met and the §0 STOP gate is satisfied. An implementer reading the
    stale sentence would conclude the round is blocked.
  * All four floors moved: CRATE_TEST 1_568 -> 2_376, coverage
    167 -> 209, authz 152 -> 194, ROUTER_SITES 199 -> 249 (the plan's
    floor list omits ROUTER_SITES_FLOOR entirely, and R50 adds six
    routes, so it moves too).
  * Line numbers throughout the plan predate R43-R48. Re-locate every
    one — the fourth round running where a printed line number drifted.

NAMED RISK, NOT A BLOCKER: R49's azp gap is live in today's JWT mode
(RFC 9700 §2.1) and R50 raises its stakes without needing it — C4
Promote is the round's human-authorized act, and under an opaque shared
token the audit row's actor is a SHARED PRINCIPAL, not a person. E14
already ships the loop inert with zero production promote callers, so the
round is safe; but the FIRST production promote will be attributed to
whatever auth is configured then. Decided and recorded, not smuggled.

The plan's substance is sound and untouched: five phases, a non-LLM
Sentinel, a database-level write fence, no error-location feedback ever,
and the refusal to ship until a false-promotion rate is measured.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
git -C "$S" --no-pager log -n1 --format='committed: %h %s'

say "7. final state"
git -C "$K" --no-pager log -n1 --format='kernel %h %s'
git -C "$S" --no-pager log -n1 --format='spine  %h %s'
echo "kernel dirty: $(git -C "$K" status --porcelain | wc -l | tr -d ' ')"
echo "spine  dirty: $(git -C "$S" status --porcelain | wc -l | tr -d ' ')"
