## Summary

What does this change do? (One or two sentences.)

## Type of change

- [ ] Bug fix
- [ ] New feature / milestone
- [ ] Docs / release wrap
- [ ] Refactor / hardening
- [ ] Dependency change

## Quality gates

Please confirm before requesting review:

- [ ] `cargo fmt --check` clean (server and, if touched, `client/`)
- [ ] `cargo clippy --all-targets --features bench,migrate -- -D warnings` clean (client: `--all-targets -- -D warnings`)
- [ ] `cargo test --all-targets --features bench,migrate` green (client: `--all-targets`)
- [ ] Client wasm build green if `client/` touched (`cargo build --target wasm32-unknown-unknown`)
- [ ] New routes documented in `openapi.yaml` + added to the authz wiring-guard table
- [ ] Schema changes reflected in `test_migration_schema_contract`
- [ ] `CHANGELOG.md` updated for user-facing changes

## Tests

Describe the test(s) you added and what behavior they pin. If none, why not.

## Honest ceilings / notes

Any deliberate simplifications (`ponytail:` ceilings), known limitations, or
follow-up work this PR defers?

## Related

Reference the issue or plan milestone, e.g. "Closes #12", "Milestone M4 of
`IMPLEMENTATION_PLAN_v1.16.7_Integrated.md`".
