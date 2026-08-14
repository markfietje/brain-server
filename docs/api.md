# API

Brain Server exposes a versioned HTTP API. Every response carries an
`X-Api-Version` header. This page is the informational overview; the complete,
machine-readable contract is at **`GET /openapi.yaml`** at runtime and
[openapi.yaml](https://github.com/markfietje/brain-server/blob/main/openapi.yaml) in the repo, with the full written contract in
[API_CONTRACT.md](./API_CONTRACT.md).

---

## Core routes

| Method | Path | Purpose |
|---|---|---|
| GET | `/health`, `/health/db`, `/ready` | Liveness + capacity + hardening |
| GET | `/stats`, `/version` | Counts, model, version |
| GET | `/openapi.yaml` | Full API contract |
| POST | `/ingest/memory` | Structured memory ingest |
| POST | `/ingest/markdown` | Markdown ingest + graph extraction |
| POST | `/ingest` | Structured ingest (explicit entities/relations) |
| POST | `/sources/reconcile` · DELETE `/sources/{id}` | Sweep deleted sources / retire a source |
| POST | `/recall` | Structured recall — the primary endpoint |
| GET | `/search` | Semantic search *(deprecated; use `/recall`)* |
| GET | `/get/{id}` · POST `/multi-get` | Fetch chunk(s) by id |
| GET | `/recall/{trace_id}/trace` | Recall-trace replay (decision-path evidence) |
| POST | `/verify` | Span verification — is a claim supported by a chunk's text? |

---

## Retrieval

**`POST /recall`** takes a structured query document (`QueryDoc`; the `query`/`limit`
fields are the `/recall`-specific ones — `q`/`k` are the `GET /search` equivalents):

```json
{
  "query": "blueberry alternative",
  "limit": 5,
  "sources": ["memory", "vault"],
  "provenance": true,
  "graph": false
}
```

- **Lexical control** — a `LexSpec` with terms, quoted phrases, exclusions
  (`-"..."`), and exact code paths.
- **Filters** — `source`/`sources` (ingest kind), `since` (ISO timestamp),
  `domain`, `min_relevance`, `include_decayed`.
- **Provenance** — per-retriever ranks, fused score, expansion terms.
- **Abstention** — returns `{decision: "low_confidence", hits: []}` rather than
  top-1 garbage when quality is too low.

---

## Knowledge graph

| Method | Path | Purpose |
|---|---|---|
| GET | `/graph/entity/{name}` | Entity + 1-hop relations |
| GET | `/graph/relations?from=&to=` | Relations between entities |
| GET | `/graph/traverse?start=&max_depth=&explain=&kind=` | Bounded walk (depth ≤ 4); `explain=true` returns structured hop paths; `kind=` filters by edge type |

---

## Governance & write-back

| Method | Path | Purpose |
|---|---|---|
| POST | `/ingest/proposal` · `/proposals/{id}/approve[?supersedes=N]` · `/reject` | Human-in-the-loop write-back (v1.14) |
| GET | `/proposals?status=` · `/decayed` | Approval queue + decayed review |
| POST | `/consolidate/propose` · `/apply` · `/undo` | Reviewable consolidation, supersession, undo |
| POST | `/suggest` · `/suggest/feedback` · GET `/suggest/metrics` | Opt-in anticipation + false-positive metric |
| POST | `/verify` | Claim span verification |
| POST | `/classify` · `/decision/{id}/evaluate` | Deterministic categorization / decision rules |
| POST | `/procedure` · GET `/procedure/{id}/steps` | Ordered procedures |

---

## Privacy & audit

| Method | Path | Purpose |
|---|---|---|
| GET | `/export` | Portable JSON export |
| POST | `/purge` | Hard, audited deletion by id or owner |
| POST | `/dsar` | Locate → export → purge → deletion certificate (supports `dry_run` footprint preview) |
| GET | `/dsar` | DSAR ledger (admin, newest-first, per-row deadline) |
| GET | `/tombstones?subject=&since=` | Deletion registry |
| GET | `/dsar/{id}/certificate` | Re-fetch certificate + live chain check |
| GET | `/audit` · `/audit/verify` | Append-only audit log + chain integrity |
| GET | `/quarantine` · `/quarantine/{id}/release` · `/delete` | Injection review |
| GET | `/retention` · POST `/retention` · GET `/art30` | Per-kind retention policy + Art 30 record |
| GET | `/snapshot/status` | Point-in-time snapshot state |

---

## UMP (Universal Memory Protocol)

| Method | Path | Purpose |
|---|---|---|
| GET | `/ump/capabilities` | Protocol negotiation (conformance level, retrieval signals, `max_recall`, writable, audit) |
| POST | `/ump/remember` · `/ump/revise` · `/ump/forget` · `/ump/feedback` | Record / patch / soft-delete / outcome-feedback |
| POST | `/ump/recall` | Ranked recall with per-result signals |
| GET | `/ump/memory/{id}` | Read one record with on-read integrity re-verification |
| GET | `/ump/subscribe` | SSE broadcast of memory events |
| POST | `/ump/audit` · GET `/ump/audit/verify` | UMP-scoped audit row family + chain verification |

---

## Auth & discovery (JWT mode)

| Method | Path | Purpose |
|---|---|---|
| POST | `/auth/refresh` · `/logout` · `/revoke` | Token lifecycle |
| GET | `/.well-known/openid-configuration` · `/.well-known/jwks.json` | OIDC + JWKS |

---

## Versioning & deprecation

- Every response carries `X-Api-Version`.
- `POST /add` and `GET /search` are deprecated (migrate to `/ingest` + `/recall`)
  and emit an RFC 8594 `Deprecation` header.
- The written contract ([API_CONTRACT.md](./API_CONTRACT.md)) states the
  stability promise and the deprecation policy.

---

## Clients

- **`brain` CLI** — status, query, get, explain, ingest-dir, reconcile, retention,
  domains, ump, backup/restore, key management, and more (see
  [CLI reference](./cli-reference.md)).
- **`mcp` binary** — search/recall/ingest exposed as MCP tools for agent clients.
- **Dioxus client** — the visual control surface served at `/app`.

---

## Next steps

- [Quickstart](./quickstart.md) — working examples.
- [Architecture](./architecture.md) — how the endpoints map to the engine.
