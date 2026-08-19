# Retrieval & Recall

This page explains **how Brain Server finds the right memory** — the retrieval pipeline, the fusion algorithm, query expansion, and how it stays honest when it doesn't know the answer. No LLM decides here; everything is deterministic and inspectable.

## The retrieval pipeline

Recall is **hybrid**: two retrieval legs run concurrently and are merged.

```
      query
        │
        ├────▶ Vector leg (vec0 KNN over quantized embeddings)
        │
        ├────▶ Lexical leg (FTS5 / BM25)
        │
         └────▶ Graph leg (Personalized PageRank, default-on; disable with graph=false / BRAIN_RECALL_GRAPH_ENABLED=false)
                     │
                     ▼
            Reciprocal Rank Fusion (RRF, k=60)
                     │
                     ▼
              rank + provenance
```

### 1. The vector leg

Embeddings are computed in-process by the static `model2vec` model — no transformer forward pass, just token lookup. Vectors are stored in a SQLite `vec0` table, int8/binary quantized (4–32× smaller) so the whole index stays small on edge hardware. KNN (k-nearest-neighbors) finds the closest vectors to the query embedding.

### 2. The lexical leg

The same text is indexed in SQLite FTS5 and scored with BM25 — the classic term-frequency/documents-frequency ranking. This catches exact terms, code identifiers, and phrases that a vector search might miss.

### 3. Fusion with Reciprocal Rank Fusion

Rather than trusting a single score, RRF merges the two ranked lists by **rank position**:

```
score(result) = Σ over each leg of  1 / (k + rank_in_that_leg)   where k = 60
```

This is deterministic and needs no learned weights. A result ranked #1 in both legs gets the highest fused score.

### 4. Graph leg (default-on, v1.11+)

The graph leg runs **Personalized PageRank** over the knowledge graph by default — the deterministic version of the HippoRAG-2 retrieval approach. It seeds from entities matched to the query and spreads probability mass over connected entities, then expands to the chunks those entities touch. It's fused into the same RRF merge as a third leg (vector + FTS + graph), so connected knowledge surfaces without opting in — a single multi-hop walk links related domains (e.g. VMware↔VxRail↔vSAN↔storage↔fabric). Callers may pass `graph=false` per-request; the process-wide kill switch is `BRAIN_RECALL_GRAPH_ENABLED=false`. The leg applies the same tenant/owner/scope predicates as the vector and FTS legs (domain label, access_scope, owner, PII flag carried on the hit), so enabling it never widens what a principal can read. In v1.12, this leg is noise-aware: taxonomy edges (`tagged_with`) weigh 0.1 and mega-hubs are dampened, so real semantic connections win.

### 5. PRF query expansion

PRF (pseudo-relevance feedback) expands the query with related terms — but only when the top result appears in **both** dense and lexical lists within a bounded rank. This cross-retriever agreement gate means expansion fires on genuine signal, never on a single fused score. It never injects content from quarantined rows.

## Structured query (`QueryDoc`)

`POST /recall` takes a structured query document:

```json
{
  "q": "blueberry alternative",
  "k": 5,
  "sources": ["memory", "vault"],
  "provenance": true,
  "graph": false
}
```

- **Lexical control** — a `LexSpec` with terms, quoted phrases, exclusions (`-"..."`), and exact code paths.
- **Filters** — `source`/`sources` (ingest kind: `memory` · `markdown` · `structured` · `manual` · `vault`), `since` (ISO timestamp), `domain`, `min_relevance`, `include_decayed`.
- **Provenance** — per-retriever ranks, fused score, expansion terms.

## Provenance

Every result carries `provenance`: per-retriever ranks, the fused score, and any expansion terms. With the **[Client GUI](./client-gui.md)** you can open the recall decision-path viewer to see *why* each chunk was chosen — the per-retriever ranks, fused score, relevance tier, and source. Since v1.27.12 each hit additionally carries its stored **provenance tags** — `source`, `node_kind`, `lawful_basis`, `region` — which the OpenClaw plugin renders as a `[src: · mk: · lb: · reg:]` line inside the untrusted-data fence, so the model can attribute (not just trust) each recalled item.

## Abstention: knowing when you don't know

When retrieval quality is too low to support a claim, `/recall` returns:

```json
{ "decision": "low_confidence", "hits": [] }
```

Instead of returning top-1 garbage. This is driven by a **calibrated multi-signal recommendation** (rank overlap, gap, lexical density) — never a magic `score < 0.3` cutoff. In v1.12, the graph leg can auto-engage as a "rescue pass" when the estimator says the query is ambiguous, before the server abstains.

## Span verification (v1.5)

`POST /verify` checks whether a claim is literally supported by a chunk's text — a deterministic, case-insensitive substring match over one chunk. It returns `{supported, decision, match_ranges}`. No embeddings, no LLM, no model load. This is the "show your work" endpoint.

## Decay & relevance (v1.14)

Chunks can carry `expires_at` (strict decay, default-excludes) and `min_relevance` tiers. Decayed chunks are excluded by default and surfaced via `GET /decayed` for operator review — nothing decays autonomously.

## Next steps

- **[Knowledge Graph](./knowledge-graph.md)** — the graph layer that powers the third recall leg (default-on).
- **[Architecture](./architecture.md)** — where retrieval sits in the whole system.
- **[API Reference](./api.md)** — the exact request/response contract.
