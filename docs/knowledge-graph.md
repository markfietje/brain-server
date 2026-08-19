# Knowledge Graph

Brain Server extracts and maintains a **knowledge graph** — entities and the relationships between them — alongside the vector and lexical indexes. This page explains how it's built, how you query it, and how it stays faithful.

## How the graph is built

The graph is built from two sources:

1. **Markdown link syntax** — `[[relation::entity]]` links in ingested markdown create directed relationships. For example:
   ```markdown
   Bignay is [[alternative_to::blueberry]]. It has [[has_property::antioxidants]].
   ```
   This creates the entities `blueberry` and `antioxidants` and the relationships `bignay --alternative_to--> blueberry` and `bignay --has_property--> antioxidants`.

2. **Explicit structured ingest** — `POST /ingest` accepts explicit `entities` and `relations`, so the caller controls the graph schema.

Entities and relationships live in `entities` / `relationships` tables with a
**four-timestamp bi-temporal model** (v1.27.22): `valid_at` / `invalid_at`
(valid time — when the fact was true in the world) plus `created_at` /
`superseded_at` (transaction time — when the store learned it and when it
stopped believing it). `superseded_at IS NULL` marks the current belief.

## Querying the graph

### Entity + one-hop relations

```bash
curl http://localhost:8765/graph/entity/bignay
```

### Relations between two entities

```bash
curl 'http://localhost:8765/graph/relations?from=alice&to=bob'
```

### Bounded traversal

```bash
curl 'http://localhost:8765/graph/traverse?start=bignay&max_depth=2'
```

The walk is bounded to depth 4 and ≤256 visited nodes, so it can never explode.

## Faithful explanations (v1.7)

With `?explain=true`, `/graph/traverse` returns **structured hop chains**, not a flat id string:

```
A --works_at--> B --ceo_of--> C
```

Each hop is `{from: {id, name}, relation, to: {id, name}}`, so a consuming agent can render the reasoning chain verbatim. The `?kind=` filter restricts the walk to edges of a specific type — exact match (`works_at`) or prefix match when it ends with `:` (`causes:` to follow the causal subgraph). Opt-in, and it never makes causal claims — a graph path is *association*, not *causation*.

## Temporal correctness

The graph is four-timestamp bi-temporal. Facts carry `valid_at` / `invalid_at`,
and `/graph/traverse` accepts `?at=` to see the graph as it was at a point in
time. When a corrected belief arrives, the superseding write sets the old
edge's `superseded_at` (transaction-time end, v1.27.22) — the old version is
**retired, never deleted**, so current reads (which filter `superseded_at IS
NULL`) return the new belief while the full history stays recoverable.

### Edge history (v1.27.22)

`GET /graph/relationships/{id}/history` (Admin, audited) returns every version
of an edge triple in order — each with its four timestamps and a `current`
flag — given any one version id. This is the read-side guarantee that
supersession never deletes: a retired belief can always be reconstructed here
even though default reads hide it.

## The graph as a retrieval leg (v1.11+)

The graph isn't just queryable directly — it also powers a retrieval leg. On `/recall`, `/search`, and `/ump/recall`, Brain Server runs **Personalized PageRank** over the graph (HippoRAG-2 style) as a **third fusion leg (vector + FTS + graph) by default** — so connected knowledge surfaces without opting in. A single multi-hop walk links related domains (e.g. VMware↔VxRail↔vSAN↔storage↔fabric), which is how an engineer in one related skill gets the connected context to resolve a related-skill case. Callers may still pass `graph=false` per-request; the process-wide kill switch is `BRAIN_RECALL_GRAPH_ENABLED=false`. In v1.12 this leg became **noise-aware**: taxonomy edges (`tagged_with`, `alias_of`) weigh 0.1 vs. 1.0 for semantic types, and mega-hubs are dampened, so the real semantic paths surface instead of tag clouds.

## Self-correction (v1.6)

`supersedes` links (approved via `/consolidate`) record that a newer fact replaces an older one. This **atomically expires the prior fact**: current recall stops returning it, but historical recall (`?at=<past>`) still does. `brain resolve` / `brain undo-resolve` / `brain check-consistency` give operators the tooling to keep the graph honest.

## Next steps

- **[Retrieval & Recall](./retrieval-and-recall.md)** — the graph leg in the retrieval pipeline.
- **[Architecture](./architecture.md)** — where the graph lives in the system.
- **[API Reference](./api.md)** — the graph endpoints in detail.
