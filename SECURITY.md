# Security Policy

**Last reviewed:** 2026-07-26 against OWASP Top 10:**2025** + Cheat Sheet Series
(Context7-verified), OWASP Multi-Tenant Security Cheat Sheet, OWASP JSON Web
Token Cheat Sheet, OWASP Secrets Management Cheat Sheet, OWASP gRPC + Microservices
Security Cheat Sheets, OWASP Transport Layer Security Cheat Sheet.

---

## Supported Versions

| Version | Supported          | Notes |
| ------- | ------------------ | ----- |
| 1.0.x   | :white_check_mark: | Current LTS line |
| 0.9.x   | :white_check_mark: | Maintained for back-compat |
| < 0.9   | :x:                | Unsupported |

## Reporting a Vulnerability

**Please do not file public issues for security vulnerabilities.**

- **Email**: security@openclaw.dev (GPG key fingerprint published separately)
- **GitHub Security Advisories**: Use the "Report a vulnerability" tab in this repository
- **SLA**: Acknowledgement within 48 hours; fix timeline within 5 business days;
  public disclosure coordinated with reporter (90-day default per Project Zero).
- **PGP-encrypted reports preferred** for sensitive disclosures (key on the
  project's `/.well-known/security.txt` once published).

---

## Threat Model (STRIDE)

Full threat model in [`THREAT_MODEL.md`](./THREAT_MODEL.md). Summary here.

| Threat class (STRIDE) | Brain-server exposure | Mitigation | Status |
|---|---|---|---|
| **S**poofing | Agent → brain-server, brain-server → peer (A2A) | Bearer token (v1.1) → JWT/JWS + mTLS (v1.2/v3.7) | ✅ / 🚧 |
| **T**ampering | Audit log, content at rest, JWT payload | Hash-chained audit (v1.1 M2.3); parameterized SQL (all versions); JWS signature (v1.2) | ✅ / 🚧 |
| **R**epudiation | "Who did this write?" | Append-only audit with `(actor, ts, hash, prev_hash)` (v1.1 M2) | ✅ |
| **I**nformation disclosure | Cross-tenant leak, PII egress | Per-tenant file isolation (v1.0); AuthZ layer (v1.2); SQLCipher + per-field encryption (v3.7) | ✅ / 🚧 |
| **D**enial of service | Burst, large body, vector query | Body limit + per-IP limiter (v0.9.4); per-tenant + tiered limiter (v2.1) | ✅ / 🚧 |
| **E**levation of privilege | Token scope escalation | Constant-time token compare (v1.1); AuthZ trait with deny-by-default (v1.2) | ✅ / 🚧 |

---

## OWASP Top 10:**2025** Coverage

This is the canonical reference. Each item maps to a control in brain-server.

### A01:2025 — Broken Access Control ✅ / 🚧

**Requirement:** Deny by default. Enforce record ownership. Per-request AuthZ
at the data-access layer. Short-lived tokens + refresh.

| Control | Where | Status |
|---|---|---|
| Loopback-only default (no token → loopback only) | `auth_middleware` | ✅ |
| AuthZ enforcement at the data layer | `AuthzPolicy::authorize` trait | 🚧 v1.2 |
| JWT short-lived (≤15 min exp) + refresh | `auth/refresh` + token verifier | 🚧 v1.2 |
| Tenant isolation via file-per-domain | `DomainRegistry` | ✅ v1.0 |
| AuthZ failures logged with `(principal, action, target)` | `audit_events` | 🚧 v1.2 |
| Cross-tenant returns 403, never 404 (don't leak existence) | AuthZ layer | 🚧 v1.2 |

**Citation:** OWASP A01:2025 — "Access control is only effective when
implemented in trusted server-side code... deny by default... enforce record
ownership... log access control failures."

### A02:2025 — Security Misconfiguration ✅

| Control | Where | Status |
|---|---|---|
| Auth off by default (loopback-safe) | `config::auth_token` | ✅ |
| CORS allowlist, no wildcard | `CorsLayer` exact origins | ✅ |
| Bind address loopback by default | `BIND_HOST=127.0.0.1` | ✅ |
| Public base URL explicit (`BRAIN_PUBLIC_BASE_URL`) | `/.well-known/` (v1.2) | 🚧 |
| Reproducible build (`Cargo.lock` checked in) | repo | ✅ |
| Hardened systemd unit on openclaw | `ProtectSystem=strict`, etc. | ✅ |

### A03:2025 — Software Supply Chain Failures ✅

| Control | Where | Status |
|---|---|---|
| `cargo audit` in CI (`.github/workflows/ci.yml`) | CI | ✅ |
| Pinned direct deps in `Cargo.toml` | repo | ✅ |
| Optional features for high-surface deps (reqwest, jsonwebtoken) | Cargo features | ✅ |
| Reproducible release build | `Cargo.lock`, `opt-level="z"`, `lto="fat"` | ✅ |

### A04:2025 — Cryptographic Failures ✅ / 🚧

| Control | Where | Status |
|---|---|---|
| Tokens compared constant-time | `auth_middleware` | ✅ |
| JWT RS256/ES256/EdDSA only (never HS256 in distributed) | `verify_access_token` | 🚧 v1.2 |
| `alg` whitelist + reject `none` + reject algorithm confusion | JWT verifier | 🚧 v1.2 |
| AES-256-GCM for backups (v0.9.7) | `backup.rs` | ✅ |
| HMAC-SHA256 for webhook verification (v0.9.7) | `webhook.rs` | ✅ |
| SQLCipher at rest (AES-256 per-page) | KMS trait | 🚧 v3.7 |
| No hardcoded secrets (env / file / KMS) | all of `config.rs` | ✅ |

**Citation:** OWASP Secrets Management Cheat Sheet — "Transitioning to
passwordless authentication shifts security requirements toward protecting
dynamic bearer tokens. Security must be maintained by ensuring tokens are
transmitted exclusively over TLS, stored in secure locations rather than
browser local storage, and validated for signature, issuer, and audience."

### A05:2025 — Injection ✅

| Control | Where | Status |
|---|---|---|
| **Every** SQL statement parameterized (`params![]`) | all `rusqlite` calls | ✅ |
| Zero string interpolation in SQL (audited) | grep-verified | ✅ |
| Prompt-injection heuristic at every query/ingest boundary | `contains_suspicious_pattern` | ✅ (ceiling documented) |
| FTS5 MATCH strings compiled with per-token quoting | `compile_lex` | ✅ |
| Domain name → filename via strict regex `^[a-z0-9][a-z0-9_-]{0,62}$` | `is_valid_domain` | ✅ |
| Content-Disposition header safe (no injection chars) | export handler | ✅ |

### A06:2025 — Insecure Design ✅ / 🚧

| Control | Where | Status |
|---|---|---|
| Threat model documented | `THREAT_MODEL.md` | ✅ |
| Capacity envelope + HTTP 507 (fail-clear on writes) | `capacity.rs` | ✅ |
| Reversible migrations (idempotent + additive + backup) | `migration.rs` | ✅ |
| Rehearsal tool for v1.0 cutover (copy → verify → rollback) | `brain-migrate-rehearse` | ✅ |
| Deny-by-default AuthZ | AuthZ trait | 🚧 v1.2 |
| Threat-model review checkpoint per major release | release process | 🚧 |

### A07:2025 — Authentication Failures ✅ / 🚧

| Control | Where | Status |
|---|---|---|
| Constant-time token compare | `auth_middleware` | ✅ |
| `401` on missing/invalid/malformed | `auth_middleware` | ✅ |
| Audit row on every auth failure | `audit_events` | 🚧 v1.1 M2.1 |
| Token rotation (file-watch) | `AUTH_TOKEN_FILE` hot reload | 🚧 v1.1 M1.4 |
| JWT `(jti, iss)` revocation | `revoked_tokens` table | 🚧 v1.2 |
| Refresh token rotation + reuse detection | refresh endpoint | 🚧 v1.2 |
| Account lockout / rate limit on auth | per-tenant limiter | 🚧 v2.1 |

**Citation:** OWASP JWT Cheat Sheet — "Use the `(jti, iss)` pair because jti
uniqueness is only guaranteed per issuer — a malicious or rogue issuer could
mint a JWT with the same jti as a legitimate one, causing a collision."

### A08:2025 — Software or Data Integrity Failures ✅ / 🚧

| Control | Where | Status |
|---|---|---|
| Signed JWTs (JWS) with key rotation | JWKS endpoint | 🚧 v1.2 |
| Tamper-evident audit (hash chain) | `audit_events.prev_hash` | 🚧 v1.1 M2.3 |
| Content integrity via XXH3-64 hash | `knowledge.content_hash` | ✅ |
| Source revision hashing | `source_revisions.revision` | ✅ |
| Webhook HMAC verification | `webhook.rs` | ✅ |
| Signed releases (GPG + git tag) | `git tag -s` | 🚧 release process |

### A09:2025 — Security Logging and Alerting Failures ✅ / 🚧

| Control | Where | Status |
|---|---|---|
| Append-only audit log | `audit_events` | ✅ |
| Per-tenant audit filter | `audit_events.tenant_id` + data-layer filter | 🚧 v1.1 M2.2 |
| All writes audited (ingest, delete, domain lifecycle) | every write handler | ✅ |
| All authN/authZ events audited | auth middleware → audit row | 🚧 v1.2 M6 |
| Quota warnings audited | `quota.warning` / `quota.exceeded` | 🚧 v2.1 M5 |
| `/health` exposes ops status | `/health` capacity + integrity | ✅ |
| `/metrics` Prometheus exporter | `--features metrics` | 🚧 v1.1 M5 |
| `x-request-id` on every request for tracing | `SetRequestIdLayer` | ✅ |

### A10:2025 — Mishandling of Exceptional Conditions ✅ / 🚧

| Control | Where | Status |
|---|---|---|
| `CatchPanicLayer` prevents process crash on handler panic | middleware stack | ✅ |
| Capacity breach → fail-clear (writes 507, reads 200) | `guard_capacity` | ✅ |
| Pool exhaustion → fail-open for health, fail-closed for writes | pool config | ✅ |
| Migration failure → transaction rollback | `migration.rs` tx | ✅ |
| WAL-active detection before destructive ops | `brain-migrate-rehearse` | ✅ |
| Out-of-memory watchdog → graceful restart | systemd `Restart=on-failure` | 🚧 v1.1 M5 |

---

## Defense in Depth Layers

```
┌────────────────────────────────────────────────────────────────────────┐
│  Reverse Proxy (TLS 1.3, WAF, IP allowlist, edge rate limit)           │
│  ─ Caddy / nginx / Cloudflare — TLS terminates here                    │
│  ─ HSTS preload-eligible; OCSP stapling; modern cipher suite           │
│  ─ Per-IP rate limit (defense against unauthenticated floods)          │
├────────────────────────────────────────────────────────────────────────┤
│  Axum Middleware Stack (inner → outer)                                 │
│  1.  RequestBodyLimitLayer      2 MB                                   │
│  2.  TimeoutLayer               30 s, returns 408                      │
│  3.  CatchPanicLayer            prevents process crash                 │
│  4.  SetSensitiveHeadersLayer   redacts Authorization/Cookie           │
│  5.  CompressionLayer           gzip/br/zstd                           │
│  6.  SetRequestIdLayer          generates x-request-id                 │
│  7.  PropagateRequestIdLayer    passes through x-request-id            │
│  8.  TraceLayer                 structured HTTP logging                │
│  9.  CorsLayer                  exact-origin allowlist                 │
│  10. security_headers_middleware HSTS, CSP, X-Frame, Permissions-Policy │
│  11. rate_limit_middleware     per-tenant + tier (v2.1)                │
│  12. auth_middleware           JWT/JWS verify + (jti, iss) revocation  │
│  13. authz_middleware          AuthzPolicy::authorize (v1.2)           │
│  14. SetResponseHeaderLayer    Server: brain-server                     │
├────────────────────────────────────────────────────────────────────────┤
│  Application Layer                                                     │
│  ─ Handler resolves authorized pool via AuthZ trait (never direct)     │
│  ─ Parameterized SQL only (no string interpolation)                    │
│  ─ Input validated before any DB work                                  │
│  ─ Every write emits an audit row in the caller's tenant               │
└────────────────────────────────────────────────────────────────────────┘
```

---

## Authentication & Authorization

### v1.1 — opaque bearer token (current)

- `AUTH_TOKEN_FILE` (preferred; 0600 file) or `AUTH_TOKEN` env var.
- Constant-time compare.
- Public routes (`/health`, `/health/db`, `/ready`, `/version`, `/.well-known/*`) bypass.
- File-watch for hot rotation (v1.1 M1.4).

### v1.2 — JWT/JWS + AuthZ layer (in progress)

- **Algorithm whitelist**: RS256, RS384, RS512, ES256, ES384, ES512, EdDSA.
- **Forbidden**: `none`, all HMAC variants (algorithm confusion CVE class).
- **Verified claims**: `iss`, `aud`, `exp`, `nbf`, `sub`, `jti`.
- **Revocation**: `(jti, iss)` table, 60s negative-cache.
- **Refresh tokens**: separate JWS, ≤24h, rotated on use, reuse-detection.
- **AuthZ trait** with deny-by-default; InMemory policy + pluggable OPA/Cedar.
- **Per-route enforcement matrix** documented in `IMPLEMENTATION_PLAN_v1.2.0_AuthN.md` §M3.3.

---

## Cryptography

| Purpose | Algorithm | Status |
|---|---|---|
| Content deduplication | XXH3-64 (non-cryptographic, fast) | ✅ |
| Source revision hash | XXH3-64 | ✅ |
| Document ID | XXH3-64 of title | ✅ |
| Audit row hash chain | SHA-256 | 🚧 v1.1 M2.3 |
| Auth token compare | Constant-time `==` | ✅ |
| JWT signature | RS256 / ES256 / EdDSA | 🚧 v1.2 |
| JWT revocation key | `(jti, iss)` SHA-256 | 🚧 v1.2 |
| Webhook verification | HMAC-SHA256 | ✅ |
| Backup encryption | AES-256-GCM | ✅ |
| Database at rest | SQLCipher AES-256 per-page | 🚧 v3.7 |
| Per-field PII encryption | AES-256-GCM via HKDF-derived key | 🚧 v3.7 |
| TLS termination | TLS 1.3 (reverse proxy) | ✅ (operator) |
| mTLS service-to-service | TLS 1.3 + SPIFFE/SPIRE-compatible identity | 🚧 v3.7 |

**No encryption at rest today** — SQLite files are plaintext. For encryption
before v3.7: filesystem encryption (LUKS, FileVault, BitLocker) or `sqlcipher`
via proxy.

---

## Multi-Tenant Isolation

OWASP Multi-Tenant Cheat Sheet (Context7-verified 2026-07-26):

> "Derive tenant context from authenticated, verified tokens. Use database-level
> isolation like RLS or schemas as a defense in depth. Include tenant_id in all
> resource queries, cache keys, and storage paths."

### brain-server's model (stronger than RLS)

- **File-per-tenant**: each tenant's data lives in a separate SQLite file
  (`team-<name>/<domain>.db`). This is **physical isolation**, stronger than
  PostgreSQL RLS (which is logical).
- **Tenant context from verified JWT** (v1.2): `claims.tenant` field, verified
  by signature. Never from query string or body.
- **AuthZ layer (v1.2)**: handler never sees a pool it isn't authorized for.
- **Per-tenant rate limiting + quotas** (v2.1).
- **Per-tenant audit filter** at the data layer (v1.1 M2.2).
- **Per-tenant encryption keys** via KMS (v3.7).

### Known ceilings

- **Shim mode (single-DB)**: tenant isolation is row-level via the `domain`
  column. AuthZ enforces at the SQL `WHERE` clause. Less strong than physical
  isolation; documented; multi-db mode is the recommended path for true
  multi-tenant deployments.
- **Global `audit_events` table in shim mode**: shared across tenants. The
  per-tenant filter at the data layer (`WHERE tenant_id = ?`) prevents leakage
  in normal reads, but a malicious SQL injection could theoretically read all
  tenants. Multi-db mode moves `audit_events` into each tenant's DB (planned
  v2.0).

---

## Rate Limiting

### v0.9.x → v1.x — per-IP in-memory (current)

- 100 req/min per IP. Single-process only. Sufficient for loopback + LAN.

### v2.1 — per-tenant + tiered (in progress)

- Keyed on `(tenant_id, route_class)`, derived from verified JWT.
- Tiered limits (free / starter / business / enterprise).
- `RateLimiter` trait: `InMemory` (default) or `RedisRateLimiter` (atomic
  Lua script via GCRA, behind `--features ratelimit-redis`).
- Standard `X-RateLimit-*` + `Retry-After` headers on every response.
- Fail-open for reads on Redis error, fail-closed for writes (configurable).

**Citation:** OWASP Multi-Tenant Cheat Sheet — "Implement per-tenant rate
limiting and quotas, and log tenant context with every operation."

---

## Secrets Management

OWASP Secrets Management Cheat Sheet (Context7-verified 2026-07-26):

> "Automating the rotation of static secrets is crucial as manual rotation is
> challenging and prone to mistakes."

### Hierarchy

1. **File (default, v0.9+)**: `~/.config/brain-server/auth-token` mode 0600.
   Hot-rotatable via file-watch (v1.1 M1.4).
2. **JWT signing keys (v1.2)**: `~/.config/brain-server/keys/` mode 0700;
   private keys mode 0600; rotated via `brain key rotate` CLI.
3. **External KMS (v3.7, BYOK pattern)**:
   - `BRAIN_KMS_PROVIDER=file|vault|aws`
   - `FileKeyProvider` (default; mode 0600 keys)
   - `VaultKeyProvider` (HashiCorp Vault; transit secret engine)
   - `AwsKmsKeyProvider` (AWS KMS; envelope encryption)

### No secrets in

- Process arguments (`ps aux` readable on shared hosts)
- Environment variables when avoidable (visible in `launchctl print` /
  `/proc/<pid>/environ`)
- Logs (`SensitiveHeadersLayer` redacts; `tracing` configured for metadata only)
- Error messages (never echo raw user input back)
- Source control (audited; `cargo audit` verifies transitive)

---

## TLS / Transport Security

**Architecture:** TLS terminates at the reverse proxy (Caddy / nginx /
Cloudflare). The brain-server Rust binary speaks HTTP on loopback only.

### Operator responsibilities

- TLS 1.3 minimum (`--tls-min-v1.3` on Caddy/nginx).
- Modern cipher suite only (Mozilla "modern" compatibility list).
- OCSP stapling.
- HSTS header (`max-age=63072000; includeSubDomains; preload`) when
  `TLS_ENABLED=1`.
- Per-IP rate limit at the edge.
- Certificate rotation (Let's Encrypt / ACME on Caddy is fine).
- mTLS for service-to-service when A2A lands (v3.7) — client cert signed by
  `BRAIN_A2A_TRUST_CA`, ≤24h lifetime, automated rotation.

### Why TLS at the proxy

The Rust binary is single-process, single-tenant at the binary level. TLS
termination requires per-connection state, SNI routing, OCSP caches — all
better handled by a purpose-built proxy. The binary focuses on application-
level security (AuthN, AuthZ, audit, isolation).

---

## Compliance Attestations (mapping)

These frameworks are not implemented as code; this table documents which
controls brain-server maps to, for procurement.

| Framework | Coverage |
|---|---|
| **SOC 2 Type II** | CC6.1 (auth), CC6.3 (AuthZ), CC7.1 (audit), CC7.2 (monitoring), CC3.3 (key rotation v3.7) |
| **ISO 27001:2022** | A.5.15 (access control), A.5.16 (identity management), A.8.2 (privileges), A.8.15 (logging), A.8.24 (cryptography) |
| **GDPR** | Art. 25 (privacy by design — local-first), Art. 30 (audit trail), Art. 32 (encryption at rest via v3.7 SQLCipher) |
| **HIPAA Security Rule** | §164.312(a)(1) access control, §164.312(b) audit, §164.312(e)(2)(ii) encryption |
| **PCI DSS v4.0** | Req 7 (least privilege), Req 8 (auth), Req 10 (logging), Req 3 (encryption at rest) |

**Honest caveat:** mapping is the engineering side only. An enterprise SOC 2
attestation requires an external auditor; the documents and policies
themselves (incident response plan, change management, employee training)
are operations work, not engineering.

---

## Secure Deployment Checklist

- [ ] Set `AUTH_TOKEN_FILE` to a 0600-mode file containing ≥32 chars of entropy.
- [ ] Set `CORS_ORIGINS` to exact production origins (no wildcards).
- [ ] Terminate TLS at reverse proxy; set `TLS_ENABLED=1` for HSTS.
- [ ] Configure proxy: TLS 1.3 min, modern cipher suite, OCSP stapling.
- [ ] Configure proxy: per-IP rate limit (defense against unauthenticated floods).
- [ ] Enable `BRAIN_MULTI_DB=true` for multi-tenant isolation (file-per-tenant).
- [ ] Run `cargo audit` before deploy; pinned `Cargo.lock`.
- [ ] Verify `/health` and `/ready` accessible to load balancer.
- [ ] Confirm `x-request-id` appears in aggregated logs.
- [ ] Enable systemd hardening drop-ins (sample in `scripts/`).
- [ ] For multi-tenant: complete the v1.2 + v2.0 + v2.1 release chain first.
- [ ] For compliance: configure external KMS (v3.7) for SQLCipher keys.
- [ ] For federation: deploy mTLS + JWKS (v3.7) before opening A2A to peers.

---

## Version History

| Version | Date | Changes |
|---|---|---|
| 1.1 (planned) | 2026-Q3 | Per-tenant audit + hash chain; file-watch token rotation; CSRF scaffold; Prometheus exporter |
| 1.2 (planned) | 2026-Q3 | JWT/JWS verification (RS256/ES256/EdDSA); `(jti, iss)` revocation; AuthZ trait + middleware; JWKS endpoint; OIDC discovery |
| 2.0 (planned) | 2026-Q4 | Multi-team tenancy consuming v1.2 AuthZ; team namespace in paths |
| 2.1 (planned) | 2026-Q4 | Per-tenant + tiered rate limiting; Redis impl; cost tracking; quota alerts |
| 3.7 (planned) | 2027-Q1 | A2A federation over mTLS; SQLCipher + KMS abstraction; per-field PII encryption; differential privacy |
| 1.0.1 | 2026-07-26 | Structured-ingest auto-create bug fix; systemd deploy on openclaw |
| 1.0.0 | 2026-07-26 | Multi-domain + cross-domain RRF + domain lifecycle + boot-time legacy cutover |
| 0.9.9 | 2026-07-25 | Capacity envelopes; migration rehearsal tool; storage layout |
| 0.9.4 | 2026-07-11 | Full middleware stack hardening (rate limit, security headers, request ID, timeout, panic catch, sensitive header redaction, compression, tracing) |
| 0.9.2 | 2026-07-11 | Vault ingest provenance, wikilink KG edges, prompt injection guard |
| 0.9.0 | 2026-07-08 | sqlite-vec migration, parameterized queries, CORS lockdown, auth scaffold |
