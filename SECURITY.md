# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.9.x   | :white_check_mark: |
| < 0.9   | :x:                |

## Reporting a Vulnerability

**Please do not file public issues for security vulnerabilities.**

Instead, report them privately via:

- **Email**: security@openclaw.dev (GPG key: `0x...`)
- **GitHub Security Advisories**: Use the "Report a vulnerability" tab in this repository

We aim to acknowledge within 48 hours and provide a fix timeline within 5 business days.

---

## Security Architecture

### Threat Model

| Threat | Mitigation |
|--------|------------|
| **Network eavesdropping** | TLS terminated at reverse proxy (Caddy/nginx/Cloudflare); `Strict-Transport-Security` header enforced when `TLS_ENABLED=1` |
| **Credential theft** | Bearer tokens only in `Authorization` header (never query string); `SensitiveHeadersLayer` redacts `Authorization`/`Cookie` from logs |
| **SQL injection** | All queries use parameterized statements (`rusqlite::params![]`); **zero** string interpolation in SQL |
| **Prompt injection (LLM01)** | Heuristic blocklist at ingest/search boundaries (`contains_suspicious_pattern`); documented ceiling — upgrade path: ML classifier |
| **DoS / resource exhaustion** | `RequestBodyLimitLayer` (1 MB), `TimeoutLayer` (30 s), in-memory rate limiter (100 req/min/IP), connection pool limits (20 max) |
| **Supply chain** | `cargo audit` in CI; pinned transitive deps where possible; `model2vec-rs` monitored for `number_prefix`/`paste` advisories |
| **XSS / content injection** | All stored content treated as plain text; `html_escape` applied only to KG entity names; `Content-Security-Policy: default-src 'none'` |
| **Path traversal** | Domain names validated against `^[a-z0-9][a-z0-9_-]{0,62}$` before use as filenames; `source_path` stored but not used in path construction |

### Defense in Depth Layers

```
┌─────────────────────────────────────────────────────────────┐
│  Reverse Proxy (TLS, WAF, rate limit, IP allowlist)         │
├─────────────────────────────────────────────────────────────┤
│  Axum Middleware Stack (inner → outer)                      │
│  1. RequestBodyLimitLayer      1 MB                         │
│  2. TimeoutLayer               30 s, returns 408            │
│  3. CatchPanicLayer            prevents process crash       │
│  4. SetSensitiveHeadersLayer   redacts Authorization/Cookie │
│  5. CompressionLayer           gzip/br/zstd                 │
│  6. SetRequestIdLayer          generates x-request-id       │
│  7. PropagateRequestIdLayer    passes through x-request-id  │
│  8. TraceLayer                 structured HTTP logging      │
│  9. CorsLayer                  exact-origin allowlist       │
│  10. security_headers_middleware HSTS, CSP, X-Frame, etc.   │
│  11. rate_limit_middleware     100 req/min/IP (in-memory)   │
│  12. auth_middleware           Bearer token validation      │
│  13. SetResponseHeaderLayer    Server: brain-server         │
└─────────────────────────────────────────────────────────────┘
```

### Input Validation

| Endpoint | Limits |
|----------|--------|
| `/add`, `/ingest/*`, `/search`, `/recall`, `/v1/embeddings` | Body ≤ 1 MB, query ≤ 2000 chars |
| `/graph/*` | Entity names: alphanumeric + ` _-` ≤ 100 chars |
| `/ingest/markdown` | Content ≤ 1 MB, title ≤ 500 chars |
| Domain names | `^[a-z0-9][a-z0-9_-]{0,62}$` (safe for filenames) |

### Authentication & Authorization

- **Optional** Bearer token auth on non-public routes. Provide the token via either:
  - `AUTH_TOKEN_FILE` — path to a `0600`-mode file containing the token (**preferred**; keeps the secret out of the process/launchd environment and off `launchctl print`)
  - `AUTH_TOKEN` — the raw env var (convenience/dev only; visible in `launchctl print` and to any same-user process)
  - `AUTH_TOKEN_FILE` takes precedence when both are set
- When set, **all non-public routes** require `Authorization: Bearer <token>`
- Public routes (always unauthenticated): `/health`, `/health/db`, `/ready`, `/version`
- Tokens compared via constant-time string comparison (Rust `==` on `&str` is constant-time for equal-length strings)

