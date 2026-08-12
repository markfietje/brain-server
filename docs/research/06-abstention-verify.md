# Calibrated Abstention + Faithful Span Verification

**File:** `src/handlers/recall.rs` (`abstention_decision`) · `src/handlers/verify.rs` (`verify_claim`)

## The problem

An agent memory that answers with a confident-looking *wrong* answer is worse
than one that says "I don't know." Retrieval systems must know when to refuse.
And a claim-verification step must be *faithful*: it should point at the exact
span of text that supports a statement, not gesture vaguely at a document.

## The reference

- **Calibrated abstention** — driven by a **multi-signal estimator**, not a
  magic `score < 0.3` cutoff. The signal is the existing
  `HeuristicEstimator`'s `Recommendation::ClarifyQuery` (overlap + gap +
  lexical-density agreement across retrievers). This is the roadmap-required
  form: "abstain when the evidence is genuinely ambiguous."
- **Deterministic span verification** — the honest, low-cost way to check a
  claim: case-insensitive substring match against a chunk's text with
  byte-offset match ranges.

## The implementation

1. **Abstention** (`v1.5.0`): when the estimator emits `ClarifyQuery`, `/recall`
   returns `{decision: "low_confidence", hits: []}` instead of top-1 garbage.
   Zero new compute — `confidence` + `recommendation` were already computed by
   the retrieval pass; `abstention_decision()` is a pure helper. v1.12
   (Discern) added the graph-rescue before abstaining (see `05-hub-dampening.md`).
2. **`POST /verify`** (`v1.5.0`): `{chunk_id, claim}` → `{supported, decision,
   match_ranges}`. Case-insensitive substring match over one chunk, O(content),
   no embeddings, no LLM. Bounded: `MAX_QUERY` (2000) on claim, `MAX_MATCH_RANGES`
   (100) on output. It reuses the `/get/{id}` SQL shape — one query, no new schema.

## Measured ceiling

- Abstention is **heuristic, not learned** — `ClarifyQuery` is calibrated on
  rank-agreement signals, not a judged corpus. A judged corpus
  (`brain eval --floor`) is the operator step that turns it into a measured
  claim.
- `/verify` is **lexical only** — no semantic/paraphrase match. "Faithful" means
  the span literally appears in the text, which is exactly the right guarantee
  for a verifiable memory store, and exactly the wrong tool for paraphrase.
- `/verify` records no audit row (pure read) — reads are audit-able via the
  opt-in read-event audit (v1.15).

*This is the "say 'I don't know' in a way a reviewer can verify" story from the
blog.*
