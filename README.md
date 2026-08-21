# 🧠 Brain Server

**A local-first semantic memory and knowledge-graph server for AI agents. No cloud, no per-query cost, no LLM in the loop.**

Brain Server gives your agent a second brain that lives on your own device. It is written in Rust and wraps a deterministic retrieval engine, a static local embedding model (with optional neural tiers), and a knowledge graph behind a versioned HTTP API. Recall never calls an LLM or an embedding API, so every query costs nothing and the data never leaves the machine.

---

<p align="center">

  [![Version](https://img.shields.io/badge/version-1.27.40-blue.svg)](#)
  [![Docs](https://img.shields.io/badge/docs-brain--server-1f6feb.svg)](https://markfietje.github.io/brain-server/)
  [![Rust](https://img.shields.io/badge/rust-2024-orange.svg?logo=rust)](#)
  [![License: MIT](https://img.shields.io/github/license/markfietje/brain-server.svg)](#)
  [![CI](https://github.com/markfietje/brain-server/actions/workflows/ci.yml/badge.svg)](https://github.com/markfietje/brain-server/actions/workflows/ci.yml)
  [![Cost](https://img.shields.io/badge/cost-%240%20per%20query-success.svg)](#)
  [![Privacy](https://img.shields.io/badge/privacy-100%25%20local-6f42c1.svg)](#)

</p>

<p align="center">

  [![UMP Conformance](https://img.shields.io/badge/UMP%201.0-L3%20verified-success.svg)](docs/universal-memory-protocol.md)
  [![Tests](https://img.shields.io/badge/tests-932%20passed-brightgreen.svg)](#)
  [![EU AI Act](https://img.shields.io/badge/EU%20AI%20Act-Art%2050%20transparency-6f42c1.svg)](COMPLIANCE.md)
  [![CoP Notice](https://img.shields.io/badge/CoP%20notice-self--attested-6f42c1.svg)](COMPLIANCE.md)
  [![GDPR](https://img.shields.io/badge/GDPR-DSAR%20ready-6f42c1.svg)](COMPLIANCE.md)
  [![Audit](https://img.shields.io/badge/audit-SHA--256%20hash%20chain-blue.svg)](COMPLIANCE.md)

</p>

<p align="center">
  Web + desktop + mobile GUI (Dioxus) · OpenAI-compatible embeddings · MCP server · Native OpenClaw memory plugin
</p>

---

## Why not a cloud memory service

Zep, Mem0, and Letta Cloud charge per read and write, hold your agent's memory in someone else's datacenter, and add a round trip to every recall. Brain Server flips all three:

| Cloud memory | Brain Server |
|---|---|
| Per-query cost | 0 decision tokens, 0 embedding tokens. Static local embeddings. |
| Data egress | Data stays on the device. SQLite, no telemetry. |
| Network latency | sub-50ms p99 recall, no round trip. |

That is the whole pitch: zero per-query cost, zero data egress, zero recall latency, one Rust binary.

## Features

- **Hybrid retrieval.** Vector KNN plus lexical FTS5, merged with Reciprocal Rank Fusion and deterministic PRF expansion. Every result carries provenance.
- **Knowledge graph.** Entities and relationships extracted from `[[relation::entity]]` in markdown, with faithful multi-hop explanations.
- **Temporal evidence.** Every ingest records `observed_at`, `valid_from`, and `valid_to`. Ask for any point in time and get the fact as it was then.
- **Honest when it is unsure.** Recall abstains with `{decision: "low_confidence"}` instead of returning a confident wrong answer. `/verify` checks that a claim really appears in a chunk's text.
- **Human-gated writes.** A proposal is scored, but it becomes memory only after a person approves it. Approvals bind to the exact bytes the reviewer saw (`content_digest`, mismatch = rejection), so a decision can never bless content that would render differently (v1.27.12).
- **Governance and compliance.** An append-only SHA-256 audit chain, a DSAR workflow that exports, purges, and issues a deletion certificate, and recall traces. Maps to ISO 42001, NIST AI RMF, and SOC 2.
- **Universal Memory Protocol (UMP 1.0).** A full implementation of the open [Universal Memory Protocol](https://github.com/edihasaj/universal-memory-protocol) standard for portable agent memory: signed records, capability tokens, HTTP + MCP + file bindings, conformance level L3. Memory written here can be read by any other UMP agent.
- **One daemon, fully embedded.** The `brain-server` daemon bundles SQLite + sqlite-vec (no external services) and runs anywhere Rust compiles; a `brain` CLI, `mcp`, `bench`, and connector binaries ride the same codebase.
- **Easy to wire up.** OpenAI-compatible embeddings, an MCP server, a `brain` CLI, a Dioxus GUI, and a native OpenClaw memory plugin.

## Quick start

```bash
cargo build --release --features bench
./target/release/brain-server
```

The server listens on `127.0.0.1:8765` and writes SQLite data to `~/.openclaw/workspace/brain.db` by default.

### Docker (pilot in under five minutes)

```bash
docker compose up -d
curl http://127.0.0.1:8765/health
```

The image bakes the embedding model at build time (offline boot), runs as a
non-root user with `read_only` rootfs, and defaults to loopback-only binding.
For SSO in front of the server: `docker compose --profile sso up -d` (see
[docs/proxy-sso.md](docs/proxy-sso.md)). Full container reference:
[docs/docker.md](docs/docker.md).

```bash
# Health
curl http://localhost:8765/health

# Ingest markdown. [[relation::entity]] links build the graph.
curl -X POST http://localhost:8765/ingest/markdown \
  -d '{"title":"Bignay","content":"Bignay is [[alternative_to::blueberry]]."}'

# Recall
curl -X POST http://localhost:8765/recall \
  -d '{"query":"blueberry alternative","provenance":true}'
```

Run it as a launchd service on macOS:

```bash
scripts/install-service.sh
```

## Documentation

| | |
|---|---|
| **Docs site** | [brain-server Docs](https://markfietje.github.io/brain-server/) — the mdBook documentation site (Overview, Human in the loop, Quickstart, Architecture, Deployment, Security, Compliance, API, Reference, Roadmap). |
| **Docs** | [`docs/`](./docs/): Overview, Human in the loop, Quickstart, Architecture, Deployment, Security, Compliance, API, Reference, Roadmap. |
| **API contract** | [`API_CONTRACT.md`](./API_CONTRACT.md) plus `GET /openapi.yaml` at runtime. |
| **Compliance** | [`COMPLIANCE.md`](./COMPLIANCE.md): ISO 42001, NIST AI RMF, SOC 2, GDPR · [`docs/AI_LITERACY.md`](./docs/AI_LITERACY.md): Art 4 literacy playbook · [`docs/RFP_RESPONSE_KIT.md`](./docs/RFP_RESPONSE_KIT.md): enterprise RFP mapping. |
| **Security** | [`SECURITY.md`](./SECURITY.md) and [`THREAT_MODEL.md`](./THREAT_MODEL.md). |
| **Product site** | [`docs/product-site/`](./docs/product-site/): landing, install, quickstart, editions. |
| **Research** | [`docs/research/`](./docs/research/): one scientific explainer per retrieval mechanism (problem → reference → deterministic implementation → ceiling). |
| **Blog** | [`docs/blog/`](./docs/blog/): one technical-buyer post per hard-won mechanism (compliance-time-bomb framing, deterministic HITL, tamper-evident audit, reference-faithful retrieval, no-lock-in, OWASP 2026 as sales doc, the honest ceiling, Profiles preview, DeepSeek Harness integration). |
| **Media kit** | [`docs/media-kit.md`](./docs/media-kit.md): positioning, one-liners, and a Brain-vs-Mem0/LangGraph/RAG sizing table with honest ceilings. |
| **Trust / proof map** | [`docs/trust/proof-map.md`](./docs/trust/proof-map.md) + [`docs/trust/reproduce.md`](./docs/trust/reproduce.md): every security/compliance claim mapped to its shipped release and live `curl`/`brain` proof. |
| **Roadmap** | [`ROADMAP.md`](./ROADMAP.md). |

## MCP & agent-harness integration

**Try it — paste this, see your MCP memory server answer in ~2 seconds:**

```bash
curl -L -o mcp https://github.com/markfietje/brain-server/releases/latest/download/mcp-darwin-arm64
chmod +x mcp
echo '{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}' | ./mcp
```

It replies `{"jsonrpc":"2.0","id":1,"result":{...,"name":"brain-server-mcp"}}` —
your MCP memory server, answering the modern (2026-07-28) protocol. Use
`darwin-x86_64` / `linux-x86_64` / `linux-aarch64` to match your platform.

Brain Server ships a **Model Context Protocol (MCP) server** (`mcp`): a
line-delimited JSON-RPC 2.0 / stdio binary that translates MCP tool calls
(`brain_search`, `brain_recall`, `brain_ingest` + nine `ump.*` governance tools)
into HTTP calls against a running brain-server. It targets both the modern
(2026-07-28, `server/discover`) and legacy (2025-11-25) MCP specs, so it drops
into any MCP-capable agent harness.

That includes **DeepSeek Harness (`dsh`)**, whose everything-is-a-plugin
architecture accepts third-party **memory MCP servers** through its generic
`@deepseek-ai/dsh-mcp-client` bridge — the documented slot Brain Server's `mcp`
binary fills, backed by the open [Universal Memory Protocol](https://github.com/edihasaj/universal-memory-protocol)
at conformance level **L3** (13/13 reference checks, CI-pinned). See the
[dsh integration guide](./docs/mcp.md#deepseek-harness-dsh) for the pinned
install + Cordis overlay, and the [background post](./docs/blog/10-dsh-deepseek-harness.md).

## Client GUI

The Dioxus app in `client/` runs on web, desktop, iOS, and Android from one Rust codebase. It gives operators a visual, WCAG 2.2 AA compliant surface with panels for Review (human-gated write-back with A/S/R/J/K keyboard), Recall (decision-path viewer), Subjects (DSAR certificate card), Security (audit chain and quarantine), Audit (filters and export), Health, plus Overview, Graph, Data, Ump, System, Ops, Register, and role-gated Clients views.

```bash
cd client && ./deploy-web.sh
```

## Universal Memory Protocol

Brain Server implements the open [Universal Memory Protocol](https://github.com/edihasaj/universal-memory-protocol) (UMP 1.0) end to end, at conformance level L3: signed records with tamper-evident integrity, capability tokens for consent-based access, batch ingest, and three bindings (HTTP, MCP tools, and a portable Markdown file format). Any UMP 1.0 agent can read memory written here, and this server can read memory exported by other UMP implementations.

```bash
# Create the operator identity
brain ump keygen

# Write and read portable memory over HTTP
curl -X POST localhost:8765/ump/remember \
  -H "Authorization: Bearer <capability>" -H "Content-Type: application/json" \
  -d '{"kind":"fact","content":"Dave runs the alpha team.","scope":"global"}'
curl localhost:8765/ump/recall \
  -H "Authorization: Bearer <capability>" -H "Content-Type: application/json" \
  -d '{"query":"alpha"}'

# Export everything as UMP Markdown, import it back on another host
brain ump export --out memory.ump.md
brain ump import memory.ump.md
```

Details live in [API_CONTRACT.md §15](API_CONTRACT.md#15-ump-binding-v1173--universal-memory-protocol-10) and the [Universal Memory Protocol docs page](docs/universal-memory-protocol.md).

## Configuration

All settings are environment variables, resolved in `src/config.rs`.

| Variable | Default | Description |
|---|---|---|
| `BIND_HOST` / `BIND_PORT` | `127.0.0.1` / `8765` | Bind address and port. `0.0.0.0` is refused unless `BIND_PUBLIC=1`. |
| `BRAIN_DB_PATH` | `~/.openclaw/workspace/brain.db` | SQLite database path. |
| `AUTH_TOKEN` / `AUTH_TOKEN_FILE` | `(none)` | Opaque bearer token(s). Newline-separated enables live rotation. |
| `BRAIN_JWT_ISSUER` | `(none)` | Enables JWT/JWS mode when set and keys are loaded. |
| `INJECTION_POLICY` | `quarantine` | `quarantine`, `reject`, or `allow`. |
| `BRAIN_AUDIT_READ_EVENTS` | `on` (JWT) / `off` (loopback) | Opt-in read-event audit. |
| `BRAIN_AUDIT_RETENTION_DAYS` | `(none)` | Audit retention window. Unset keeps everything. |
| `BRAIN_SUGGEST_ENABLED` | `true` | `/suggest/*` kill switch. |

The full list is on the [Configuration docs page](docs/configuration.md).

## API

The complete contract is served at `GET /openapi.yaml` and documented in [`API_CONTRACT.md`](./API_CONTRACT.md). Every response carries `X-Api-Version`.

**Probe, core, and ops**

| Method | Path | Purpose |
|---|---|---|
| GET | `/health` · `/health/db` · `/ready` · `/version` · `/stats` | Probe (minimal `{status, version}`), Read-gated detail, readiness, version, stats. |
| POST | `/recall` | Structured recall, the primary endpoint. |
| POST | `/ingest` · `/ingest/markdown` · `/ingest/memory` | Ingest structured, markdown, or memory. |
| GET | `/get/{id}` · POST `/multi-get` | Fetch chunks by id. |
| GET | `/graph/entity/{name}` · `/graph/relations` · `/graph/traverse` · `/graph/relationships/{id}/history` | Knowledge-graph queries (bounded walks, hop explanations, edge supersession lineage). |
| POST | `/verify` | Claim span verification. |
| POST | `/classify` · `/decision/{id}/evaluate` · `/procedure` · GET `/procedure/{id}/steps` | Deterministic categorization, decision rules, procedural memory + reversal steps. |
| POST | `/v1/embeddings` | OpenAI-compatible embeddings endpoint. |
| POST | `/reindex` · GET `/metrics` · GET `/events` · GET `/openapi.yaml` | Index rebuild, Prometheus metrics, SSE event feed, API contract. |
| POST | `/webhooks/{kind}` | Signed external webhook ingestion. |
| POST | `/sources/reconcile` · DELETE `/sources/{id}` | Supervised source reconciliation and source deletion. |
| GET | `/quarantine` · POST `/quarantine/{id}/release` · `/quarantine/{id}/delete` | Prompt-injection quarantine review surface. |

**Governance and write-back**

| Method | Path | Purpose |
|---|---|---|
| POST | `/ingest/proposal` · `/proposals/{id}/approve` · `/reject` · `/proposals/{id}/edit` | Human-in-the-loop write-back; approvals bind to the displayed bytes (`content_digest`, v1.27.12). |
| POST | `/consolidate/propose` · `/apply` · `/undo` | Reviewable consolidation. |
| GET/POST | `/dsar` · GET `/tombstones` | DSAR (with dry-run footprint preview) and the deletion registry. |
| GET | `/dsar/{id}/certificate` · `/recall/{trace_id}/trace` | DSAR certificates and recall decision traces. |
| GET | `/audit` · `/audit/verify` | Audit log and chain integrity. |
| POST | `/suggest` · `/suggest/feedback` · GET `/suggest/metrics` | Opt-in anticipation + feedback and recency metrics. |
| GET | `/export` · POST `/purge` · DELETE `/memory/{id}` | GDPR export and hard, audited deletion. |
| GET | `/decayed` · `/retention` · `/art30` · `/retention/report` · `/snapshot/status` | Expiry review, per-kind retention, Art 30 register, snapshot self-check. |

**Policy — profiles, roles, connectors**

| Method | Path | Purpose |
|---|---|---|
| GET/POST | `/profiles/{name}` · `/domains/{name}/profile` | Preset knob bundles + domain binding (v1.21). |
| GET/POST | `/roles/{name}` | Role postures and capability sets (v1.23). |
| GET | `/connectors` · POST `/connectors/register` | Connector registry, profile-gated registration (v1.24). |

**Domains**

| Method | Path | Purpose |
|---|---|---|
| GET/POST | `/domains` | List (per-domain counts) / create a domain pool. |
| DELETE | `/domains/{name}?confirm=<name>` | Delete a domain (echo-confirm; `global` protected). |
| POST | `/domains/{name}/vacuum` · `/domains/recompute` · `/domains/move` | Reclaim pages, recompute centroids, bulk relabel. |
| GET/POST | `/domains/{name}/export` · `/domains/{name}/import` | Consistent SQLite snapshot out / restore into a new domain. |

**BPO clients register (v1.27)**

| Method | Path | Purpose |
|---|---|---|
| POST/GET | `/clients` · GET `/clients/{name}` | Client register (one domain per client; auditor row-filtering). |
| POST | `/clients/{name}/dsar` · `/clients/{name}/hold` · `/clients/{name}/end` | Per-client DSAR, legal hold, termination. |
| GET | `/clients/{name}/proposals` | Supervisor QA queue. |

**Compliance — breach, legal hold, transfers**

| Method | Path | Purpose |
|---|---|---|
| POST | `/legal-hold` · `/legal-hold/{id}/release` · GET `/legal-holds` | Per-domain holds; held ids frozen (v1.22). |
| POST | `/breach` · `/breach/{id}/event` · `/breach/{id}/close` | Breach-notification workflow with jurisdiction deadlines (v1.25). |
| POST/GET | `/transfers` · GET `/transfers/{id}/tia` · `/transfers/{id}/dpa` | Cross-border transfer register + TIA/DPA evidence (v1.26). |

**Portability (UMP 1.0)**

| Method | Path | Purpose |
|---|---|---|
| GET | `/ump/capabilities` · `/.well-known/ump.json` | UMP discovery. |
| POST | `/ump/remember` · `/ump/recall` | Portable memory writes and retrieval. |
| GET | `/ump/memory/{id}` · POST `/ump/revise` · `/ump/forget` | Record reads, updates, and consent-based deletion. |
| POST | `/ump/feedback` · GET `/ump/audit` · `/ump/audit/verify` | UMP feedback and audit surface + chain verification. |
| GET | `/ump/subscribe` | SSE change feed. |

**Auth and discovery (JWT mode)**

| Method | Path |
|---|---|
| POST | `/auth/refresh` · `/logout` · `/revoke` |
| GET | `/.well-known/openid-configuration` · `/.well-known/jwks.json` |

## CLI

```bash
brain status                  # health and stats
brain query "q" --k 3         # structured recall
brain explain "q"             # provenance and telemetry
brain ingest-dir ./vault      # ingest a vault
brain check-consistency       # duplicates, conflicts, stale sources
brain resolve <new> <old>     # supersede a fact
brain suggest "<context>"     # opt-in anticipation
brain backup <out-path> --passphrase-file PATH  # Argon2id + AES-256-GCM encrypted backup (--format v1|v2|v3, v3 default; passphrase required)
brain token rotate                          # atomically rotate the bearer token (new 0600 file, old token invalidated on restart)
```

## Security

- **Loopback-safe by default.** Refuses `0.0.0.0` unless `BIND_PUBLIC=1`.
- **Two auth modes.** Opaque bearer (default) or JWT/JWS (opt-in, RS256/ES256/EdDSA only) with per-route AuthZ and OIDC/JWKS discovery.
- **Append-only SHA-256 audit chain.** Tamper-evident and hash-only.
- **Prompt-injection quarantine.** Suspicious input is stored but excluded from retrieval.
- **Untrusted-evidence boundary.** Every result serializes `untrusted: true` (OWASP LLM01:2025), and recalled context carries per-hit provenance tags (ingest kind, memory kind, lawful basis, region) inside the untrusted-data fence so the model can attribute what it recalls (v1.27.12).
- **Audited approval integrity.** `/proposals` returns the read-canonical review form plus a stable `content_digest`; approvals with a stale digest are rejected (`409`) — the reviewer's decision binds to the bytes shown.
- **Rotatable bearer tokens.** `brain token rotate` atomically replaces the auth token; the server refuses to run with group/world-readable secret files (fail-closed).
- **Encrypted backup.** Argon2id KDF + AES-256-GCM (format v3), checksummed, and excludes secrets.

See [`SECURITY.md`](./SECURITY.md) and the [Security docs page](docs/security.md).

## Tech stack

Rust · Axum · rusqlite (WAL) · r2d2 · tokio · model2vec (`minishlab/potion-retrieval-32M`) · Dioxus (client)

| | |
|---|---|
| **Model** | Default `minishlab/potion-retrieval-32M` (512-dim, static); neural tiers via `MODEL_PROFILE=enterprise` (BGE-M3, 1024-d), `MODEL_PROFILE=desktop` (gte-base-en-v1.5, 768-d), or `MODEL_PROFILE=compact` (potion-base-2M — light English static, formerly the mislabeled `multilingual`); opt-in cross-encoder rerank tier: `mixedbread-ai/mxbai-rerank-large-v1` (fallback `bge-reranker-v2-m3`) on enterprise/desktop/quality-local |
| **Latency** | sub-50ms p99 recall |
| **License** | MIT |

## Contributing

Contributions are welcome. Please read [`CONTRIBUTING.md`](./CONTRIBUTING.md) and the [Code of Conduct](./CODE_OF_CONDUCT.md). CI enforces these gates, so run them locally first:

```bash
cargo fmt --check
cargo clippy --all-targets --features bench -- -D warnings
cargo test --features bench
```

For security issues, do not open a public issue. Use the GitHub *Report a vulnerability* tab.

## Contact

Maintained by **Mark Fietje** — connect on
[LinkedIn](https://www.linkedin.com/in/markfietje/). For project support and
feature requests, prefer GitHub Issues.

## License

[MIT](./LICENSE) © 2026 Mark Fietje
