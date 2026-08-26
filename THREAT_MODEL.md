# Threat Model — brain-server

**Methodology:** STRIDE (Microsoft). **Reference standards:** OWASP Top 10:2025
+ Cheat Sheet Series (Context7-verified 2026-07-26), NIST SP 800-63B (digital
identity), NIST SP 800-207 (zero-trust architecture).

This document is the engineering-side threat model. For per-release progress
against the controls below, see [`SECURITY.md`](./SECURITY.md).

**Agentic-AI coverage:** the LLM/agent-specific threat classes (prompt
injection, memory poisoning, tool misuse, agentic supply chain, lies-in-the-
loop) are inventoried and mapped to controls in
[`docs/OWASP_AGENTIC_2026.md`](./docs/OWASP_AGENTIC_2026.md) (OWASP Top 10 for
Agentic Applications 2026) — read it as the companion layer to this STRIDE
model, not a substitute.

---

## 1. System boundaries

```
                         ┌──────────────────────────────────────────┐
                         │  Internet / untrusted                    │
                         └──────────────────────────────────────────┘
                                          │
                                          ▼
                         ┌──────────────────────────────────────────┐
                         │  Reverse Proxy (operator-managed)        │
                         │  ─ TLS 1.3 termination                   │
                         │  ─ Per-IP rate limit                     │
                         │  ─ WAF / IP allowlist                    │
                         │  ─ HSTS                                 │
                         └──────────────────────────────────────────┘
                                          │ (loopback HTTP)
                                          ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  brain-server (Rust binary, single process)                              │
│  ─ AuthN middleware: JWT/JWS verify + (jti, iss) revocation (v1.2)       │
│  ─ AuthZ middleware: AuthzPolicy::authorize (v1.2)                       │
│  ─ Rate limiter: per-tenant + tiered (v2.1)                              │
│  ─ Audit log: append-only, hash-chained, per-tenant (v1.1)              │
│  ─ SQLite (WAL) or per-domain SQLite (multi-db mode)                    │
│  ─ Optional: A2A federation via mTLS (v3.7)                             │
└──────────────────────────────────────────────────────────────────────────┘
                       │                              │
                       ▼                              ▼
       ┌───────────────────────────┐    ┌───────────────────────────┐
       │  Filesystem (local)       │    │  Peer brain-server (v3.7) │
       │  ─ SQLite DBs             │    │  ─ A2A over mTLS           │
       │  ─ Auth token file (0600) │    │  ─ JWKS verified           │
       │  ─ JWT keys (0700 dir)    │    └───────────────────────────┘
       └───────────────────────────┘
```

**Trust boundaries crossed:**
1. **Internet → reverse proxy** — TLS termination, IP allowlist, per-IP rate limit.
2. **Reverse proxy → brain-server** — loopback only; AuthN/AuthZ at the app.
3. **brain-server → filesystem** — same host; assumes disk not tampered (LUKS
   recommended for full-disk encryption; SQLCipher for at-rest app encryption
   lands in v3.7).
4. **brain-server → peer brain-server** (A2A, v3.7) — untrusted; mTLS + JWS
   verified, scoped capability, data residency allowlist.

---

## 2. STRIDE per asset

### Asset 1: Knowledge graph data (per-tenant)

| Threat | Attack | Mitigation | Status |
|---|---|---|---|
| **S**poofing | Attacker forges tenant identity | JWT/JWS verify + tenant from signed claim (v1.2) | ✅ |
| **T**ampering | Direct DB edit on disk | Filesystem perms; SQLCipher + KMS (v3.7) | 🚧 |
| **T**ampering | Modify a proposal between display and approval | Approve carries the SHA-256 `content_digest` of the read-canonical form; any drift → `409` inside the tx (v1.27.12) | ✅ |
| **R**epudiation | "I didn't write that" | Audit hash chain (v1.1 M2.3) | ✅ |
| **I**nformation disclosure | Tenant A reads tenant B | Per-tenant files + AuthZ at data layer (v1.0+v1.2) | ✅ |
| **D**enial of service | Burst fills the DB | Capacity envelope 507 (v0.9.9); per-tenant limiter (v2.1) | ✅/🚧 |
| **E**levation of privilege | L1 frontline reads L2 escalation | AuthZ trait with deny-default + escalation rules (v1.2) | ✅ |

### Asset 2: Authentication tokens

