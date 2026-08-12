# Reference-faithful retrieval, no LLM in the loop

*2026. Deterministic retrieval is not a compromise — it's a feature.*

There's a seductive idea in the agent-memory space: make recall smart by making
it *generate*. Ask the model what's relevant, let the model decide what to
retrieve, let the model write the memory. The problem is that a model deciding
what to retrieve is a model **you can't audit and can't budget**. Every call is
a token. Every answer is a fresh coin-flip. And "why did the agent recall
this?" has an answer no reviewer can verify.

We took the other path: **deterministic, reference-faithful retrieval, with no
LLM in the loop.** Recall never has to think. A static, local embedding model
plus a deterministic pipeline answer the question — zero per-query cost, zero
data egress, zero latency on a 4 GB ARM device.

## This isn't "dumb" retrieval — it's research-grade retrieval, made deterministic

Each mechanism in the retrieval stack implements a published technique *without*
the LLM its authors used:

| Technique | Reference | Deterministic here |
|---|---|---|
| Bi-temporal facts | Graphiti (Zep) | `src/temporal.rs` marker extraction + validity filters |
| Submodular evidence packing | arXiv:2607.00725 (+5.1 F1) | lazy-greedy under a token knapsack, MMR diversity |
| Typed graph paths | arXiv:2607.00339 (TRACE) | typed hop chains, bounded BFS, `?at=` validity |
| Personalized PageRank graph leg | HippoRAG 2 | pure-Rust CSR power iteration, `damping=0.5` |
| Hub dampening + type weights | GAAMA, MemORAI | `w_ij·min(1,θ/deg)`, `tagged_with`→0.1 |
| Calibrated abstention | roadmap evidence-gating | estimator-driven `ClarifyQuery` → "I don't know" |

The key move: **take the *arithmetic*, drop the LLM.** Hub dampening is a
formula, not a model. PPR is a power iteration, not a generation call. Every
mechanism has a documented ceiling (see `docs/research/`), because a
deterministic system is one you can state the limits of — which is exactly why
it's defensible in a bakeoff.

## What you actually get

- **Reproducibility:** the same query returns the same answer, every time. You
  can pin behavior in a test, not pray it holds.
- **No token bill:** recall and writes cost nothing per query. The `< 5 W`
  manifesto is literal.
- **Verifiable provenance:** every hit carries its per-retriever rank, fused
  score, and evidence — `source_uri` + `revision_id` linking to the exact
  source revision, with byte-offset highlights *within* the revealed snippet.
  The server never fabricates a snippet.
- **An honest ceiling:** when the estimator says the query is too ambiguous,
  the system **abstains** — it says "I don't know" rather than top-1 garbage.
  (See the abstention explainer.)

## Why "no LLM in the loop" is the 2026 differentiator

Every competitor's cost is "an LLM call per query." Yours is `0`. Every
competitor's answer to "why did it recall this?" is a hand-wave. Yours is a
recorded, replayable decision path. In an era of agentic-security pressure and
per-query cost scrutiny, deterministic retrieval is not the cheap fallback — it
is the *defensible* choice.

**The takeaway:** if an agent's memory can be verified and budgeted, it can be
trusted at enterprise scale. Retrieval that generates is retrieval you pay for
every turn and can't replay. Retrieval that computes is retrieval you can pin,
audit, and run on a device you own.

*Deep dives: [`docs/research/`](../research/). The framework-agnostic story
continues in the next post.*
