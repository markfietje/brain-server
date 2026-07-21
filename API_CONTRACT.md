# Brain Server — API Contract (`/recall` + `/ingest`)

> **Wire contract** for the brain-server HTTP API. The JSON shapes here are the
> source of truth; the Rust `serde` structs are kept equal to these shapes.
>
> **Status:** `/recall` and `/ingest` are both **implemented and live** in the
> current source (see `src/handlers/recall.rs`, `src/handlers/ingest.rs`). They
> supersede the legacy `/search` and `/ingest/markdown`; the legacy
> endpoints remain for direct/CLI compatibility (documented in `README.md` and
> `SPECS.md`, out of scope here).
>
> **Versioning:** the server reports `SERVER_VERSION = env!("CARGO_PKG_VERSION")`
> via `/version` and `/health`, and sets an
> `X-Api-Version: <semver>` response header on **every** route. Contract
> version: `api v1`.

---

## Versioning & deprecation policy

Applies from v0.9.5 ("Inspect" M3) onward, before third parties depend on the
API surface.

- **Version discovery.** Every response carries `X-Api-Version: <semver>`
  (the crate version from `Cargo.toml`). Clients SHOULD log/record it; a major
  bump (`1.x` → `2.x`) signals a breaking wire change.
- **Structured queries.** The canonical query contract is the `QueryDoc`
  (see `src/search/query.rs` and `openapi.yaml#/components/schemas/QueryDoc`),
  sent to `POST /recall`. The legacy `GET /search` (flat `q`/`lex`/`source`) and
  `POST /add` remain functional but are **deprecated**.
- **Deprecation signal.** Deprecated routes return an RFC 8594 `Deprecation`
  header (e.g. `Deprecation: version="0.9.5"`). The header names the version in
  which the route entered deprecation, *not* the version it will be removed.
  Removal only happens on a major-version boundary, and only after a minimum of
  one minor release of overlap with the replacement route.
- **Migration mapping.**
  | Deprecated | Replacement |
  |---|---|
  | `GET /search?q=...` | `POST /recall` with `QueryDoc` (structured `lex`, `sources`, `intent`, `explain`) |
  | `POST /add` | `POST /ingest/memory` (raw body) or `POST /ingest/markdown` (with title) |
- **Stability promise.** Within a major version, existing response *shapes* are
  additive (new optional fields only). A removed field or changed type is a
  breaking change and requires a major bump.

> The full machine-readable route set lives in `openapi.yaml` (served at
> `GET /openapi.yaml`); keep the two in sync — the `test_openapi_covers_routes`
> unit test enforces it.

---

## 0. Conventions

| Concern | Rule |
|---|---|
| Content-Type | `application/json` (UTF-8) for all request/response bodies with a body |
| Auth | `Authorization: Bearer <token>` when a server-side token is configured (`AUTH_TOKEN`, or `AUTH_TOKEN_FILE` pointing at a `0600` file — the latter is preferred). Loopback may be exempt — server policy. Constant-time compare. |
| Unknown fields | **Ignored** on deserialize (forward-compatible). Servers MUST NOT reject unknown keys. |
| Missing optional fields | Omitted, not `null`. With `exactOptionalPropertyTypes` on the TS side, undefined keys are not serialized (conditional spread). |
| IDs | Knowledge IDs are `i64` (serialized as JSON number). Entity/relation IDs are not exposed over the wire by these endpoints. |
| Strings | UTF-8; all bounds are UTF-8 byte lengths unless noted. |
| Errors | Uniform envelope (§5). Never leak internals (paths, SQL, stack). |
| Timeouts | Server enforces a 30 s per-request deadline + an 8 s `/recall` search budget; client also sets `AbortController`. |

### Field bounds (enforced server-side → `400` on violation)

