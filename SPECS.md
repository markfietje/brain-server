# Brain Server — Technical Specification (SPECS)

**Scope:** This documents the actual system as built — the code, schema, retrieval pipeline, and
HTTP contract described here correspond to the current source. Forward-looking changes
are noted in release milestones.

> **Framing note.** This file is the **baseline-retrieval spec** and is kept
> accurate as a *historical/architecture* reference. The retrieval pipeline
> (§7), provenance (§7.6), and build (§2) sections are maintained current.
> The **schema (§4)** and **HTTP API (§5)** tables are a **v1.0-era snapshot**
> and are **not** the live surface — the current schema and route inventory are
> far larger and live in `docs/api.md` (routes) + `docs/API_CONTRACT.md`
> (wire shapes), with the versioned schema guarded by the
> `test_migration_schema_contract` test in `src/main.rs`. Treat §4/§5 as the
> historical baseline, not the contract.

---

## 1. Overview

Brain Server is a single-process Rust HTTP service that provides **hybrid retrieval using SQLite FTS5 and sqlite-vec (vec0) with Reciprocal Rank Fusion (RRF), adaptive retrieval-quality assessment, and optional pseudo-relevance feedback (PRF)** plus a
**knowledge graph** over a local SQLite database, intended as a long-term "second brain" for an
AI agent running on a Jetson Nano (4 GB RAM, ARM Cortex-A57).

- **Embeddings:** static (no neural net) via `model2vec` / `minishlab/potion-retrieval-32M`. Stored as int8-quantized vectors in `vec0` with binary bit vectors for archive tier.
- **Lexical index:** SQLite FTS5 (`porter unicode61` tokenizer) on title + content.
- **Fusion:** Reciprocal Rank Fusion (RRF, `k=60`) merges vec0 KNN and FTS5 BM25 ranks.
- **Graph retrieval (v1.12.0 "Discern"):** noise-aware third RRF leg —
  deterministic Personalized PageRank over the existing `entities`/`relationships`
  KG (`?graph=true` on `/search`/`/recall`; opt-in, disabled path adds zero
  latency). Edge-type weights (`tagged_with`/`alias_of` → 0.1, semantic types
  → 1.0) + GAAMA-style per-source hub dampening (`w_ij·min(1, θ/deg(i))`,
  θ=50) counter the taxonomy-heavy KG; complexity-gated auto-activation
  (v1.5.0 `ClarifyQuery` → one bounded graph-augmented rescue pass,
  `BRAIN_GRAPH_RESCUE_ENABLED` kill switch). Query→entity seeding via exact
  entity-name containment; seed→chunk expansion via
  `relationships.knowledge_id`. No LLM, no embeddings in the graph leg.
- **Quality assessment:** Heuristic estimator computes overlap, gap, reciprocal rank, lexical density → emits `Recommendation` (`Return | RunPrf | RunReranker | IncreaseTopK | ClarifyQuery`).
- **Optional PRF:** When confidence is moderate, top-K vector hits expand the query with high-weight FTS terms; re-search fused with original via RRF.
- **Storage:** embedded SQLite (WAL), one database file.
- **Interface:** Axum HTTP JSON API on loopback. Consumed via the `brain` CLI, MCP, or HTTP clients.

```
┌────────────────────────────────────────────────────────────────────┐
│  Axum 0.8 HTTP  ──►  r2d2 pool (SQLite, WAL)                        │
│                        │                                            │
│   model2vec            ▼                                            │
│   potion-retrieval-32M ─► knowledge, embeddings (vec0:int8+bit),    │
│   (static, shared)       fts5, entities, relationships             │
│                                                                     │
│   Search pipeline:                                                 │
│   Query → Embed → [vec0 KNN] ──┐                                    │
│              → [FTS5 BM25] ────┼──► RRF (k=60)                      │
│                    │           ▼                                    │
│              ┌──────┴──────┐                                        │
│              ▼             ▼                                        │
│         QualityEstimator → Recommendation                          │
│              │                                                    │
│              ├── Return                                            │
│              ├── RunPrf → expand → re-search → RRF                │
│              ├── RunReranker → high-confidence, no refinement    │
│              ├── IncreaseTopK                                      │
│              └── ClarifyQuery                                      │
└────────────────────────────────────────────────────────────────────┘
```

---

## 2. Package & Dependencies

From `Cargo.toml` (`name = "brain-server"`, `version = "1.27.22"`, `edition = "2021"`):

