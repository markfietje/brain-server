# Hybrid Fusion: RRF over BM25 + quantized vectors

**File:** `src/search/mod.rs` (`RRF_K`, vector + FTS legs, `rrf_fuse`) ·
`src/main.rs` (`vec0` int8/binary) · `src/chunker.rs` (structure-aware split)

## The problem

A single retrieval strategy is rarely enough. Pure lexical search (BM25) finds
exact terms but misses paraphrase; pure vector search finds semantics but
misses rare, exact identifiers and code paths. Merging two ranked lists is
itself the hard part: naively averaging scores from different scales destroys
ranking quality. Brain Server fuses three legs with a single, parameter-free,
rank-based method and stores vectors in a space-efficient quantized form.

## The references

- **Reciprocal Rank Fusion (RRF).** Cormack, G. V., Clarke, C. L. A., &
  Büttcher, S. (2009). *Reciprocal Rank Fusion Outperforms Condorcet and
  Individual Rank Learning Methods.* SIGIR '09. RRF scores each document
  `1/(k + rank)` and sums across result lists — it needs only ranks, not
  scores, so it fuses lists on incomparable scales. The paper reports it
  outperforming individual systems and Condorcet/CombMNZ on TREC + LETOR.
  Brain Server uses the same constant `RRF_K = 60` (`src/search/mod.rs:29`),
  the standard value from the paper.
- **BM25 (lexical leg).** Robertson, S. E., & Zaragoza, H. (2009). *The
  Probabilistic Relevance Framework: BM25 and Beyond.* Foundations and Trends
  in IR 3(4). Brain Server's lexical leg is SQLite **FTS5** with BM25 ranking.
- **Product / scalar quantization (vector leg).** Jégou, H., Douze, M., &
  Schmid, C. (2011). *Product Quantization for Nearest Neighbor Search.* IEEE
  TPAMI 33(1). Brain Server stores vectors in **int8 and binary** quantized
  form in a `vec0` table (`vec_quantize_int8(…,'unit')` +
  `vec_quantize_binary(…)`), trading a little precision for 4–32× smaller
  storage and faster scans — the same quantization family PQ belongs to.

## The implementation

- **Vector leg** — a `vec0` KNN over int8/binary-quantized embeddings from the
  static local model (`model2vec` / `minishlab/potion-retrieval-32M`).
- **Lexical leg** — SQLite FTS5 / BM25 for exact terms, phrases, exclusions,
  and code paths.
- **Graph leg (opt-in `?graph=true`)** — Personalized PageRank, fused as a
  third RRF leg (see [Personalized PageRank](./04-ppr-graph.md)).
- **Fusion** — `rrf_fuse` sums `1/(k + rank)` across the legs with
  `RRF_K = 60`. Because RRF is rank-based, the vector and lexical scores never
  need to be normalized against each other.
- **Deterministic query expansion (PRF)** — only fires when the cross-retriever
  evidence agrees (see [The PRF Gate](./07-prf-evidence.md)), so expansion is a
  *gate*, not a blanket rewrite.
- **Structure-aware chunking** — `src/chunker.rs` splits CommonMark-aware
  (heading splits, code-fence-safe) rather than at fixed byte boundaries, so a
  code path or a heading isn't torn across chunks.

## Measured ceiling

- RRF is unsupervised and parameter-light — a strength (no tuning) and a
  ceiling (it does not learn per-query fusion weights; learned fusion is a v2.x
  option).
- int8/binary quantization reduces precision relative to float32 embeddings;
  the honest trade is storage/speed for recall at the margins.
- Structure-aware chunking is an **engineering practice**, not a single citable
  algorithm. The RAG framing that made chunk-then-retrieve standard is Lewis,
  Perez, Piktus, et al. (2020), *Retrieval-Augmented Generation for
  Knowledge-Intensive NLP Tasks* (NeurIPS 2020); chunking-strategy trade-offs
  are surveyed in Gao et al. (2023), *Retrieval-Augmented Generation for Large
  Language Models: A Survey* (arXiv:2312.10997). Brain Server's heading-aware
  splitter is its own choice, benchmarked against fixed-size in
  `src/chunker.rs` tests.

## Related

- [Personalized PageRank graph retrieval](./04-ppr-graph.md) — the third RRF leg.
- [The PRF gate + evidence-faithful snippet](./07-prf-evidence.md) — when expansion fires.
- [Bi-temporal knowledge graph](./01-bi-temporal.md) — the `?at=` filter applied across legs.
- [Retrieval & recall](../retrieval-and-recall.md) — the operator view.