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
| `BRAIN_CLIENT_DIR` | `client/dist` | Directory served at `/app` (the web GUI) |
| `BRAIN_CHAIN_CHECK_SECS` | `60` | How often the background audit-chain integrity check runs |
| `BRAIN_MULTI_DB` | — | Enables per-domain SQLite files (multi-DB mode) |
| `BRAIN_CONTROLLER_NAME` | — | Operator/controller identity label |
| `MODEL_PROFILE` | — | Model profile selector (affects embedding defaults) |
| `DOMAIN_MIN_COUNT` | — | Minimum chunk count for a domain to be listed/used |

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
| `BRAIN_INJECTION_THRESHOLD_HIGH` | — | Classifier banding: score ≥ this → reject |
| `BRAIN_INJECTION_THRESHOLD_LOW` | — | Classifier banding: score ≥ this (below high) → quarantine |
| `BRAIN_PROPOSAL_TTL_SECS` | `604800` (7 d) | How long a proposal can sit pending before auto-expire (audited). |
| `BRAIN_DSAR_WINDOW_DAYS` | `30` | GDPR Art 17 response window shown on DSARs |
| `BRAIN_DSAR_LEDGER_DAYS` | `30` | Retention window for the DSAR ledger |
| `BRAIN_RETENTION_ENABLED` | — | Enable per-kind query-time retention expiry |
| `BRAIN_RETENTION_KIND_DAYS` | — | Per-kind retention overrides (`kind=days,kind=days`) |
| `BRAIN_ALERT_WEBHOOK_URL` / `BRAIN_ALERT_WEBHOOK_SECRET` | — | Outbound alert webhook sink (uses the hardened egress client) |

## Observability & audit (v1.15)

| Variable | Default | Description |
|---|---|---|
| `BRAIN_AUDIT_READ_EVENTS` | `on` (JWT) / `off` (loopback) | When `on`, `/recall`, `/search`, `/get/{id}`, `/multi-get` emit hash-chained audit rows (no content, no raw query). |
| `BRAIN_AUDIT_READ_SAMPLE_RATE` | `1.0` | Read-event sampling (0.0..=1.0); `1.0` = every read event. |
| `BRAIN_AUDIT_RETENTION_DAYS` | unset = forever | Audit retention window; when set, expired rows are pruned and the chain re-anchored. Deployers subject to AI Act Art 26(6) guidance: set ≥180. |
| `BRAIN_DSAR_WEBHOOK_URL` / `BRAIN_DSAR_WEBHOOK_SECRET` | — | Opt-in Art 19 onward-notification: on a completed DSAR purge, POSTs `{subject, certified_at, certificate_id}` HMAC-SHA256-signed. Fail-soft. |
| `BRAIN_OTEL_ENABLED` / `BRAIN_OTEL_ENDPOINT` | off | Optional OpenTelemetry export (endpoint + on/off) |
| `CORS_METHODS` | `GET,POST,PUT,DELETE,OPTIONS` | Allowed CORS methods |
| `CORS_HEADERS` | `content-type,authorization` | Allowed CORS request headers |

## Features & kill switches

| Variable | Default | Description |
|---|---|---|
| `BRAIN_SUGGEST_ENABLED` | `true` | v1.9 kill switch: when `false`, the `/suggest/*` routes return `501`. |

## Capacity envelope (v0.9.9)

| Variable | Default | Description |
|---|---|---|
| `CAPACITY_MAX_DOCS` / `CAPACITY_MAX_DB_MIB` / `CAPACITY_MAX_RSS_MIB` | capacity profile | Tighten the `/health` capacity envelope. Writes over the envelope return HTTP 507; reads are never blocked. |

> **The single source of truth** for every tunable is `src/config.rs` in the repository.

## Next steps

- **[Installation](./deployment.md)** — applying these in practice.
- **[Security](./security.md)** — how the auth variables work together.
- **[API Reference](./api.md)** — the contract those configs gate.
