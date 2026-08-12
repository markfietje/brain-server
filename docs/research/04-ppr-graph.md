# Personalized PageRank Graph Retrieval (HippoRAG-2-style)

**File:** `src/search/graph_ppr.rs`

## The problem

Vector + lexical retrieval find a chunk that *contains* the answer, but they
cannot follow a multi-hop association ("who works at acme and reports to
carol?"). Graph retrieval walks the knowledge graph to bridge that gap — yet a
naive BFS over a noisy graph returns garbage.

## The reference

**HippoRAG 2** (`OSU-NLP-Group/HippoRAG`): a **Personalized PageRank** over the
entity graph as an additional retrieval leg, fused with the dense/lexical
results. Verified verbatim against the reference:
`igraph.personalized_pagerank(damping=0.5, directed=False, weights='weight', reset=node_weights)`.

## The implementation

`src/search/graph_ppr.rs` is a pure-Rust CSR sparse graph with power iteration,
faithful to the reference:

- `PPR_ALPHA = 0.5` (the reference's real default, not the 0.85 some drafts
  quote), `PPR_EPSILON = 1e-6`, `MAX_PPR_ITER = 50`, `MAX_VISITED = 256`.
- **No LLM, no new schema, no embeddings in the graph leg** — the `< 5 W`
  manifesto holds. Edge weight = `COUNT(DISTINCT knowledge_id)` per pair,
  scaled by relation-type (see the Discern explainer).
- Seeds = query→entity-name containment via the existing linker vocabulary;
  top entities expand back to chunks (respecting `flagged=0` / `valid_to IS
  NULL` visibility).
- Opt-in `?graph=true` as a **third RRF leg** (`RRF_K = 60`, rank-based, shared
  with the in-domain fusion) — the disabled path pays zero latency.

## Measured ceiling

- **Live multi-hop quality is corpus-bound.** On the working 8.5k-doc DB ~94%
  of KG edges are `tagged_with` taxonomy noise; the mechanism ships but the
  cleanest multi-hop paths were the synthetic bench fixture. Corpus quality is
  an operator concern (vault re-ingest with the v1.4.1 heading-hierarchy linker
  grows the semantic edge set). This drove the v1.12 "Discern" fix.
- No DPR passage scores in the seed (an embedding in the leg is out of scope);
  `PASSAGE_NODE_WEIGHT = 0.05` documents the upgrade path.
- Cross-domain graph federation is v2.0 work.

*See `02-submodular-packing.md` for how PPR output feeds the budgeted evidence
set.*
