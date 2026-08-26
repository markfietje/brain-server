# Configuration

Brain Server is configured entirely through **environment variables**, all resolved in `src/config.rs`. There is no config file to edit. This page is the complete reference, grouped by concern.

## Core server

| Variable | Default | Description |
|---|---|---|
| `BIND_HOST` | `127.0.0.1` | Bind address. `0.0.0.0` refused unless `BIND_PUBLIC=1`. |
| `BIND_PORT` | `8765` | Listen port |
| `BRAIN_DB_PATH` | `~/.openclaw/workspace/brain.db` | SQLite database path |
| `BRAIN_DATA_ROOT` | — | v1.0 relocation knob — root for all on-disk paths |
| `BRAIN_WORKER_THREADS` | # cores | Tokio runtime worker threads (set `2` on Jetson) |
| `CORS_ORIGINS` | `http://localhost:3000,http://localhost:8080` | CORS allowlist |
| `BRAIN_CLIENT_DIST` | `client/dist` | Directory served at `/app` (the web GUI) |
| `BRAIN_CHAIN_CHECK_SECS` | `60` | How often the background audit-chain integrity check runs |
| `BRAIN_MULTI_DB` | — | Enables per-domain SQLite files (multi-DB mode) |
| `BRAIN_CONTROLLER_NAME` | — | Operator/controller identity label |
| `MODEL_PROFILE` | `edge-default` | Retrieval profile selector → embedding model + rerank arming. See [Retrieval profiles & embedding models](#retrieval-profiles--embedding-models). |
| `DOMAIN_MIN_COUNT` | `1` | Minimum chunk count for a domain's routing centroid (below it, the centroid is deleted so routing skips the near-empty bucket) |
| `BRAIN_MODEL_MANIFEST` | — | Path to a SHA-256 model manifest; when set, boot **fails closed** unless every pinned artifact matches |
| `BRAIN_REGION` | — | Data-residency stamp (e.g. `eu-west-1`, `ph-manila`) written onto stored rows + certificates; unset = no stamp |
| `BRAIN_FCR_WINDOW_DAYS` | `7` | First-contact-resolution repeat-contact attribution window on the workflow scoreboard (a recurring contact within the window counts the predecessor as not resolved) |
| `BRAIN_REASK_WINDOW_DAYS` | `3` | Re-ask duplicate-detection window: two OPEN CRM cases with the same hashed subject within this window file a pending `case_merge_suggested` HITL proposal (exact hash match only — no fuzzy matching, nothing merges automatically) |
| `BRAIN_CASE_STATUS_KEY_FILE` | — | 0600-mode salt file for the public case-status ref HMAC (`BRAIN_CASE_STATUS_KEY` inline as last resort). Unreadable/wide-mode file fails closed; without any salt configured, ref minting refuses |

## Authentication

| Variable | Default | Description |
|---|---|---|
| `AUTH_TOKEN` / `AUTH_TOKEN_FILE` | — | Opaque bearer token(s). Newline-separated = live rotation. **Off if unset.** |
| `BRAIN_JWT_ISSUER` | — | Enables **JWT mode** when set + keys loaded. URL of the issuer (verified against the `iss` claim). |
| `BRAIN_JWT_KEY_DIR` | `~/.config/brain-server/keys/` | Directory holding JWT signing key PEMs (mode 0700; private keys 0600). |
| `BRAIN_JWT_AUDIENCE` | `brain-server` | Expected `aud` claim value. |
| `BRAIN_PUBLIC_BASE_URL` | — | Public base URL for OIDC discovery. **Never** inferred from `Host`. |
| `BRAIN_UMP_KEY_DIR` | `~/.config/brain-server/ump/` | Directory holding the UMP operator Ed25519 signing key (distinct from the JWT key dir). |
| `BRAIN_TRUST_PROXY` | off | When set, trust `X-Forwarded-For` from the named proxy for real-IP + rate-limit accounting. Off by default so a spoofed header can't bypass rate limits. |

## Retrieval & expansion

| Variable | Default | Description |
|---|---|---|
| `PRF_ENABLED` | `true` | PRF query expansion on/off |
| `PRF_DEPTH` | `10` | PRF expansion depth |
| `PRF_TERMS` | `5` | Number of expansion terms |
| `PRF_MAX_RANK` | `5` | Max rank for expansion candidates |
| `BRAIN_RECALL_ROUTING_ENABLED` | `true` | Automatic retrieval routing (v1.13.1). `false` restores legacy shim behavior. |
| `BRAIN_GRAPH_RESCUE_ENABLED` | `true` | Complexity-gated graph rescue pass on abstention (v1.12) |

### Retrieval profiles & embedding models

`MODEL_PROFILE` (also `BRAIN_MODEL_PROFILE`) selects the retrieval profile. Each resolves to an
embedding model via `config::model_id_for_profile` + `embed::embedder_for_profile`. Note: the old
`multilingual` profile name is wrong — `potion-base-2M` is an **English** model (distilled from
`BAAI/bge-base-en-v1.5`), not multilingual. It was renamed **`compact`** (the smallest static
model); `MODEL_PROFILE=multilingual` still resolves to the same profile for backward compatibility.

| Profile | Embedding model | Dim | Backend | Rerank tier armed at boot |
|---|---|---|---|---|
| `edge-default` (default) | `minishlab/potion-retrieval-32M` | 512 | static `model2vec` | no |
| `quality-local` | `minishlab/potion-retrieval-32M` | 512 | static `model2vec` | yes |
| `compact` (was `multilingual`) | `minishlab/potion-base-2M` | 512 | static `model2vec` | no |
| `air-gapped` | `minishlab/potion-retrieval-32M` | 512 | static `model2vec` | no |
| `enterprise` | `BAAI/bge-m3` (`--features neural-embed`) | 1024 | FastEmbed `BGEM3Q` | yes |
| `desktop` | `Alibaba-NLP/gte-base-en-v1.5` (`--features neural-embed`) | 768 | FastEmbed `GTEBaseENV15` | yes |

`enterprise`/`desktop` require the `neural-embed` Cargo feature (pulls `fastembed`); without it they
fall back to the static default model. The migration creates `vec_knowledge` at the active
embedder's `store_dim()` and stamps `embedding_dim` — switching profiles across dimensions fails
closed (a 1024-d DB refuses an `edge-default` start with the `--re-embed` instruction).

### Rerank tier

The cross-encoder rerank tier (`rerank-tier` Cargo feature) runs **after** RRF fusion on the profiles
that arm it (see table above); it is **off by default** (edge stays pure-static, the v0.9.5
doctrine). The server sets `BRAIN_RERANK_ENABLED=1` at boot for those profiles. It is **fail-open**
(a model/output fault leaves the RRF order untouched) and **boot-warmed** (never downloaded in the
request path). Model resolution, in order: the golden **`mixedbread-ai/mxbai-rerank-large-v1`**
(BYO-ONNX, int8) loaded from a local dir, falling back to the in-enum **`BAAI/bge-reranker-v2-m3`**.

| Variable | Default | Description |
|---|---|---|
| `BRAIN_RERANK_MODEL_DIR` | `models/mxbai-rerank-large-v1/` | Local dir holding the mxbai-rerank-large-v1 files (`onnx/model_quantized.onnx` + the 4 tokenizer files) for the BYO-ONNX seam. |
| `BRAIN_RERANK_TOP_N` | `50` | Max candidates scored per rerank call; beyond this the provenance `rerank_truncated` flag reports the drop honestly. |

## Write-back gating (v1.14)

PII control is deterministic **read-time output redaction** (always-on for
principals without `pii:read`/Admin); there is no write-time placeholder vault
and no `BRAIN_REDACT_PII` knob (removed v1.20.19).

| Variable | Default | Description |
|---|---|---|
| `INJECTION_POLICY` | `quarantine` | `quarantine` \| `reject` \| `allow` — how prompt-injection-suspicious input is handled. |
| `BRAIN_INGEST_SKIP_PATTERNS` | — (off) | Newline- or comma-separated prefixes; text beginning with any is skipped at ingest (e.g. `!redacted,```). Opt-in; default behavior unchanged. |
| `BRAIN_INJECTION_CLASSIFIER` | — | Injection classifier selector |
| `BRAIN_INJECTION_TOKENIZER` | — | Tokenizer used by the injection classifier |
| `BRAIN_INJECTION_THRESHOLD_HIGH` | `0.9` | Classifier banding: score ≥ this → reject |
| `BRAIN_INJECTION_THRESHOLD_LOW` | `0.7` | Classifier banding: score ≥ this (below high) → quarantine |
| `BRAIN_PROPOSAL_TTL_SECS` | `604800` (7 d) | How long a proposal can sit pending before auto-expire (audited). |
| `BRAIN_DSAR_WINDOW_DAYS` | `30` | GDPR Art 17 response window shown on DSARs |
| `BRAIN_DSAR_LEDGER_DAYS` | `30` | Retention window for the DSAR ledger |
| `BRAIN_RETENTION_ENABLED` | enabled (`true`) | Per-kind query-time retention expiry; `false\|0\|no\|off` restores exact legacy behavior (only per-chunk `expires_at` governs decay) |
| `BRAIN_RETENTION_KIND_DAYS` | JSON map over SDK defaults | Per-kind overrides as a **JSON map** (`{"fact":365,"episodic":30}`), merged over the built-in table — fact 365, episodic 30, procedure/step/decision 730, entitlement 1825 (single owner: `crates/brain-engine-sdk/src/policy.rs`). Unknown keys are accepted; invalid JSON or non-integer values degrade to the default per key |
| `BRAIN_WRITE_POSTURE` | `open` | Agent-write posture (Seatbelt): `open` writes insert directly; `review` routes the six agent-facing write surfaces through the proposal queue instead (agents propose, operators dispose). An unknown value **refuses boot** |
| `BRAIN_ALERT_WEBHOOK_URL` / `BRAIN_ALERT_WEBHOOK_SECRET` | — | Outbound alert webhook sink (uses the hardened egress client) |

## Observability & audit (v1.15)

| Variable | Default | Description |
|---|---|---|
| `BRAIN_AUDIT_READ_EVENTS` | `on` (JWT) / `off` (loopback) | When `on`, `/recall`, `/search`, `/get/{id}`, `/multi-get` emit hash-chained audit rows (no content, no raw query). |
| `BRAIN_AUDIT_READ_SAMPLE_RATE` | `1.0` | Read-event sampling (0.0..=1.0); `1.0` = every read event. |
| `BRAIN_AUDIT_RETENTION_DAYS` | unset = forever | Audit retention window; when set, expired rows are pruned and the chain re-anchored. Deployers subject to AI Act Art 26(6) guidance: set ≥180. |
| `BRAIN_DSAR_WEBHOOK_URL` / `BRAIN_DSAR_WEBHOOK_SECRET` | — | Opt-in Art 19 onward-notification: on a completed DSAR purge, POSTs `{subject, certified_at, certificate_id}` HMAC-SHA256-signed. Fail-soft. |
| `BRAIN_OTEL_ENABLED` / `BRAIN_OTEL_ENDPOINT` | enabled on `--features otel` builds / `http://127.0.0.1:4318/v1/traces` | OpenTelemetry OTLP export. Kill-switch only: `0\|false\|no\|off` disables the compiled-in exporter (a default build compiles no exporter at all) |
| `CORS_METHODS` | `GET,POST,PUT,DELETE,OPTIONS` | Allowed CORS methods |
| `CORS_HEADERS` | `content-type,authorization` | Allowed CORS request headers |

## Features & kill switches

| Variable | Default | Description |
|---|---|---|
| `BRAIN_SUGGEST_ENABLED` | `true` | v1.9 kill switch: when `false`, the `/suggest/*` routes return `501`. |
| `BRAIN_RECALL_GRAPH_ENABLED` | `true` | v1.12 kill switch for the graph (Personalized PageRank) recall leg — `false` disables it process-wide (per-request `graph=false` still works). |
| `BRAIN_MAX_DOMAIN_DBS` | `256` | v1.27.16 cap on registered per-domain SQLite files; registration beyond the cap fails closed (`507 insufficient_storage`). |

## Capacity envelope (v0.9.9)

| Variable | Default | Description |
|---|---|---|
| `CAPACITY_MAX_DOCS` / `CAPACITY_MAX_DB_MIB` / `CAPACITY_MAX_RSS_MIB` | capacity profile | Tighten the `/health` capacity envelope. Writes over the envelope return HTTP 507; reads are never blocked. |

> **The single source of truth** for every tunable is `src/config.rs` in the repository.

## Next steps

- **[Installation](./deployment.md)** — applying these in practice.
- **[Security](./security.md)** — how the auth variables work together.
- **[API Reference](./api.md)** — the contract those configs gate.
