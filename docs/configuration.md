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
| `AUTH_TOKEN` / `AUTH_TOKEN_FILE` | — | Opaque bearer token(s). Newline-separated = live rotation. **Off if unset.** Twokeys (v1.28.70): with a token FILE, line 1 = operator (full authority) and line 2 = the agent token — agent bearers authenticate as the scoped `agent@loopback` principal (no Admin, no purge/domains/revoke/dsar, no DPO boards; writes land as proposals under `BRAIN_WRITE_POSTURE=review`; the Blackout kill-switch revokes it by name). A single line keeps the legacy all-superuser posture — a boot warn is the nudge, never a forced migration. `AUTH_TOKEN` env content keeps the all-operator semantics. |
| `BRAIN_REQUIRE_AUTH` | — | Refuse unauthenticated boot (v1.28.80): `1` fails startup when no token resolves; unset keeps the loopback single-user default with a loud boot warning. Any other value refuses boot (fail-closed parse). |
| `BRAIN_ALLOW_WILDCARD_GRANT` | — | Admit total-grant scopes (v1.28.80): `1` lets a scope wildcarding both team and domain (`*/*`) grant; unset means such scopes grant nothing. Loud boot warning when admitted. |
| `AGENT_TOKEN_FILE` | — | Alternative agent-token source (0600 file, one bearer string — same secret-file law as `AUTH_TOKEN_FILE`). When set, the agent token comes from here and the operator token file's ENTIRE content stays operator. Boot-time source: a swapped agent file takes effect at restart (the rotation watcher follows the operator file; a line-2 edit reloads live with it). A leaked (group/world-readable) or empty agent file **refuses the boot**. |
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

`MODEL_PROFILE` selects the retrieval profile. (`BRAIN_MODEL_PROFILE` is not
a config key; it appears only inside a re-embed hint string.) Each resolves to an
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
| `BRAIN_INJECTION_CLASSIFIER` | `on` | Layer-2 classifier selector (v1.28.71 "Pores" auto-on): `on`/unset loads when the default artifact `~/.config/brain-server/models/injection-classifier/{model.onnx,tokenizer.json}` resolves (`absent` posture otherwise, layer 1 unaffected); `off` opts out; any other value is an explicit model path — a non-existent path **refuses the boot** (fail-closed). Echoed as `injection_classifier: on\|off\|absent` on `/health/db`. Operators who prefer an external verdict (a guard-model HTTP endpoint in front of ingest) can leave this `off` and enforce at their own seam; layer 1 still runs. |
| `BRAIN_INJECTION_TOKENIZER` | — | Tokenizer used by the injection classifier (required alongside an explicit `BRAIN_INJECTION_CLASSIFIER` path) |
| `BRAIN_INJECTION_THRESHOLD_HIGH` | `0.9` | Classifier banding: score ≥ this → reject |
| `BRAIN_INJECTION_THRESHOLD_LOW` | `0.7` | Classifier banding: score ≥ this (below high) → quarantine |
| `BRAIN_PROPOSAL_TTL_SECS` | `604800` (7 d) | How long a proposal can sit pending before auto-expire (audited). |
| `BRAIN_APPROVAL_QUORUM` | `1` | Two-principal approvals (v1.28.80): `2` requires two distinct approvers before a proposal promotes (first returns `pending_second`, same-principal repeat refused). Any other value refuses boot. |
| `BRAIN_EXPORT_MAX_BYTES` | `1073741824` (1 GiB) | Ceiling on the materialized GDPR export bundle; a bare byte count overrides, anything else (including 0) refuses boot. The chunked export path is the escape hatch past it. |
| `BRAIN_DSAR_WINDOW_DAYS` | `30` | GDPR Art 17 response window shown on DSARs |
| `BRAIN_DSAR_LEDGER_DAYS` | `30` | Retention window for the DSAR ledger |
| `BRAIN_RETENTION_ENABLED` | enabled (`true`) | Per-kind query-time retention expiry; `false\|0\|no\|off` restores exact legacy behavior (only per-chunk `expires_at` governs decay) |
| `BRAIN_RETENTION_KIND_DAYS` | JSON map over SDK defaults | Per-kind overrides as a **JSON map** (`{"fact":365,"episodic":30}`), merged over the built-in table — fact 365, episodic 30, procedure/step/decision 730, entitlement 1825 (single owner: `crates/brain-engine-sdk/src/policy.rs`). Unknown keys are accepted; invalid JSON or non-integer values degrade to the default per key |
| `BRAIN_WRITE_POSTURE` | `open` | Agent-write posture (Seatbelt): `open` writes insert directly; `review` routes the six agent-facing write surfaces through the proposal queue instead (agents propose, operators dispose). An unknown value **refuses boot**. v1.28.75: the installer writes `review` into the plist only when NO explicit posture is set yet (new-install default) — an operator-set value (including a deliberate `open`) is never stomped by a re-run; the compiled default stays `open` so unattended upgrades never change behavior |
| `BRAIN_SYNCHRONOUS` | `full` | Per-connection SQLite durability on the MAIN pool (Headroom): `full` fsyncs every commit (the pre-1.28.59 effective behavior — a fresh pooled connection always ran the compile default); `normal` is the WAL-mode tuning posture (commit fsyncs move to checkpoint time; on power loss recent commits may roll back but the DB stays uncorrupted). Applied beside `busy_timeout=5000` at every pooled connection's init; the applied policy is echoed by `/health/db` under `durability`. An unknown value **refuses boot**. |
| `BRAIN_WAL_AUTOCHECKPOINT` | `1000` | WAL autocheckpoint threshold in pages (Headroom) — the SQLite compile default and the pre-1.28.59 effective value. Lower = checkpoints run more often, bounding `brain_wal_pages_pending` lag at the cost of more frequent checkpoint I/O. Integer, `1..=65536`; `0` (autocheckpoint off — unbounded WAL) and out-of-range values **refuse boot** |
| `BRAIN_LOOM` | off | Opt-in CPU parallelism for the two loom fan-out sites (Loom): the batch-ingest embed stage and the consolidate near-dup scan's pure-CPU preprocessing. Active only when ALL THREE hold: the `loom` cargo feature is compiled in, the capacity target is not `jetson`, and this var is `1`. `0`/unset keeps the byte-identical serial path; any other value **refuses boot** (fail-closed parse). The pool is capped at `min(cores-1, 4)` so ingest never starves the tokio blocking pool; the resolved decision is echoed by `/health/db` as `loom: active (N threads)` or `off:no-feature` / `off:jetson` / `off:env`. No cross-chunk reduction exists by design — every fan-out is an ordered per-item map (`loom_preserves_fused_ranks`) |
| `BRAIN_ALERT_WEBHOOK_URL` / `BRAIN_ALERT_WEBHOOK_SECRET` | — | Outbound alert webhook sink (resolve → validate → pin egress: a private/metadata sink refuses the boot unless `BRAIN_EGRESS_ALLOW_PRIVATE=1`) |

