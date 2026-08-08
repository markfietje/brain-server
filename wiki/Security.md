# Security

Brain Server is a local-first memory component for AI agents, so its security model centers on three questions: **who is allowed to talk to it, what can they do, and can anyone tamper with its records.** This page is the informational summary; the full threat model lives in `SECURITY.md` and `THREAT_MODEL.md` in the repository.

## Principles

- **Loopback-safe by default.** The server refuses to bind `0.0.0.0` unless `BIND_PUBLIC=1`. The default posture is that the memory lives on the host.
- **No data egress.** There is no telemetry to third parties. Outbound HTTP is limited to an opt-in Art 19 DSAR webhook and is disabled unless configured.
- **Authentication is explicit.** Off by default if no token resolves; when on, it is either opaque bearer or JWT/JWS.
- **Least privilege.** A deny-by-default AuthZ layer gates every non-public route.

## Authentication modes

### Opaque bearer (default)

Set `AUTH_TOKEN` or `AUTH_TOKEN_FILE`. Multiple tokens are accepted (newline-separated) for live rotation. Comparison is **constant-time** (the token compare is pinned by a regression test). The install script relocates any plaintext token out of the launchd plist into a 0600 file.

### JWT/JWS (opt-in)

Set `BRAIN_JWT_ISSUER` and load signing keys:

```bash
brain key generate    # RSA keypair, private key 0600
brain key list        # show loaded keys
brain key prune       # drop expired keys from JWKS
```

- **Algorithms**: RS256/ES256/EdDSA only. HS256 and `none` are rejected unconditionally (the OWASP algorithm-confusion defense).
- **Claims**: `iss`, `aud`, `exp`, `nbf`, `sub`, `jti` all validated.
- **Revocation**: `(jti, iss)` denylist; refresh-chain reuse detection burns the whole family.
- **Discovery**: OIDC at `/.well-known/openid-configuration`, JWKS at `/.well-known/jwks.json`.

## Access control

A deny-by-default AuthZ layer (`Action`: Read / Write / Admin / Traverse; `Scope` grammar with wildcards) gates every non-public route at handler entry. In JWT mode, record-level `access_scope` + `owner` filter data so a principal only sees what it may. Denials return `403`, never `404`, so route existence is not leaked.

## Data protections

- **Append-only audit log** — a SHA-256 hash chain. Each row links to its predecessor; `/audit/verify` proves no row was modified or removed. Read events are opt-in (default on in JWT mode, off in loopback).
- **Prompt-injection quarantine** — suspicious input is stored but excluded from retrieval until reviewed (deterministic structural control, not a classifier).
- **PII** — opt-in write-time placeholder mode (`BRAIN_REDACT_PII=1`) swaps detected email/phone/card patterns for placeholders; output redaction masks PII for principals without `pii:read`.
- **Untrusted-evidence boundary** — every retrieved result serializes `untrusted: true` (OWASP LLM01:2025).
- **Parameterized SQL** — no SQL-injection surface.
- **Encrypted backup** — AES-256-GCM, checksummed, excludes secrets.
- **Verified webhooks** — HMAC verification, replay-window enforcement, idempotency.

## What it deliberately does not do

- No credentials stored in plaintext (connector configs are 0600, atomic-write).
- No cookies (bearer headers make CSRF structurally impossible).
- No untrusted content ever rendered as trusted HTML (the client bans `dangerous_inner_html`, grep-guarded in CI).

## Security research & reference links

- **OWASP Top 10 for LLM Applications** — the source of the LLM01:2025 untrusted-evidence boundary and prompt-injection guidance: [owasp.org www-project-top-10-for-large-language-model-applications](https://owasp.org/www-project-top-10-for-large-language-model-applications/).
- **RFC 9116** — the `security.txt` disclosure standard: [rfc-editor.org/rfc/rfc9116](https://www.rfc-editor.org/rfc/rfc9116).
- **OWASP JWT Cheat Sheet** — the basis of the JWT verification matrix: [cheatsheetseries.owasp.org jwt](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_for_Java_Cheat_Sheet.html).
- **OWASP Query Parameterization Cheat Sheet** — basis of the parameterized-SQL guarantee: [cheatsheetseries.owasp.org query_parameterization](https://cheatsheetseries.owasp.org/cheatsheets/Query_Parameterization_Cheat_Sheet.html).

## Supported versions

| Line | Status |
|---|---|
| Current minor (`1.16.x`) | Supported — receives fixes |
| Previous minor | Supported |
| `0.9.x` / `1.0.x` | Maintained for back-compat / security fixes |
| < 0.9 | Unsupported |

Disclosure endpoint: `/.well-known/security.txt` (RFC 9116). To report a vulnerability, use the GitHub Security Advisories tab — **do not file public issues for security findings.**

## Next steps

- **[Governance & Compliance](Governance-and-Compliance)** — how the controls map to ISO 42001 / SOC 2.
- **[Installation](Installation)** — configuring auth in practice.
- **[Configuration](Configuration)** — the auth environment variables.