| Field | Bound | Error code |
|---|---|---|
| `query` | 1 ≤ len ≤ 2,000 (utf8 bytes) | `query_empty` / `query_too_long` |
| `limit` | 1 ≤ n ≤ 100 | `limit_out_of_range` |
| `title` | 1 ≤ len ≤ 500 | `title_invalid` |
| `content` | 1 ≤ len ≤ 1,000,000 (1 MiB) | `content_empty` / `content_too_large` |
| `domain` | matches `^[a-z0-9][a-z0-9_-]{0,62}$` | `domain_invalid` |
| entity/relation `name` | 1 ≤ len ≤ 100, `^[A-Za-z0-9 _-]+$` | `name_invalid` |
| entity `type` | len ≤ 64 | `entity_invalid` |
| relation `type` | 1 ≤ len ≤ 64, `^[a-z0-9_]+$` (snake_case) | `relation_invalid` |
| arrays (`entities`/`relations`) | ≤ 200 each per request | `too_many_entities` / `too_many_relations` |

> Domain names are **lowercase** by convention. The server normalizes to lowercase
> (trim + lower) before comparison (so `Health` → `health`). Entity/relation names
> are also normalized to lowercase internally; their surrounding whitespace is
> collapsed. A well-formed but unregistered forced `domain` resolves to
> `domain_invalid` today (the `domain_unknown` distinction is reserved for a future
> per-domain registry; see §2).

---

## 1. Common types

### `Domain`
A domain name string (see bounds above). The reserved domain `global` is the fallback sink.

### `Entity`
```json
{ "name": "vitamin d3", "type": "supplement" }
```
- `name` — required, the entity surface form (case-insensitive unique within a domain).
- `type` — optional free-form label (e.g. `"supplement"`, `"person"`, `"concept"`).

### `Relation`
```json
{ "from": "vitamin d3", "to": "inflammation", "type": "helps" }
```
- `from`/`to` — entity names (must match an `Entity.name` in the same payload OR an existing entity in the domain; server upserts entities as needed).
- `type` — snake_case relation label.

### `RecallHit`
```json
{
  "id": 42,
  "title": "Vitamin D3 notes",
  "content": "Vitamin D3 supports immune function...",
  "score": 0.87,
  "domain": "health",
  "source": "both",
  "provenance": { "vector_rank": 0, "fts_rank": 1, "fused_score": 0.0327 }
}
```
| Field | Type | Always? | Notes |
|---|---|---|---|
| `id` | integer | yes | knowledge id |
| `title` | string \| null | no | omitted if absent |
| `content` | string | yes | the matched chunk with a bounded, faithful snippet window |
| `score` | number (float) | yes | normalized similarity/fusion score |
| `domain` | string | no | the domain the hit came from (present when `provenance=true`) |
| `source` | `"vector"` \| `"fts"` \| `"both"` \| `"graph"` | no | retrieval path (present when `provenance=true`) |
| `provenance` | object | no | per-retriever ranks + fused score (present when `provenance=true`) |

### `provenance` (per-hit)
The shape of `RecallHit.provenance` (defined in `src/search/mod.rs`):

| Field | Type | Notes |
|---|---|---|
| `vector_rank` | integer \| omitted | rank the vector retriever assigned (0 = best) |
| `fts_rank` | integer \| omitted | rank the FTS5 retriever assigned |
| `fused_score` | number \| omitted | RRF-fused score |
| `rerank_score` | number \| omitted | cross-encoder score (only if the rerank tier ran) |
| `rerank_truncated` | boolean | doc was length-capped before reranking |
| `prf_expanded` | boolean | hit surfaced via the PRF-expanded pass |
| `top_retrieval_mode` | `"vector"` \| `"fts"` \| `"both"` \| omitted | which retriever(s) contributed the top result |
| `retrieval_strategy` | string \| omitted | overall strategy, e.g. `hybrid` or `hybrid_prf` |
| `quality_assessment` | object \| omitted | heuristic confidence + recommendation (see `src/search/quality.rs`) |
| `prf_decision` | object \| omitted | why PRF did/didn't fire |

---

## 2. `POST /recall` — deterministic recall

The server does **everything**: embed the query → auto-route via domain centroids →
search (hybrid vec0 + FTS5, RRF fusion) → optional PRF query expansion → optional
cross-encoder rerank → cross-domain fallback on miss → cap → return.