| Purpose | Crate | Version |
|---|---|---|
| Embeddings (default) | `model2vec-rs` | `0.1.4` |
| Embeddings (neural, optional) | `fastembed-rs` | optional — pulled only by `neural-embed` / `rerank-tier` |
| DB | `rusqlite` (feature `bundled`) | `0.40.1` |
| Pool | `r2d2` / `r2d2_sqlite` | `0.8.10` / `0.35.0` |
| HTTP | `axum` | `0.8.9` |
| HTTP engine | `hyper` | `1.10.1` |
| CORS / middleware | `tower-http` (feature `cors`) | `0.6.11` |
| Runtime | `tokio` (feature `full`) | `1.53.0` |
| Serde | `serde` / `serde_json` | `1.0.229` / `1.0.150` |
| Util | `anyhow`, `xxhash-rust` (`xxh3`), `sha2`, `chrono`, `dirs`, `sysinfo` | latest |
| Annotator deps | `regex`, `toml`, `log` | `1.11` / `0.8` / `0.4` |
| Tracing | `tracing` / `tracing-subscriber` (`env-filter`) | `0.1` / `0.3` |
| Dev | `tempfile` | `3` |

**Release profile:** `opt-level = 2` (speed), `lto = "fat"`, `codegen-units = 1`, `strip = true`,
`panic = "abort"` (all transitive packages also `opt-level = 2`). This is well-tuned for the
warm-speed/ARM balance on the shipped binaries.

---

## 3. Configuration & Constants

All tunables live in `src/config.rs`. **`#![allow(dead_code)]`** is set there — some constants
below are *defined but not actually used* by the code path they name. Flagged inline.

| Constant | Value | Actually used? |
|---|---|---|
| `MODEL_ID` | `"minishlab/potion-retrieval-32M"` | ✅ |
| `SERVER_VERSION` | `env!("CARGO_PKG_VERSION")` | ✅ now driven from `Cargo.toml` |
| `DEFAULT_K` / `MAX_K` | `5` / `100` | ✅ |
| `MAX_REQUEST_SIZE` | 1 MiB | ✅ (also re-checked inline in handler) |
| `MAX_QUERY_LENGTH` | 2000 | ✅ |
| `REQUEST_TIMEOUT_SECS` | 30 | ✅ (per-request timeout) |
| `SEARCH_TIMEOUT_SECS` | 8 | ✅ |
| `SHUTDOWN_DRAIN_SECS` | 60 | ✅ |
| `POOL_MAX_SIZE` / `POOL_MIN_IDLE` | 20 / 2 | ⚠️ defined but the pool is built with literal `20` / `2` in `main()` |
| `POOL_*_SECS` (conn/lifetime/idle) | 30 / 300 / 60 | ⚠️ same — literals in `main()` |
| `CONTENT_MAX_LENGTH` / `TITLE_MAX_LENGTH` | 1,000,000 / 500 | ✅ (enforced inline) |
| `CONNECTION_WATCHDOG_*` | 30 / 300 | ✅ |
| `ENTITY_NAME_MAX_LENGTH` | 100 | ⚠️ defined; entity insertion does not enforce length |
| `TRAVERSE_MAX_DEPTH` | 3 | ✅ |
| `CORS_DEFAULT_ORIGINS/METHODS/HEADERS` | localhost:3000,8080 / GET,POST,PUT,DELETE,OPTIONS / content-type,authorization | ❌ **not used** — see §6 (CORS hardcoding) |
| `CORS_MAX_AGE_SECS` | 3600 | ❌ not used |

### Environment variables

| Variable | Default | Effect | Notes |
|---|---|---|---|
| `BIND_HOST` | `127.0.0.1` | Bind address | Invalid value falls back to `0.0.0.0` (open!) |
| `BIND_PORT` | `8765` | Listen port | Non-numeric falls back to `8765` |
| `RUST_LOG` | `info` | tracing filter | |
| `BRAIN_WORKER_THREADS` | number of cores | tokio multi-thread runtime worker count (v1.3.0). Jetson target = `2` to save ~10 MB RSS + context-switch overhead; unset = cores. | Ignored if ≤ 0 |
| `ANNOTATOR_ENABLED` | — | **documented but ignored** | The annotator is constructed with `enabled: true` unconditionally in `main()` (see §8) |
| `CORS_ORIGINS` / `CORS_METHODS` / `CORS_HEADERS` | — | **documented but ignored** | CORS is hardcoded `Any` (see §6) |

