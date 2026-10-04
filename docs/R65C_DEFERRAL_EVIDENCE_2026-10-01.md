# R65 — deferral decision · EVIDENCE

**Date:** 2026-10-01 · **Repo:** `brain-server` @ `8552ccee` + this round's uncommitted work
**Plan:** `brain-steward-ip/plans/EXECUTION_PLAN_R65C_DEFERRAL_DECISION_2026-10-01.md`
**Context:** PoC. The knowledge matrix was adjudicated by a simulated SME in chat and is
recorded as such. **No architecture fork was decided by chat** — see §5.

---

## 1 · What shipped this round

| Piece | File | Status |
|---|---|---|
| The pure deferral decision | `src/workflow/confidence.rs` | **new** |
| Its module declaration | `src/workflow/mod.rs` | one line |
| The wire receipt | `src/handlers/procedure.rs` (`POST /classify`) | additive |
| The CLI surface | `src/bin/brain.rs` (`brain classify`) | additive, degrades on old servers |
| The contract | `openapi.yaml` (`DeferralReceipt`) | additive |
| The operator doc | `docs/api.md` | one row |
| The decision's pins | `tests/r65_confidence_pins.rs` | +9 tests |
| The ladder default arm (prior increment) | `src/handlers/gate.rs` + `tests/r65b_ladder_pins.rs` | +15 tests |

**The decision:** `decide_deferral(class, features)` is a total, pure function returning
`Defer` / `Clarify` / `Stop`. **Every class resolves to `HumanRequired` today**, because
`AUTO_CLASSES` is empty and no per-class reliability (`meta_d`) has been measured.

That is the shipped state, not an unfinished stub. It is what makes the *next*
measurement meaningful: granting a class becomes a measurement event with recorded
provenance rather than an unauditable code change.

---

## 2 · Red-proofs — each shown failing against a planted defect, then restored

Method: `cp` to a saved copy under `target/`, restore by `cp`, every restore proven with
**sha256 + cmp**. No `git checkout`, no `git stash`.

| ID | Plant | Observed failure | Restored |
|---|---|---|---|
| `I65.2a` | a fabricated `meta_d` for `Technology` | `Technology carries a measured reliability — the table is no longer empty` | ✅ |
| `I65.2b` | typo `"complaince"` mapped onto `Compliance` | `"complaince" must resolve to the absence class, never to a neighbour` — `left: Compliance, right: HumanUnmeasured` | ✅ |
| `I65.2c` | classifier label `legal_contracts` with no policy variant | `the classifier emits a label the deferral policy does not model` (+ the independent LEXICON pin also fired) | ✅ |
| `I65.3` | `evidence >= 2 → Stop` | `class Technology decided Defer at 0 keywords but Stop at 2 — the outcome must not depend on how many keywords happened to fire` | ✅ |
| wire | the receipt hardcoded, decoupled from the decision | `the receipt must key on the label the classifier returned … left: Some("general"), right: Some("compliance")` | ✅ |

The wire proof exists because **nothing in the tree asserted the `/classify` response
body** — the authz matrix checks the route's status, not its shape. So without it, the
receipt could stop shipping and every consumer would silently fall back to inferring
"the machine was unsure" from a confidence number.

### Two red-proofs were defective before they were convincing

**A. The `I65.2b` plant was initially ineffective.** The first plant used
`starts_with("complianc")`, which does not match the fixture's `"complaince"` — the
guard never fired and **the pin passed with the defect planted**. That is trap #1
occurring in the *plant*, not the pin. Corrected, the pin failed as it should. The pin
was sound; the plant was not. Recorded because "the red-proof passed" is exactly the
sentence that hides this.

**B. The `I65.3` property was vacuous twice.** Ranked `Stop` as the *most* human
outcome, so "more evidence never reduces human involvement" passed against an inversion.
Rewrote to assert `requires_human()`, which is `true` for **every** outcome by
construction — so it could not distinguish `Defer` from `Stop` and passed again.