### Request
```jsonc
{
  "query": "supplements for inflammation",
  "limit": 3,
  "domain": "health",        // optional: force a domain (disables auto-routing)
  "strict": false,           // optional: true = no cross-domain fallback
  "provenance": true,        // optional: include per-hit domain + source + provenance + telemetry
  // ── optional structured-query overrides (power tools) ──
  "source": "structured",    // filter by knowledge.source
  "since": "2026-01-01",     // ISO-8601 / RFC3339; rows with created_at > since
  "lex": "inflammation -fever", // lexical (FTS5) query override
  "vec": "immune support",   // semantic embedding-query override
  "hyde": "Vitamin D3 reduces...", // hypothetical-answer embedding override (beats `vec`)
  "intent": "lookup"         // free-form intent label, recorded for provenance
}
```
| Field | Type | Required | Default | Notes |
|---|---|---|---|---|
| `query` | string | **yes** | — | the user turn / search text |
| `limit` | integer | no | `5` | capped 1–100 |
| `domain` | string | no | (auto-route) | force a specific domain |
| `strict` | boolean | no | `false` | disable fallback fan-out |
| `provenance` | boolean | no | `false` | include `domain`/`source`/`provenance` per hit + `domainsSearched` + `telemetry` |
| `source` | string | no | — | filter by `knowledge.source` |
| `since` | string | no | — | ISO-8601 (RFC3339 or `YYYY-MM-DD HH:MM:SS`). Validated inside the search path; a malformed value is **silently swallowed** on the recall path today (the failing target contributes no hits) rather than surfacing a 400 |
| `lex` | string | no | — | lexical (FTS5) query override (exact terms, phrases, `-exclusions`) |
| `vec` | string | no | — | semantic embedding-query override |
| `hyde` | string | no | — | hypothetical-answer embedding override; takes priority over `vec` |
| `intent` | string | no | — | free-form intent label, recorded for provenance |

### Response — `200 OK`
```json
{
  "hits": [
    { "id": 42, "title": "Vitamin D3 notes", "content": "...", "score": 0.87, "domain": "health", "source": "both", "provenance": { "..." : "..." } },
    { "id": 88, "title": "Omega-3", "content": "...", "score": 0.71, "domain": "global", "source": "fts" }
  ],
  "domain": "health",
  "domainsSearched": ["health", "global"],
  "telemetry": { "embed_ms": 1.2, "vector_ms": 3.4, "fts_ms": 1.1, "fusion_ms": 0.1, "confidence": 0.78 }
}
```
| Field | Type | Always? | Notes |
|---|---|---|---|
| `hits` | `RecallHit[]` | yes | ordered by descending `score`; length ≤ `limit` |
| `domain` | string | yes | the **primary** domain chosen by routing (or the forced domain) |
| `domainsSearched` | string[] | no | every domain queried (incl. fallback). Present when `provenance=true`. |
| `telemetry` | object | no | per-stage retrieval telemetry. Present when `provenance=true`. |

### `telemetry` (per-response)
The shape of `RecallResponse.telemetry` (defined in `src/search/mod.rs::SearchTelemetry`):

| Field | Type | Notes |
|---|---|---|
| `embed_ms` / `vector_ms` / `fts_ms` / `fusion_ms` / `prf_ms` / `rerank_ms` | number | per-stage latency (ms) |
| `retrieval_ms_vec` / `retrieval_ms_fts` | number | retrieval latency excluding embedding |
| `vec_candidates` / `fts_candidates` / `fused_count` | integer | candidate counts before/after RRF |
| `rrf_k` | integer | RRF k parameter (`60`) |
| `confidence` | number | heuristic quality-estimator score (0–1) |
| `recommendation` | string \| omitted | `"return"` / `"run_prf"` / `"run_reranker"` / `"increase_top_k"` / `"clarify_query"` |
| `intent` / `embedding_query` | string \| omitted | effective intent / embedding query used |

### Routing semantics
1. **`domain` provided** → search only that domain. Unknown/unresolvable → `400 domain_invalid`.
2. **`domain` omitted (auto-route):**
   a. Embed query once (model2vec).
   b. Compare to every domain **centroid** (int8/binary, Hamming/cosine). Rank domains.
   c. Primary domain = top centroid **above** `DOMAIN_CONFIDENCE_THRESHOLD` (`0.55`).
   d. If none above threshold → primary = `global`.