### Database Security

- SQLite WAL mode + `foreign_keys=ON`
- Prepared statements exclusively — **no dynamic SQL construction**
- Per-domain isolation via `BRAIN_MULTI_DB=true` (separate `brain-<domain>.db` files)
- Pre-migration backup via `VACUUM INTO` before schema changes
- Content deduplication via XXH3-64 content hash (prevents duplicate injection)

### Prompt Injection Mitigations (OWASP LLM01)

```rust
// ponytail: deliberate simplification — string matching on a tiny blocklist.
// Ceiling: trivially bypassed by encoding, homoglyphs, token smuggling, or
// adversarial suffixes. Upgrade path: replace with a proper classifier
// (e.g., DistilBERT-based prompt-injection detector) when threat model demands.
pub fn contains_suspicious_pattern(input: &str) -> bool { ... }
```

Applied at:
- `/add` (text ingest)
- `/search` & `/recall` (query)
- `/ingest/markdown` (content + title)
- `/ingest/memory` (parsed entries)

### Secure Defaults

| Setting | Default | Override |
|---------|---------|----------|
| CORS origins | `localhost:3000,localhost:8080` (loopback only) | `CORS_ORIGINS` |
| CORS methods | `GET,POST,PUT,DELETE,OPTIONS` | `CORS_METHODS` |
| CORS headers | `content-type,authorization` | `CORS_HEADERS` |
| Bind address | `127.0.0.1:8765` | `BIND_HOST`, `BIND_PORT` |
| Auth | **disabled** (loopback-only safe default) | `AUTH_TOKEN_FILE` (preferred) or `AUTH_TOKEN` |
| TLS | **off** (delegated to proxy) | `TLS_ENABLED=1` for HSTS |

### Observability Security

- `SensitiveHeadersLayer` prevents `Authorization`, `Cookie`, `Set-Cookie` from appearing in structured logs
- `TraceLayer` logs request/response metadata **only** (no bodies)
- `x-request-id` generated/propagated for request correlation across services

### Supply Chain

```bash
# Run locally
cargo audit

# CI runs on every PR
cargo audit --deny warnings
```

Pinned direct dependencies in `Cargo.toml`. Transitive advisories tracked via `cargo audit`.

### Incident Response

1. **Contain**: Revoke `AUTH_TOKEN`, rotate, deploy proxy IP block
2. **Investigate**: Correlate via `x-request-id` in logs
3. **Remediate**: Patch, test, deploy
4. **Postmortem**: Document in `SECURITY_ADVISORIES.md` (private until disclosure)

---

## Secure Deployment Checklist

- [ ] Set `AUTH_TOKEN` to a high-entropy value (≥32 chars). Prefer `AUTH_TOKEN_FILE` pointing at a `0600` file over the raw env var.
- [ ] Set `CORS_ORIGINS` to exact production origins (no wildcards)
- [ ] Terminate TLS at reverse proxy; set `TLS_ENABLED=1` for HSTS
- [ ] Configure proxy rate limits (e.g., Caddy `rate_limit`, nginx `limit_req`)
- [ ] Enable `BRAIN_MULTI_DB=true` for multi-tenant isolation
- [ ] Run `cargo audit` before deploy
- [ ] Verify `/health` and `/ready` endpoints accessible to load balancer
- [ ] Confirm `x-request-id` appears in aggregated logs

---

## Cryptography

| Purpose | Algorithm |
|---------|-----------|
| Content deduplication | XXH3-64 (non-cryptographic, fast) |
| Source revision hash | XXH3-64 |
| Document ID | XXH3-64 of title |
| Tokens | Opaque bearer tokens (compare via `==`) |

**No encryption at rest** — SQLite files are plaintext. For encryption, use filesystem encryption (LUKS, FileVault, BitLocker) or a proxy like `sqlcipher`.

---

## Version History

| Version | Date | Changes |
|---------|------|---------|
| 0.9.4 | 2026-07-11 | Full middleware stack hardening (rate limit, security headers, request ID, timeout, panic catch, sensitive header redaction, compression, tracing) |
| 0.9.2 | 2026-07-11 | Vault ingest provenance, wikilink KG edges, prompt injection guard |
| 0.9.0 | 2026-07-08 | sqlite-vec migration, parameterized queries, CORS lockdown, auth scaffold |