## Observability & audit (v1.15)

| Variable | Default | Description |
|---|---|---|
| `BRAIN_AUDIT_CHAIN_KEY_FILE` | — | Explicit path to the audit-chain HMAC key. Resolution order: this env → `audit-chain.key` beside the DB → a generated 0600 key. A resolution failure is a loud warning, not a boot refusal; writes to `hmac256`-epoch DBs fail closed per-write until a key resolves |
| `BRAIN_AUDIT_SIGNING_KEY_FILE` | — | Explicit path to the Art 50/decision-provenance Ed25519 signing key (0600; installer-provisioned). Absent = marks are present but visibly unsigned |
| `BRAIN_AUDIT_READ_EVENTS` | `on` (JWT) / `off` (loopback) | When `on`, `/recall`, `/search`, `/get/{id}`, `/multi-get` emit hash-chained audit rows (no content, no raw query). |
| `BRAIN_AUDIT_READ_SAMPLE_RATE` | `1.0` | Read-event sampling (0.0..=1.0); `1.0` = every read event. |
| `BRAIN_AUDIT_RETENTION_DAYS` | unset = forever | Audit retention window; when set, expired rows are pruned and the chain re-anchored. Deployers subject to AI Act Art 26(6) guidance: set ≥180. |
| `BRAIN_DSAR_WEBHOOK_URL` / `BRAIN_DSAR_WEBHOOK_SECRET` | — | Opt-in Art 19 onward-notification: on a completed DSAR purge, POSTs `{subject, certified_at, certificate_id}` HMAC-SHA256-signed. Fail-soft. |
| `BRAIN_EGRESS_ALLOW_PRIVATE` | — | The ONE egress opt-out (Deadbolt): `1` admits a private/loopback/metadata webhook sink at boot with a LOUD warn (the sink stays DNS-pinned). Unset = private sinks refuse the boot; any other value refuses the boot (fail-closed parse). |
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

## Webhooks, standby, keys & misc (the unglamorous but real knobs)

