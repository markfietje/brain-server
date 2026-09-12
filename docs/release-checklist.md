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

> **T5-01 law (2026-09-12): the gate is the FULL `cargo test` invocation —
> sliced runs (`--lib`, `--test main_suite`, name filters) are diagnostic
> ONLY and never count as green.** A sliced "green" certified a red tree
> once: the v1.28.82 closure record listed lib + main_suite green while
> `authz_matrix` (the binary that owns the kill-switch contract) was 7/22
> red, and main was unreleasable. Every test binary ships a contract —
> `authz_matrix` (kill-switch/authz), `main_suite` (seams), plus the lib
> units — and `cargo test` with no `--test`/`--lib` selector is the only
> invocation that runs all of them. If time forces a slice during
> development, the release entry must still record the full run.

The local gate above is not the whole CI matrix (the v1.28.29 and v1.28.31
lessons). Before every main push, also run the CI dry-run from `AGENTS.md`:
default-feature clippy/test, the crates + steward-harness + otel jobs, the
`lipstyk --diff "$(git rev-parse origin/main)" --exclude-tests src client plugin`
changed-line gate, and `cargo fmt --manifest-path client/Cargo.toml -- --check`.

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

## Honest scope: SBOM + OpenAPI + well-known (v1.28.87 docs-truth)

### SBOM scope (what the committed file does and does NOT cover)

`sbom/brain-server-<version>.cdx.json` (1.28.83: **375 components** vs
**520** `Cargo.lock` packages) covers the shipped runtime closure as
emitted by `cargo-cyclonedx`. The ~145-package gap is dev-dependencies +
build-transitive crates that never ship in the release binary — excluded by
the generator's default scope, not by hand-editing. Per the CISA 2026
Minimum Elements for SBOM (published 29 Jul 2026, supersedes the NTIA 2021
baseline): this file satisfies the minimum-elements shape for the RUNTIME
surface; it is NOT a whole-tree (dev + build) inventory, and the release
notes MUST NOT claim it is. If a consumer needs the dev/build-transitive
closure, regenerate with the dev-inclusive flag and commit it as a
separate `-dev.cdx.json` — never silently widen the release file.

### OpenAPI intentional exclusions (in the router, NOT in `openapi.yaml`)

8 production registrations are deliberately absent from the contract —
static seats and redirects, no auth/token surface, so excluding them keeps
the API contract honest:

| Path | Source | Why excluded |
|---|---|---|
| `/` | `src/server/router/core.rs` (301 → `/app/`) | redirect, not an API |
| `/app/` + `/app/{*path}` | `core.rs:35-36` (SPA index + static) | static bundle seat |
| `/app/boot.json` | `core.rs:37` | static boot manifest |
| `/app/boot.js` | `core.rs:38` | static boot script |
| `/app/boot.pub` | `core.rs:39` | static boot public key |
| `/app/sw.js` | `core.rs:40` | static service worker |
| `/app/sw-register.js` | `core.rs:42-45` | static SW registration |

Correction to the plan's "9": `/private` and `/webhooks/gh` appear ONLY
in auth-middleware unit tests (`src/server/router/auth.rs:599-600,689,802`
`stub` apps) — they are NOT production routes, so they are not
router-only exclusions. Counted production set: 8.

### Well-known wiring table (each route confirmed individually)

| Route | Router registration | Handler |
|---|---|---|
| `/.well-known/openid-configuration` | `src/server/router/auth.rs:518` | `src/handlers/well_known.rs:22` |
| `/.well-known/jwks.json` | `auth.rs:521` | `well_known.rs:28` |
| `/.well-known/security.txt` | `auth.rs:523` | `well_known.rs:44` |
| `/.well-known/ai-notice` | `auth.rs:527` | `well_known.rs:74` |
| `/.well-known/ai-literacy` | `auth.rs:531` | `well_known.rs:91` |
| `/.well-known/cop-notice` | `auth.rs:535` | `well_known.rs:109` |
| `/.well-known/ump.json` | `src/server/router/ump.rs:27` | `src/handlers/ump_ops.rs:1` (`capabilities`) |

All 7 are also public-path listed (`route_guards.rs:19-40` `PUBLIC_PATHS`)
and present in `openapi.yaml` (:2990 ump.json, :6538-:6651 the six).
Standing rule: a new well-known route MUST land in all three places
(router + `PUBLIC_PATHS` + openapi) or fail review.

## Scripts appendix

| Script | Purpose | Documented |
|---|---|---|
| `install-service.sh` | Build + install binaries, launchd plist, strips macOS provenance xattr. | deployment.md / AGENTS.md |
| `release.sh` | Tag + publish; blocks on green CI for the tagged SHA. | this page / AGENTS.md |
| `release-sign.sh` | Sign release artifacts (also signs `brain kb build` tarballs). | cli-reference.md (kb) |
| `badges.sh` | Regenerate README badges from the real build; `--selfcheck` drift guard. | this page |
| `env-truth.sh` | Docs-vs-code env-var truth gate (tiers live, docs qualified + Loop-tracked). | this page |
| `sbom.sh` | SBOM generation for CRA/security docs. | cra.md |
| `cra-kit.sh` | CRA evidentiary kit generator. | cra.md |
| `admt-kit.sh` | ADMT transparency kit generator. | admt.md |
| `gen-model-manifest.sh` | Emit a `BRAIN_MODEL_MANIFEST` file for local model artifacts (fail-closed boot pin). | configuration.md |
| `sync-plugin.sh` | Rsync `plugin/` into the openclaw workspace's deployed extension (parity discipline). | plugin/README.md |
| `publish-wiki.sh` | Publish the `wiki/` directory to the GitHub wiki. | here only |