| Threat | Attack | Mitigation | Status |
|---|---|---|---|
| **S**poofing | Stolen token reuse | Short-lived JWT (≤15 min) + refresh rotation + revocation (v1.2) | ✅ |
| **T**ampering | Readable token/key files (group/world) | Startup fails closed on wide modes (`mode & 0o077`); `brain token rotate` writes 0600 temp + fsync + atomic rename (v1.27.12) | ✅ |
| **T**ampering | Modify JWT payload | JWS signature (RS256/ES256/EdDSA only) (v1.2) | ✅ |
| **R**epudiation | "I didn't issue that token" | `iss` claim verified; key rotation log (v1.2) | ✅ |
| **I**nformation disclosure | Token in URL/logs | `Authorization: Bearer` header only; `SensitiveHeadersLayer` redacts logs (v0.9.4) | ✅ |
| **D**enial of service | Token-storm | Per-tenant rate limit (v2.1) | 🚧 |
| **E**levation of privilege | Token with broadened scope | Scope enforced per-request via AuthZ (v1.2); `alg:none` rejected | ✅ |

### Asset 3: Audit log

| Threat | Attack | Mitigation | Status |
|---|---|---|---|
| **S**poofing | Forge audit entries | Append-only; writer is the authenticated process only | ✅ |
| **T**ampering | Edit existing rows | Keyed hash chain — HMAC-SHA256 over the full row under a per-DB epoch + head pin `(id, hash, epoch)`; break is detectable on read (v1.1 M2.3; keyed epoch shipped v1.27.31) | ✅ |
| **R**epudiation | "The log is wrong" | Keyed chain proves integrity (`/audit/verify`); signed release tags prove code provenance (keyed epoch shipped v1.27.31) | ✅ |
| **I**nformation disclosure | Tenant A reads tenant B's audit | Data-layer filter `WHERE tenant_id = ?` + AuthZ on `/audit` (v1.1 M2.2) | 🚧 |
| **D**enial of service | Fill audit table | Bounded by writes; rotation policy documented | 🚧 |
| **E**levation of privilege | Non-admin queries `/audit` | `admin:<tenant>/*` scope required (v1.2) | ✅ |

### Asset 4: Binary / supply chain

| Threat | Attack | Mitigation | Status |
|---|---|---|---|
| **S**poofing | Malicious binary in place of legit | Build from source; signed git tags (`git tag -s`) | 🚧 |
| **T**ampering | Backdoored transitive dep | `cargo audit` in CI; pinned direct deps; minimal feature flags | ✅ |
| **R**epudiation | "We didn't ship that" | Reproducible build via `Cargo.lock`; tag history | ✅ |
| **I**nformation disclosure | Source leaks secrets | Audited; no secrets in repo; `.env*` in `.gitignore` | ✅ |
| **D**enial of service | CVE in dep causes crash | `CatchPanicLayer`; advisory monitoring; rapid patch process | ✅ |
| **E**levation of privilege | Dep with CVE pre-auth | Pin versions; `cargo audit --deny warnings` in CI | ✅ |
| **T**ampering | Timing sidechannel on RSA private-key ops (`rsa` crate, RUSTSEC-2023-0071 "Marvin") | No fixed release exists anywhere (verified 2026-08-04: `rsa` 0.10.0-rc.18 and `jsonwebtoken` 11 both still affected). Accepted with documentation in `.cargo/audit.toml`: local-daemon timing model (attacker with local timing access already owns the machine), keys at 0600, EdDSA (Ed25519) keys avoid RSA entirely and are supported since v1.2 | audit.toml ignore + docs |

### Asset 5: Network transport

| Threat | Attack | Mitigation | Status |
|---|---|---|---|
| **S**poofing | MITM impersonates server | TLS 1.3 at proxy; mTLS for A2A (v3.7); cert pinning for native clients | ✅/🚧 |
| **T**ampering | Modify traffic in transit | TLS 1.3 (proxy); JWS non-repudiation for A2A payloads (v3.7) | ✅/🚧 |
| **R**epudiation | "I didn't send that request" | `x-request-id` for tracing; JWS for A2A non-repudiation | ✅/🚧 |
| **I**nformation disclosure | Eavesdropper reads traffic | TLS 1.3 terminates at the operator's reverse proxy (the server itself is loopback HTTP); HSTS is a proxy-layer header | ✅ |
| **D**enial of service | SYN flood / slowloris | Proxy handles; per-IP rate limit; per-tenant rate limit (v2.1) | ✅/🚧 |
| **E**levation of privilege | — | (no transport-level privilege concept) | n/a |

