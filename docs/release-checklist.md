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
| 4 | `docs/roadmap.md` | the current-status paragraph names the release. (The root-level `ROADMAP.md` was never git-tracked and moved to the private plans archive on 2026-10-04 — the in-repo roadmap is `docs/roadmap.md`.) | `grep -n "the current server line" docs/roadmap.md` |
| 5 | `README` badges | version + test-count badges regenerated from the real build. | `scripts/badges.sh` |
| 6 | `AGENTS.md` | header version note + the Agent entry recording the session. | read the entry you added |

## The gates that must stay green

Run these before tagging — the tree is only "released" when every one passes:

```sh
cargo test --features bench,migrate      # the real test count badges.sh reports
cargo clippy --all-targets --features bench,migrate -- -D warnings
cargo fmt --check
scripts/badges.sh --verify-count         # the REAL test-count comparison
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

After the push, `scripts/release.sh` cuts the tag and pushes it to `public`,
where — per the 2026-10-06 billing law (private-repo Actions disabled, the
free 2,000 min/month gone) — the tag push itself runs the full `ci.yml`
matrix, and `release.yml`'s publication step fail-closes unless that matrix
is green for the exact tagged SHA: red or absent ⇒ binaries build but
nothing publishes. `release.sh` watches the same runs and exits non-zero on
a not-green verdict; the enforcement is the workflow's, not the helper's.
These local gates are the pre-tag discipline — the tag is cut only from a
tree that already passed them. CI-side facts the releaser should know are
current as of 1.29.3: the `audit` job runs the `cargo-audit` binary over
every tracked lockfile (`.github/workflows/ci.yml`), and the conformance
pack follows the two-door rule (explicit `GDL_R10_PACK_DIR` = fail-closed
operator request; plain absence on CI = named skip —
`src/handlers/case_run.rs`).

## Badges are facts, not hand-typed claims

`scripts/badges.sh` derives the version from `Cargo.toml` and the test count
from an actual `cargo test` run, so the README badge can never drift from the
build. Paste its output into the README badge block.

**Two modes, and the difference matters.** `--selfcheck` is the cheap path and
runs on every CI push: it re-derives the version, checks the README against it,
requires the committed SBOM, and requires the test badge's own block to point at
`--verify-count`. It deliberately does **not** compare the test NUMBER — that
needs a full compile, and a gate too slow to run is a convention. `--verify-count`
is the arm that compares, and it costs one full `cargo test --features
bench,migrate` run; CI invokes it in the `lint-test` job for that reason.

This split exists because the count was previously unchecked by anything: the
badge read 3 120 while the build derived 3 156, and every gate stayed green.
That gap is why `--verify-count` exists, not because the count is hard to
derive.

## Honest scope: SBOM + OpenAPI + well-known (v1.28.87 docs-truth)

### SBOM scope (what the committed file does and does NOT cover)

`sbom/brain-server-<version>.cdx.json` (1.29.2: **365 components** vs
**514** `Cargo.lock` packages) covers the shipped runtime closure as
emitted by `cargo-cyclonedx`. **Spec version (v1.28.88):** the file is
CycloneDX **1.5** — the ceiling of cargo-cyclonedx 0.5.9 (latest; it emits
1.3/1.4/1.5 and reads no config file), pinned as `--spec-version 1.5` in
`scripts/sbom.sh`; bump that one flag when upstream ships 1.6/1.7. The
~149-package gap is dev-dependencies +
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
in auth-middleware unit tests (the `stub` apps in
`src/server/router/auth.rs`'s `#[cfg(test)]` — e.g. `:758-760`) — they are
NOT production routes, so they are not router-only exclusions. Counted
production set: 8. (Line numbers here are verified-true at 1.29.2; re-grep
before trusting them after a router edit.)

### Well-known wiring table (each route confirmed individually)

| Route | Router registration | Handler |
|---|---|---|
| `/.well-known/openid-configuration` | `src/server/router/auth.rs:678` | `src/handlers/well_known.rs:24` |
| `/.well-known/jwks.json` | `auth.rs:681` | `well_known.rs:30` |
| `/.well-known/security.txt` | `auth.rs:683` | `well_known.rs:50` |
| `/.well-known/ai-notice` | `auth.rs:687` | `well_known.rs:79` |
| `/.well-known/ai-literacy` | `auth.rs:691` | `well_known.rs:97` |
| `/.well-known/cop-notice` | `auth.rs:695` | `well_known.rs:113` |
| `/.well-known/ump.json` | `src/server/router/ump.rs:27` | `src/handlers/ump_ops.rs:1` (`capabilities`) |

All 7 are also public-path listed (`route_guards.rs:19-40` `PUBLIC_PATHS`)
and present in `openapi.yaml` (ump.json + the six — grep the path to locate
them; the file is re-measured per release, not assumed: at 1.29.2 it is
10,928 lines, `x-api-version: "1.29.2"` — the 1.29.x delivery line moved the
stamp).
Standing rule: a new well-known route MUST land in all three places
(router + `PUBLIC_PATHS` + openapi) or fail review.

### Standing rule (v1.28.87, F7-07): site-table row in the same commit

A new content-returning route — any read surface that emits stored text —
adds its row to the `stored_text_fields_pass_the_read_seam` site table
(`tests/main_suite.rs`) in the SAME commit as the route, with the seam call
it requires (`sanitize_read` / `sanitize_read_cow` / `sanitize_read_opt` /
`sanitize_stored` / a named composition such as `sanitize_value_strings`).
The guard's `handler_body` extractor comment-strips sources before matching
(a comment naming the symbol cannot false-pass), but it is a regression lock
for LISTED sites, not a detector for new ones — the same-commit row is the
process that keeps the table honest. Same rule for a new direct write
surface: add it to `ingest_write_sites_route_through_screen`.

## Scripts appendix

| Script | Purpose | Documented |
|---|---|---|
| `install-service.sh` | Build + install binaries, launchd plist, strips macOS provenance xattr. | deployment.md / AGENTS.md |
| `release.sh` | Tag + publish; watches the public runs for the tagged SHA (the fail-closed green gate is `release.yml`'s). | this page / AGENTS.md |
| `release-sign.sh` | Sign release artifacts (also signs `brain kb build` tarballs). | cli-reference.md (kb) |
| `badges.sh` | Regenerate README badges from the real build; `--verify-count` is the test-count drift guard, `--selfcheck` the cheap derivations + completeness. | this page |
| `env-truth.sh` | Docs-vs-code env-var truth gate (tiers live, docs qualified + Loop-tracked). | this page |
| `sbom.sh` | SBOM generation for CRA/security docs. | cra.md |
| `cra-kit.sh` | CRA evidentiary kit generator. | cra.md |
| `admt-kit.sh` | ADMT transparency kit generator. | admt.md |
| `gen-model-manifest.sh` | Emit a `BRAIN_MODEL_MANIFEST` file for local model artifacts (fail-closed boot pin). | configuration.md |
| `sync-plugin.sh` | Rsync `plugin/` into the openclaw workspace's deployed extension (parity discipline). | plugin/README.md |
| `publish-wiki.sh` | Publish the `wiki/` directory to the GitHub wiki. | here only |
