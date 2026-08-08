# Contributing to brain-server

Thanks for your interest in brain-server. This project is a memory backend with
a strict set of engineering conventions; following them makes review faster and
keeps the release chain clean. Please read `README.md` first for the project
overview and the current feature set.

## Ground rules

- **No new dependencies unless unavoidable.** This is a deliberately
  dependency-light project (low-power manifesto — the server runs on ARM).
  Check whether the standard library or an already-listed dependency covers the
  need before adding a crate. A new dependency needs a justification in the PR.
- **No abstractions that weren't asked for.** Prefer the smallest correct diff.
- **Mark honest simplifications.** If you cut a real corner (global lock,
  O(n²) scan, heuristic threshold), leave a `ponytail:` comment naming the
  ceiling and the upgrade path.
- **Tests prove intent.** Non-trivial logic lands with at least one small
  test that fails if the behavior breaks. The migration/audit wiring has
  contract tests (`test_migration_schema_contract`,
  `test_openapi_covers_routes`, `authz_gates_cover_every_non_public_route`) —
  keep them in sync when you touch schema, routes, or authz.

## What to work on

- The authoritative backlog is `ROADMAP.md` and the `IMPLEMENTATION_PLAN_*.md`
  files. Each release has a plan; a PR that matches a plan milestone is the
  easiest to review.
- Open issues and bugs are welcome regardless.
- **Don't tackle a release milestone without checking in first.** Releases are
  versioned and tagged (`vX.Y.Z`); coordinate with the maintainers so two
  people don't ship the same slot.

## Getting started

```sh
# Build all four binaries (the bench binary is feature-gated)
cargo build --release --features bench --bin brain-server --bin brain --bin mcp --bin bench

# Client (Dioxus control surface)
cd client && cargo build
```

## The quality gates (must pass before a PR)

```sh
# Server
cargo fmt --check
cargo clippy --all-targets --features bench,migrate -- -D warnings   # zero warnings enforced
cargo test --all-targets --features bench,migrate

# Client
cd client
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --target wasm32-unknown-unknown
```

CI runs the same gates (plus `cargo audit`). A PR that fails any of them will
be asked to fix them.

## Security

- **Do not file public issues for security vulnerabilities.** Use the GitHub
  "Report a vulnerability" tab. See `SECURITY.md` for the full policy and SLA.
- Never commit secrets, keys, or tokens. The live auth token is loaded from
  `AUTH_TOKEN_FILE`/`AUTH_TOKEN`; nothing like it belongs in the tree.
- Every non-public route is authz-gated; new routes must call `authorize(...)`
  at handler entry and be added to the wiring-guard table and `openapi.yaml`.

## Pull requests

- Small, focused PRs. One logical change per PR if possible.
- Write a clear title and describe **what** and **why**, not just the diff.
- Reference the plan milestone or issue you're addressing.
- Keep the existing commit-style conventions (conventional-ish prefixes like
  `feat(scope):`, `fix(scope):`, `docs(release):`, `refactor(scope):`).

## Release process (maintainers)

Releases are tagged (`git tag vX.Y.Z`) and pushed; CI builds the release
binaries and publishes a GitHub release. Docs (`CHANGELOG.md`, `README.md`)
are updated in the release commits. `AGENTS.md`, the `CLIENT_ROADMAP.md`, and
`IMPLEMENTATION_PLAN_*.md` files are gitignored working documents — they carry
the release chain but are not part of the committed tree.

## Questions

Open an issue, or reach out via the contact channel in `README.md`.