---

## 3. v1.2 "AuthN" — AuthN/AuthZ threat mitigations

v1.2.0 introduces JWT/JWS verification + a real AuthZ layer. The five
threat classes below are the ones v1.2 directly mitigates. Each maps to a
control verified by a unit/integration test (308 green).

| Threat | Attack | v1.2 mitigation | Test |
|---|---|---|---|
| **Token replay** | Stolen access token reused after legitimate logout | Access tokens short-lived (≤15 min `exp`) + `(jti, iss)` denylist lookup on every authenticated request; 60s negative cache (bounded eventual consistency — see residual risk §6) | `missing_jti_rejected`, revocation tests |
| **Algorithm confusion** | Attacker sends `alg:none`, or HS256 with the server's public key as the HMAC secret, hoping the verifier falls back to HMAC verification with the public key as the secret | `ALLOWED_ALGS` whitelist (RS256/384/512, ES256/384, EdDSA) checked **before** key lookup; `none`, all HS\*, all PS\* rejected unconditionally | `none_algorithm_rejected`, `hs256_rejected_even_with_matching_key`, `algorithm_whitelist_rejects_ps256` |
| **Cross-tenant data access** | Tenant A's token attempts to read tenant B's chunks | `tenant` claim is taken from the **signed** token (never from query string / body — OWASP Multi-Tenant Cheat Sheet); AuthZ at the data-access layer (`authorize(principal, action, team, domain)`) — handlers cannot resolve a pool they aren't authorized for; default-deny → **403, never 404** (no existence leakage — OWASP A01:2025) | AuthZ cross-tenant integration test |
| **Key compromise** | Signing key exfiltrated from `BRAIN_JWT_KEY_DIR` | Private keys mode 0600, dir mode 0700; `brain key generate` + `prune` rotation keeps two keys live during the overlap window; revocation burns the compromised `jti` set without re-issuing unaffected tokens; future KMS (v3.7) moves keys off the filesystem entirely | key rotation tests, `revoke` tests |
| **Refresh token theft** | Attacker steals a refresh token and races the legitimate user to `/auth/refresh` | Refresh-chain reuse detection: the chain id is derived from `(iss, sub)`; presenting a stale refresh token calls `revoke_chain` and **burns the whole family** (OWASP pattern). The legitimate user's next refresh returns `refresh_reuse_detected` (403) | refresh-chain reuse test |

**Tenant context source (OWASP Multi-Tenant Cheat Sheet, Context7-verified
2026-07-26):**

> "Derive tenant context from authenticated, verified tokens. Use database-
> level isolation like RLS or schemas as a defense in depth. Include
> tenant_id in all resource queries, cache keys, and storage paths."

brain-server goes further than RLS: in multi-db mode (`BRAIN_MULTI_DB=true`),
each tenant's data lives in a **separate SQLite file** (physical isolation).
The `tenant` claim is verified by signature before any data-access call.

### v1.2 honest ceilings (accepted risks, see §5 exit-gate matrix)

- **Revocation is eventually consistent (≤60s).** A stolen token has at most
  60s of access after `/auth/logout` or `/auth/revoke`. Tighter would require
  a per-request DB lookup (latency cost); the bounded cache is the standard
  JWT trade-off. Distributed revocation (Redis-backed denylist) is v2.1.
- **Refresh-chain reuse detection burns the chain silently.** The legit user
  is not notified out-of-band; they discover the burn on their next refresh.
  A user-facing notification channel is v2.1.
- **No hot key reload.** Adding/removing signing keys requires an
  `install-service.sh` restart. File-watch for keys is a small follow-up.
- **EC/Ed JWK emission not implemented.** EC/Ed keys verify correctly but
  don't appear in `/.well-known/jwks.json`; rotate to RSA for any key a third
  party must discover via JWKS.

---

## 4. Residual risk (acceptances)

These are explicit risk acceptances, not bugs. Each is documented in code with
a `ponytail:` comment naming the ceiling and upgrade path.

1. **Shim-mode tenant isolation is row-level, not file-level.** Mitigation:
   SQL `WHERE tenant_id` filter at the data layer. Risk: a SQL injection in
   any query would bypass. Accepted because: every query is parameterized
   (grep-verified), and multi-db mode is the recommended path for true
   multi-tenant deployments.

