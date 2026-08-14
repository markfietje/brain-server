# Structure-Aware Markdown Chunking

**File:** `src/chunker.rs` (`chunk_markdown`, `MAX_CHUNK_BYTES = 1000`)

## The problem

Retrieval quality starts at the split. Fixed-size byte chunking tears a code
path in half, splits a heading from its paragraph, and breaks the very
boundaries a hybrid retriever depends on (FTS5 phrase matches, graph
`[[relation::entity]]` extraction, heading breadcrumbs). A chunker that destroys
structure makes every downstream leg worse — before any ranking happens.

## The reference

There is **no single canonical paper** for markdown/hierarchical chunking — it is
an engineering practice, not a named algorithm. The honest, citable framing is:

- **RAG** — Lewis, Perez, Piktus, et al. (2020), *Retrieval-Augmented Generation
  for Knowledge-Intensive NLP Tasks*, NeurIPS 2020 — the architecture that made
  *chunk-then-retrieve* the standard unit.
- **Chunking-strategy trade-offs** (fixed-size vs. structure-aware) are surveyed
  in Gao et al. (2023), *Retrieval-Augmented Generation for Large Language
  Models: A Survey*, arXiv:2312.10997.
- **Hierarchical organization** appears in RAPTOR (Sarthi et al., 2024, ICLR)
  and GraphRAG (Edge et al., 2024, arXiv:2404.16130), which summarize/embed
  clustered or hierarchical text — a related lineage, though neither is
  "markdown chunking" per se.

## The implementation

`src/chunker.rs` is a **CommonMark-compliant** splitter (via `pulldown-cmark`
0.13) with three properties:

- **Structure-aware boundaries.** Chunks break at heading boundaries; the
  heading path becomes a `heading_path` breadcrumb on every chunk.
- **Atomic blocks.** Code blocks are never split mid-fence; the atomic unit is a
  block (paragraph / code block / list item / table). A byte target of
  `MAX_CHUNK_BYTES = 1000` (≈ a few hundred tokens, inside the static model's
  sweet spot) is a soft bound — hard-capped only inside an intact code block.
- **Character-preservation warranty.** Every byte of input survives verbatim
  into the chunk `text` — `#`-comments inside code fences, unicode, backticks,
  brackets. Only ATX/setext heading lines are consumed (into the breadcrumb).
  `#![deny(unsafe_code)]`; pure, allocation-only, no I/O.

This is why a hybrid retriever can trust the chunks: FTS5 matches stay
term-accurate, code paths are never torn, and the `[[relation::entity]]` scanner
sees whole text.

## Measured ceiling

- It is an engineering choice, benchmarked against fixed-size in `src/chunker.rs`
  tests — not a citable algorithm. The honest references are the RAG framing
  (Lewis 2020) and the chunking survey (Gao 2023).
- The heading split is **structural, not semantic**: it respects document
  headings but does not infer meaning-based boundaries (semantic chunking is a
  v2.x option). The static-model sweet-spot target is empirical, not proven
  optimal.

## Related

- [Hybrid Fusion: RRF over BM25 + quantized vectors](./08-hybrid-fusion.md) — the retrieval the chunks feed.
- [Knowledge graph](../knowledge-graph.md) — `[[relation::entity]]` extraction needs intact text.
- [The memory lifecycle](../memory-lifecycle.md) — markdown ingest path.