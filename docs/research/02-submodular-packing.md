# Submodular Evidence Packing (token-budgeted, diverse evidence)

**File:** `src/search/packing.rs`

## The problem

When an agent's context window is finite, recall must choose *which* of many
candidate chunks to surface. Naive top-k over a single score over-selects the
same story and wastes tokens on near-duplicates. You want a set of evidence
that is jointly **relevant, novel, and representative** under a hard token
budget.

## The reference

**arXiv:2607.00725** — budgeted monotone submodular maximization with lazy
greedy, achieving the classic **(1 − 1/e) optimality bound**, shown to gain
+5.1 F1 on HotpotQA. The objective rewards coverage and penalizes redundancy;
a diversity gate keeps the set from collapsing onto one cluster.

## The implementation

`src/search/packing.rs::pack` is a deterministic lazy-greedy under a knapsack:

- `DEFAULT_MAX_CONTEXT_TOKENS = 160`, `MAX_CANDIDATES = 64` cap the work.
- Objective = **relevance + coverage + representativeness** (the `Weights`
  config, tunable via env), gated by an MMR-style diversity bound:
  `DEDUP_SIMILARITY = 0.85` — a candidate whose best overlap to an already-
  chosen chunk exceeds 0.85 is dropped.
- `est_tokens(text)` estimates tokens at `CHARS_PER_TOKEN = 4` — a cheap,
  deterministic proxy (no tokenizer in the hot path).
- `/recall?max_context_tokens=` triggers packing; the response reports
  `packed_tokens` and (with a `gold_answer`) the `answer_in_context`
  diagnostic — is the answer actually inside the chosen evidence?

## Measured ceiling

- Diversity is **lexical Jaccard**, not embedding cosine (a cheap, deterministic
  proxy; cosine would pull the model into the packer).
- The weights are corpus-independent defaults; `weights_from_env()` lets an
  operator calibrate without a rebuild.
- Greedy is near-optimal, not optimal — the honest (1 − 1/e) claim is stated
  plainly, not exceeded.

*The `answer_in_context` diagnostic is the bridge to a judged-corpus recall
floor (`brain eval`).*
