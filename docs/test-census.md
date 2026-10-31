# Test census — what the suite is made of

Machine-derived by `scripts/test-census.py`. **Re-derive, never edit:**

```
scripts/test-census.py --check    # exits non-zero on drift
```

No commit hash is recorded here on purpose: a hash is the one field
the script cannot derive from the tree, so recording one would make
`--check` depend on an environment variable and stop the table from
re-deriving byte-for-byte. Git already records which commit shipped it.

## The classes, and why they are ordered

First match wins; every test gets exactly one class. Full rule text,
including the stdlib allowlist that decides rule 4, is the module
docstring in `scripts/test-census.py` — the rules live with the script
that applies them, not in prose that can drift from it.

1. `residual_documentation` — asserts a stated limitation ON PURPOSE.
2. `control` — the anti-vacuity arm that makes a pin's failure readable.
3. `structural` — reads the TREE: the read is in the body, or one call
   deep in a named helper that reads a repo-shaped file. A read of
   runtime data (a key file, a backup the test made) is NOT structural
   — and neither is a process spawn: running a script and reading its
   exit status drives a real subject.
4. `behavioral_pin` — POSITIVE evidence of a drive, in either call shape:
   qualified (`brain_server::audit::hash`) or BARE (`verify_claim(c, q)`).
   A bare call is how a Rust unit test names its own module's subject,
   so a rule 4 that only accepts qualified calls misses most of this repo.
5. `unclassified` — drives no subject and reads no tree. Reported, never
   folded into 4 by default: "no read found" is precisely the shape of
   the four vacuous census pins R123 shipped on first draft.

## The count

| Class | Tests | Share |
|---|---:|---:|
| `behavioral_pin` | 3178 | 86.3% |
| `structural` | 372 | 10.1% |
| `control` | 62 | 1.7% |
| `residual_documentation` | 63 | 1.7% |
| `unclassified` | 9 | 0.2% |
| **total** | **3684** | |

Scope: `src/` + `tests/`, the two trees the badge's count comes from.
The census reads HIGHER than the README badge for two reasons, both
expected. The badge counts tests that
*passed* a real run, after twelve `--skip` families remove the
platform-only sandbox tests (`scripts/badges.sh`); the census counts
tests as written. And a raw `rg` count of `#[test]` over-reads this
tree by ~64, because tests that pin a text-scanner carry `#[test]`
inside their own string literals (tests/rbac_evaluation_pins.rs has
eleven such cases). The census counts what the compiler does.

## The top ten files by size

| File | Tests |
|---|---:|
| `tests/main_suite.rs` | 356 |
| `src/workflow/gdl.rs` | 143 |
| `src/workflow/decide/router.rs` | 59 |
| `src/agentloop/run_loop.rs` | 56 |
| `tests/authz_matrix.rs` | 56 |
| `src/workflow/delivery.rs` | 54 |
| `src/bin/mcp.rs` | 53 |
| `src/search/tests.rs` | 44 |
| `src/bin/brain.rs` | 43 |
| `src/screen.rs` | 43 |

## `unclassified` — the honesty check on the rule set

9 tests drive no subject and read no tree.
They are listed so a reader can move them by adding evidence, not
by re-deciding. Read this bucket as "the census could not see a drive", NOT as "this test is worthless": all six assert a
constant or a declared bound, which is a real pin that happens to
have no call in it.

| Test |
|---|
| `src/auth/revocation.rs:398` `revocation_cache_is_stateless_unit_struct` |
| `src/authz/policy.rs:300` `r47_oracle_is_total_over_its_closed_vocabulary` |
| `src/handlers/relay.rs:486` `decline_reason_validation_bounds_hold` |
| `src/trace.rs:47` `trace_traversal_caps_are_bounded` |
| `src/workflow/decide/router.rs:351` `alias_checkpoints_table_is_the_three_names` |
| `src/workflow/decide/router.rs:361` `alias_bundle_repo_is_the_english_root` |
| `src/workflow/gdl.rs:6288` `adversarial_task_bound_is_pinned_by_value` |
| `src/workflow/gdl.rs:6299` `artifact_bounds_are_pinned_by_value` |
| `src/workflow/session_log.rs:1161` `scope_caps_are_pinned_by_value` |

## What this census does NOT police

It is not a gate on class counts. There is no floor on any class in
this round: a floor written before its population is measured is a
number nobody checked. The only thing that can fail is **drift** — the
committed table disagreeing with a fresh derivation (`--check`).

