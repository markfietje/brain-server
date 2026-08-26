# Glossary

A plain-language dictionary of the terms used throughout this wiki. Aimed at readers who are new to semantic memory, knowledge graphs, or AI agent infrastructure.

## A

- **Abstention** — the retrieval engine's ability to say "I don't know." When confidence is too low, `/recall` returns `{decision: "low_confidence", hits: []}` instead of a confidently wrong top-1 result.
- **Audit chain** — an append-only log where each row stores the SHA-256 hash of the previous row, so any modification or deletion is detectable.

## B

- **Bearer token** — a secret string sent in the `Authorization` header to authenticate a request. Brain Server supports opaque bearer tokens (default) and JWT/JWS.
- **Bi-temporal** — recording both *when a fact is valid in the world* (`valid_at`/`invalid_at`) and *when the system knew it* (`observed_at`/`superseded_at`). Graph edges carry all four timestamps; `superseded_at IS NULL` marks the current belief. Enables point-in-time recall.
- **BM25** — the classic lexical scoring function (term-frequency × inverse-document-frequency) used by SQLite's FTS5 full-text index.

## C

- **Capacity envelope** — a configurable bound on docs / DB size / RSS. Writes that exceed it return HTTP 507; reads are never blocked.
- **Chunk** — a unit of memory stored in a `knowledge` row. Text is split into chunks by a CommonMark-aware splitter (heading-boundary splits, code-fence-safe).
- **CommonMark** — a standard, unambiguous specification of Markdown. Brain Server's chunker uses a CommonMark parser so all constructs are handled correctly.
- **Complaint remedy matrix** — the deterministic remedy suggestions proposed on a complaint run (each citing its legal basis and the published code-of-conduct clause); applying one is always a human decision.
- **Connector** — a supervised ingester (e.g. GitHub issues) that backfills external sources through the source/revision pipeline.
- **Content-digest binding** — an approval must echo the SHA-256 `content_digest` of exactly the review form the operator saw (`409` on any drift), so a decision binds to the shown bytes.
- **CSP (Content Security Policy)** — an HTTP header controlling what resources a page may load. Brain Server serves a strict CSP for the API and a relaxed one for the WASM client.

## D

- **Decision path / trace** — the recorded record of a recall: injected chunks, fused scores, abstention decision, access scope, principal, and domains searched. Replayable via `GET /recall/{trace_id}/trace`.
- **Domain** — a scoped memory namespace (health, business, code…) with its own knowledge graph. Retrieval auto-routes between domains by centroid and falls back on a miss.
- **DSAR** — Data Subject Access Request. Brain Server's `/dsar` workflow locates → exports → purges → issues a chain-verifiable deletion certificate.

## E

- **Embedding** — a numeric vector representing text, such that semantically similar texts are close in vector space. Brain Server's *default* profile uses *static* embeddings (`model2vec`, no transformer forward pass); the opt-in `enterprise` / `desktop` profiles use local transformer embeddings (`BGE-M3` / `gte-base-en-v1.5`).
- **Egress** — data leaving your device/network. Brain Server has no data egress by default.
- **Entitlement** — a memory kind for what someone is owed (warranty, plan, SLA rights); carries the longest default retention (1,825 days).
- **Evidence** — the verbatim snippet, line span, source link, and highlight ranges attached to a retrieved chunk — what a result is *actually* based on.

## F

- **FTS5** — SQLite's full-text-search index, scored with BM25. The lexical retrieval leg.
- **Fusion** — merging multiple ranked lists into one. Brain Server uses Reciprocal Rank Fusion.

## G

- **Graph leg** — the optional third retrieval leg: Personalized PageRank over the knowledge graph, opt-in via `?graph=true`.
- **Governance** — the layer that keeps memory honest and auditable: audit log, quarantine, write-back gating, DSAR, retention.

## H

- **Hybrid retrieval** — combining vector (semantic) and lexical (keyword) search. Brain Server runs both legs concurrently and fuses them.
- **Hub dampening** — a technique that reduces the influence of very-high-degree graph nodes (mega-hubs), so taxonomy tag clouds don't drown out real semantic edges.

## I

- **Ingest** — the act of adding memory: `POST /ingest`, `/ingest/memory`, or `/ingest/markdown`.

## J

- **JWT / JWS** — JSON Web Token / JSON Web Signature. The opt-in enterprise authentication mode. Only RS256/ES256/EdDSA allowed (never HS256 or `none`).

## K

- **KCS article** — a Knowledge-Centered Service capture: a solved case distilled into reusable knowledge; complaint clusters rank above incident repeaters.
- **Knowledge graph** — entities and the relationships between them, extracted from markdown links. Traversable and queryable.
- **KNN** — k-nearest-neighbors, the vector search that finds the closest embeddings to a query.

