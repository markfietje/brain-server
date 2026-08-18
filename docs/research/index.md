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
- [Hybrid Fusion: RRF over BM25 + quantized vectors](./08-hybrid-fusion.md) — Cormack & Clarke RRF, Robertson & Zaragoza BM25, Jégou quantization
- [Opt-in Anticipation (the Suggest surface)](./09-anticipation.md) — Generative Agents / MemGPT / Mem0, honestly bounded
- [Structure-Aware Markdown Chunking](./10-chunking.md) — CommonMark split, Lewis 2020 RAG framing
- [Centroid Domain Auto-Routing](./11-domain-routing.md) — the nearest-centroid classifier, carving the store by domain
- [Deterministic Consolidation](./12-consolidation.md) — record-linkage duplicates/conflicts/stale-source sweep, reviewable not autonomous

Every mechanism is a deterministic implementation of *specific* published
techniques over a local store — no LLM in the retrieval loop, no data egress.
The [proof map](../trust/proof-map.md) ties each to a shipped release and a live
`curl`/`brain` verification.
