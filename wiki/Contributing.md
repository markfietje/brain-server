# Contributing

Contributions to Brain Server are welcome. This page explains how to set up, build, and submit changes, and what quality bar is enforced.

## Code of conduct

Please read the **[Code of Conduct](https://github.com/markfietje/brain-server/blob/main/CODE_OF_CONDUCT.md)** before opening a PR. The full contributing guidelines live in **[CONTRIBUTING.md](https://github.com/markfietje/brain-server/blob/main/CONTRIBUTING.md)** in the repository.

## Set up

```bash
git clone git@github.com:markfietje/brain-server.git
cd brain-server
```

You need stable Rust with `cargo` (via [rustup.rs](https://rustup.rs)).

## Build

```bash
# All binaries
cargo build --release --features bench

# With the GitHub connector
cargo build --release --features bench,connector-github
```

## Quality gates (enforced in CI)

Run these locally before opening a PR:

```bash
cargo fmt --check
cargo clippy --all-targets --features bench -- -D warnings
cargo test --features bench
```

- `cargo clippy -- -D warnings` must be **clean** — zero warnings enforced.
- `cargo fmt --check` must pass.
- All tests must pass. The suite includes schema-contract tests, route-coverage tests, and mutation-proven guards (e.g. the audit-chain, authz-wiring, and constant-time compares).

## Where to look

| Area | Location |
|---|---|
| Retrieval engine | `src/search/` |
| Handlers (HTTP) | `src/handlers/` |
| Governance (audit, gate) | `src/audit.rs`, `src/gate.rs` |
| Knowledge graph | `src/linker.rs`, `src/graph*` |
| Sources & connectors | `src/sources.rs`, `src/connector/` |
| Migration | `src/migration.rs` |
| Client (Dioxus GUI) | `client/` |

## Reporting security vulnerabilities

For security vulnerabilities, do **not** open a public issue. Use the GitHub **"Report a vulnerability"** tab. See the **[Security](Security)** page and `SECURITY.md` in the repository.

## Next steps

- **[Quickstart](Quickstart)** — get the server running first.
- **[Architecture](Architecture)** — understand the codebase.
- **[Glossary](Glossary)** — terminology used in the code and docs.
