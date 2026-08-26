# Release Checklist — the six-part wrap

Every release touches the same six artifacts. The ordering below keeps them
consistent so the tag, the docs, and the badges never disagree. This is the
**documented path**; it does not replace operator judgement — a docs-only
release (e.g. v1.20.5) intentionally skips step 1 (no `Cargo.toml` bump) and
steps 2 (no OpenAPI change).

| # | Artifact | What changes | Verify |
|---|----------|--------------|--------|
| 1 | `Cargo.toml` (+ `Cargo.lock`) | `version = "x.y.z"` bump for the released component (server or client). | `grep '^version' Cargo.toml` |
| 2 | `openapi.yaml` | `version` + `x-api-version` stamps (server releases only; skip if the server version didn't move). | `grep -n 'x-api-version' openapi.yaml` |
| 3 | `CHANGELOG.md` | `## [x.y.z]` entry describing the release, honest ceilings included. | `grep "^## \[x.y.z\]" CHANGELOG.md` |
| 4 | `ROADMAP.md` | released-version header + the shipped row marked Shipped/Released. | `grep -n "Released version" ROADMAP.md` |
| 5 | `README` badges | version + test-count badges regenerated from the real build. | `scripts/badges.sh` |
| 6 | `AGENTS.md` | header version note + the Agent entry recording the session. | read the entry you added |

## The gates that must stay green

Run these before tagging — the tree is only "released" when every one passes:

```sh
cargo test --features bench,migrate      # the real test count badges.sh reports
cargo clippy --all-targets --features bench,migrate -- -D warnings
cargo fmt --check
scripts/badges.sh --selfcheck            # version + checklist completeness guards
```

After the push, `scripts/release.sh` BLOCKS until the CI run for the exact
tagged commit is green (fail-closed — the tag itself re-runs no tests, so
that wait is the only automated bridge between "pushed main" and
"shipped binaries"). These local gates are the pre-push redundancy, not a
substitute for the wait.

## Badges are facts, not hand-typed claims

`scripts/badges.sh` derives the version from `Cargo.toml` and the test count
from an actual `cargo test` run, so the README badge can never drift from the
build (the 665-vs-659 drift this release fixed). Paste its output into the
README badge block; `--selfcheck` guards the derivations + this checklist's
own completeness.
