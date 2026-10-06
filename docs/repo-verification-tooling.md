# Repo Verification Tooling — the gates with no other doc home

The scripts below enforce repo hygiene but are documented nowhere else.
[`scripts/env-truth.sh`](../scripts/env-truth.sh) (the env-var truth gate) is the
sibling reference: it is already listed in the [Scripts appendix](release-checklist.md#scripts-appendix).
This page gives each unlisted gate the same treatment: what it checks, when it
runs, the exact invocation, how to read a failure, and its honest ceiling.

Related doors: [release-checklist.md](release-checklist.md) (the six artifacts +
the gates that must stay green) and [CONTRIBUTING](CONTRIBUTING.md) (the
`fmt` / `clippy` / `test` quality gates every PR must pass). The local
[pre-push hook](../.git/hooks/pre-push) enforces CHANGELOG release notes +
`cargo fmt --check` + `lipstyk-gate.sh --hook`.

## `scripts/docs-truth.sh` (+ `scripts/docs-truth.py`)

What it checks: three-way doc truth — SOURCE (`src/server/router/*.rs`
`.route("…", method(` registrations) vs CONTRACT (`openapi.yaml` paths) vs
DOCS (`docs/api.md` coverage), plus a sweep of living `docs/*.md` for
`` `path/to/src/*.rs:NN` `` citations that resolve to no file on disk.
The `.sh` is a thin wrapper: `exec python3 "$(dirname "$0")/docs-truth.py" "$@"`.

When it runs: CI (`ci.yml` "docs-truth + env-truth gates" step runs
`bash scripts/docs-truth.sh` with no flags) and inside
`scripts/verification-sweep.sh`. Otherwise manual.

Exact invocations (repo root):

```sh
scripts/docs-truth.sh            # the check
scripts/docs-truth.sh --verbose  # adds one INFO row (routes/openapi/docs counts)
```

Interpreting failures: exit is non-zero **only on HIGH** — a route registered
but absent from the contract, a documented path that is NOT registered, a
method mismatch (`registered […] but openapi declares …`), or a registered
route absent from `api.md`. `MED` (dangling `rs:NN` citation) and `LOW`
(asset / `/private` / `/` / the kept `/webhooks/gh` alias) print but do not
fail. Output rows are `[SEV ] <file>` + a one-line mechanical finding.

Honest limits: the census is regex-shaped (route-macro shape, `openapi.yaml`
path-line shape), not a type-checked contract; `api.md` uses a
sibling-segment heuristic after a middle dot, so odd formatting can mislead it;
sealed history (`CHANGELOG.md`, `*_AUDIT_*.md`, `*_PROOF_*.md`, `AUDIT.md`,
`AGENTS_HISTORY.md`, `roadmap-and-release-history.md`,
`LOOP_AUTOCLOSE_RECONCILIATION.md`, `MEMGHOST_MITIGATION.md`,
`dioxus-wasm-split-research.md`) is skipped by design — stale claims there are
history, not lies. `MED` never fails the gate; a dangling citation outside a
HIGH diff still needs a human.

## `scripts/check-doc-links.py`

What it checks: every relative markdown link under `docs/` resolved against
the filesystem. Only `](…​.md)` targets are checked; anchors are stripped and
bare URLs skipped.

When it runs: manual, from the repo root, and as cited evidence in round /
audit notes. No CI step invokes it (checked `ci.yml`).

Exact invocation:

```sh
python3 scripts/check-doc-links.py
```

Interpreting failures: prints `checked N relative .md links under docs/`; on
breakage prints `BROKEN (M):` with `file: target` rows and exits 1. `all
resolve` means exactly that — nothing more.

Honest limits: markdown-link syntax **only** — a bare backtick path in a table
cell is invisible to it (the AUDIT R8-02 dead reference proved this); scope is
`docs/` alone, so root-level `*.md` links are out of scope; it verifies the
target file exists, not that a `#anchor` inside it does.

## `scripts/lipstyk-gate.sh`

What it checks: the lipstyk diff-watchdog locally — changed Rust/TypeScript
lines under `src client plugin crates` vs a base that cannot move. Fails
closed on the two modes that make a naive local run lie: a moving base
(post-push `origin/main == HEAD` ⇒ empty diff ⇒ vacuous pass) and invisible
new files (untracked files appear in no `git diff`, closed via
`git add -N` intent-to-add, content unstaged and reversible with `git reset`).

When it runs: the [pre-push hook](../.git/hooks/pre-push) runs
`scripts/lipstyk-gate.sh --hook`; CI runs the equivalent diff-watchdog job
(the scan list is pinned against this script by
`lipstyk_gate_scan_paths_match_the_ci_watchdog`, so the two cannot drift);
`scripts/verification-sweep.sh` runs it bare. Otherwise manual.

Exact invocations:

```sh
scripts/lipstyk-gate.sh                # base = upstream merge-base, else HEAD~1
scripts/lipstyk-gate.sh <base>         # explicit base: HEAD~N, old remote tip, v<last-release-tag>
scripts/lipstyk-gate.sh --hook         # pre-push mode (see below)
```

Interpreting failures: prints `base=<base> changed: <files>` then execs
`lipstyk --diff <base> --exclude-tests <scan paths>` — real findings block the
push (hook prints `pre-push: lipstyk-gate failed`). An empty changed-line set
is a hard failure (`REFUSING to pass vacuously`), except in `--hook` mode,
where nothing-to-lint passes with a note (a docs-only push is an honest pass,
not a lie). A missing lipstyk binary passes with a note in `--hook` mode (CI
is the canonical backstop) and fails hard otherwise.

Honest limits: `fuzz/` and the three `tools/*` workspace nodes are unscanned
by this script's scope, stated in its header — not silently covered. After a
multi-commit push, `HEAD~1` recovery diffs only one commit: pass the old
remote tip or the last release tag. In `--hook` mode a tool-less machine can
push past the watchdog; CI still enforces.

## `scripts/aqueduct-eval.sh`

What it checks: the recall-quality floor on a frozen 25-doc corpus (general +
migration-vertical docs 10–14 + legal-vertical 15–19 + troubleshoot-vertical
20–24): seed a scratch instance, `ingest-dir` the corpus, then
`brain eval --floor r5=0.85 --floor r10=0.85 --floor mrr=0.85`. Mirrors CI's
recall-eval lane (same `ingest-dir` + same floors in `ci.yml`).

When it runs: manual local gate. Nothing calls it automatically.

Exact invocation:

```sh
scripts/aqueduct-eval.sh <port>   # port defaults to 18484 when omitted
```

Prerequisites read from the script: release binaries at
`target/release/brain-server` and `target/release/brain`, `curl`, a free port.
It writes the scratch dir path to `/tmp/aqueduct-eval-scratch` and the server
PID to `/tmp/aqueduct-eval-pid`, waits up to 60 s on `/health`, kills the
server on the way out, and exits with the eval's status (`tail -6` of eval
output is shown).

