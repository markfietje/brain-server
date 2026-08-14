# Security

Brain Server is a local-first memory component for AI agents, so its security
model centers on three questions: **who is allowed to talk to it, what can they
do, and can anyone tamper with its records.** The full threat model lives in
[Threat model](./THREAT_MODEL.md); this page
is the informational summary.

---

## Principles

- **Loopback-safe by default.** The server refuses to bind `0.0.0.0` unless
  `BIND_PUBLIC=1`. The default posture is that the memory lives on the host.
- **No data egress.** There is no telemetry to third parties. Outbound HTTP is
  opt-in and off unless configured: an Art 19 DSAR webhook and a system-alert
  webhook (`BRAIN_ALERT_WEBHOOK_URL`), both Standard Webhooks signed and
  redirect-refusing.
- **Authentication is explicit.** Off by default if no token resolves; when on, it
  is either opaque bearer or JWT/JWS.
- **Least privilege.** A deny-by-default AuthZ layer gates every non-public route.

---

## Authentication modes

### Opaque bearer (default)
Set `AUTH_TOKEN` or `AUTH_TOKEN_FILE`. Multiple tokens are accepted (newline-
separated) for live rotation. Comparison is constant-time. The install script
relocates any plaintext token out of the launchd plist into a 0600 file.

### JWT/JWS (opt-in)
Set `BRAIN_JWT_ISSUER` and load signing keys:

```bash
brain key generate    # RSA keypair, private key 0600
brain key list        # show loaded keys
brain key prune       # drop expired keys from JWKS
```

- **Algorithms**: RS256/ES256/EdDSA only. HS256 and `none` are rejected
  unconditionally (algorithm-confusion defense).
- **Claims**: `iss`, `aud`, `exp`, `nbf`, `sub`, `jti` all validated.
- **Revocation**: `(jti, iss)` denylist; refresh-chain reuse detection burns the
  whole family.
- **Discovery**: OIDC at `/.well-known/openid-configuration`, JWKS at
  `/.well-known/jwks.json`.

---

## Access control

A deny-by-default AuthZ layer (`Action`: Read / Write / Admin / Traverse; `Scope`
grammar with wildcards) gates every non-public route at handler entry. In JWT
mode, record-level `access_scope` + `owner` filter data so a principal only sees
what it may. Denials return `403`, never `404`, so route existence is not leaked.

---

## Data protections

- **Append-only audit log** — a SHA-256 hash chain. Each row links to its
  predecessor; `/audit/verify` proves no row was modified or removed. Read events
  are opt-in (default on in JWT mode, off in loopback).
- **Prompt-injection quarantine** — suspicious input is stored but excluded from
  retrieval until reviewed (deterministic structural control, not a classifier).
- **PII** — deterministic read-time output redaction masks email/phone/card for
  principals without `pii:read`; plaintext is never stored in a placeholder
  vault (there is no `pii_map`, removed v1.20.19).
- **Untrusted-evidence boundary** — every retrieved result serializes
  `untrusted: true` (OWASP LLM01:2025). v1.20.28 wraps each injected block in
  `UNTRUSTED_BEGIN`/`UNTRUSTED_END` sentinels and drops any hit not explicitly
  tagged `untrusted` (fail-safe toward the security wedge).
- **EchoLeak / markdown-exfil strip** — the read seam rewrites markdown image/link
  references (`![label](url)` → `[label]`, `[text](url)` → `text`) so a recalled
  chunk cannot exfiltrate context via a rendered URL (v1.20.27).
- **Parameterized SQL** — no SQL-injection surface.
- **Encrypted backup** — AES-256-GCM, checksummed, excludes secrets.
- **Constant-time / verified-writes guards** — the token compare and the audit
  chain verification are pinned by regression tests.
- **Fail-closed bind** — the server refuses to start on a non-loopback bind when
  no auth (bearer token or JWT) is configured, so an unauthenticated
  superuser API is never exposed off the loopback (v1.20.29).
- **SSRF-hardened egress** — outbound webhook/alert calls use a single client
  with redirects disabled (`redirect: none`), so a misconfigured callback URL
  that 302s to a cloud-metadata or loopback address is surfaced, never followed
  (v1.20.26).

---

## What it deliberately does not do

- No credentials stored in plaintext (connector configs are 0600, atomic-write).
- No cookies (bearer headers make CSRF structurally impossible).
- No untrusted content ever rendered as trusted HTML (the client bans
  `dangerous_inner_html`; grep-guarded in CI).
- No autonomous write-back: captured fragments are scored, not stored, and become memory
  only through the human gate. See [**Human in the loop**](./human-in-the-loop.md).
- No **agent-callable erasure**: an agent can read memory and propose writes, but cannot
  delete it. The `memory_forget` agent tool was removed (v1.20.25); erasure is human-only via
  the operator console and the HTTP API (`DELETE /memory/{id}`, `POST /purge`, DSAR — the
  `brain` CLI has no erasure command). The
  full authority split is in [**Human in the loop**](./human-in-the-loop.md).

---

## Supported versions

| Line | Status |
|---|---|
| Current minor (`1.20.x`) | Supported — receives fixes |
| Previous minor | Supported |
| `0.9.x` / `1.0.x` | Maintained for back-compat / security fixes |
| < 0.9 | Unsupported |

Disclosure endpoint: `/.well-known/security.txt` (RFC 9116). To report a
vulnerability, use the GitHub Security Advisories tab. **Do not file public
issues for security findings.**

---

## Next steps

- [Compliance](./compliance.md) — how the controls map to ISO 42001 / SOC 2.
- [Deployment](./deployment.md) — configuring auth in practice.
