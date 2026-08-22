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
| GET | `/health` | Liveness probe (minimal `{status, version}`; detail on `/health/db`) |
| GET | `/health/db` | Read-gated detail — capacity, pool, hardening, model, otel, DPO |
| GET | `/stats`, `/version` | Counts, model, version |
| GET | `/openapi.yaml` | Full API contract |
| POST | `/v1/embeddings` | OpenAI-compatible embeddings endpoint |
| POST | `/ingest/memory` | Structured memory ingest |
| POST | `/ingest/markdown` | Markdown ingest + graph extraction |
| POST | `/ingest` | Structured ingest (explicit entities/relations) |
| POST | `/sources/reconcile` · DELETE `/sources/{id}` | Sweep deleted sources / retire a source |
| POST | `/recall` | Structured recall — the primary endpoint |
| GET | `/search` | Semantic search *(deprecated; use `/recall`)* |
| GET | `/get/{id}` · POST `/multi-get` | Fetch chunk(s) by id |
| GET | `/recall/{trace_id}/trace` | Recall-trace replay (decision-path evidence) |
| POST | `/verify` | Span verification — is a claim supported by a chunk's text? Binds the `X-Brain-Domain` label in SQL (an id cannot cross domains in shim mode) + the record gate. |
| POST | `/reindex` | Rebuild indexes |
| GET | `/metrics` | Prometheus metrics (auth-gated) |
| GET | `/events` | SSE broadcast of memory events |
| POST | `/webhooks/{kind}` · `/webhooks/gh` | Webhook delivery receiver (HMAC-verified) |

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
- **Provenance** — per-retriever ranks, fused score, expansion terms, and
  per-hit `source` / `node_kind` / `lawful_basis` / `region` tags (present when
  stored; absorbed into the `RecallHit` wire shape, v1.27.12).
- **Abstention** — returns `{decision: "low_confidence", hits: []}` rather than
  top-1 garbage when quality is too low.

---

## Knowledge graph

| Method | Path | Purpose |
|---|---|---|
| GET | `/graph/entity/{name}` | Entity + 1-hop relations |
| GET | `/graph/relations?from=&to=` | Relations between entities |
| GET | `/graph/traverse?start=&max_depth=&explain=&kind=` | Bounded walk (depth ≤ 4); `explain=true` returns structured hop paths; `kind=` filters by edge type |
| GET | `/graph/relationships/{id}/history` (Admin) | Edge supersession lineage — every version of an edge triple (v1.27.22) |

---

## Governance & write-back

| Method | Path | Purpose |
|---|---|---|
| POST | `/ingest/proposal` · `/proposals/{id}/approve[?supersedes=N][&digest=...]` · `/reject` · `/proposals/{id}/edit` | Human-in-the-loop write-back (v1.14). Since v1.27.12 `approve` accepts an optional `digest` (SHA-256 of the read-canonical review form, as served by `GET /proposals`); any drift → `409` — the approval binds to the bytes the reviewer saw |
| GET | `/proposals?status=` · `/decayed` | Approval queue + decayed review. Each row is a `ProposalView` (`content` = read-canonical form, `content_digest` = SHA-256 the approve verb binds to, v1.27.12) |
| POST | `/consolidate/propose` · `/apply` · `/undo` | Reviewable consolidation, supersession, undo |
| POST | `/suggest` · `/suggest/feedback` · GET `/suggest/metrics` | Opt-in anticipation + false-positive metric |
| POST | `/verify` | Claim span verification |
| POST | `/classify` · `/decision/{id}/evaluate` | Deterministic categorization / decision rules |
| POST | `/procedure` · GET `/procedure/{id}/steps` | Ordered procedures (steps bind the `X-Brain-Domain` label + record gate) |

---

## Profiles, roles & connectors (policy)

| Method | Path | Purpose |
|---|---|---|
| GET | `/profiles` · GET/POST `/profiles/{name}` | Preset system (v1.21): fetch/upsert a typed knob bundle |
| GET | `/roles` · GET/POST `/roles/{name}` | Role postures + capability sets (v1.23) |
| GET | `/connectors` | Registered connector registry (v1.24) |
| POST | `/connectors/register` | Validate + register a connector against the domain's profile gate (v1.24) |

---

## Privacy & audit