The property that survived and has teeth: **the outcome is a function of the class
alone, not of the evidence.** A deferral policy that answers differently as keywords
pile up is the same keyword-share defect the round exists to repair, reappearing one
layer up. Minimal failing input: `class_index = 0, low = 0, delta = 2`.

**A pin is evidence only after it fails for the right reason.**

---

## 3 · Two real defects this change surfaced, fixed at the root

| Guard | Failure | Fix |
|---|---|---|
| `dup_guard` | `decide defined 2x: workflow/confidence.rs, loom.rs:94` | Renamed to `decide_deferral`. `loom::decide` is a private fan-out gate and semantically unrelated; an `ALLOWED_DUPES` entry would have been the wrong fix — the new name is more descriptive and needs no allowlist |
| `error_taxonomy` | `error-enum set drifted — classify the newcomer` | Registered `DeferralError` in `KNOWN_ERRORS` (category `workflow`: it refuses a case rather than failing a write) |

Neither was anticipated by the plan. Both are the guards doing their job.

---

## 4 · Verification

Full transcript: `target/r65-verify.log`, produced by `scripts/verification-sweep.sh`
(renamed from `r65-verify-sweep.sh` on 2026-10-04 — the script names its subject,
not the round that wrote it; the log it wrote keeps the old name and is left as-is).
Detached so it survives the terminal. The sweep runs its lanes **sequentially** —
parallel cargo on one target directory serialises on the cargo lock anyway, and separate
target directories would force full rebuilds.

| Lane | Result |
|---|---|
| `cargo test --all-targets` | PASS |
| clippy `bench` · `default` · `otel` | PASS (0 errors under `-D warnings`) |
| feature lanes: `loom` · `neural-embed` · `injection-classifier` · `multivec` | PASS |
| feature lanes: `rerank-tier` · `compliance-pack` | PASS on re-run — see below |
| `cargo test --manifest-path crates/Cargo.toml --all-targets` | PASS |
| `cargo test --manifest-path tools/steward-harness/Cargo.toml` | PASS |
| `cargo audit` | PASS |
| `scripts/docs-truth.sh` · `env-truth.sh --selfcheck` · `badges.sh --selfcheck` | PASS |
| `scripts/lipstyk-gate.sh` | PASS (exit 0) |
| `cargo test --features bench` | **2 964 passed / 0 failed** |
| `cargo test --all-targets` | **2 975 passed / 0 failed** |
| `cargo fmt --check` | clean |
| `CRATE_TEST_FLOOR` | 2 749 → **2 758**, equal to the census needle |
| **Sweep total** | **`SWEEP_EXIT=0` — 15 of 15 lanes PASS** |

### Invariants

```
Cargo.lock             0
crates/Cargo.lock      0
src/migration.rs       0
openapi.yaml           1     <- the intended additive DeferralReceipt
LATEST_KNOWN_SCHEMA    1.32.23 (unmoved)
```

**Zero new dependency edges.** Both lockfiles byte-identical. No migration. Schema
unmoved — the receipt is an additive response field, not a stored one.

### Two lane failures that were NOT defects — and why that matters

The first sweep reported `lane-rerank-tier` and `lane-compliance-pack` FAILED with
``cannot find function `json` in this scope`` at `tests/r65b_ladder_pins.rs`.

That was **my own mid-flight edit**: the sweep compiled the test file in the window
between appending the classify-receipt tests and adding the `json()` helper they call.
Both lanes pass on re-run against the same tree (0 errors each).

Worth stating plainly because the failure mode is the dangerous one: a red lane that is
actually a **stale compile** looks exactly like a red lane caused by the change, and the
cheap wrong move is to "fix" working code until the noise stops. The check that
distinguishes them is one command — does the symbol the compiler wants exist in the file
now — and the answer decided it without touching the implementation.

### A proptest artifact removed deliberately

`tests/r65_confidence_pins.proptest-regressions` was generated by my own **planted**
defect. Committing it would encode a bug that never shipped as a permanent regression
seed. **No `*.proptest-regressions` file is tracked anywhere in this repo's history.**
Removed rather than kept "because proptest recommends it".

