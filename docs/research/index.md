# Research

One scientific explainer per retrieval mechanism. Each follows the same honest
arc — the **problem** the paper solves, the **reference** implementation it
cites, the **deterministic** way brain-server implements it, and the **ceiling**
(built from published research, not SOTA-parity claims).

- [Bi-temporal Knowledge Graph](./01-bi-temporal.md) — validity-aware facts, `?at=` recall
- [Submodular Evidence Packing](./02-submodular-packing.md) — token-budgeted, diverse evidence
- [TRACE Typed Edges + Faithful Explanation Paths](./03-trace-edges.md)
- [Personalized PageRank Graph Retrieval](./04-ppr-graph.md) — HippoRAG-2-style
- [Noise-Aware Graph + Hub Dampening](./05-hub-dampening.md) — the Discern release
- [Calibrated Abstention + Faithful Span Verification](./06-abstention-verify.md)
- [The PRF Gate + Evidence-Faithful Snippet](./07-prf-evidence.md) — grounding the answer

Every mechanism is a deterministic implementation of *specific* published
techniques over a local store — no LLM in the retrieval loop, no data egress.
The [proof map](../trust/proof-map.md) ties each to a shipped release and a live
`curl`/`brain` verification.