| Method | Path | Purpose |
|---|---|---|
| GET | `/export` | Portable JSON export |
| POST | `/purge` | Hard, audited deletion by id or owner |
| DELETE | `/memory/{id}` | Hard, audited deletion of one chunk (human-only erasure; the agent tool was removed v1.20.25) |
| POST | `/dsar` | Locate → export → purge → deletion certificate (supports `dry_run` footprint preview) |
| GET | `/dsar` | DSAR ledger (admin, newest-first, per-row deadline) |
| GET | `/tombstones?subject=&since=` | Deletion registry |
| GET | `/dsar/{id}/certificate` | Re-fetch certificate + live chain check |
| GET | `/audit` · `/audit/verify` | Append-only audit log + chain integrity (v1.27.31: verify covers every registered domain; rows carry their `domain` tag in multi-db mode) |
| GET | `/quarantine` · `/quarantine/{id}/release` · `/delete` | Injection review |
| GET | `/retention` · POST `/retention` · GET `/art30` · GET `/retention/report` | Per-kind retention policy + Art 30 record + per-domain×kind retention report |
| GET | `/snapshot/status` | Point-in-time snapshot state |

---

## Domains & routing

| Method | Path | Purpose |
|---|---|---|
| POST | `/domains` | Create a domain pool (200 = existed, 201 = created; body `{domain}`) |
| GET | `/domains` | List known domains (single global pool when multi-db is off) |
| DELETE | `/domains/{name}?confirm=<name>` | Delete a domain + all its data (echo-confirm guard, `global` protected) |
| POST | `/domains/{name}/vacuum` | `VACUUM` one domain pool (returns `{name, vacuumed: true}`) |
| GET | `/domains/{name}/export` | Consistent SQLite snapshot download (`VACUUM INTO`, `attachment; filename="brain-<name>.db"`) — Read in multi-db; **Admin in shim mode** (the snapshot is the whole shared pool there) |
| POST | `/domains/{name}/import` | Restore a snapshot into a NEW domain (raw bytes body; 201 `{name, imported: true, bytes}`) |
| POST | `/domains/recompute` | One-shot centroid recompute sweep over every domain (`{recomputed: [[domain, n], …]}`) |
| POST | `/domains/move` | Move chunks to another domain |

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

## Legal hold & breach (v1.22 / v1.25)

| Method | Path | Purpose |
|---|---|---|
| POST | `/legal-hold` · `/legal-hold/{id}/release` · GET `/legal-holds` | Per-domain legal holds; held ids are frozen (purge/DSAR defer) |
| POST | `/breach` · `/breach/{id}/event` · `/breach/{id}/close` | Breach-notification workflow (open / append event / close) |
| GET | `/breaches` · `/breaches/{id}` | Breach register + detail |
| GET | `/workflow/scoreboard` | Workflow outcome/efficiency scoreboard over recent runs (DPO/admin; rates in integer ten-thousandths, fail-closed audit linkage) |

---

## Cross-border transfers (v1.26)

| Method | Path | Purpose |
|---|---|---|
| POST | `/transfers` · GET `/transfers` | Register / list cross-border transfers (validated mechanism + jurisdiction) |
| GET | `/transfers/{id}/tia` | Transfer-impact assessment (Schrems II, pre-filled evidence) |
| GET | `/transfers/{id}/dpa` | Data-processing agreement (Art 28, pre-filled evidence) |

---

## Clients register (v1.27 BPO)

| Method | Path | Purpose |
|---|---|---|
| POST | `/clients` · GET `/clients` | Register / list clients (one domain per client) |
| GET | `/clients/{name}` | Client detail (client-auditor: row-filtered to granted domains) |
| POST | `/clients/{name}/dsar` | Per-client jurisdiction-aware DSAR + certificate |
| POST | `/clients/{name}/hold` | Per-client legal hold (resolves the client's domain) |
| POST | `/clients/{name}/end` | Termination: purge-or-return + archive + certificate |
| GET | `/clients/{name}/proposals` · POST `/clients/{name}/proposals/{id}/coach` | Supervisor QA queue (same `ProposalView` shape as `/proposals`) + coaching note (v1.27.8, Admin) |

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

## Tooling clients

- **`brain` CLI** — status, query, get, explain, ingest-dir, reconcile, retention,
  domains, ump, backup/restore, key management, and more (see
  [CLI reference](./cli-reference.md)).
- **`mcp` binary** — search/recall/ingest exposed as MCP tools for agent clients.
- **Dioxus client** — the visual control surface served at `/app`.

---

## Next steps

- [Quickstart](./quickstart.md) — working examples.
- [Architecture](./architecture.md) — how the endpoints map to the engine.