3. Search the primary domain (hybrid vec0 KNN + FTS5 BM25, RRF fusion; optional PRF + rerank).
4. **Fallback** (unless `strict=true`): if no confident route → fan out across all known domains + `global`; merge by score; tag each hit's `domain`.
5. Cap to `limit`; return.

> Empty result is **not** an error — `200` with `hits: []`.

### Errors
| Status | Code | When |
|---|---|---|
| 400 | `query_empty` / `query_too_long` | missing/oversized query |
| 400 | `query_rejected` | query matches a blocked prompt-injection pattern |
| 400 | `limit_out_of_range` | limit outside 1–100 |
| 400 | `domain_invalid` | malformed or unresolvable forced `domain` |
| 401 | `unauthorized` | missing/invalid bearer |
| 429 | `rate_limited` | per-IP/domain rate limit breach |
| 503 | `recall_unavailable` | search task failed or exceeded the 8 s budget |

> `domain_unknown` is **reserved** for a future per-domain registry that distinguishes "well-formed but unregistered" from "malformed." Today both resolve to `domain_invalid`.

---

## 3. `POST /ingest` — structured store (the KG write path)

Stores a knowledge entry + its embedding (auto-resolved domain if omitted), plus optional
explicit entities/relations that populate the domain's knowledge graph. The server
**trusts** the caller's graph data after validation (no server-side extraction — the
annotation engine was retired in v0.9.0).

### Request
```jsonc
{
  "title": "Vitamin D3 benefits",
  "content": "Vitamin D3 supports immune function and helps with inflammation...",
  "domain": "health",                          // optional: resolved domain if omitted
  "entities": [
    { "name": "vitamin d3", "type": "supplement" },
    { "name": "inflammation", "type": "condition" }
  ],
  "relations": [
    { "from": "vitamin d3", "to": "inflammation", "type": "helps" }
  ]
}
```
| Field | Type | Required | Notes |
|---|---|---|---|
| `title` | string | **yes** | 1–500 chars (trimmed) |
| `content` | string | **yes** | 1–1,000,000 chars (not trimmed) |
| `domain` | string | no | force domain; omit → resolved to `global` |
| `entities` | `Entity[]` | no | upsert into the domain KG |
| `relations` | `Relation[]` | no | upsert; `from`/`to` upserted as entities if new |

### Response — `200 OK`
```json
{
  "id": 42,
  "status": "created",
  "domain": "health",
  "entitiesAdded": 2,
  "relationsAdded": 1
}
```
| Field | Type | Always? | Notes |
|---|---|---|---|
| `id` | integer | yes | knowledge id. On `duplicate`, returns the **existing** knowledge id. |
| `status` | `"created"` \| `"duplicate"` | yes | duplicate = content_hash already present (xxh3-64 of content) |
| `domain` | string | yes | the domain actually written to (forced or `global`) |
| `entitiesAdded` | integer | yes | count of entities in the request that were processed (upsert is idempotent, so this is the request count, not the delta of newly-inserted rows) |
| `relationsAdded` | integer | yes | count of relations in the request that were processed (same caveat) |

### Behavior
- **Dedup:** content hashed (xxh3-64); exact dup → `status: "duplicate"`, the **existing** id, **no** embedding work, **no** entity/relation mutation (`entitiesAdded: 0`, `relationsAdded: 0`).
- **Domain resolution:** if `domain` omitted → resolved to `global` (no centroid routing on the write path today). After a successful write the server best-effort **recomputes that domain's centroid** so future `/recall` auto-routing can target it.
- **Entities/relations** are scoped to the resolved domain. `INSERT OR IGNORE` semantics (idempotent). `from`/`to` in `relations[]` are resolved to existing entity rows (they must already exist in `entities[]` or in the domain — relation insert fails if a referenced entity cannot be resolved).
- **Embedding:** content is embedded once (model2vec) and stored in `vec_knowledge` as int8 + binary quantized vectors. The legacy f32 JSON `embeddings` column is no longer written.
- **Atomicity:** knowledge + vec0 + entities + relations in one SQLite transaction.

