# Ignored-test inventory and recommended disposition

**Date:** 2026-09-26 · **Repo:** `/Users/mark/Sites/brain-server` · **HEAD:** `d5e774a`
**Status:** DECISION DOCUMENT for the operator. No test is modified by this file.

## The census, and why three different numbers are all true

The number "ignored tests" is ambiguous in this repo, and stating it imprecisely
is what let a security control sit unverified. Three counts, each correct
against its own question:

| Count | Value | What it measures |
|---|---|---|
| Raw `#[ignore]` attribute occurrences | **17** | includes **12 prose mentions inside comments** (`src/embed.rs:170`, `src/workflow/gdl_eval.rs:1087`, `src/connector/github/client.rs:257`, `src/handlers/case_run.rs:1382`, `tests/main_suite.rs:2412`, `:11983`, `:12139`, and others) that merely *describe* an ignore |
| Real `#[ignore]` attributes on test fns | **5** | the actual disabled tests |
| `cargo test` "ignored" tally | **3** | excludes two that live behind build/config paths not in the default test run |

**The reviewable figure is 5 disabled tests.** Anyone quoting "17" is counting
comment prose.

## Inventory

| # | Test | Stated reason | Security-relevant? | Verified disposition |
|---|---|---|---|---|
| 1 | `src/workflow/delivery.rs`-adjacent: `tests/main_suite.rs:12148` `ump_suite_parity_l1_to_l3` | was *"loads model2vec"* | **YES** — UMP L1–L3 parity | **KEEP IGNORED — real defect.** Reproduced: fails at `main_suite.rs:12300` with `Option::unwrap()` on `None`. The test sets `BRAIN_UMP_KEY_DIR` to make the instance L3 and the record returns **without** its `ed25519:`-prefixed signature — an unkeyed-instance bug in the test's own setup, not a weights problem. **A failing test must not be promoted.** Owner: UMP keying. |
| 2 | `src/embed.rs:625` `static_embedder_matches_model2vec_golden` | fetches model from HuggingFace | No — byte-parity regression proof for the edge path | **KEEP IGNORED.** Network dependency; needs an offline fixture of golden vectors to enable. Owner: retrieval. |
| 3 | `src/embed.rs:658` `neural_loads_and_emits_three_outputs` | downloads BGE-M3 (~600MB) | No | **KEEP IGNORED.** Size/network; belongs on a nightly, not a PR gate. Owner: retrieval. |
| 4 | `src/handlers/case_run.rs:1396` `live_configured_case_real_provider` | requires operator-supplied `BRAIN_GDL_PROVIDER_*` | Low | **KEEP IGNORED.** By design — a live-provider drill. Owner: operator runbook. |
| 5 | `src/connector/github/client.rs:261` `live_github_list_issues_smoke` | needs real GitHub | Low | **KEEP IGNORED.** Live integration; belongs on a credentialed nightly. Owner: connectors. |

**Recommendation: no change to any of the five.** Four are correctly ignored
with accurate, dated reasons. One (#1) is a genuine defect that should be
fixed at source and then enabled — but that is a UMP-keying change, not a
test-policy change, and it is out of scope for a release-train closeout.

## What was already fixed, and why it mattered

An earlier revision of this document recommended promoting two tests. Both
turned out to be **disabled on a rationale that was factually false** — the
stated blocker was *"loads model2vec"*, but the model2vec weights are vendored
in-tree and the tests pass in the default suite:

| Test | Was | Now | Why it mattered |
|---|---|---|---|
| `ingest_screens_injection_like_its_siblings` | ignored | **enforced** | The only automated evidence that `/ingest` screens hostile content. A security control whose test was opt-in was, in practice, unverified. |
| `procedure_screens_injection_like_its_siblings` | ignored | **enforced** | Same, for `/procedures`. |
| `ump_batch_ingest_round_trip` | ignored | **enforced** | Wire round-trip; rationale false. |
| `profiles_end_to_end_wizard_and_ingest` | ignored | **enforced** | Rationale false. |
| `eval_recall_harness` | ignored | **enforced** | Rationale false; its own 10-doc caveat is retained and it is explicitly NOT a release gate. |

Verified: `cargo test --test main_suite` → **246 passed, 0 failed, 1 ignored**
after promotion. The full suite went from `2325 passed / 8 ignored` to
`2332 passed / 3 ignored`.

**The generalisable finding:** an `#[ignore]` reason is a *claim*, and claims
rot like any other. No CI job runs `--ignored`, so a false rationale is
undetectable. The five promotions happened only because someone ran the ignored
tests instead of trusting their comments. That is the disposition discipline for
any future ignore: **verify the stated reason before accepting it.**
