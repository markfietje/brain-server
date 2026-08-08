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

Entities and relationships live in `entities` / `relationships` tables with **bi-temporal validity** (`valid_at` / `invalid_at`): they know *when* a fact was true.

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

The graph is bi-temporal: facts carry `valid_at` / `invalid_at`, and `/graph/traverse` accepts `?at=` to see the graph as it was at a point in time. Superseded facts are expired (`invalid_at` set), never deleted — historical queries still return them, current queries do not.

## The graph as a retrieval leg (v1.11+)

The graph isn't just queryable directly — it also powers a retrieval leg. With `?graph=true` on `/recall`, Brain Server runs **Personalized PageRank** over the graph (HippoRAG-2 style) to retrieve chunks connected to the query's entities. In v1.12 this leg became **noise-aware**: taxonomy edges (`tagged_with`, `alias_of`) weigh 0.1 vs. 1.0 for semantic types, and mega-hubs are dampened, so the real semantic paths surface instead of tag clouds.

## Self-correction (v1.6)

`supersedes` links (approved via `/consolidate`) record that a newer fact replaces an older one. This **atomically expires the prior fact**: current recall stops returning it, but historical recall (`?at=<past>`) still does. `brain resolve` / `brain undo-resolve` / `brain check-consistency` give operators the tooling to keep the graph honest.

## Next steps

- **[Retrieval & Recall](Retrieval-and-Recall)** — the graph leg in the retrieval pipeline.
- **[Architecture](Architecture)** — where the graph lives in the system.
- **[API Reference](API-Reference)** — the graph endpoints in detail.