### Errors
| Status | Code | When |
|---|---|---|
| 400 | `title_invalid` / `content_empty` / `content_too_large` | bounds violations |
| 400 | `name_invalid` | bad entity/relation `name` (empty, > 100, bad charset) |
| 400 | `entity_invalid` | entity `type` > 64 chars |
| 400 | `relation_invalid` | bad relation `type` (empty, > 64, not snake_case) |
| 400 | `too_many_entities` / `too_many_relations` | array > 200 |
| 400 | `domain_invalid` | malformed or unresolvable forced domain |
| 401 | `unauthorized` | auth |
| 413 | *(bare status)* | body > 1 MiB (`MAX_REQUEST_SIZE`), enforced by the HTTP `RequestBodyLimitLayer` before the handler runs — returned as a plain 413, **not** the JSON envelope. (`HandlerError::payload_too_large` exists but is not invoked by this route.) |
| 429 | `rate_limited` | per-IP/domain write limit |
| 500 | `internal_error` | DB/embedding/transaction failure |

---

## 4. Supporting endpoints

### `GET /health` → `200`
```json
{
  "status": "ok",
  "version": "0.9.4",
  "model": "minishlab/potion-retrieval-32M",
  "system": { "memory_used_mb": 220, "memory_total_mb": 4096, "memory_percent": 5.4 },
  "pool":   { "connections": 2, "idle_connections": 1, "busy_connections": 1 }
}
```
The primary consumer probes this to confirm the server is up (it only
reads `status`). On failure the server returns `{ "status": "error", "version": "...", "error": "..." }`.
`version` is `env!("CARGO_PKG_VERSION")`.

### `DELETE /memory/{id}` → `200` / `404`
```json
{ "deleted": true }
```
Cascades to the entry's `vec_knowledge` row (cleaned explicitly — vec0 has no FK),
embeddings (FK CASCADE), and owned relations (FK SET NULL); the FTS trigger removes the
FTS row. A `tombstones` row records the deletion for provenance. `id` is parsed as `i64`
(non-numeric → 400). `404` body: `{ "error": { "code": "not_found", "message": "..." } }`.

### `GET /domains` → `200`  (ops/debug)
```json
{
  "domains": [
    { "name": "global", "entries": 1307, "entities": 2341, "relations": 1892, "hasCentroid": false },
    { "name": "health", "entries": 412,  "entities": 2341, "relations": 1892, "hasCentroid": false }
  ]
}
```
Not used by the recall hot path, but useful for the `brain` CLI and for surfacing
`knownDomains`.

> **Known limitation (current source):** domains are derived by `GROUP BY domain` on the
> `knowledge` table (single-DB tagged model). `hasCentroid` is **always `false`** today
> (the centroid layer is computed but not yet surfaced here), and `entities`/`relations`
> are **global totals copied into every row** (per-domain KG counts land with per-domain
> DBs). Do not rely on per-row entity/relation numbers being domain-scoped yet.

---

## 5. Error envelope (uniform)

Every non-2xx response uses this shape:

```json
{
  "error": {
    "code": "domain_invalid",
    "message": "domain 'heath' is not registered",
    "details": { "max": 200 }
  }
}
```
| Field | Type | Always? | Notes |
|---|---|---|---|
| `error.code` | string | yes | machine-readable snake_case code (see per-endpoint tables) |
| `error.message` | string | yes | safe human text; **never** includes paths/SQL/secrets |
| `error.details` | object | no | structured context (e.g. `{min, max}` for range errors) |

Consumers SHOULD treat any non-2xx as an error, distinguishing 404 from
other statuses. `401 unauthorized` MUST be surfaced (not silently swallowed) for
security visibility.

---

## 6. Rust (Axum + serde) — canonical definitions

The shared response/error types live in `src/handlers/mod.rs`; the per-endpoint request
types live alongside their handlers. Uses crates already in `Cargo.toml` (`serde`,
`serde_json`, `axum 0.8`).

### `src/handlers/mod.rs` — shared types

```rust
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HitSource { Vector, Fts, Both, Graph }

#[derive(Debug, Serialize)]
pub struct RecallHit {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub content: String,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<HitSource>,
    /// Per-retriever ranks + fused score. Present only when `provenance=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<crate::search::Provenance>,
}