Interpreting failures: `seed ingest failed (expected '25 ingested')` means the
corpus did not land (server/log in the scratch dir is the next read); a
non-zero eval exit means a floor (`r5` / `r10` / `mrr` < 0.85) was missed.

Honest limits: release binary only (no debug fallback); fixed corpus and
fixed floors — it proves the frozen 25, not the live workspace; scratch lives
in `/tmp` and the server log stays there, not in `target/`.

## `scripts/aqueduct-smoke.sh`

What it checks: end-to-end recall legs against a scratch **copy** of the live
workspace DB (copied via `sqlite3 … ".backup …"` — the live DB is never
touched): multi-db domain create, screened benign ingest, dedup receipt + id
match, quarantined scrape ingest (stored + `flagged=1`), second-domain
ingest, cross-domain recall with provenance, hash-only trace replay, and
`/audit/verify` over every chain.

When it runs: manual local smoke. Nothing calls it automatically.

Exact invocation:

```sh
scripts/aqueduct-smoke.sh [port]   # port defaults to 18485
```

Environment (set by the script): `BRAIN_MULTI_DB=1`,
`BRAIN_AUDIT_READ_EVENTS=true`, `BRAIN_DB_PATH=<scratch>/brain.db`; server PID
in `/tmp/aqueduct-smoke-pid` with an EXIT trap kill; scratch path printed and
kept (`SMOKE COMPLETE (scratch kept at …)`).

Interpreting failures: `set -e` plus `curl -fsS`, so the first failed leg
aborts the run — read the last `ok` line to see how far it got
(`health ok` → `domain db file ok` → `screened ingest ok` → `dedup receipt ok`
→ `dedup id match ok` → `quarantine flag ok` → `recall federation ok` →
`trace replay ok (hash-only)` → `audit verify ok`). The trace leg asserts the
raw query text appears nowhere in the trace JSON (hash-only or fail).

Honest limits: source DB path is operator-machine fixed
(`~/.openclaw/workspace/brain.db`) and `sqlite3` CLI is required; release
binary only; the multi-db and audit-read-events env are drill scaffolding, not
production defaults.

## `scripts/verification-sweep.sh`

