# Gate-Law Register

The standing artifact the audits kept finding missing: a guard cannot be
added to this tree without a row here, and every row carries its red-proof —
the demonstration that the guard FAILS when the property it polices is
violated. A green gate that cannot fail is decoration; this register is
where a guard's teeth are recorded, so the next reader checks instead of
trusts.

The law, three sentences:

1. A new guard lands with its row, its red-proof, and its class — behavioral
   (drives the real subject), structural (reads the source/wiring), or
   exhaustive (scans every scalar/row).
2. A structural row is a REGRESSION LOCK, not proof: an env-conditioned wrap
   or a comment can satisfy text. Every structural row carries an
   `owed` column — the behavioral conversion that would retire it — and the
   row stays until the conversion lands.
3. A red-proof asserted for the first time in a round is executed in that
   round (the mutant runs, the pin fails, the record says so).

## The standing guards

| Guard | Subject | Red-proof | Class |
|---|---|---|---|
| `no_sql_in_handlers_enforced` | `src/service/mod.rs` walks `src/handlers` recursively | `sql_statement_counter_still_fires` (planted statement + negative control) | structural + self-pin |
| Spire floors (`CRATE_TEST_FLOOR`, `ROUTER_SITES_FLOOR`, coverage/authz rows) | `src/spire_inventory.rs` | each scanner self-pins a planted violation string | structural + self-pin |
| `stored_text_fields_pass_the_read_seam` | `tests/main_suite.rs` (the machine table) | `handler_body` comment-strips, then asserts the seam symbol per emission site; planted-comment false-pass red-proven | structural (comment-stripped) |
| The law-9 authz matrix | `tests/authz_matrix.rs`, every route x principal class through the composed app | behavioral: a deleted gate flips cells (the F8-02 history); rows pinned against `AUTHZ_GATES` so a renamed route fails loudly | behavioral |
| `invisible_set_fixture_is_exhaustive_truth` | `src/strip_invisible.rs` x `plugin/fixtures/invisible-classes.json` | exhaustive over ALL scalar values; one-sided edits fail a lane by construction | exhaustive |
| `badges.sh --verify-count` | the README test-count badge | re-derives and refuses on drift (R72: the green gate on a lie) | behavioral |
| Lock-freshness lane | every tracked `Cargo.lock` vs its manifest | five red-proofs at the lock round (stale lock, dropped flag, renamed lane, rev drift, branch form) | behavioral |
| The register pin (`r73_register_rows_carry_a_disposition`) | `AUDIT.md`, one arm per findings table | per-id arms; the OPEN-but-names-a-shipped-round refusal | structural per-table |
| `r47_gate_rows_read_their_declared_action` | `AUTHZ_GATES` table literal | reads expectation from the table, never from the code under test | structural |
| `consolidate_apply_refuses_the_agent_class` + the class rows | `tests/agent_write_posture_pins.rs`, `tests/authz_matrix.rs` | red-first observed (agent walked the route with the refusal stashed) | behavioral |

## The text-bound rows (owed conversions)

| Guard | Subject | Class | Owed |
|---|---|---|---|
| `the_ingest_audit_row_precedes_its_commit` | `ingest_memory` (the audit-before-commit ordering) | structural (offset) | a crash-injection drill (the sibling add-arm's rollback-twin shape) |
| `the_edge_history_surface_is_bounded_and_discloses` | `get_edge_history` (the cap + `truncated` disclosure) | structural (substring) | a behavioral cap assertion through the composed app with `EDGE_HISTORY_MAX`+1 versions |
| The rate-limit wiring pin | `tools/signal-gateway/tests/s8_04_rate_limit_wired.rs` (the `apply_rate_limit(app,` literal) | structural (substring) | an env-conditioned wrap satisfies the behavioral e2e while production stays off — the wrap should be unconditional by construction |
| The Art 50 wiring check | `src/reg_watch.rs` (file-granular over four classes) | structural (file-granular) | per-emission-point wiring assertions (the behavioral provenance meta-test is the real guard today) |
| The release/CI band pin | `tests/lock_discipline_pins.rs` (`the_release_and_ci_band_carries_its_gates`) | structural (substring over the workflow YAML) | behavioral: the workflows only execute on the runner — the red-proof for each arm is the register row it closes (F4-13/14/16/17/22), observed open in the tree before the change |
| Mutable container base images | `Dockerfile` (`rust:1-bookworm`, `debian:bookworm-slim`), `docker-compose.yml` (`oauth2-proxy:latest`) | none — OPEN | digest pins (`@sha256:…`) require resolving real digests from the registry at merge time: an external act, recorded owed (F4-19) |
| cargo-audit staleness | `.cargo/audit.toml` prose re-check triggers, no gate | none — OPEN | `--deny warnings` flips CI red on the standing yanked-crate warning; the gate needs the ignore list pruned FIRST (a lockfile decision), recorded owed (F4-21) |
| WCAG evidence tags | `docs/trust/wcag22-aa-checklist.md` cites client-lane gates as shell evidence | none — OPEN | retarget the evidence tags to the shipping artifact (the shell a11y lane), owed (F4-15) |
| Crate islands census | 5 of 13 `crates/` members unconsumed, unpinned | none — OPEN | `crates_consumed_set_is_pinned` with a per-entry reason (F4-18) |
