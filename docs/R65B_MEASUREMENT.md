# R65 increment B' — measurement record

Commit under measurement: `8552ccee` (clean, main).
Needle for `CRATE_TEST_FLOOR`: `find src tests -name '*.rs' | xargs cat | grep -o '#\[test\]' | wc -l`.

## Premises that MEASURED TRUE

| Prompt premise | Measured |
|---|---|
| Ten `if kind ==` branches, no else/match/default | TRUE. Nine `if kind ==`; the tenth (`:1175`, crew skills) uses `matches!(kind.as_str(), ...)`, which a `^if kind ==` grep misses. |
| Unrecognized kind falls through to `:1373` → `promote_chunk_insert` | TRUE. `kind: &kind` at `:1424`; `promote_chunk_insert` binds `p.kind` as param `?7` (`src/service/gate.rs:193`). |
| `node_kind` is `TEXT NOT NULL DEFAULT 'fact'`, no CHECK | TRUE (`src/migration.rs:836`). |
| `promote_chunk_insert` returns `Result<i64, GateError>` | TRUE (`src/service/gate.rs:181`). |
| `case_merge_suggested` reaches approve with zero branches | TRUE. Written at `src/connector/crm/mod.rs:465`; the only other mention is a test. |
| `draft` is exempt from creation validation, zero approve branches | TRUE (`src/handlers/gate.rs:217`). |
| `is_strict_valid` would reject the governed kinds | TRUE — it admits only the six `MemoryKind` strings, and 14 branch-claimed kinds are not `MemoryKind`s. |
| `CRATE_TEST_FLOOR` is 2 740 against walk truth 2 740 | TRUE. Floor `src/spire_inventory.rs:177` = 2 740; needle = 2 740. |

## Premises that MEASURED FALSE

### P1 — "Two kinds reach this today". **Measured: eight.**

The prompt names `case_merge_suggested` and `draft`. Six more kinds are written by
PRODUCTION writers, have zero ladder branches, and today promote silently:

| Kind | Writer | Production? |
|---|---|---|
| `case_merge_suggested` | `src/connector/crm/mod.rs:465` | yes |
| `draft` | `src/service/webhook_ingest.rs:119`, `src/handlers/gate.rs:217` | yes |
| `delivery/artifact` | `src/workflow/delivery.rs:1174` (`ARTIFACT_PROPOSAL_KIND`, `:131`) | yes |
| `decision_review` | `src/service/review.rs:648` (`DECISION_REVIEW_PROPOSAL_KIND`, `:618`) | yes |
| `gdl_gap_new`, `gdl_gap_update`, `gdl_rca`, `gdl_complaint_rca`, `gdl_proposal` | `src/workflow/proficiency.rs:167-185`, reached from `gdl_checkpoint.rs:684` on every resolved GDL case | yes |

### P2 — "All 14 governed kinds still reach their branches". **Measured: the ladder claims 15 kinds across 10 branches, and two of the prompt's own line numbers point at non-branches.**

Counted from the source, branch by branch:

1. `:701` `PROP_KIND_REGISTRY_LIFECYCLE` = `registry_lifecycle`
2. `:781` `KIND_PUBLISH` = `kcs_publish`
3. `:891` `KIND_REMEDY` = `complaint_remedy`
4. `:949` `KIND_CONSENT` = `outreach_consent`
5. `:998` `KIND_CAMPAIGN` = `outreach_campaign`
6. `:998` `KIND_FOLLOWUP` = `outreach_followup`  ← same `if`
7. `:1036` `PROP_KIND_CHANNEL_TEMPLATE` = `channel/template`
8. `:1175` `KIND_SKILLS_UPDATE` = `crew_skills_update`
9. `:1203` `PROP_KIND_USER_MAP` = `channel/user_map`
10. `:1240` `KIND_TRANSLATE` = `kcs_translate`
11. `:1274` `KIND_NEW` = `kcs_new_article`
12. `:1274` `KIND_UPDATE` = `kcs_update_article`
13. `:1274` `KIND_LINK_ONLY` = `kcs_link_only`
14. `:1274` `KIND_RCA` = `complaint_rca`
15. — **no branch** — the six `MemoryKind` strings (`fact`, `procedure`, `step`, `decision`, `episodic`, `entitlement`) reach `:1373` as the ORDINARY promote path, claimed by nobody.

So "14" is the count of branch-claimed kinds, which is right; but it is NOT the set of
kinds that reach `:1373`. The set that reaches `:1373` is the six `MemoryKind`s +
`draft` + the six unhandled families above. A pin that enumerates only the 14 would
pin the wrong set.

### P3 — "the 14 legitimate governance kinds that the ladder branches handle". **Measured: the ordering claim is inverted.**

The prompt says to place the arm after all ten branches so it does not shadow them.
That part is right. But it then says not to use `is_strict_valid` "because it would
reject the 14 legitimate governance kinds that the ladder branches handle". Those 14
never reach the new arm — they have already `return`ed. The kinds that DO reach the arm
are precisely the ones `is_strict_valid` would accept (the six `MemoryKind`s) plus
`draft`. So `is_strict_valid` is not merely "not the right gate" — at this position it
is nearly the correct gate, and using it alone would still admit `draft`, which must
promote. Rejecting it wholesale (as instructed) is harmless but the stated reason is wrong.