2. **No encryption at rest before v3.7.** Mitigation: filesystem encryption
   (LUKS/FileVault/BitLocker) recommended in deployment checklist. Risk: a
   disk image captures plaintext DBs. Accepted because: brain-server targets
   single-host trusted-disk deployments; SQLCipher is the v3.7 fix.

3. **Prompt-injection guard is heuristic, not ML-classifier-based.** Ceiling
   documented in `contains_suspicious_pattern`. Accepted because: edge-only
   threat model; recall always marked `untrusted: true` so the consuming
   agent enforces the data/instruction boundary.

4. **Per-IP rate limit before v2.1.** Single-process in-memory. Risk: a
   distributed attacker from many IPs can exceed the per-IP cap. Mitigation:
   edge rate limit at the reverse proxy; per-tenant limit (v2.1) keys on the
   verified principal, not IP.

5. **`VACUUM INTO '<path>'` is unparameterized** (SQLite DDL limitation).
   Risk: a path containing `'` would break SQL. Mitigation: paths come from
   operator-controlled env vars (`BRAIN_DB_PATH`, `BRAIN_DATA_ROOT`), not
   from request input. Accepted because: pre-existing pattern across
   `backup.rs`, `migration.rs`, and the rehearsal tool.

6. **Token revocation is eventually consistent (≤60s).** Mitigation: the
   negative cache TTL is bounded; an attacker with a stolen token has at most
   60s of access after revocation. Accepted because: this is the standard
   JWT revocation tradeoff; tighter would require per-request DB lookup
   (latency cost).

---

## 5. Per-release security exit gates

Each major release must complete these exit gates (in addition to fmt/clippy/test):

| Gate | v1.0 ✅ | v1.1 | v1.2 | v2.0 | v2.1 | v3.7 |
|---|---|---|---|---|---|---|
| THREAT_MODEL.md updated | ✅ | ✅ | ✅ | □ | □ | □ |
| OWASP Top 10:2025 coverage checked | ✅ | ✅ | ✅ | □ | □ | □ |
| `cargo audit --deny warnings` clean | ✅ | ✅ | ✅ | □ | □ | □ |
| Penetration test report (3rd-party for v2.0+) | — | — | — | □ | □ | □ |
| AuthN test matrix (OWASP JWT Cheat Sheet) | n/a | partial | ✅ | ✓ | ✓ | ✓ |
| AuthZ test matrix (cross-tenant) | n/a | partial | ✅ | □ | ✓ | ✓ |
| Rate limit test (per-tenant + tiered) | n/a | n/a | n/a | n/a | □ | ✓ |
| Encryption audit (KMS + per-field) | n/a | n/a | n/a | n/a | n/a | □ |
| Audit hash-chain verification | n/a | ✅ | ✅ | ✓ | ✓ | ✓ |
| Compliance checklist (SOC 2 / ISO 27001 mappings) reviewed | ✅ | ✅ | ✅ | □ | □ | □ |

---

## 6. What this threat model does NOT cover

- **Physical access to the host.** Assumes the operator controls physical
  access (full-disk encryption is the operator's concern).
- **Social engineering.** Out of scope; covered by ops policies, not code.
- **Insider threat from the operator themselves.** The operator can read every
  DB. For true multi-party computation, federate (v3.7 A2A) so no single
  party has all data.
- **Quantum computing attacks.** Asymmetric crypto (RSA, ECDSA) is quantum-
  vulnerable. Post-quantum algorithms (ML-DSA / ML-KEM from NIST PQC) are
  reserved for a future major release when libraries stabilize.
- **Supply chain of the operating system.** Assumes the OS / kernel / libc
  are trusted. Hardened OS images (Flatcar, Talos) are an operator choice.
- **Payment data (PCI DSS — explicit non-scope).** Payment-card data is never
  ingested, stored, or transited by this system; no PCI scope is claimed or
  achievable through this component. Content screening + PII masking exist
  for privacy law, not as PCI controls.

---

## 7. Review cadence

- **Per major release**: full STRIDE review, update this doc, update OWASP
  coverage in `SECURITY.md`.
- **Per CVE in a direct dep**: immediate patch release.
- **Per discovered vuln (security advisory)**: immediate patch, retro on
  why the threat model missed it, update doc.
- **Annual**: third-party penetration test for any version marketed as
  "enterprise-ready" (target: v2.0+).
