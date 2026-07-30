# Brain Server

**A local-first semantic-memory and knowledge-graph server for AI agents. Runs on a 4 GB ARM device drawing < 5 W — no GPU, no cloud, no per-query cost.**

Static (no-neural-net) embeddings via `model2vec` / `minishlab/potion-retrieval-32M` make recall essentially free on CPU. Designed for edge hardware (Jetson Nano, Raspberry Pi 5, mini PC) but runs anywhere Rust compiles. The competitive wedge vs. cloud memory services (Zep, Mem0, Letta Cloud): **zero per-query cost, zero data egress, zero network latency on recall.**

| | |
|---|---|
| **Version** | 1.4.2 "Link" (noise-reduction: pipe-table/list-item-bold exclusion, heading-number stripping, orphan sweep) |
| **Model** | `minishlab/potion-retrieval-32M` (512-dim, static, ~120 MiB RSS) |
| **Stack** | Rust 2021 · Axum · rusqlite (WAL) · r2d2 · tokio |
| **Power envelope** | < 5 W idle on Jetson Nano (the selling point) |
| **Latency** | sub-50ms p99 recall on the reference device |
| **Documentation** | [API Contract](./API_CONTRACT.md) · [`GET /openapi.yaml` live](#) · [Technical Spec](./SPECS.md) |

---

## Features

- **Hybrid retrieval** — vector KNN (`vec0`) + lexical FTS5 (BM25) fused via Reciprocal Rank Fusion, with deterministic PRF query expansion and full per-result provenance.
- **Structured query** — `QueryDoc` with `LexSpec` (phrases, exclusions, code paths), multi-source OR scope, temporal `since`/`as_of` predicates.
- **Temporal evidence** — every ingest stamps `observed_at` / `valid_from` / `valid_to` / `authority`. Point-in-time recall returns the revision active at a timestamp. `RecallHit.conflict` flags contradictory/superseded hits.
- **Knowledge graph** — entities and relationships extracted from `[[relation::entity]]` syntax in markdown. Traverse, query, and follow links.
- **Source lifecycle** — every chunk carries provenance (`source` + immutable `revision`). Connectors backfill external sources through a supervised pipeline; `POST /reconcile` sweeps orphans from deleted sources.
- **Connectors** — supervised ingesters (GitHub issues via App auth) that backfill through the existing source/revision pipeline. Extensible connector contract.
- **Prompt-injection quarantine** — suspicious content stored but excluded from retrieval until reviewed.
- **Append-only audit log** — ingest and auth-denial events recorded hash-only.
- **Encrypted backup/restore** — AES-256-GCM, checksummed backups.
- **OpenAI-compatible embeddings** — `POST /v1/embeddings`.
- **MCP server** — `mcp` binary exposes search/recall/ingest as MCP tools.

---

## Quick start

```bash
# Build all binaries
cargo build --release --features bench
# or: cargo build --release --features bench,connector-github

# Run the server
./target/release/brain-server

# Or install as a service (macOS launchd)
scripts/install-service.sh
```

The server binds to `127.0.0.1:8765` and creates a SQLite database at the configured path (default: `brain.db` in the current directory, or `BRAIN_DB_PATH`).

```bash
# Health check
curl http://localhost:8765/health
curl http://localhost:8765/stats

# Ingest a markdown document
curl -X POST http://localhost:8765/ingest/markdown \
  -H 'Content-Type: application/json' \
  -d '{"title":"Bignay","content":"Bignay is [[alternative_to::blueberry]]. It has [[has_property::antioxidants]]."}'

# Structured recall
curl -X POST http://localhost:8765/recall \
  -H 'Content-Type: application/json' \
  -d '{"query":"blueberry alternative","provenance":true}'

# Knowledge graph
curl http://localhost:8765/graph/entity/bignay
curl 'http://localhost:8765/graph/traverse?start=bignay&max_depth=2'
```

---

## API

| Method | Path | Purpose |
|---|---|---|
| GET | `/health`, `/health/db`, `/ready` | Liveness |
| GET | `/stats`, `/version` | Counts + model + version |
| GET | `/openapi.yaml` | Full API contract |
| POST | `/add` | Raw text ingest *(deprecated)* |
| POST | `/ingest/memory` | Structured memory ingest |
| POST | `/ingest/markdown` | Markdown ingest + graph extraction |
| POST | `/ingest` | Structured ingest (explicit entities/relations) |
| GET | `/search?q=&k=&lex=&sources=` | Semantic search *(deprecated; use `/recall`)* |
| POST | `/recall` | Structured recall (`QueryDoc`, primary endpoint) |
| GET | `/get/{id}` · POST `/multi-get` | Fetch chunk(s) by id |
| POST | `/reindex` | Rebuild vector/FTS indexes |
| DELETE | `/memory/{id}` | Delete a chunk (tombstone) |
| GET | `/graph/entity/{name}` | Entity + 1-hop relations |
| GET | `/graph/relations?from=&to=` | Relations between entities |
| GET | `/graph/traverse?start=&max_depth=` | Recursive walk (max 3) |
| POST | `/sources/reconcile` · DELETE `/sources/{id}` | Source lifecycle |
| GET | `/domains` | Multi-domain status (debug) |
| GET | `/connectors` | Registered connectors |
| POST | `/webhooks/{kind}` | Verified webhook ingest (HMAC, replay-protected) |
| POST | `/auth/refresh` · `/auth/logout` · `/auth/revoke` | Token lifecycle (JWT mode) |
| GET | `/.well-known/openid-configuration` · `/.well-known/jwks.json` | OIDC discovery + JWKS |
| GET | `/audit` | Append-only audit events (hash-only) |
| GET | `/quarantine` · POST `/quarantine/{id}/release` · `/delete` | Injection review |
| POST | `/v1/embeddings` | OpenAI-compatible embeddings |

Every response carries `X-Api-Version`. Full contract at [API_CONTRACT.md](./API_CONTRACT.md) and `GET /openapi.yaml`.

---

## CLI (`brain`)

| Command | Purpose |
|---|---|
| `brain doctor` / `brain status` | Health + stats |
| `brain query "q"` [`--phrase` …] [`--exclude` …] [`--code` …] [`--source` …] [`--since` …] [`--k N`] [`--explain`] | Structured recall |
| `brain get <id>` | Fetch a chunk |
| `brain explain "q"` | Provenance + telemetry |
| `brain ingest-dir <path>` [--dry-run] | Ingest a vault directory |
| `brain reconcile <path>` [--dry-run] | Sweep deleted sources |
| `brain source-delete <id>` | Retire a source |
| `brain connect github --app-id N --install-id N --key-file PATH --repo O/R` | Configure GitHub connector |
| `brain sync [github]` `[--config PATH \| --instance NAME]` | Run a connector sync |
| `brain connector-status` | List registered connectors |
| `brain audit [--kind K] [--limit N]` | Read audit log |
| `brain key generate` [`--kind rsa`] | Generate a JWT signing keypair (JWT mode) |
| `brain key list` / `brain key prune` | List / prune JWT signing keys |
| `brain backup <db> <out>` / `brain restore <backup> <db>` | Encrypted backup/restore |

---

## Configuration

| Variable | Default | Description |
|---|---|---|
| `BIND_HOST` | `127.0.0.1` | Bind address. `0.0.0.0` refused unless `BIND_PUBLIC=1`. |
| `BIND_PORT` | `8765` | Listen port |
| `BRAIN_DB_PATH` | `brain.db` | SQLite database path |
| `CORS_ORIGINS` | `localhost:3000,localhost:8080` | CORS allowlist |
| `AUTH_TOKEN` / `AUTH_TOKEN_FILE` | — | Opaque bearer token(s) (v1.1 default). Newline-separated = live rotation. Off if unset. |
| `BRAIN_JWT_ISSUER` | — | Enables **JWT mode** when set + keys loaded. URL of the issuer (verified against the `iss` claim). |
| `BRAIN_JWT_KEY_DIR` | `~/.config/brain-server/keys/` | Directory holding JWT signing key PEMs (mode 0700; private keys 0600). |
| `BRAIN_PUBLIC_BASE_URL` | — | Public base URL for OIDC discovery (`/.well-known/openid-configuration`). Never inferred from `Host`. |
| `BRAIN_JWT_AUDIENCE` | `brain-server` | Expected `aud` claim value. |
| `INJECTION_POLICY` | `quarantine` | `quarantine` \| `reject` \| `allow` |
| `PRF_ENABLED` / `PRF_DEPTH` / `PRF_TERMS` / `PRF_MAX_RANK` | `true` / `10` / `5` / `5` | PRF expansion |

All tunables: [`src/config.rs`](./src/config.rs).

---

## Security

- **Loopback-safe by default.** Refuses `0.0.0.0` unless `BIND_PUBLIC=1`.
- **Two authentication modes** (JWT is opt-in, opaque is the default):
  - **Opaque bearer** (v1.1 default): `AUTH_TOKEN` / `AUTH_TOKEN_FILE`.
    Multiple tokens accepted (live rotation). Constant-time compare.
  - **JWT/JWS** (v1.2 opt-in): set `BRAIN_JWT_ISSUER` + load keys via
    `brain key generate`. RS256/ES256/EdDSA only (never HS256/`none`);
    `(jti, iss)` revocation; refresh-chain reuse detection; per-route AuthZ.
    OIDC discovery at `/.well-known/openid-configuration`, JWKS at
    `/.well-known/jwks.json`.
- **Verified webhooks** — HMAC verification, replay-window enforcement, idempotency.
- **Append-only audit log** — ingest + auth-denial events, hash-only.
- **Prompt-injection quarantine** — suspicious input stored but excluded from retrieval. Deterministic structural control, not a classifier.
- **Untrusted-evidence boundary** (OWASP LLM01:2025) — every result serializes `untrusted: true`.
- **Encrypted backup/restore** — AES-256-GCM, checksummed, excludes secrets.
- Connector credentials at mode `0600` with atomic writes. No outbound HTTP in the server process.

See [SECURITY.md](./SECURITY.md).

---

## Build

The release profile is size-optimized: `opt-level="z"`, `lto="fat"`, `codegen-units=1`, `strip`, `panic="abort"`.

```bash
# Release build (all binaries: brain-server, brain, mcp, bench, brain-connector-stub)
cargo build --release --features bench

# With GitHub connector
cargo build --release --features bench,connector-github

# CI
cargo fmt --check
cargo clippy --all-targets --features bench -- -D warnings
cargo test --features bench
```

---

## License

MIT.