## L

- **Legal hold** — an operator-set hold that suspends retention expiry and purge for affected content until explicitly released; hold paths fail closed.
- **LexSpec** — the structured lexical query: terms, quoted phrases, exclusions (`-"..."`), and exact code paths.
- **Loopback** — `127.0.0.1`, the local machine. Brain Server is loopback-safe by default (refuses `0.0.0.0` unless `BIND_PUBLIC=1`).

## M

- **MCP** — Model Context Protocol, a standard for exposing tools to agents. Brain Server ships an `mcp` binary.
- **Mesh** — the multi-site federation shape: regional deployments exchanging signed knowledge parcels; site-to-site routing is v3.x.
- **Multi-domain** — running several scoped domain databases that auto-route and cross-reference on a miss.

## P

- **Parcel** — a signed export/import bundle of knowledge crossing a site boundary (`POST /parcels/export|import`); every crossing is signed and human-gated.
- **PII** — personally identifiable information. Brain Server applies deterministic read-time output redaction to PII; there is no write-time placeholder vault (v1.20.19).
- **PRF** — pseudo-relevance feedback: deterministic query expansion that fires only when the top result appears in *both* retrieval legs within a bounded rank.
- **Proposal** — a write-back candidate scored by the server but held in a queue until a human approves it. Nothing enters memory autonomously.
- **Provenance** — per-retriever ranks, fused score, expansion terms, and evidence attached to each result.

## Q

- **Quarantine** — the injection screen's holding state: suspicious content is stored but excluded from recall and the knowledge graph until an operator releases or deletes it. Recall never reads quarantined rows.
- **QueryDoc** — the structured query document accepted by `/recall` (query, filters, provenance flag, graph flag).

## R

- **Recall** — retrieval. `POST /recall` is the primary endpoint.
- **Reciprocal Rank Fusion (RRF)** — a deterministic, weight-free merge: `score = Σ 1/(k + rank)`, with `k = 60`.
- **Retention** — how long content stays in default recall: a per-kind TTL decay policy set via `POST /retention` and `BRAIN_RETENTION_KIND_DAYS`, with defaults owned by the SDK policy table. Decayed rows leave default recall (historical `?at=` recall still finds them); audit rows honor `BRAIN_AUDIT_RETENTION_DAYS` if set.

## S

- **Scoreboard** — the outcome/efficiency dashboard behind `GET /workflow/scoreboard`: FCR, resolution mix, and the goodwill ledger over closed runs.
- **Span verification** — `POST /verify` checks whether a claim is literally supported by a chunk's text (deterministic lexical match, no LLM).
- **Static embedding model** — a model with no transformer forward pass, just token lookup (`model2vec` / `potion-retrieval-32M`). Cheap on CPU. This is the default embedder; the opt-in neural tiers (`BGE-M3`, `gte-base-en-v1.5`) are transformer models.
- **Supersede** — marking a new fact as replacing an old one. Atomically expires the old fact from current recall; historical recall still returns it.
- **SQLite vec0** — a SQLite extension for vector search (KNN over quantized embeddings).

## T

- **Temporal evidence** — the `observed_at` / `valid_from` / `valid_to` / `authority` stamps that make point-in-time recall possible.
- **Tombstone** — a hash-only record left when data is purged, proving a deletion occurred.
- **Trace** — see *Decision path*.

## U

- **UMP** — Universal Memory Protocol: the wire contract for portable memory operations, implemented by the `/ump/*` routes and the `ump.*` MCP tools.
- **Untrusted-evidence boundary** — the OWASP LLM01:2025 pattern where every retrieved result serializes `untrusted: true`, signaling the consuming agent to treat it as untrusted evidence.

## V

- **Vector** — see *Embedding*.
- **vec0 KNN** — the vector search leg over quantized embeddings.

## W

- **WAL** — Write-Ahead Logging, SQLite's concurrency mode used by Brain Server (with a busy timeout so concurrent writers queue rather than fail).
- **Workflow run** — an unbounded durable session for one governed case, recorded as queryable lineage events; rewind branches a run instead of rotating sessions.
- **Worktype** — the post-sale work class a run routes to (troubleshoot, return, complaint, safety_recall…); each maps deterministically to an SLA priority class.
- **Write posture** — `BRAIN_WRITE_POSTURE`: `open` writes directly; `review` converts agent-facing writes into proposals. Unknown values refuse boot.
- **Write-back gate** — the human-in-the-loop mechanism that scores a candidate but requires approval before it becomes memory.
