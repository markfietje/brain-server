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
| **Art 50(2) provenance marks on engine-generated artifacts** (Attestation) | v1.28.62 | a remedy draft / ADR packet / outreach export / `kb_manifest.json` carries `"provenance": {mark: AIGEN, generator, generated_at, signed_by, sig}`; flip one byte anywhere → `provenance::verify_artifact` refuses (pinned by `provenance_marks_present_on_all_four_classes` + `tampered_provenance_fails_verify`) |
| **Principal kill-switch (ASI03/07)** (Attestation) | v1.28.62 | `POST /ops/agents/revoke {principal, reason}` (Admin) → every card use / dispatch / result refuses `403 principal_revoked`; in-flight runs drain to `cancelled` with `delegation/revoked` lineage events; `GET /ops/agents/revocations` lists the register; the audit chain carries revoke + drain in one tx |
| **Crypto inventory + algorithm-agility seams (PQC)** (Attestation) | v1.28.62 | `docs/crypto-inventory.md` — SP 1800-38B-shaped table (algorithm · what it protects · HNDL verdict · swap path) + the JWT ML-DSA landing procedure (`auth/jwt.rs::ALLOWED_ALGS` seam) + the UMP did:key multicodec version-prefix rule; pinned by `pqc_inventory_seam_deliverable` |
| **Approval-fatigue telemetry (ASI09)** (Attestation) | v1.28.62 | `GET /workflow/scoreboard` (DPO/admin) → `review_independence_risk` + `approval_uniformity_ratio` + `review_decisions_window`; pinned to the client detector's arithmetic by `scoreboard_uniformity_matches_client_math` |
| **Calendar-as-code regulatory watches** (CRA/AI Act/PQC) | v1.28.58–.62 | `cargo test --lib reg_watch` — CRA Art 14 runbook + standby/revocation drill records + the Art 50 marking deliverable + the PQC inventory, each a CI gate |
| **Provable embedding deletion — purge is not a row delete** (EDPB CEF, Preflight) | v1.28.75 | Ingest → note id, purge id, then `vec0` re-recall negative proves embedding gone (see `reproduce.md` § "Embedding deletion proof"); idempotent — a re-purge of the tombstoned id is a no-op (`purged: 0`), and ids are AUTOINCREMENT so nothing ever re-occupies the erased slot; pinned by DSAR cert `held_ids`/`chain_verifies` + the `/tombstones` registry |
| **Transport never follows redirects** (Lockdown) | v1.28.80 | Plugin `fetchJson` sends `redirect: manual`; any 3xx refuses as `network` before the bearer can ride it (pinned by `a 3xx refuses without following`) |
| **Two-principal approval quorum** (Lockdown) | v1.28.80 | `BRAIN_APPROVAL_QUORUM=2`: first approval returns `pending_second` with a hash-chained row; same-principal repeat gets `quorum_same_principal`; distinct second principal promotes (pinned by `quorum_gate_defers_first_and_refuses_same_principal`) |
| **Visible cross-domain mixing** (Lockdown) | v1.28.80 | Domain-routed recall borrowing global rows returns `included_global: true` (pinned by `global_rescue_flag_marks_cross_domain_mixing`) |
| **Signed catalog-pin acks** (Lockdown) | v1.28.80 | Pin file carries a detached Ed25519 signature; forged or unsigned files rebuild loudly with every tool re-notifying (pinned by `forged_pins_rebuild_loudly`) |

## Claims that are ceilings (owned, not shipped)

These are stated in the docs as **honest ceilings** — check them in
`OWASP_AGENTIC_2026.md` residual-risk + `ROADMAP.md`:

- LLM01 has **no prevention** per OWASP 2026 (segregation + gates + least-
  privilege are the surviving controls). v2.x re-evaluation.
- Multi-team **tenancy + per-tenant limits** — planned v2.0/v2.1, no code yet.
- **At-rest encryption, mTLS, A2A federation, OIDC authorization-code** — v2.x
  ceilings, named owners in the matrix.
- **Classical signatures until a PQC stack lands** — the crypto inventory
  (v1.28.62) maps every primitive's swap path; JWT ML-DSA waits on the IdP,
  UMP signatures land via the did:key multicodec prefix. Printed ceiling,
  owned.
- **SOC 2 Type II evidence program** — v1.20.10 + the operator runs it; this
  map is the raw material (refreshed against the current surface in
  v1.28.80 — the Attestation rows plus the Lockdown rows above).

## Reproduce end to end

The scripted walk-through lives in [`reproduce.md`](./reproduce.md). It runs
every row above against a fresh throwaway instance, so a reviewer can prove the
whole posture in one pass without touching production data.