### P4 — `delivery/artifact` and `decision_review`: refusing them is NOT obviously safe.

`src/workflow/delivery.rs:129-130` states the artifact kind "never becomes a knowledge
row. It stays a proposal or it does not exist." No branch enforces that today, so an
artifact approval promotes today. `decision_review` in DETERMINISTIC mode is asserted to
approve 200 at `src/handlers/decision_runs.rs:1287`, with the comment "The deterministic
twin approves (the generic promote path)."

So a blanket "refuse everything unclaimed" arm turns a **green test into a 400**.
This is the central design risk in the increment and is called out in §10.

### P5 — Pre-existing failure at entry.

`cargo test --features bench gate` fails at `8552ccee` **before any edit**:
`handlers::model_registry::tests::registry_promotion_requires_gate_approval_no_direct_status_route`
— `pending_second` vs `approved`. It passes alone and fails under the filter. Cause: a
`set_var("BRAIN_APPROVAL_QUORUM", "2")` test (`src/service/review.rs:757`) leaks process-wide
env into concurrent tests. Pre-existing, order-dependent, unrelated to this increment.
**Unfixed here** — fixing an env-leak flake in a test-support module is not this
increment's scope, and doing so silently would have hidden a real defect from the gate.

## The design consequence of P1/P4

The prompt's gate — "is this kind claimed by some branch?" — would **refuse `fact`**,
because the plain `MemoryKind` vocabulary reaches the promote tail by design and is
claimed by no branch. Measured, then implemented as an explicit ALLOW-list
(`PROMOTABLE_KINDS`) rather than a branch-derivation. Refusing `decision_review` or
`delivery/artifact` would have turned a green assertion
(`src/handlers/decision_runs.rs:1287`) into a 400.

## Red-proofs — each shown failing with the defect planted, then restored

Method: `cp` to a saved copy under `target/`, restore by `cp`, every restore proven with
`sha256` + `cmp`. No `git checkout`, no `git stash`. Baseline hash of `src/handlers/gate.rs`:
`c4889455353f5ddd0a80e1fd7b341b8326b9ff928665eb2d281ac34ac39a9e30` (identical before and
after every plant/restore cycle).

### B'.1 — arm removed (the original defect)
```
assertion `left == right` failed: kind "case_merge_suggested" must be refused,
  got 200 OK {"chunk_id":1,"proposal_id":1,"status":"approved","superseded":null}
  left: 200
 right: 400
```
Six pins failed, including B'.3 and B'.4.

### B'.2 — the SAME arm moved to the TOP of the ladder
```
branch "if kind == crate::workflow::registry::PROP_KIND_REGISTRY_LIFECYCLE" sits at
  byte 34655, AFTER the default arm at 33954 — the arm shadows it
```
**This is the trap the prompt predicted, observed directly**: B'.1, B'.3 and B'.4 all
still PASSED with a shadowing arm. Only the no-shadowing pin caught it.

### B'.3 — proposal consumed and committed before the refusal
```
assertion `left == right` failed: the refusal consumed the proposal;
  the operator's queue lost the row
  left: "approved"
 right: "pending"
```
A first attempt planted the residue INSIDE the tx and was correctly neutralised by the
rollback (caught instead by the positive control, not by B'.3) — so the pin was re-proved
with residue that survives the rollback, rather than being declared green on a plant
that did not test it.

### B'.4 — generic refusal message
```
the refusal must name the offending kind, got "proposal kind is not promotable — refused"
the refusal for "gdl_rca" must name "gdl_rca", got "proposal kind is not promotable — refused"
```

### One pin was itself wrong on first run

`b3_the_arm_precedes_the_promote_and_embed_path` FAILED against correct code: it compared
the arm against the FIRST `encode_one` in the file, which belongs to the KCS branch
hundreds of lines earlier. That is trap #1 in disguise — a whole-file `find` reported a
healthy ordering as broken. The search is now scoped to the text after the arm, and the
pin additionally asserts the resolved embed precedes the promote call.

## Final state

`cargo test --features bench` 2952 passed / 0 failed. `cargo test --all-targets` 2963
passed / 0 failed. `r65b_ladder_pins` 15/15. clippy (bench + default) clean under
`-D warnings`. `cargo fmt --check` clean. lipstyk exit 0. `cargo audit` clean.
docs-truth 17 pre-existing LOWs (route/api.md notes, unrelated). env-truth selfcheck OK.

Invariants all print 0: `Cargo.lock`, `crates/Cargo.lock`, `src/migration.rs`,
`openapi.yaml`. `LATEST_KNOWN_SCHEMA` unmoved at `1.32.23`.

`CRATE_TEST_FLOOR` 2 740 → 2 749, re-measured by the census's own needle
(`find src tests -name '*.rs' | xargs cat | grep -o '#\[test\]' | wc -l` = 2 749). The
prompt's 2 740 figure measured TRUE at entry; the raise is the walk truth after this
commit's nine new `#[test]`s, not a delta.