> **No env override for the DB path or domains dir.** Both are hardcoded to a default
> workspace directory.

---

## 4. Database Schema

Single file at `brain.db` in the default workspace directory (parent dir auto-created, configurable via `BRAIN_DB_PATH`).

Connection PRAGMAs (set at migration): `journal_mode=WAL`,
`synchronous=NORMAL`, `foreign_keys=ON`, `cache_size=-64000` (64 MB), `temp_store=MEMORY`.

### `knowledge`
```sql
CREATE TABLE knowledge (
  id              INTEGER PRIMARY KEY,
  title           TEXT,
  content         TEXT NOT NULL,
  knowledge_type  TEXT,
  source          TEXT DEFAULT 'manual',
  content_hash    TEXT,            -- xxh3-64 hex (16 chars); dedup key
  created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  flagged         INTEGER NOT NULL DEFAULT 0,   -- v0.9.1: quarantine guardrail
  domain          TEXT NOT NULL DEFAULT 'global', -- v0.9.1: domain isolation
  observed_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP, -- v0.9.1: temporal memory
  valid_from      TIMESTAMP,                   -- v0.9.1: temporal validity
  valid_to        TIMESTAMP,
  document_id     TEXT,                        -- v0.9.1: structure-aware chunking
  chunk_index     INTEGER,
  heading_path    TEXT,
  line_start      INTEGER,
  line_end        INTEGER,
  source_path     TEXT                         -- v0.9.2: vault ingest provenance
);
CREATE UNIQUE INDEX idx_knowledge_hash ON knowledge(content_hash);
CREATE INDEX idx_knowledge_source_path ON knowledge(source_path);```
```

### `knowledge_fts` — FTS5 full-text index
```sql
CREATE VIRTUAL TABLE knowledge_fts USING fts5(
  title, content, content_hash UNINDEXED,
  content='knowledge', content_rowid='id', tokenize='porter unicode61'
);
```
Triggers on `knowledge` (`AFTER INSERT/UPDATE/DELETE`) keep FTS5 in sync. The `content_hash`
column is `UNINDEXED` so it's stored but not tokenized.

### `knowledge_fts_vocab` — FTS5 vocabulary (instance mode) for PRF
```sql
CREATE VIRTUAL TABLE knowledge_fts_vocab USING fts5vocab(
  knowledge_fts, 'instance'
);
```
Exposes one row per (term, document, column) with `cnt` (occurrence count). PRF query expansion
joins this against top-K rowids to rank expansion terms by corpus-weighted frequency
(BM25-style signal), replacing the naive in-memory DF heuristic.

### `vec_knowledge` — sqlite-vec `vec0` quantized vector store
```sql
CREATE VIRTUAL TABLE vec_knowledge USING vec0(
  knowledge_id INTEGER PRIMARY KEY,
  embedding_bit  BIT[512],       -- binary tier (archive/first-pass)
  embedding_int8 INT8[512],      -- int8 tier (default search)
  source       TEXT,             -- metadata column (enables filter pushdown)
  created_at   TEXT              -- metadata column (enables filter pushdown)
);
```
- **Distance metric:** `cosine` (required — `vec0` defaults to L2; cosine is set at creation).
- **Quantization:** `model.encode() → f32[512]` → both `vec_quantize_int8(..., 'unit')` and
  `vec_quantize_binary(...)`. Raw `f32` never enters the hot path.
- **Migration:** Legacy `embeddings(vector TEXT)` JSON rows are backfilled once into `vec0`;
  parity is verified, then the old column is dropped in a follow-up release.
- **Metadata columns** (`source`, `created_at`) enable metadata-filtered KNN
  (`WHERE source = 'health' AND created_at > :since`).

> **Historical note:** Prior to v0.9.3 the server stored JSON vectors in
> `embeddings.vector` and performed brute-force cosine scans. This was replaced by the
hybrid FTS5 + vec0 retrieval architecture.

### `entities`
```sql
CREATE TABLE entities (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
  entity_type TEXT,
  created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_entities_name ON entities(name);
CREATE INDEX idx_entities_type ON entities(entity_type);
```

### `relationships`
```sql
CREATE TABLE relationships (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  from_entity_id INTEGER NOT NULL,
  to_entity_id   INTEGER NOT NULL,
  relation_type  TEXT NOT NULL,
  knowledge_id   INTEGER,
  created_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY(from_entity_id) REFERENCES entities(id) ON DELETE CASCADE,
  FOREIGN KEY(to_entity_id)   REFERENCES entities(id) ON DELETE CASCADE,
  FOREIGN KEY(knowledge_id)   REFERENCES knowledge(id) ON DELETE SET NULL
);
CREATE INDEX idx_rels_from ON relationships(from_entity_id);
CREATE INDEX idx_rels_to   ON relationships(to_entity_id);
CREATE UNIQUE INDEX idx_rels_unique ON relationships(from_entity_id, to_entity_id, relation_type);
```

---

## 5. HTTP API

Bound to `BIND_HOST:BIND_PORT` (default `127.0.0.1:8765`). All routes are layered with the
(global) CORS layer and share an `Arc<AppState>`.

| Method | Path | Handler | Notes |
|---|---|---|---|
| GET | `/health` | `health` | liveness |
| GET | `/health/db` | `health_db` | DB round-trip check |
| GET | `/ready` | `ready` | readiness (model + DB) |
| GET | `/stats` | `stats` | counts + model + version |
| GET | `/version` | `version` | ✅ returns `env!("CARGO_PKG_VERSION")` (now `1.4.0`) |
| POST | `/add` | `add_chunk` | text ingest (raw), embeds + stores |
| POST | `/ingest/memory` | `ingest_memory` | structured memory ingest |
| GET | `/search?q=&k=` | `search` | semantic search (brute-force cosine) |
| POST | `/v1/embeddings` | `embeddings` | OpenAI-compatible embeddings endpoint |
| POST | `/ingest/markdown` | `ingest_markdown` | markdown ingest + annotation extraction |
| GET | `/graph/entity/{name}` | `get_entity` | entity + 1-hop relations |
| GET | `/graph/relations?from=&to=` | `get_relations` | relations between entities |
| GET | `/graph/traverse?start=&max_depth=` | `traverse_graph` | recursive graph walk (≤ `TRAVERSE_MAX_DEPTH`) |
| GET | `/audit?kind=&tenant=&limit=` | `list_audit` | operator audit-log diagnostics (hashes only); `tenant` filters at the SQL layer (v1.1.0) |
| GET | `/audit/verify` | `verify_audit_chain` | v1.1.0 — returns `{ ok: bool }` after walking the SHA-256 hash chain |
| GET | `/metrics` | `metrics` | v1.1.0 — Prometheus text-format exporter (no dep) |

### Request/response shapes (selected)

`POST /add`:
```json
{ "text": "...", "title": "...", "source": "manual" }
```
`{ "source": "manual" }` default via `default_source()`. Embedding generated server-side;
content hashed with xxh3-64; duplicates short-circuit (`status: "duplicate"`).

`GET /search?q=&k=` → `{ results: [{ id, score, title, content, provenance }] }`, `k` defaults to 5, capped at 100.

`provenance` object per result:
```json
{
  "source": "vector" | "fts" | "both",
  "vector_rank": 0,
  "fts_rank": 1,
  "fused_score": 0.042,
  "rerank_score": 0.91,
  "rerank_truncated": false,
  "prf_expanded": false,
  "top_retrieval_mode": "both",
  "retrieval_strategy": "hybrid_prf",
  "quality_assessment": { "version": 1, "confidence": {...}, "recommendation": "run_reranker" },
  "prf_decision": "expanded"
}
```

`POST /v1/embeddings` (OpenAI-compatible):
```json
{ "input": "text" | ["a","b"], "model": "minishlab/potion-retrieval-32M" }
```
→ `{ object: "list", data: [{ object: "embedding", embedding: [...], index }], model, usage }`.

`POST /ingest/markdown`:
```json
{ "title": "required", "content": "max 1MB" }
```
Extracts annotations (inline `[[rel::entity]]` + TOML domain engine), embeds content, inserts
knowledge + entities + relationships. Caps: title ≤ 500, content ≤ 1,000,000.

---

## 6. CORS  ✅ env-driven (v0.9.0+)

The router builds CORS from `config::cors_origins/methods/headers()` which read
`CORS_ORIGINS` / `CORS_METHODS` / `CORS_HEADERS` env vars with a **loopback-only fallback**
(defaults: `localhost:3000,localhost:8080` / `GET,POST,PUT,DELETE,OPTIONS` / `content-type,authorization`).

```rust
let cors = CorsLayer::new()
    .allow_origin(AllowOrigin::predicate(move |origin, _| {
        origin.to_str().map(|o| origins.iter().any(|a| a == o)).unwrap_or(false)
    }))
    .allow_methods(methods.iter().filter_map(|m| m.parse().ok()).collect::<Vec<_>>())
    .allow_headers(headers.iter().filter_map(|h| h.parse().ok()).collect::<Vec<_>>())
    .max_age(Duration::from_secs(config::CORS_MAX_AGE_SECS));
```

Non-loopback origins are rejected unless the deployer explicitly sets `CORS_ORIGINS`.

---

## 7. Retrieval Architecture (Baseline Retrieval v1.0)

### 7.1 Overview

Hybrid retrieval pipeline combining semantic (vec0) and lexical (FTS5) search with adaptive
quality assessment and optional expansion/rerank tiers.

```
Query
  │
  ├─► Embed (model2vec static, 512-d)
  │
  ├─► vec0 KNN (cosine on int8[512])  ──┐
  │                                     ├─► RRF (k=60)
  └─► FTS5 BM25 (porter unicode61) ─────┘       │
                      │                         ▼
                      ▼              ┌───────────────────────┐
                      │              │ RetrievalQualityEstim │
                      ▼              │  (HeuristicEstimator) │
              ┌───────────────┐      │  overlap, gap, RR,    │
              │ Recommendation│      │  lexical_density      │
              └───────────────┘      └───────────────────────┘
                      │                         │
          ┌───────────┼───────────┬─────────────┼──────────────┐
          ▼           ▼           ▼             ▼              ▼
      Return    RunPrf    RunReranker   IncreaseTopK      ClarifyQuery
      (top-k)   (expand   (cross-encoder           (wider
               query →    on candidate           candidate
               re-search)  window)                window)

```

### 7.2 Pipeline Stages

| Stage | Implementation | Key Parameters |
|---|---|---|
| **Embed** | `model2vec-rs` static encoding | 512-d, `spawn_blocking`, 30s timeout |
| **vec0 KNN** | `sqlite-vec` `vec0` virtual table | `embedding_int8` (cosine), `embedding_bit` (archive), metadata columns `source`, `created_at` for filter pushdown |
| **FTS5 BM25** | SQLite FTS5 `knowledge_fts` | `porter unicode61` tokenizer, triggers sync with `knowledge` table |
| **RRF Fusion** | `rrf_fuse()` in `search/mod.rs` | `RRF_K = 60`, `RRF_OVERFETCH = 200` |
| **Quality Assessment** | `HeuristicEstimator` in `search/quality.rs` | See §7.3 |
| **PRF Expansion** | `prf_extract_terms_fts()` + `fuse_prf_passes()` | `PRF_DEPTH` (default 30), `PRF_TERMS` (default 8), env-tunable via `PrfConfig::from_env()` |

### 7.3 Retrieval Quality Estimation

`HeuristicEstimator` computes four signals from hybrid results:

| Signal | Computation |
|---|---|
| **Overlap** | Fraction of top-k results with both `vector_rank` and `fts_rank` present |
| **Gap** | Normalized score difference: `(score@1 - score@2) / score@1` |
| **Reciprocal Rank** | `1 / (1 + min(vector_rank, fts_rank))` of best result |
| **Lexical Density** | Query term coverage in top result snippet/content |

Weighted combination → `Confidence.score` ∈ [0,1]. Maps to `Recommendation`:

| Confidence | Recommendation | Trigger |
|---|---|---|
| ≥ `rerank_threshold` (0.85) | `RunReranker` | Cross-encoder can refine ordering |
| ≥ `confidence_threshold` (0.6) | `RunPrf` | Expand query with PRF terms |
| ≥ 0.35 | `IncreaseTopK` | Widen candidate window |
| < 0.35 | `ClarifyQuery` | Ask user to reformulate |
| Overlap < `agreement_min`/10 | `IncreaseTopK` | Hard gate: low vector/lexical agreement |
| Gap < `gap_threshold` (0.023) | `RunPrf` | Hard gate: small top-1/top-2 gap |

Configurable via env (`QUALITY_*`) — see `QualityConfig` in `config.rs`.

### 7.4 PRF (Pseudo-Relevance Feedback)

When `Recommendation::RunPrf`:
1. Top-`PRF_DEPTH` results from pass 1 joined against `knowledge_fts_vocab` (instance mode)
2. Terms ranked by corpus-weighted frequency (BM25-style)
3. Top `PRF_TERMS` appended to original query
4. Re-search with expanded query → fused with pass 1 via deterministic RRF (`fuse_prf_passes`)
5. Original-query matches protected from demotion

### 7.5 Optional Cross-Encoder Rerank — removed in v0.9.5, re-added as an opt-in tier in v1.20.30

The rerank tier was **deleted in v0.9.5** (`3fcac72`): the BGE cross-encoder pegged the M1 CPU
and blew the 8s recall timeout, and was too heavy for the Jetson edge GPU. The `rerank`
Cargo feature and `src/search/rerank.rs` were removed, not stubbed.

**Current state (v1.20.30+, retuned post-v1.27.25):** rerank is an **opt-in tier**, off by
default (the `rerank-tier` Cargo feature; the server arms it at boot — sets `BRAIN_RERANK_ENABLED=1` —
when the active `MODEL_PROFILE` is `enterprise`, `desktop`, or `quality-local`). The default build
(edge/Jetson) stays on the static potion model **with no rerank**.

`src/search/rerank.rs` loads, in order of preference:
1. **`mixedbread-ai/mxbai-rerank-large-v1`** — the golden pick (Apache-2.0,
   DeBERTa-v3-large, ~435M params, single-label cross-encoder → `logits[:, 0]`).
   Not in the FastEmbed in-enum registry, so it is
   loaded through the BYO-ONNX `UserDefinedRerankingModel` seam from a local dir (default
   `models/mxbai-rerank-large-v1/`, override `BRAIN_RERANK_MODEL_DIR`), using the official int8
   `onnx/model_quantized.onnx`.
2. **`BAAI/bge-reranker-v2-m3`** — the in-enum fallback (FastEmbed `TextRerank` +
   `RerankerModel::BGERerankerV2M3`) when the mxbai files are absent or fail to load, so the tier
   never fails to boot.

It is **fail-open** (a model/output fault leaves the RRF order untouched, `rerank_score = None`)
and **boot-warmed** (`search::rerank::warmup()` force-loads at boot so the first recall never pays
the download in the request path). Top-N is `BRAIN_RERANK_TOP_N` (default 50). `Qwen3-Reranker-0.6B`
and `mxbai-rerank-large-v2` are deliberately **not** wired: they are causal-LM (ChatML + last-token
logit scoring), architecturally incompatible with fastembed's `(query, doc)` → `logits[:, 0]`
rerank seam — they would load, run, and return meaningless scores (v1.30's ColBERT rerank, and
real LLM runtimes, are the paths that can consume them). Neural tiers (`neural-embed`,
`rerank-tier`) are separate features. See `IMPLEMENTATION_PLAN_v1.20.30_Caliber.md`.

The API fields `rerank_score` / `rerank_truncated` / `rerank_ms` are retained for contract
stability (always `null` / `false` / `0` unless the rerank tier is active).

Historical record (what §7.5 documented before removal):

Behind `cfg(feature = "rerank")` + `RERANK_ENABLED=true`:
- Candidate window: `max(k, RERANK_CANDIDATES)` = 30
- Documents truncated to `RERANK_MAX_CHARS = 4096`
- `fastembed-rs` `TextRerank` with `RerankerModel::BGERerankerV2M3`
- Fail-open: any error → returns unreranked results, status logged via `RerankStatus`
- Observable via `/stats` and `SearchTelemetry.rerank_ms`

### 7.6 Provenance & Observability

Every `SearchResult` carries `Provenance`:

```rust
pub struct Provenance {
    pub vector_rank: Option<usize>,
    pub fts_rank: Option<usize>,
    pub fused_score: Option<f32>,
    pub rerank_score: Option<f32>,
    pub rerank_truncated: bool,
    pub prf_expanded: bool,
    pub top_retrieval_mode: Option<SearchSource>,
    pub retrieval_strategy: Option<RetrievalStrategy>,
    pub quality_assessment: Option<RetrievalAssessment>,
    pub prf_decision: Option<PrfDecision>,
}
```

Per-request `SearchTelemetry` (returned when `provenance=true`):

```rust
pub struct SearchTelemetry {
    pub embed_ms: f32,
    pub vector_ms: f32,
    pub fts_ms: f32,
    pub fusion_ms: f32,
    pub prf_ms: f32,
    pub rerank_ms: f32,
    pub vec_candidates: usize,
    pub fts_candidates: usize,
    pub fused_count: usize,
    pub rrf_k: u32,
    pub intent: Option<String>,
    pub embedding_query: Option<String>,
    pub retrieval_ms_vec: f32,
    pub retrieval_ms_fts: f32,
    pub confidence: f32,
    pub recommendation: Option<Recommendation>,
}
```

---
- **Graceful shutdown:** `axum::serve(...).with_graceful_shutdown(...)` listens for SIGINT/SIGTERM,

---

## 8. Knowledge Graph & Annotation (inline scanner only)

The KG (`entities`/`relationships`) is populated at ingest from a **single source**:

1. **Inline `[[relation::entity]]` syntax** — `parse_annotations()` in `main.rs`, a hand-rolled
   byte scanner over the markdown body. **Always active.** Only `[A-Za-z0-9_-]` relation/entity
   names are accepted; `[[` … `::` … `]]`; the `from` entity is the lowercased title.
   - Also used by `POST /ingest/markdown` (v0.9.2+) which additionally extracts:
     - **Wikilinks** `[[Target]]` → `references` edges (note → note)
     - **Frontmatter `tags`** → `tagged_with` edges
     - **Frontmatter `aliases`** → `alias_of` edges (alias → note)

2. **Structured ingest** — `POST /ingest` with explicit `entities[]` / `relations[]` arrays
   (the primary KG write path since v0.9.0; see `API_CONTRACT.md` §3).

> **v0.9.0:** the TOML domain engine (`src/annotator/`) was removed entirely. It was already
> a no-op on default deploys (no configs → disabled fallback). Domain-specific extraction
> is now the caller's responsibility via structured ingest.

---

## 9. Reliability & Process Lifecycle

- **Pool:** r2d2, `max_size(20)`, `min_idle(Some(2))`, conn timeout 30 s, max lifetime 300 s,
  idle timeout 60 s, `test_on_check_out(false)`.
- **Pool health check:** a `tokio::spawn` loop pings `SELECT 1` every 30 s.
- **Connection leak detection:** `ConnectionTracker` assigns each acquired connection an id +
  timestamp; `spawn_connection_watchdog` logs long-running acquisitions (threshold 300 s).
- **Rate limiter:** simple in-memory per-IP window (`RateLimiter`, 100 req/window in tests).
- **Graceful shutdown:** `axum::serve(...).with_graceful_shutdown(...)` listens for SIGINT/SIGTERM,
  then drains for `SHUTDOWN_DRAIN_SECS` (60 s) before exiting. (Note: it sleeps the full drain
  window unconditionally — does not exit early once in-flight requests finish. See Phase 5.)

---

## 10. Security Posture (current)

- **Authentication is on by default in modern releases.** The v0.9.0-era "no
  authentication, loopback bind" baseline below is **historical**. Current posture:
  bearer token auth (`AUTH_TOKEN_FILE` → `AUTH_TOKEN`, 0600 secret),
  **JWT/JWS** verification (RS256/ES256/EdDSA, alg whitelist, `(jti, iss)`
  revocation, refresh-chain reuse detection), a deny-by-default **AuthZ** layer,
  per-domain capability tokens, OIDC/JWKS discovery, role-based postures
  (`admin`/`solo`/`controller`/`dpo`/`qa`/`agent`/`client-auditor`/`bpo-ops`),
  fail-closed identity (`auth::TokenRead`, poisoned store = 500), and per-IP rate
  limiting. The default loopback bind is a safety default, **not** the security
  boundary — auth gates every non-loopback surface.
- **Prompt-injection pattern detector:** `contains_suspicious_pattern()` rejects inputs
  containing `"ignore previous"`, `"system:"`, `"you are now"`, `"### instruction"`,
  `"### system"`, `"def "`, `"import "`, `"exec("`, `"eval("` (case-insensitive). Applied to
  ingest/search titles and content.
- **HTML escaping** of titles before storage (`html_escape`).
- **Size caps:** content ≤ 1 MB, title ≤ 500 chars, query ≤ 2000 chars.
- **CORS:** env-driven with loopback-only fallback (§6) — non-loopback origins rejected unless
  `CORS_ORIGINS` is explicitly set.
- **No TLS** termination in-process (assumed handled by a gateway/reverse proxy).

v0.9.0+/v1.1.0 add bearer auth, real origin allowlist, per-domain capability tokens, and an
audit log. v1.2.0 adds JWT/JWS verification (RS256/ES256/EdDSA, alg whitelist, `(jti, iss)`
revocation, refresh-chain reuse detection) + a deny-by-default AuthZ layer + OIDC/JWKS
discovery. v1.3.0 "Bedrock" hardens the binary itself: zero `unwrap`/`expect`/`panic!` in
production paths, every `unsafe` block documented with a `// SAFETY:` comment, and a
`hardening` object on `/health` exposing the memory-safety posture (`unsafe_blocks`,
`panics_caught`, `memory_leaks_detected`). v1.20.24+ fails closed on misconfigured secrets;
v1.27.16 + v1.27.21 close the read/identity fail-open gaps (see `CHANGELOG.md`).

## 11. Known Issues / Debt (carried into ROADMAP Phase 0)

1. ~~`SERVER_VERSION` hardcoded `"0.8.1"` ≠ `Cargo.toml` `0.8.6` → `/version` lies.~~ ✅ **Fixed in v0.9.0** — now `env!("CARGO_PKG_VERSION")`.
2. ~~CORS hardcoded `Any`; `CORS_*` env vars and constants unused.~~ ✅ **Fixed in v0.9.0** — env-driven with loopback-only fallback.
3. ~~`ANNOTATOR_ENABLED` env var documented but not consulted.~~ ✅ **Fixed in v0.9.0** — TOML annotator module removed entirely.
4. ~~`TRAVERSE_MAX_DEPTH` constant defined but unused (handler uses literal `min(3)`).~~ ✅ **Fixed in v0.9.0** — dead constant removed; literal remains in handler.
5. ~~Vectors stored as JSON text (the central perf problem).~~ ✅ **Fixed in v0.9.3** — migrated to `vec0` int8 + binary quantized.
6. ~~Brute-force in-RAM cosine scan, re-deserializing every row per query.~~ ✅ **Fixed in v0.9.3** — replaced by `vec0` KNN + FTS5 BM25 hybrid with RRF.
7. ~~Graceful-shutdown drain sleeps the full window unconditionally.~~ ✅ **Fixed in v0.9.4** — removed hard sleep; axum now waits for in-flight requests to complete naturally.

> **Historical note:** Items 6–7 described the pre-v0.9.3 architecture (JSON vectors +
> brute-force cosine). The current Baseline Retrieval v1.0 uses hybrid FTS5 + vec0 with
> adaptive quality assessment, optional PRF, and optional cross-encoder rerank.

---

## Retrieval Architecture Policy

**Baseline Retrieval v1.0 is considered stable.** The hybrid FTS5 + vec0 + RRF + quality
assessment + optional PRF/rerank pipeline is the reference architecture.

Future retrieval changes must be validated through:

- **Benchmark improvements:** `cargo bench` showing latency/throughput delta
- **Calibration:** Quality estimator recommendations match ground-truth relevance
- **Latency regression testing:** p50/p95/p99 within tolerance on target hardware (Jetson Nano)
- **CI comparison:** Automated `cargo eval` gate (see §Evaluation)

Architecture changes require updating `benchmarks/retrieval-v1/` baseline.

---

## Evaluation & Benchmark Policy

### `crates/eval` (planned)

Dedicated evaluation crate with:

```bash
cargo eval
```

Produces:

| Metric | Target |
|---|---|
| Recall@10 | ≥ 0.85 |
| nDCG@10 | ≥ 0.75 |
| MRR | ≥ 0.70 |
| Latency p50 | ≤ 50 ms |
| Latency p95 | ≤ 150 ms |
| Calibration (ECE) | ≤ 0.10 |
| Recommendation distribution | Logged per query |

### Calibration

`HeuristicEstimator` confidence scores must be calibrated against held-out relevance judgments.
Expected calibration error (ECE) tracked in CI.

### Recommendation Distribution

Per-query `Recommendation` logged (`Return`, `RunPrf`, `RunReranker`, `IncreaseTopK`, `ClarifyQuery`)
to detect drift (e.g., sudden spike in `ClarifyQuery` indicates index/retrieval degradation).

---

## 12. Build & Deploy

```bash
# Rust + Axum release build
RUSTFLAGS="-C target-cpu=native -C opt-level=3 -C codegen-units=1" cargo build --release
./target/release/brain-server
```

CI (`.github/workflows/ci.yml`): `cargo fmt --check`, `cargo clippy --all-targets --features bench -- -D warnings`,
`cargo test --features bench`, `cargo audit`.