What it checks: everything, sequentially — the lanes that never ran elsewhere.
In order: `cargo test --all-targets`; `clippy --all-targets --features otel`;
per-feature clippy lanes (`loom rerank-tier neural-embed
injection-classifier compliance-pack multivec`); `cargo test` for
`crates/` and `tools/steward-harness`; `cargo audit` over **every on-disk**
`Cargo.lock` (a `find`, not the root lockfile alone — the RUSTSEC-2026-0285
`tools/*` lesson); lock **freshness** via full-form
`cargo metadata --locked` over every **tracked** lockfile (the `--no-deps`
form passes vacuously on exactly the stale locks this lane exists to catch);
then `docs-truth`, `env-truth --selfcheck`, `badges --selfcheck`, and
`lipstyk-gate.sh` bare. Sequential on purpose (parallel cargo serialises on
one target-dir lock anyway).

When it runs: manual (round §0 sweep; transcript consumer:
`docs/R65C_DEFERRAL_EVIDENCE_2026-10-01.md`). Not a CI job — it aggregates
local equivalents of CI lanes.

Exact invocation (no flags):

```sh
bash scripts/verification-sweep.sh
```

Transcript: `target/r65-verify.log` (`### <lane>` + `PASS`/`FAIL` rows).

Interpreting failures: **read the tail, not the exit code** — the script
propagates via the `SWEEP_EXIT=0|1` line in the log and on stdout; a `FAIL
<lane>` row names the lane and the log above it holds the tool output.
`RUSTFLAGS="-D warnings"` is exported, so warnings fail clippy lanes here
even if they pass under a bare local invocation.

Honest limits: slow by construction (full test + per-feature clippy +
`--verify`-class lanes, one lane at a time); the audit lane scans on-disk
lockfiles including the gitignored `fuzz/Cargo.lock`, so it covers one more
than CI — coverage errs high; the freshness lane covers tracked lockfiles only
(`git ls-files`); the final `lipstyk` lane needs the binary on PATH (unlike
`--hook` mode it does not soft-pass a missing tool).

## `scripts/clean-cycle-drill.sh`

What it checks: the clean power-cycle (E1 drill): fingerprint the store with
`brain anchor`, SIGTERM graceful stop with measured drain time, prove cold
(nothing on the port), cold start with measured boot-to-serving time, and
require the anchor fingerprint **byte-identical** across the cycle, then
post-cycle `/audit/verify` + a recall probe.

When it runs: manual, on the MiniPC host (paths are host-fixed:
`/home/mark/brain-demo`, release binary under
`/home/mark/brain-server/target/release/`, port `8766`, tmux session
`braindemo-run`). Takes no arguments.

Exact invocation (on that host):

```sh
bash scripts/clean-cycle-drill.sh
```

Log: `/tmp/clean-cycle-drill-<UTC-stamp>.log` (tee'd live).

Interpreting failures: `THE VERDICT` prints `PASS — the fingerprint is
BYTE-IDENTICAL across the cycle` or `FAIL — the fingerprint MOVED:` with the
diff (before/after anchors in `/tmp/r48-anchor-before.txt` /
`/tmp/r48-anchor-after.txt`). `MISSING <binary>` at step 0 means the release
binary was never deployed; a hang at step 3/5 points at drain or boot, with
the measured ms printed next to it.

Honest limits: "read-only against the live install" means the drill serves
its own instance on 8766 with its own data dir — but on that host it is NOT
side-effect-free: it SIGTERMs the R48 unit process and kills/recreates the
`braindemo-run` tmux session. Seed and probes use the `/ingest/memory` seat
only; other ingest seats are not exercised.

## Honest ceilings (whole page)

- These gates are redundancy for human process, not proofs: `docs-truth`
  fails only on HIGH, `check-doc-links.py` sees only `](…​.md)` syntax, the
  lipstyk hook soft-passes a missing binary, `aqueduct-eval` proves a frozen
  corpus, the smoke proves a DB copy, the sweep reports via a log line rather
  than its exit code, and the drill moves processes on its host.
- Where a gate is weaker than CI (hook missing-binary pass, sweep's extra
  gitignored lockfile, `env-truth` bare-run vs `--selfcheck` — see the
  ceiling noted in `ci.yml`'s docs-truth step), the stronger door is named
  above; do not present the weaker as the proof.
- Anything not read from a script header or the cited CI/hook wiring is
  deliberately absent. If a flag or behavior is missing here, the script — not
  this page — is the source of truth.

Excluded by scope (one line): one-off / non-gate helpers
`commit-loose-changes.sh`, `rename-round-test-files.sh`, `repo-brief.sh`, and
`build-desktop.sh` are intentionally not covered here.
