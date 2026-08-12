# Features

Brain Server packs a lot of capability into a single Rust binary. This page is the complete feature tour — grouped by what the feature does for you.

## Retrieval

- **Hybrid retrieval** — vector KNN (`vec0`) + lexical FTS5 (BM25) fused via Reciprocal Rank Fusion, with deterministic PRF query expansion and full per-result provenance.
- **Structured query** — `QueryDoc` with `LexSpec` (phrases, exclusions, code paths), multi-source OR scope, temporal `since`/`as_of` predicates.
- **Optional graph leg** — Personalized PageRank over the knowledge graph as a third, opt-in `?graph=true` RRF leg (HippoRAG-2 style).
- **Noise-aware graph retrieval** (v1.12) — hub dampening + edge-type weights tame taxonomy-noise mega-hubs; the graph leg auto-engages as a rescue pass when the estimator says the query is ambiguous.
- **Calibrated abstention** (v1.5) — when retrieval quality is too low, `/recall` returns `{decision: "low_confidence", hits: []}` instead of top-1 garbage. No magic score cutoff — a calibrated multi-signal recommendation drives it.
- **Span verification** (v1.5) — `POST /verify` checks whether a claim is supported by a chunk's actual text (deterministic lexical match, no LLM).

## Temporal & knowledge

- **Temporal evidence** — every ingest stamps `observed_at` / `valid_from` / `valid_to` / `authority`. Point-in-time recall returns the revision active at a timestamp.
- **Knowledge graph** — entities and relationships extracted from `[[relation::entity]]` syntax in markdown. Traverse, query, and follow links.
- **Faithful explanations** (v1.7) — `/graph/traverse?explain=true` returns structured hop chains (`A --works_at--> B --ceo_of--> C`), not a flat id string. Edge-type filter via `?kind=`.
- **Ordered procedures** (v1.10) — `POST /procedure` ingests a root + ordered steps in one transaction; `GET /procedure/{id}/steps` returns them via `next_step` edges.
- **Deterministic classification** (v1.10) — `POST /classify` routes text to a category by matched keywords (auditable); `POST /decision/{id}/evaluate` fires the matched branch of a stored decision rule. No LLM.

## Self-correction & maintenance

- **Self-correction** (v1.6) — operator-approved `supersedes` links atomically expire the prior fact; historical recall (`?at=<past>`) still returns it. `brain resolve` + `brain check-consistency` surface action items.
- **Reviewable proposals** (v1.8) — `/consolidate/propose` detects exact duplicates, subject conflicts, unresolved contradictions, **stale sources** (deleted vault files), and **near-duplicates** (cosine > 0.95). `brain undo-resolve` reverses prior resolutions without retrieval regression.
- **Write-back gating** (v1.14) — `POST /ingest/proposal` scores a candidate (novelty via KNN, conflict via consolidation, salience via heuristics) but creates **no** `knowledge` row; it becomes memory only via human approval.

## Anticipation & suggestions

- **Opt-in anticipation** (v1.9) — `POST /suggest` returns related-but-not-surfaced chunks (tagged `reason: "anticipated"`); `POST /suggest/feedback` records accept/dismiss; `GET /suggest/metrics` reports the false-positive rate. No push, no decay, no hidden personalization — the agent asks explicitly.

## Source lifecycle & connectors

- **Source lifecycle** — every chunk carries provenance (`source` + immutable `revision`). Connectors backfill external sources through a supervised pipeline; `POST /reconcile` sweeps orphans from deleted sources.
- **Connectors** — supervised ingesters (GitHub issues via App auth) that backfill through the existing source/revision pipeline. Extensible connector contract.

## Governance, privacy & compliance

- **Append-only audit log** — ingest and auth-denial events recorded hash-only in a SHA-256 hash chain.
- **Prompt-injection quarantine** — suspicious content stored but excluded from retrieval until reviewed.
- **Read-event audit** (v1.15) — recall/search/get emit rows into the hash chain (opt-in), plus a replayable **recall trace**.
- **DSAR workflow** (v1.15) — `POST /dsar` locate → export → purge → chain-verifiable deletion certificate; `GET /tombstones` registry; `GET /dsar/{id}/certificate`.
- **GDPR export/purge** (v1.14) — `GET /export` portable JSON; `POST /purge` hard audited delete by id or owner.
- **PII controls** (v1.14) — output redaction + opt-in write-time placeholder mode (`BRAIN_REDACT_PII=1`).

## Security

- **Two authentication modes** — opaque bearer (default) or JWT/JWS (opt-in), with per-route AuthZ and record-level access scoping.
- **Verified webhooks** — HMAC verification, replay-window enforcement, idempotency.
- **Encrypted backup/restore** — AES-256-GCM, checksummed, excludes secrets.

## Integration surface

- **OpenAI-compatible embeddings** — `POST /v1/embeddings`.
- **MCP server** — `mcp` binary exposes search/recall/ingest as MCP tools.
- **`brain` CLI** — the operator surface: status, query, ingest, reconcile, audit, backup, and more.
- **Client control surface** (v1.16) — a Dioxus app (web + desktop + iOS + Android) with connection state machine, honest-batch review, recall decision-path viewer, DSAR certificate card, auth-failure feed, and audit filters + export.

## Next steps

- See how it all works in **[Architecture](./architecture.md)**.
- Try the **[Quickstart](./quickstart.md)**.
- Browse the **[API Reference](./api-reference.md)**.
