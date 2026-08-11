# Security Policy

**Last reviewed:** 2026-08-08 against OWASP Top 10:**2025** + Cheat Sheet Series
(Context7-verified), OWASP Multi-Tenant Security Cheat Sheet, OWASP JSON Web
Token Cheat Sheet, OWASP Secrets Management Cheat Sheet, OWASP gRPC + Microservices
Security Cheat Sheets, OWASP Transport Layer Security Cheat Sheet.

---

## Supported Versions

| Version | Supported          | Notes |
| ------- | ------------------ | ----- |
| 1.16.x  | :white_check_mark: | Current — client ("Integrated") + v1.14/v1.15 governance |
| 1.14.x  | :white_check_mark: | Gate — PII redaction + record-level access_scope |
| 1.12.x  | :white_check_mark: | Harden line — AuthZ wiring + audit fixes |
| 1.2.x   | :white_check_mark: | AuthN line — JWT/JWS + AuthZ foundation (opaque bearer back-compat) |
| 1.0.x   | :white_check_mark: | LTS line (opaque bearer, domains) |
| 0.9.x   | :white_check_mark: | Maintained for back-compat |
| < 0.9   | :x:                | Unsupported |

**Support window.** The current minor (`1.16.x`) and the previous minor receive
fixes; the `0.9.x`/`1.0.x`/`1.2.x` lines receive back-compat/security fixes only.
There is no fixed end-of-life date; any line's deprecation is announced at least
one minor release in advance. A machine-readable disclosure endpoint is published
at [`/.well-known/security.txt`](http://127.0.0.1:8765/.well-known/security.txt)
(RFC 9116) — `Expires`/`Canonical` always, and `Contact` only when
`BRAIN_SECURITY_CONTACT` is set (`BRAIN_PUBLIC_BASE_URL` adds `Canonical`).

## Reporting a Vulnerability

**Please do not file public issues for security vulnerabilities.**

- **GitHub Security Advisories**: Use the "Report a vulnerability" tab in this repository
- **SLA**: Acknowledgement within 48 hours; fix timeline within 5 business days;
  public disclosure coordinated with reporter (90-day default per Project Zero).

## SBOM (Software Bill of Materials)

A CycloneDX SBOM is generated for each release by `scripts/sbom.sh` (requires
`cargo cyclonedx`; writes `sbom/brain-server-<version>.cdx.json`). The SBOM
lists the full dependency tree from `Cargo.lock` so consumers can scan for
known vulnerabilities (EU CRA Art 13/14; OWASP A03:2025 supply-chain coverage).

Since **v1.17.5** the tag release workflow runs the same script and stages
the SBOM into `dist/` alongside the binaries — every GitHub release ships its
own `brain-server-<version>.cdx.json`, so consumers never need to build to
obtain it. Local operator path unchanged: `scripts/sbom.sh`.

---

## Threat Model (STRIDE)

Full threat model in [`THREAT_MODEL.md`](./THREAT_MODEL.md). Summary here.

| Threat class (STRIDE) | Brain-server exposure | Mitigation | Status |
|---|---|---|---|
| **S**poofing | Agent → brain-server, brain-server → peer (A2A) | Bearer token (v1.1) / JWT/JWS (v1.2); mTLS (v3.7) | ✅ / 🚧 |
| **T**ampering | Audit log, content at rest, JWT payload | Hash-chained audit (v1.1 M2.3); parameterized SQL (all versions); JWS signature (v1.2) | ✅ |
| **R**epudiation | "Who did this write?" | Append-only audit with `(actor, ts, hash, prev_hash)` (v1.1 M2) | ✅ |
| **I**nformation disclosure | Cross-tenant leak, PII egress | Per-tenant file isolation (v1.0); AuthZ layer (v1.2); record-level `access_scope` + owner (v1.14, JWT-mode deny-by-default filter); PII output redaction + opt-in write-time placeholder mode (`pii_map`, v1.14); SQLCipher + per-field encryption (v3.7) | ✅ / 🚧 |
| **D**enial of service | Burst, large body, vector query | Body limit + per-IP limiter (v0.9.4); per-tenant + tiered limiter (v2.1) | ✅ / 🚧 |
| **E**levation of privilege | Token scope escalation | Constant-time token compare (v1.1); AuthZ trait with deny-by-default (v1.2) | ✅ |

---

## OWASP Top 10:**2025** Coverage

This is the canonical reference. Each item maps to a control in brain-server.

### A01:2025 — Broken Access Control ✅

**Requirement:** Deny by default. Enforce record ownership. Per-request AuthZ
at the data-access layer. Short-lived tokens + refresh.

| Control | Where | Status |
|---|---|---|
| Loopback-only default (no token → loopback only) | `auth_middleware` | ✅ |
| AuthZ enforcement at the data layer | `AuthzPolicy::authorize` trait | ✅ v1.2 |
| JWT short-lived (≤15 min exp) + refresh | `auth/refresh` + token verifier | ✅ v1.2 |
| Tenant isolation via file-per-domain | `DomainRegistry` | ✅ v1.0 |
| AuthZ failures logged with `(principal, action, target)` | `audit_events` | ✅ v1.2 M6 |
| Cross-tenant returns 403, never 404 (don't leak existence) | AuthZ layer | ✅ v1.2 |

**Citation:** OWASP A01:2025 — "Access control is only effective when
implemented in trusted server-side code... deny by default... enforce record
ownership... log access control failures."

### A02:2025 — Security Misconfiguration ✅

| Control | Where | Status |
|---|---|---|
| Auth off by default (loopback-safe) | `config::auth_token` | ✅ |
| CORS allowlist, no wildcard | `CorsLayer` exact origins | ✅ |
| Bind address loopback by default | `BIND_HOST=127.0.0.1` | ✅ |
| Public base URL explicit (`BRAIN_PUBLIC_BASE_URL`) | `/.well-known/` | ✅ v1.2 |
| Reproducible build (`Cargo.lock` checked in) | repo | ✅ |
| Hardened systemd unit on openclaw | `ProtectSystem=strict`, etc. | ✅ |

### A03:2025 — Software Supply Chain Failures ✅

| Control | Where | Status |
|---|---|---|
| `cargo audit` in CI (`.github/workflows/ci.yml`) | CI | ✅ |
| Pinned direct deps in `Cargo.toml` | repo | ✅ |
| Optional features for high-surface deps (reqwest, jsonwebtoken) | Cargo features | ✅ |
| Reproducible release build | `Cargo.lock`, `opt-level="z"`, `lto="fat"` | ✅ |
| Known-issue ledger: RUSTSEC-2023-0071 (rsa "Marvin") — no fix exists in any release (verified 2026-08-04); accepted + documented in `.cargo/audit.toml`; local-daemon timing model, keys 0600, EdDSA avoids RSA entirely | `.cargo/audit.toml` + `THREAT_MODEL.md` | ✅ accepted |

### A04:2025 — Cryptographic Failures ✅

| Control | Where | Status |
|---|---|---|
| Tokens compared constant-time | `auth_middleware` (`subtle::ConstantTimeEq`) | ✅ v1.1.2 |
| JWT RS256/ES256/EdDSA only (never HS256 in distributed) | `verify_access_token` | ✅ v1.2 |
| `alg` whitelist + reject `none` + reject algorithm confusion | JWT verifier | ✅ v1.2 |
| AES-256-GCM for backups (v0.9.7) | `backup.rs` | ✅ |
| HMAC-SHA256 for webhook verification (v0.9.7) | `webhook.rs` | ✅ |
| Webhook replay window (opt-in, v1.20.4 G6): `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED=1` demands the Standard Webhooks header set (`webhook-id`/`webhook-timestamp`/`webhook-signature`) and verifies `v1,<base64>` HMAC-SHA256 over `{id}.{timestamp}.{raw body}` in constant time — the timestamp rides inside the HMAC, so a replay cannot re-stamp it. **GitHub's replay protection is `x-github-delivery` idempotency, not a timestamp** (its sender is a trusted third party); first-party senders opt into the hard window via the spec headers + flag. Default unchanged (legacy `sha256=` path). | `verify_standard_signature` + `receive_standard` | ✅ |
| SQLCipher at rest (AES-256 per-page) | KMS trait | 🚧 v3.7 |
| No hardcoded secrets (env / file / KMS) | all of `config.rs` | ✅ |
| Dependency timing sidechannels (RSA "Marvin", RUSTSEC-2023-0071) | documented accept: `.cargo/audit.toml`; EdDSA keys avoid RSA (supported since v1.2) | ✅ accepted |

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
| Prompt-injection heuristic at every query/ingest boundary | `contains_suspicious_pattern` (layer 1) | ✅ (ceiling documented) |
| Optional local ONNX classifier for novel/obfuscated injections (v1.20.3 G5, off by default) | `screen::screen` layer 2 (`injection-classifier` feature) | ✅ (feature-gated; blocklist + `flagged`/`untrusted` stay the always-on defense) |
| auto-capture (G2): plugin routes captures through the human review queue by default; the `/ingest` write core screens + quarantines/rejects injectable input (v1.20.1) | `plugin captureMode` + `ingest_one` | ✅ |
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
| Deny-by-default AuthZ | AuthZ trait | ✅ v1.2 |
| Threat-model review checkpoint per major release | release process | 🚧 |

### A07:2025 — Authentication Failures ✅

| Control | Where | Status |
|---|---|---|
| Constant-time token compare | `auth_middleware` (`subtle::ConstantTimeEq`) | ✅ v1.1.2 |
| `401` on missing/invalid/malformed | `auth_middleware` | ✅ |
| Audit row on every auth failure | `audit_events` | ✅ v1.1 M2.1 |
| Token rotation (file-watch) | `AUTH_TOKEN_FILE` hot reload | ✅ v1.1 M1.4 |
| JWT `(jti, iss)` revocation | `revoked_tokens` table | ✅ v1.2 |
| Refresh token rotation + reuse detection | refresh endpoint | ✅ v1.2 |
| Account lockout / rate limit on auth | per-tenant limiter | 🚧 v2.1 |

**Citation:** OWASP JWT Cheat Sheet — "Use the `(jti, iss)` pair because jti
uniqueness is only guaranteed per issuer — a malicious or rogue issuer could
mint a JWT with the same jti as a legitimate one, causing a collision."

### A08:2025 — Software or Data Integrity Failures ✅

| Control | Where | Status |
|---|---|---|
| Signed JWTs (JWS) with key rotation | JWKS endpoint | ✅ v1.2 |
| Tamper-evident audit (hash chain) | `audit_events.prev_hash` | ✅ v1.1 M2.3 |
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
| All authN/authZ events audited | auth middleware → audit row | ✅ v1.2 M6 |
| Quota warnings audited | `quota.warning` / `quota.exceeded` | 🚧 v2.1 M5 |
| `/health` exposes ops status | `/health` capacity + integrity | ✅ |
| `/metrics` Prometheus exporter (hand-rolled text format) | always-on, auth-gated | ✅ v1.1 |
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
  │  1.  RequestBodyLimitLayer      1 MB                                   │
│  2.  TimeoutLayer               30 s, returns 408                      │
│  3.  CatchPanicLayer            prevents process crash                 │
│  4.  SetSensitiveHeadersLayer   redacts Authorization/Cookie           │
│  5.  CompressionLayer           gzip/br/zstd                           │
│  6.  SetRequestIdLayer          generates x-request-id                 │
│  7.  PropagateRequestIdLayer    passes through x-request-id            │
│  8.  TraceLayer                 structured HTTP logging                │
│  9.  CorsLayer                  exact-origin allowlist                 │
│  10. security_headers_middleware HSTS, CSP, X-Frame, Permissions-Policy │
  │  11. rate_limit_middleware     per-IP sliding window (10k/min)        │
  │  12. auth_middleware           JWT/JWS verify + (jti, iss) revocation  │
  │  13. authz at handler entry    every non-public handler calls authorize│
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

brain-server has two auth modes, resolved at startup from `BRAIN_JWT_ISSUER`
(and the presence of a key dir). Both modes coexist; JWT is opt-in and the
opaque mode is the back-compat default.

### v1.1 — opaque bearer token (default; back-compat)

- `AUTH_TOKEN_FILE` (preferred; 0600 file) or `AUTH_TOKEN` env var.
- Constant-time compare via `subtle::ConstantTimeEq` (v1.1.2).
- Public routes (`/health`, `/health/db`, `/ready`, `/version`, `/.well-known/*`)
  bypass.
- File-watch for hot rotation (v1.1 M1.4); fail-safe on delete/empty.
- Single-user / single-tenant / loopback deployments.

### v1.2 — JWT/JWS + AuthZ layer (opt-in; multi-tenant)

Enabled by setting `BRAIN_JWT_ISSUER` + loading keys via `brain key generate`.
The two-layer middleware runs JWT verification outermost; the v1.1 opaque
layer short-circuits when the JWT layer has already injected a `Principal`.

- **Algorithm whitelist** (checked **before** key lookup — OWASP algorithm-
  confusion defense): RS256, RS384, RS512, ES256, ES384, EdDSA.
  **Forbidden**: `none`, all HMAC variants (HS*), all PS* variants, ES512
  (jsonwebtoken v10 exposes no ES512 variant).
- **Verified claims**: `iss`, `aud`, `exp`, `nbf`, `sub`, `jti`. No "soft"
  validation; missing claim = reject. 30s leeway for clock skew.
- **Revocation**: `(jti, iss)` table per OWASP JWT Cheat Sheet, 60s negative-
  lookup cache (eventual consistency, bounded TTL).
- **Refresh tokens**: separate JWS, ≤24h, rotated on use. Reuse detection
  burns the whole chain (OWASP pattern).
- **AuthZ trait** with deny-by-default; `InMemoryPolicy` default (no external
  deps); OPA/Cedar impls are the swappable v2.1+ upgrade path. `Action` enum
  (Read/Write/Admin/Traverse) + `Scope` (`<action>:<team>/<domain>` with
  wildcards). Escalation: write implies read down, admin implies both.
- **Per-route enforcement matrix** documented in
  `IMPLEMENTATION_PLAN_v1.2.0_AuthN.md` §M3.3.
- **OIDC discovery** at `GET /.well-known/openid-configuration` (RFC 8414) +
  **JWKS** at `GET /.well-known/jwks.json` (RFC 7517). Issuer pinned to
  `BRAIN_PUBLIC_BASE_URL` — never inferred from the `Host` header (OWASP
  A02:2025 Security Misconfiguration).

### JWT Cheat Sheet compliance checklist (v1.2)

Source: `IMPLEMENTATION_PLAN_v1.2.0_AuthN.md` §M1 (Context7-verified at plan
write time; OWASP cheat-sheet URLs were 404ing on the v1.2 ship date, so the
plan's encoded checklist was the source of truth). Every item is pinned by a
unit test in `src/auth/jwt.rs` (14 tests covering the full failure matrix).

- [x] **`alg` whitelist** — RS256/384/512, ES256/384, EdDSA only.
- [x] **Reject `none`** — unsigned tokens rejected before any signature work.
- [x] **Reject HS\*** — algorithm-confusion CVE class (attacker HMACs the
      public key); rejected even if a matching HMAC key exists.
- [x] **Reject PS\*** — cryptographically fine but excluded from the
      whitelist to minimize the accepted algorithm surface.
- [x] **Reject tampered payloads** — signature mismatch → 401.
- [x] **Validate `exp`** — expired tokens rejected (30s leeway).
- [x] **Validate `nbf`** — not-yet-valid tokens rejected (30s leeway).
- [x] **Validate `iss`** — must match `BRAIN_JWT_ISSUER`.
- [x] **Validate `aud`** — must match `BRAIN_JWT_AUDIENCE`.
- [x] **Require `jti`** — missing `jti` → 401 (revocation needs it).
- [x] **Require `kid`** — missing `kid` → 401 (key lookup needs it).
- [x] **Reject unknown `kid`** — key not in JWKS → 401.
- [x] **`(jti, iss)` revocation** — denylist lookup before accept (per OWASP:
      `jti` is unique per `iss` only).
- [x] **Token-type discrimination** — refresh tokens (`typ: refresh`)
      rejected on data routes; access tokens rejected on `/auth/refresh`.
- [x] **Refresh-chain reuse detection** — reuse burns the family.

---

## Cryptography

| Purpose | Algorithm | Status |
|---|---|---|
| Content deduplication | XXH3-64 (non-cryptographic, fast) | ✅ |
| Source revision hash | XXH3-64 | ✅ |
| Document ID | XXH3-64 of title | ✅ |
| Audit row hash chain | SHA-256 | ✅ v1.1 M2.3 |
| Auth token compare | Constant-time (`subtle::ConstantTimeEq`) | ✅ v1.1.2 |
| JWT signature | RS256 / ES256 / EdDSA | ✅ v1.2 |
| JWT revocation key | `(jti, iss)` | ✅ v1.2 |
| Webhook verification | HMAC-SHA256 | ✅ |
| Backup encryption | AES-256-GCM | ✅ |
| Database at rest | SQLCipher AES-256 per-page | 🚧 v3.7 |
| Per-field PII encryption | AES-256-GCM via HKDF-derived key | 🚧 v3.7 |
| TLS termination | TLS 1.3 (reverse proxy) | ✅ (operator) |
| mTLS service-to-service | TLS 1.3 + SPIFFE/SPIRE-compatible identity | 🚧 v3.7 |

**No encryption at rest today** — SQLite files are plaintext. For encryption
before v3.7: filesystem encryption (LUKS, FileVault, BitLocker) or `sqlcipher`
via proxy.

## UMP identity + capability tokens (v1.17.3)

The Universal Memory Protocol (UMP 1.0) identity model, implemented at
conformance level L3, binds every record to an Ed25519 operator key and gates
every write, read, and export behind capability tokens. The sections below
cover key storage, token minting, and the boundaries of that model.

- **Key storage** — the UMP operator key (`BRAIN_UMP_KEY_DIR`, default
  `~/.config/brain-server/ump/operator.key`) follows the v1.2 JWT key
  posture exactly: directory 0700, key file 0600, Ed25519 seed generated by
  `brain ump keygen` (refuses to overwrite; rotation = delete then regen).
  The key signs §2.8 record integrity + §5.2 capability tokens; the
  public `did:key` is what clients verify against.
- **Capability tokens (§5.2)** — `alg.payload.sig` compact EdDSA tokens
  `{iss, verbs, scope, exp}`. Accepted only on `/ump/*` + `/export`;
  signature, key, and `exp` are checked at the auth middleware, verbs ×
  scope at handler entry (`cap_gate` after `authorize`). No admin verb
  exists — `audit`/`audit/verify` always deny token bearers. Off-surface
  paths (e.g. `/search`, `/ingest`) reject capability tokens with 401.
- **Injection-resistant rehydration (§5.3)** — server-side obligations
  are the recall pipeline order already: integrity verify-before-emit,
  owner-scope filter before ranking. Client obligations (documented, not
  server-enforced): treat record bodies as untrusted data — structural
  framing only, never execute or render the body as a command channel
  (the same posture as the v0.9.2 prompt-injection guard: memory content
  is data, not instructions).
- **Key rotation** — delete `operator.key` + regenerate; tokens signed by
  the old key stop verifying immediately (no distributed cache — same
  single-process posture as the v1.2 revocation list).

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

- 10,000 req/min per IP (60s sliding window). Single-process only. Sufficient for
  loopback + LAN.

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
   private keys mode 0600; rotated via `brain key generate/list/prune` CLI.
3. **External KMS (v3.7, BYOK pattern)**:
   - `BRAIN_KMS_PROVIDER=file|vault|aws`
   - `FileKeyProvider` (default; mode 0600 keys)
   - `VaultKeyProvider` (HashiCorp Vault; transit secret engine)
   - `AwsKmsKeyProvider` (AWS KMS; envelope encryption)

### Bring Your Own Key (BYOK)

Per `IMPLEMENTATION_PLAN_v1.2.0_AuthN.md` §5.3 (OWASP Secrets Management).

| Tier | Mechanism | Status |
|---|---|---|
| **File-based (default)** | Operator places PEM files in `BRAIN_JWT_KEY_DIR` (default `~/.config/brain-server/keys/`, mode 0700; private keys 0600). `brain key generate` creates a fresh RSA keypair with the right modes. `brain key prune` drops retired keys after the rotation window. | ✅ v1.2 |
| **Customer-managed (multi-tenant)** | Per-tenant key directories scoped by tenant id; the `KeyStore` resolves the verifying set from the request's verified `tenant` claim. The compliance wedge for regulated multi-tenant deployments. | 🚧 v2.0+ |
| **External KMS** | `BRAIN_KMS_PROVIDER` + `BRAIN_KMS_KEY_ID` env vars are **reserved** in the config surface (parsed but not yet wired to a provider). Future impls: HashiCorp Vault (transit secret engine), AWS KMS (envelope encryption). No KMS code ships in v1.2 — file-based is sufficient for the single-host trusted-disk threat model. | 🚧 v3.7 |

**Rotation procedure (v1.2, file-based):**

```sh
brain key generate           # mints a new keypair; old key stays in JWKS
# …wait ≥ max token lifetime (24h refresh)…
brain key prune              # drops retired keys from JWKS
scripts/install-service.sh   # restart to reload the key set
```

Two keys stay live during rotation; the old key drops from JWKS only after
every cached client token has expired (the 1h JWKS cache header + 24h refresh
lifetime bound the overlap window).

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
- [ ] **For JWT mode**: set `BRAIN_JWT_ISSUER` + `BRAIN_PUBLIC_BASE_URL`, run
      `brain key generate`, restart via `install-service.sh`.
- [ ] **For JWT mode**: front `/.well-known/jwks.json` with a reverse proxy +
      IP allowlist if the server is reachable beyond loopback (public keys,
      but not unlimited scrape traffic).
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
|---|---|---|---|
| 1.20.3 | 2026-08-11 | "Classify" (G5): two-layer injection screen — layer 1 deterministic blocklist (always on) + optional feature-gated local ONNX classifier (layer 2, off by default; the blocklist + `flagged`/`untrusted` stay the always-on defense). Canonical invisible-char predicate shared by screen/classifier/client-render boundary (client strips invisible smuggling chars from displayed hits; raw bytes never rewritten). `screen_verdict` review badge recomputed at read. Policy + thresholds read per call |
| 1.16.7 | 2026-08-08 | Client "Integrated": deep links + PWA (offline shell caches app shell only — never the API, no content caching) + paginated audit + command palette + recall debounce + hardening (drawer focus trap, aria-live, `dir="auto"`). Client-only; server + API contract unchanged |
| 1.16.6 | 2026-08-08 | Client "Mobile": secure token storage via OS keyring (`keyring`, non-web; web stays in-memory — browser localStorage is not a secure credential store per MASVS-STORAGE) + responsive tab-bar UX; Dioxus pinned to 0.7.10 (wasm-hotpatch TOCTOU/UB + panic-resilience fixes compiled in) |
| 1.16.5 | 2026-08-08 | Client "Secure": JWT refresh lifecycle — silent refresh-on-401 (single-flight mutex), pre-emptive expiry refresh, principal identity pillar, revocation-aware error mapping |
| 1.16.4 | 2026-08-08 | Client "Styled": shadcn/ui design-system restyle; deploy-web.sh stale-CSS fix |
| 1.16.3 | 2026-08-08 | Client "Serve": web bundle served under `/app`; path-aware CSP (`CLIENT_CSP` gains `'unsafe-eval'` for wasm-bindgen glue — API CSP stays strict); same-origin connect default |
| 1.16.2 | 2026-08-08 | Client "Harden + Accessible": server serves the client + path-aware CSP; WCAG 2.2 AA client pass (focus-to-`<h1>`, document titles, no-`<div onclick>` semantic gate, AA-contrast token); ErrorBoundary + error mapping; cancel-safe batch summary; grep guards (`xss_escape_hatch_is_unused`, `credentials_stay_in_memory`) |
| 1.16.0 | 2026-08-08 | Client "Client": Dioxus control surface (web + desktop + iOS + Android, one Rust codebase) — connection state machine, honest-batch review, recall decision-path viewer, DSAR certificate card, auth-failure feed, audit filters, semantic-token layer |
| 1.15.0 | 2026-08-08 | "Observe": read-event audit (recall/search/get emit rows into the SHA-256 chain; opt-in), recall trace endpoint, DSAR workflow (locate→export→purge→chain-verifiable deletion certificate, `GET /tombstones`), Art 19 HMAC-SHA256 webhook (opt-in; reqwest now required for outbound HTTP); ISO 42001/NIST AI RMF/SOC 2 map in `COMPLIANCE.md` |
| 1.14.0 | 2026-08-07 | "Gate": human-in-the-loop write-back (`POST /ingest/proposal` → `POST /proposals/{id}/approve`, one tx, optional `?supersedes`); per-chunk `expires_at` decay (strict `<`, nothing decays autonomously); record-level `access_scope` + `owner` (JWT-mode deny-by-default filter); PII output redaction (`[redacted:…]`) + opt-in write-time placeholder mode (`BRAIN_REDACT_PII=1`, `pii_map` vault); GDPR `GET /export` + `POST /purge` (hard audited delete across tables, tombstone + audit) |
| 1.13.2 | 2026-08-06 | "Harden": `PRAGMA busy_timeout=5000` on every SQLite pool init (write contention now queues up to 5s instead of failing with `SQLITE_BUSY`); `/graph/traverse` `name`/`entity` aliases; `/recall` `explain` alias |
| 1.13.1 | 2026-08-06 | "Recall" fix: automatic retrieval routing in shim mode (matched domain + `global` rescue leg); kill switch `BRAIN_RECALL_ROUTING_ENABLED` |
| 1.13.0 | 2026-08-06 | "Route": domain relabeling migration (`global` + per-domain labels); retrieval routing by centroid |
| 1.12.2 | 2026-08-04 | "Harden" (audit-fix): `/auth/refresh` check-then-act race closed (`record_and_rotate` under `BEGIN IMMEDIATE` — concurrent presentations serialize, exactly one winner, family burned once, mutation-proven); DB stack bumped (rusqlite 0.40.1 / sqlite-vec 0.1.9 / r2d2_sqlite 0.35.0 → bundled SQLite 3.53.2, fts3_tokenizer + CVE-2022-35737-class fixes); CI `cargo audit` green via `.cargo/audit.toml` (RUSTSEC-2023-0071 "Marvin" accepted + documented — no fixed release exists in any rsa/jsonwebtoken release; EdDSA keys avoid RSA entirely) |
| 1.12.1 | 2026-08-04 | AuthZ wiring completion ("Harden"): closes the S1 audit finding — every non-public route now calls `authorize()` at handler entry with the v1.2 §3.3 matrix action (20 previously-ungated handlers wired: search/stats/embeddings/get*/multi-get/graph*/quarantine-list/audit/audit-verify/metrics/recall/verify/propose/connectors/revoke/domains/suggest-metrics/procedure-steps; `reindex` + `DELETE /memory/{id}` upgraded Write→Admin; `/audit` gains Admin gate + cross-tenant 403 via `audit_scope`; `/auth/revoke` finally enforces its documented admin requirement). Back-compat preserved: `None` principal (opaque mode) stays superuser; webhooks stay HMAC-internal. New wiring-guard test pins every non-public route to an `authorize()` call (mutation-proven) + router-level middleware tests. 465 tests green |
| 1.12.0 | 2026-08-03 | Noise-aware graph retrieval + complexity-gated activation ("Discern"): edge-type weights (`tagged_with`/`alias_of` → 0.1 vs semantic types → 1.0) + GAAMA-style hub dampening (`w_ij·min(1, θ/deg(i))`, θ=50) tame the taxonomy-heavy KG (94% of live edges are tag noise; degree-73/101/150 mega-hubs); graph leg auto-engages as a bounded rescue pass on the would-be-abstention path (`ClarifyQuery` only, `BRAIN_GRAPH_RESCUE_ENABLED` kill switch, arXiv:2602.03578). Retrieval-only, deterministic, no LLM, no new schema, no re-ingest; θ and weights are corpus-calibrated constants, not learned |
| 1.11.0 | 2026-08-03 | HippoRAG-2-style graph retrieval ("Associate"): deterministic Personalized PageRank over the existing `entities`/`relationships` KG as an opt-in third RRF leg (`?graph=true`). No new schema, no embeddings, no LLM; bounded power iteration (`MAX_PPR_ITER`/`MAX_VISITED`) prevents a dense/taxonomy-heavy KG from blowing the compute budget; exact-name entity seeding (no fuzzy matching on untrusted input); retrieval-only — the leg reads the KG and never writes |
| 1.4.0 | 2026-07-30 | Surpass-human retrieval ("Calibrate"): bi-temporal edges (`relationships.valid_at`/`invalid_at`, Graphiti valid-time model, Context7-verified); deterministic temporal-marker extraction (no LLM); submodular evidence packing (budgeted monotone maximization, MMR-style dedup, arXiv:2607.00725); TRACE typed-edge prefixes + bounded validity-aware traversal; `?at=` bi-temporal filter on `/recall` + `/graph/traverse`; `knowledge.node_kind`/`parent_id` schema reservation; pure eval metrics (P@k/R@k/MRR/NDCG) + `bench eval` regression harness; `multivec` feature reserved (M4 deferred per lazy-dev escape hatch) |
| 1.3.0 | 2026-07-29 | Memory-safety hardening ("Bedrock"): zero `unwrap`/`expect`/`panic!` in production paths; 10 duplicate `unsafe` transmute blocks collapsed into one documented `register_sqlite_vec()`; every remaining `unsafe` carries a `// SAFETY:` comment; cargo-fuzz targets; proptests for chunker/validator/capacity invariants; `/health` exposes a `hardening` object (`unsafe_blocks`, `panics_caught`, `memory_leaks_detected`); `BRAIN_WORKER_THREADS` runtime tuning |
| 1.2.1 | 2026-07-29 | Dead-code elimination + panic fixes found during the v1.3.0 audit (unused `AuthzPolicy` trait, `TokenType::as_str`, `DEFAULT_ALG`, etc.); `authorize()` now uses `principal.tenant` as team context |
| 1.2 | 2026-07-29 | JWT/JWS verification (RS256/ES256/EdDSA, alg whitelist, no HS256/`none`); `(jti, iss)` revocation; refresh-chain reuse detection; AuthZ trait (deny-by-default); OIDC discovery + JWKS; key management CLI |
| 1.1.2 | 2026-07-29 | Constant-time auth hardening: bearer-token compare swapped from a hand-rolled fold (LLVM-short-circuitable) to `subtle::ConstantTimeEq::ct_eq` (asm/`black_box`-backed); query parameterization audit (no injection surface) |
| 1.1.1 | 2026-07-29 | Audit hash-chain false-negative fix: `verify_chain` no longer fails on the NULL→`Some` boundary created by the v1.0→v1.1 additive `prev_hash` migration; `record_tenant` wrapped in `SAVEPOINT` for caller-transaction safety; `/metrics` `brain_audit_chain_ok` TTL-cached |
| 1.1 | 2026-07-28 | Per-tenant audit + SHA-256 hash chain; fail-safe file-watch token rotation; rolling backups + `/health` integrity; graceful-shutdown drain cap + WAL checkpoint; RSS watchdog; Prometheus `/metrics` exporter |
| 2.0 (planned) | 2026-Q4 | Multi-team tenancy consuming v1.2 AuthZ; team namespace in paths |
| 2.1 (planned) | 2026-Q4 | Per-tenant + tiered rate limiting; Redis impl; cost tracking; quota alerts |
| 3.7 (planned) | 2027-Q1 | A2A federation over mTLS; SQLCipher + KMS abstraction; per-field PII encryption; differential privacy |
| 1.0.1 | 2026-07-26 | Structured-ingest auto-create bug fix; systemd deploy on openclaw |
| 1.0.0 | 2026-07-26 | Multi-domain + cross-domain RRF + domain lifecycle + boot-time legacy cutover |
| 0.9.9 | 2026-07-25 | Capacity envelopes; migration rehearsal tool; storage layout |
| 0.9.4 | 2026-07-11 | Full middleware stack hardening (rate limit, security headers, request ID, timeout, panic catch, sensitive header redaction, compression, tracing) |
| 0.9.2 | 2026-07-11 | Vault ingest provenance, wikilink KG edges, prompt injection guard |
| 0.9.0 | 2026-07-08 | sqlite-vec migration, parameterized queries, CORS lockdown, auth scaffold |