#[derive(Debug, Serialize)]
pub struct RecallResponse {
    pub hits: Vec<RecallHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domains_searched: Option<Vec<String>>,
    /// Per-stage retrieval telemetry. Present only when `provenance=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<crate::search::SearchTelemetry>,
}

#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub id: i64,
    pub status: &'static str, // "created" | "duplicate"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities_added: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relations_added: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ForgetResponse { pub deleted: bool }

// ---------- uniform error envelope ----------

#[derive(Debug, Serialize)]
pub struct ErrorBody { pub error: ApiError }

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Handler error type → renders the uniform `ErrorBody` envelope.
#[derive(Debug)]
pub struct HandlerError { pub status: StatusCode, pub inner: ApiError }

impl IntoResponse for HandlerError {
    fn into_response(self) -> axum::response::Response {
        (self.status, axum::response::Json(ErrorBody { error: self.inner })).into_response()
    }
}
```

### `src/handlers/recall.rs` — request

```rust
#[derive(Debug, Deserialize)]
pub struct RecallRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    pub domain: Option<String>,
    #[serde(default)] pub strict: bool,
    #[serde(default)] pub provenance: bool,
    #[serde(default)] pub source: Option<String>,
    #[serde(default)] pub since: Option<String>,
    #[serde(default)] pub lex: Option<String>,
    #[serde(default)] pub vec: Option<String>,
    #[serde(default)] pub hyde: Option<String>,
    #[serde(default)] pub intent: Option<String>,
}
```

### `src/handlers/ingest.rs` — request

```rust
#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub title: String,
    pub content: String,
    pub domain: Option<String>,
    #[serde(default)] pub entities: Vec<EntityInput>,
    #[serde(default)] pub relations: Vec<RelationInput>,
}

#[derive(Debug, Deserialize)]
pub struct EntityInput {
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>, // wire key is "type" (a Rust keyword)
}

#[derive(Debug, Deserialize)]
pub struct RelationInput {
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub kind: String,
}
```

### Validation constants & helpers (`src/handlers/mod.rs`)

```rust
pub const DOMAIN_RE: &str = r"^[a-z0-9][a-z0-9_-]{0,62}$";
pub const NAME_RE:   &str = r"^[A-Za-z0-9 _-]{1,100}$";
pub const RELTYPE_RE: &str = r"^[a-z0-9_]{1,64}$";

pub const MAX_QUERY: usize     = 2_000;
pub const MAX_TITLE: usize     = 500;
pub const MAX_CONTENT: usize   = 1_000_000;
pub const MIN_LIMIT: u32       = 1;
pub const MAX_LIMIT: u32       = 100;
pub const MAX_ENTITIES: usize  = 200;
pub const MAX_RELATIONS: usize = 200;
pub const MAX_BODY: usize      = 2 * 1024 * 1024; // 2 MiB — defined but UNUSED; real body cap is the HTTP layer (MAX_REQUEST_SIZE = 1 MiB)

pub const DEFAULT_RECALL_LIMIT: u32   = 5;
pub const DOMAIN_CONFIDENCE_THRESHOLD: f32 = 0.55;

pub fn normalize_domain(raw: &str)   -> Result<String, HandlerError>; // → domain_invalid
pub fn normalize_name(raw: &str)     -> Result<String, HandlerError>; // → name_invalid
pub fn normalize_rel_type(raw: &str) -> Result<String, HandlerError>; // → relation_invalid
```

> `provenance` (`src/search/mod.rs::Provenance`) and `telemetry`
> (`src/search/mod.rs::SearchTelemetry`) are larger structs with nested quality-assessment
> and PRF-decision types (see §1 / §2 for their serialized field lists). Their full Rust
> definitions live in `src/search/mod.rs` and `src/search/quality.rs`.

---

## 7. JSON Schema generation (optional, future)

For a single machine-readable source of truth, derive JSON Schemas from the Rust structs via
[`schemars`](https://crates.io/crates/schemars) (`#[derive(JsonSchema)]`) and publish them
alongside the OpenAPI spec (`openapi.yaml`). The TS types can then be code-generated from
those schemas, eliminating manual drift. Noted in ROADMAP Phase 6.
