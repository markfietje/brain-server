# Architecture

Brain Server is a single Rust binary that couples a **retrieval engine**, an
**embedding model**, a **knowledge graph**, and a **governance layer** behind a
versioned HTTP API. Everything runs in one process; the only external dependency is
an on-disk SQLite database.

```
                    ┌───────────────────────────────────────────────┐
                    │              brain-server (one process)        │
  HTTP clients ───▶ │                                               │
  (agent plugin,   │   ┌──────────┐   ┌───────────┐   ┌──────────┐  │
   brain CLI, MCP, │   │  Handlers│──▶│  Recall   │──▶│ SQLite   │  │
   Dioxus client)  │   │  (Axum)  │   │  Engine   │   │ (WAL)    │  │
                    │   └────┬─────┘   └─────┬─────┘   │  vec0    │  │
                    │        │ auth/AuthZ    │         │  FTS5    │  │
                    │        ▼               ▼         │  KG      │  │
                    │   ┌──────────┐   ┌───────────┐   └──────────┘  │
                    │   │ Audit log│   │ Static    │                 │
                    │   │ (hash    │   │ embeddings │                 │
                    │   │  chain)  │   │ (model2vec)│                 │
                    │   └──────────┘   └───────────┘                 │
                    └───────────────────────────────────────────────┘
```

---

## Retrieval engine

Recall is **hybrid**: a vector leg and a lexical leg run concurrently on independent
pooled read connections and are fused.

- **Vector leg** — `sqlite-vec` (`vec0`) KNN over embeddings. Embeddings are
  computed in-process by the static `model2vec` model; vectors are int8/binary
  quantized (4–32× smaller) for edge memory bounds.
- **Lexical leg** — SQLite FTS5 (BM25).
- **Fusion** — Reciprocal Rank Fusion (`k = 60`), a deterministic, weight-free
  merge.
- **Expansion** — deterministic PRF (pseudo-relevance feedback) expands the query
  when the top pass-1 result appears in **both** dense and lexical lists within a
  bounded rank. It fires only on cross-retriever agreement, never on a fused score
  threshold alone.
- **Graph leg (optional)** — Personalized PageRank over the knowledge graph, opt-in
  via `?graph=true`, as a third RRF leg.

Every result carries **provenance**: per-retriever ranks, the fused score, any
expansion terms, and (optionally) a rerank score.

### Abstention

When retrieval quality is too low to support a claim, `/recall` returns
`{decision: "low_confidence", hits: []}` instead of top-1 garbage. This is driven
by a calibrated multi-signal recommendation (rank overlap, gap, lexical density) —
never a magic score cutoff.

---

## Ingest pipeline

1. **Markdown / structured / memory** ingest arrives at a handler.
2. Text is **chunked** with a CommonMark-aware splitter (heading-boundary splits,
   code-fence-safe, one chunk per `knowledge` row).
3. Chunks are **embedded** by the static model and written to `vec0`.
4. Text is tokenized into FTS5.
5. `[[relation::entity]]` links (and explicit entities/relations) build the
   **knowledge graph**.
6. **Temporal stamps** (`observed_at` / `valid_from` / `valid_to` / `authority`)
   and **source provenance** (`source` + immutable `revision`) are recorded.

Ingest is governed by a **write-back gate** (v1.14): a candidate can be scored
(novelty via KNN, conflict via consolidation, salience via heuristics) and held in
a proposal queue **without creating a `knowledge` row**. It becomes memory only via
human approval.

---

## Knowledge graph

Entities and relationships live in `entities` / `relationships` tables with
bi-temporal validity (`valid_at` / `invalid_at`). `/graph/traverse` walks the graph
(bounded to depth 4, ≤256 visited) and, with `?explain=true`, returns **faithful
hop chains** (`A --works_at--> B --ceo_of--> C`) rather than a flat id string.

`supersedes` links (approved via `/consolidate`) atomically expire the prior fact:
historical recall (`?at=<past>`) still returns it, current recall does not.

---

## Governance layer

- **Append-only audit log** — a SHA-256 hash chain. Each row records the hash of
  the previous row; `/audit/verify` proves the chain is intact. Read events
  (recall/search/get) are opt-in.
- **Prompt-injection quarantine** — suspicious input is stored but excluded from
  retrieval until reviewed.
- **DSAR / GDPR** — locate → export → purge → chain-verifiable deletion
  certificate (`POST /dsar`), plus a queryable `/tombstones` registry.
- **Calibrated abstention**, **span verification** (`/verify`), and **reviewable
  proposals** keep the memory honest without an LLM.

---

## Data storage

- **SQLite** in WAL mode, with `busy_timeout` so concurrent writers queue rather
  than fail.
- **`vec0`** for quantized embeddings; **FTS5** for lexical search; relational
  tables for the knowledge graph, sources/revisions, and governance.
- **Backup/restore** — AES-256-GCM encrypted, checksummed, excludes secrets.

---

## Multi-domain

Memories can live in scoped **domain databases** (health, business, code, …), each
with its own graph. Retrieval **auto-routes** by per-domain centroids and falls back
across domains on a miss, so one domain's memory never leaks into another's answers.
This is a v1.x foundation (see [Roadmap](./roadmap.md)).

---

## See also

- [Deployment](./deployment.md) — running, configuring, and backing up.
- [Security](./security.md) — the threat model and controls.
- The [API reference](./api.md) and the full [API_CONTRACT.md](../API_CONTRACT.md).