### A pre-existing failure, NOT fixed here

`handlers::model_registry::tests::registry_promotion_requires_gate_approval_no_direct_status_route`
fails under the `gate` filter and passes alone. Cause: a
`set_var("BRAIN_APPROVAL_QUORUM", "2")` test (`src/service/review.rs:757`) leaks
process-wide env into concurrent tests. **Pre-existing at `8552ccee`, before any edit.**
Not fixed: an env-leak fix in test support is not this round's scope, and fixing it
silently would hide a real defect from the gate.

---

## 5 · What was NOT decided, and why

The operator asked to dispatch four sessions. **Three are blocked on decisions only the
operator can make**, and one premise measured false:

1. **"Gold set is built — wire `classify_replay` into promotion."** Measured **false.**
   `classify_replay` (`src/workflow/create/replay_gate.rs:91`) has **zero production
   callers**; every hit is its own test. **Zero classifier-axis gold fixtures exist** —
   all seven are `qc_report` / `admission` / `gdl_cases`, and none reference `classify`,
   `routing_class`, or `technology`. Wiring a blocking gate to promotion on absent
   evidence is the exact error the deferral round exists to prevent.
2. **R55/R58a remain blocked by the repo's own record.** `AGENTS.md:155`:
   `gates_vacuous` is still advisory-only and R55/R58a are still blocked. R53's halt is
   the operator's fork and was not resolved by the knowledge-matrix adjudication — that
   adjudication covered *content*, not *architecture*.
3. **R66 consumes R65's `RoutingClass`** (`EXECUTION_PLAN_R66_INBOUND_ROUTING:89`), so it
   is sequential, not parallel. It can start the moment this work is committed.
4. **R55's registry fork — undecided.** Extend `write_agreement_label`
   (`src/workflow/agreement.rs:384`, already a structural clone of `write_label` at
   `kappa.rs:301` — same base/glob idempotency, same latest-wins) or give body labels
   their own table (migration, schema 1.32.24). Schema consequence; not guessed.
5. **The deferral axis is now decided by shipped code**: `RoutingClass` keys on the eight
   `CATEGORIES`, the only classifier wired today. `WORKTYPE_TABLE`'s nine worktypes
   intersect nothing. Reversible, but recorded.

---

## 6 · Non-claims

1. **Not a measured `meta_d`.** The table ships with zero auto-authorisations.
2. **Not a calibrated threshold.** `confidence` remains an uncontested keyword share.
3. **Not the deferral seam** — no `proposals` row, no approve arm. The decision is
   computed and exposed; nothing consumes it to gate a write yet.
4. **Not a calibration.** Gate agreement, not truth.
5. **Not an autonomy widening.** The knowledge ring stays fixed-proficiency.
6. **Unblocks nothing.** Not R55 / R58a / R58b / R61.
7. **Does not fix** the `human_pass` doc comment at `drift_census.rs:244`, the stale
   comments at `agreement.rs:178` / `gate.rs:1551`, `route_domain_label`'s unrepresentable
   `None` fallback, or the R53 halt.

---

## 7 · Exact commands and their output

```sh
cd ~/Sites/brain-server && export RUSTFLAGS="-D warnings"
bash scripts/verification-sweep.sh      # was r65-verify-sweep.sh; detached; writes target/r65-verify.log
cargo test --features bench
cargo test --all-targets
cargo clippy --all-targets --features bench -- -D warnings
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/lipstyk-gate.sh
# INVARIANTS
for f in Cargo.lock crates/Cargo.lock src/migration.rs; do
  printf "%-22s %s\n" "$f" "$(git --no-pager diff --numstat "$f" | wc -l | tr -d ' ')"
done
git --no-pager diff --numstat openapi.yaml   # EXPECTED NON-ZERO: the additive receipt
grep -n LATEST_KNOWN_SCHEMA src/storage_layout.rs
```

`<exact per-lane output inserted at ship>`