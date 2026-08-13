# Proof Map — every claim, its release, its live evidence

The rule: **a compliance claim you can't verify live is not a claim, it's a
promise.** Every statement in `SECURITY.md`, `COMPLIANCE.md`, and
`OWASP_AGENTIC_2026.md` maps below to (a) the release that shipped it and
(b) the exact live command that proves it. A reviewer can reproduce each row
against a running instance.

## How to verify live

Every command is safe (read-only unless marked `WRITE`). Run them against a
running instance (default `localhost:8765`). The `brain` CLI and a bearer token
are assumed; swap `BRAIN_TOKEN_FILE`/`-H 'authorization: Bearer …'` as needed.

## The map

| Claim (doc) | Shipped in | Live proof |
|---|---|---|
| **Tamper-evident audit hash chain** (`COMPLIANCE.md` §3, `SECURITY.md`) | v1.1.0 | `curl -s localhost:8765/audit/verify` → `{"ok":true}`; `/audit` rows carry `prev_hash` |
| **DSAR → chain-verifiable deletion certificate** (`COMPLIANCE.md` §DSAR) | v1.15.0 | `curl -s -X POST localhost:8765/dsar -d '{"owner":"..."}'` → cert id; `curl -s localhost:8765/dsar/{id}/certificate` shows `chain_verifies` |
| **DSAR footprint preview (dry-run)** | v1.20.21 | `curl -s -X POST localhost:8765/dsar -d '{"subject":"alice","dry_run":true}'` → `footprint` counts, **zero rows deleted**, no ledger row, no certificate |
| **DSAR 30-day Art 17 window visible on the ledger** | v1.20.22 | `curl -s localhost:8765/dsar` → `requests[]` rows carry `deadline` = `created_at + BRAIN_DSAR_WINDOW_DAYS` (default 30); `POST /dsar` response carries `created_at`/`deadline` |
| **Deletion registry** | v1.15.0 | `curl -s localhost:8765/tombstones` → rows with `content_hash` + `purged_at` |
| **Opt-in Art 19 webhook** (outbound, HMAC-signed) | v1.15.0 | env `BRAIN_DSAR_WEBHOOK_URL`/`_SECRET`; sign a purge and see the signed POST |
| **Read-event audit (opt-in)** | v1.15.0 | env `BRAIN_AUDIT_READ_EVENTS=on`; a `/recall` then appears as `kind=recall` in `/audit` |
| **Art 50 AI transparency notice** | v1.16.7 | `curl -s localhost:8765/.well-known/ai-notice` → JSON with `origin_metadata` |
| **JWT/JWS AuthN, no HS256/`none`** | v1.2.0 | `/.well-known/openid-configuration` + `/.well-known/jwks.json`; a forged `alg=none` token → 401 |
| **Deny-by-default AuthZ** | v1.2.0 + v1.12.1 wiring | a read-scoped token on `/reindex` → 403; cross-tenant `/audit` filter → 403 |
| **OIDC discovery + JWKS** | v1.2.0 | `curl -s localhost:8765/.well-known/jwks.json` → RSA/EC/Ed keys |
| **UMP 1.0 / L3 conformance** | v1.17.3/.4 | `curl -s localhost:8765/ump/capabilities` → `conformance: "UMP 1.0 / L3"` |
| **Capability tokens, least-privilege** | v1.17.3 | `brain ump keygen`; a read-only token on `/ump/remember` → 401 |
| **Injection screen (blocklist + classifier)** | v1.20.1/.3 | a flagged payload → stored `flagged`; `/health` shows `injection_classifier_loaded` |
| **Human-in-the-loop write gate** | v1.14.0 + v1.20.1 | `POST /ingest/proposal` creates NO knowledge row; promote only via `/proposals/{id}/approve` |
| **Proposal TTL auto-reject** | v1.20.1 | `BRAIN_PROPOSAL_TTL_SECS`; a stale approve → 400 `proposal_expired` |
| **PII redaction (`[redacted:…]`)** | v1.14.0 | a PII-bearing row returned to a non-`pii:read` principal → masked; `/verify` never leaks |
| **`/health` hardening + capacity** | v1.3.0 / v0.9.9 | `curl -s localhost:8765/health` → `hardening.unsafe_blocks`, `capacity` object |
| **SBOM (CycloneDX)** | v1.17.5 | `scripts/sbom.sh` → `dist/*.cdx.json` on release |
| **OWASP 2026 matrix = 100% control coverage** | v1.20.5 | `docs/OWASP_AGENTIC_2026.md` — each row cites a shipped feature or owned ceiling |
| **Origin provenance (`human`/`model`/`imported`)** | v1.18.2 | `/export` returns `provenance_summary {total, by_origin, by_source}` |
| **Standard Webhooks signed timestamp** | v1.20.4 | `BRAIN_WEBHOOK_TIMESTAMP_REQUIRED=1`; `/webhooks/{kind}` verifies `v1,<base64>` HMAC |
| **SNI/zero-telemetry** | v1.16.0+ | nothing collects data; the grep guard `credentials_stay_in_memory` passes in CI |

## Claims that are ceilings (owned, not shipped)

These are stated in the docs as **honest ceilings** — check them in
`OWASP_AGENTIC_2026.md` residual-risk + `ROADMAP.md`:

- LLM01 has **no prevention** per OWASP 2026 (segregation + gates + least-
  privilege are the surviving controls). v2.x re-evaluation.
- Multi-team **tenancy + per-tenant limits** — planned v2.0/v2.1, no code yet.
- **At-rest encryption, mTLS, A2A federation, OIDC authorization-code** — v2.x
  ceilings, named owners in the matrix.
- **SOC 2 Type II evidence program** — v1.20.10 + the operator runs it; this
  map is the raw material.

## Reproduce end to end

The scripted walk-through lives in [`reproduce.md`](./reproduce.md). It runs
every row above against a fresh throwaway instance, so a reviewer can prove the
whole posture in one pass without touching production data.