| Variable | Default | Description |
|---|---|---|
| `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED` | off | Enforce the Standard-Webhooks timestamp tolerance on webhook receivers (replay-window hardening). |
| `BRAIN_SIGNAL_WEBHOOK_SECRET_FILE` / `BRAIN_KB_FEEDBACK_SECRET_FILE` | — | Per-surface HMAC secrets (Signal gateway; KB feedback relay). |
| `BRAIN_STANDBY_DIR` | `~/.local/share/brain-server/standby` | Warm-standby follower directory (`brain standby start/status/promote-check`). |
| `BRAIN_CAPACITY_TARGET` | `desktop` (`jetson` when unset on unknown hosts — unknown values fail closed to jetson) | The capacity envelope tier (`desktop`\|`jetson`); also gates the loom CPU-parallelism tier. |
| `BRAIN_RSS_RESTART` | — | RSS watchdog restart threshold (breach → graceful self-restart request). |
| `BRAIN_CONNECTOR_CONFIG_DIR` | platform config dir | Connector config dir; included in backups. |
| `BRAIN_AUDIT_CHAIN_KEY` / `_FILE` | — | Key for the hmac256 audit-chain epoch (absent = SHA-256 links; keyed chains refuse to write without the key). |
| `BRAIN_AUDIT_SIGNING_KEY` / `_FILE` | — | Art.12 decision-record signing key. |
| `BRAIN_BACKUP_PASSPHRASE` / `BRAIN_BACKUP_PASSPHRASE_FILE` | — | The backup/restore passphrase ladder (the `--passphrase-file` flag reads the same seam). |
| `BRAIN_TOKEN` / `BRAIN_TOKEN_FILE` | `~/.config/brain-server/auth-token` | The `brain` CLI's bearer resolution ladder (server side: `AUTH_TOKEN_FILE` → `AUTH_TOKEN`). |
| `BRAIN_DPO_CONTACT` / `BRAIN_SECURITY_CONTACT` | — | DPO + security contact strings surfaced on `/health/db` and `/.well-known/security.txt`. |
| `BRAIN_ENGINE_EXEC_ALLOWLIST` / `BRAIN_ENGINE_HTTP_ALLOWLIST` / `BRAIN_ENGINE_WORKDIR` | — | The hostcall door's allowlists + workdir (the engine's tool-effect boundary). |
| `MCP_TRANSPORT` / `MCP_HTTP_PORT` / `MCP_HTTP_ADDR` / `MCP_HTTP_TOKEN` | stdio | The MCP binary's transport: stdio (default) or Streamable HTTP + SSE. See docs/mcp.md. |
| `PACKING_WEIGHTS` | built-in | Evidence-packing weight overrides (advanced). |
| `BRAIN_STEWARD_BIN` | — | Override the workflow-crank harness binary. MUST be an ABSOLUTE path (relative refuses; PATH is never consulted) or the binary lives beside the kernel. |

> **The single source of truth** for every tunable is `src/config.rs` in the repository.

## Next steps

- **[Installation](./deployment.md)** — applying these in practice.
- **[Security](./security.md)** — how the auth variables work together.
- **[API Reference](./api.md)** — the contract those configs gate.

## Auxiliary binaries & harness (client-side env)

These are read by the operator CLIs and optional binaries — not the
server process — so they sit outside the main table.

| Variable | Default | Description |
|---|---|---|
| `BRAIN_URL` | `http://127.0.0.1:8765` | Base URL every client-side binary addresses (`brain`, `mcp`, `bench`, the connector stubs) |
| `BRAIN_MCP_SCOPE` | `full` | MCP dispatch scope (`read\|full`, fail-closed parse): `read` refuses `brain_ingest`, `ump.remember`, `ump.revise`, `ump.forget` at the dispatch seam and annotates them `x-brain-scope: read-denied` in `tools/list` |
| `BRAIN_GH_APP_TOKEN` | — | GitHub App installation token for `brain-connector-gh` (the binary refuses to run on the placeholder) |
| `BRAIN_EVAL_JUDGMENTS` | — | Judged-query fixture path for `bench --eval` (missing file fails the eval run) |

`BENCH_*` harness knobs (`BENCH_SCALES`, `BENCH_SEARCHES`, `BENCH_CLIENTS`,
`BENCH_SEED`, `BENCH_ENVELOPE`, …) are documented in the `bench` binary's
own header (`src/bin/bench.rs`) with worked invocations in
[`BENCHMARKS.md`](../BENCHMARKS.md).
