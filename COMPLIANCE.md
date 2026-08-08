# COMPLIANCE.md — Compliance Posture & Technical File

**Version:** 1.16.7 "Integrated" · **Last updated:** 2026-08-08

This is the buyer-facing technical file: what brain-server IS, what it logs,
how it erases, and how it maps to the frameworks procurement asks about. It is
a documented engineering posture, **not a certification** — ISO/IEC 42001 /
SOC 2 attestation are organization-level audits that happen outside this
repository.

Companion engineering docs: `SECURITY.md` (OWASP Top 10:2025 map),
`THREAT_MODEL.md`, `AUDIT.md`.

---

## 1. System Description

brain-server is a single-node, loopback-first memory component for an AI
system (the "client": an agent or an application using this server as its
memory backend). It stores knowledge chunks, their embeddings (vec0 KNN), a
lexical index (FTS5), and a knowledge graph (entities + relationships), and it
serves deterministic retrieval (`/recall`, `/search`). All data stays on the
host (SQLite; `~/.openclaw/workspace/brain.db` by default); there is no cloud,
no telemetry to third parties, and no data egress by default.

**Data flows** (all loopback HTTP unless stated):

```
client ── ingest ──► /ingest, /ingest/memory, /ingest/markdown ──► SQLite
client ── recall ──► /recall ──► embed → hybrid (vec0 + FTS5, RRF) → rank
                        └──► audit read-event (opt-in) ──► audit_events (hash chain)
operator ── DSAR ──► /dsar ──► locate → export → purge → tombstone → certificate
                          └──► Art 19 webhook (opt-in, outbound, HMAC-signed)
```

**Purpose limitation.** The system stores only what the client sends it. It has
no web crawler, no email, no location, no biometric collection. Ingestion
paths are explicit client calls; nothing is inferred or scraped.

## 2. Purpose Limitation & Data Minimization

- The server stores exactly the content it is given, chunked for retrieval. It
  does not enrich, infer, or profile.
- `POST /ingest` trusts the client's declared entities/relations — the client
  controls the schema of the graph.
- `BRAIN_REDACT_PII=1` (opt-in, v1.14) swaps detected PII for placeholders at
  write time (email/phone/Luhn-card patterns, conservative pattern match —
  "control, not a classifier").
- Read-event auditing is **off by default in loopback mode** (noise + the
  personal-use contract) and **on by default in JWT mode** (enterprise
  posture). Override: `BRAIN_AUDIT_READ_EVENTS`, sampling via
  `BRAIN_AUDIT_READ_SAMPLE_RATE`.

## 3. Logging Specification (EU AI Act Art 12 / Art 26(6) posture)

The audit is an append-only, tamper-evident hash chain (`audit_events`,
v1.1). Every row is SHA-256-chained to its predecessor (`prev_hash` over
`ts|kind|actor|target_hash|prev`), verified by `/audit/verify` and reported as
`brain_audit_chain_ok` in `/metrics`.

| Event kind | Recorded | Payload |
|---|---|---|
| Ingest (all paths) | always | hash-only target, kind, actor |
| AuthN/AuthZ | always | token-verified / rejected (reason) / authz-denied (principal/action/team/domain) |
| Webhook ingress | always | HMAC-verified |
| Reconcile / supersede / undo / DSAR | always | hash-only |
| **Read events (`recall`, `search`, `get`, `multi-get`)** | **opt-in (v1.15)** | caller principal (or `loopback`), target (query text / chunk refs), scope decision; recall also stores the replayable trace (chunk ids + scores + decision) in `recall_traces` |

**Hash-only invariant.** The audit row stores hashes, never content. The raw
query/identifier must never appear in the row (pinned by test).

**Retention.** `BRAIN_AUDIT_RETENTION_DAYS` (unset = keep forever for personal
use). When set, rows older than the window are pruned on read-event writes and
the hash chain is re-anchored — the oldest surviving row becomes the new
genesis. Deployers subject to AI Act Art 26(6) guidance should set ≥180.

**Decision-path evidence (Art 22 "meaningful information about the logic").**
`/recall?trace=true` returns a `trace_id`; `GET /recall/{trace_id}/trace`
replays the exact injected chunks, fused scores, abstention decision, access
scope applied, principal, and domains searched. The trace is the replayable
artifact proving *what* informed a retrieval — the log next to the answer.

## 4. DSAR Workflow (GDPR Art 15/17/19 + CCPA/ADMT)

`POST /dsar {subject, action: export|purge|both}` (Admin-only):

1. **Locate** — every `knowledge` row with `owner = subject`, plus all
   transitive `derived_from` descendants (bounded walk, depth 8).
2. **Export** — portable JSON bundle of the located rows (no `pii_map`
   values).
3. **Purge** — hard delete in one transaction: `knowledge` + `vec_knowledge`
   + `relationships` + `evidence_links` + `proposals` references; a tombstone
   row is left with the reason (`owner:<subject>` / `derived`) and, for
   derived chunks, the root `origin_id`.
4. **Certificate** — `{subject, action, found_count, purged_ids,
   tombstone_root, certified_at, chain_head}`; the `chain_head` is the audit
   chain tip at certification time. Re-fetchable via
   `GET /dsar/{id}/certificate` with a live `chain_verifies` recomputation.
5. **Art 19 onward-notification** — opt-in webhook (`BRAIN_DSAR_WEBHOOK_URL`
   [+ `BRAIN_DSAR_WEBHOOK_SECRET`]): on a completed purge, POSTs
   `{subject, certified_at, certificate_id}` HMAC-SHA256-signed
   (`X-Brain-Signature-256: sha256=<hex>`). Fail-soft: bounded retries, then
   logged; a webhook failure never rolls back the purge.

The deletion registry (`GET /tombstones?subject=&since=`) is the queryable,
hash-only, append-only tombstone view (the EDPB Coordinated Enforcement
Framework ask). Nothing is ever deleted autonomously; every deletion is an
explicit operator action.

## 5. Risk Controls (recap)

| Risk | Control |
|---|---|
| Memory injection / prompt poisoning | v1.14 proposal gate (`POST /ingest/proposal` → human approval); quarantine + flagged-row exclusion (v0.9.7) |
| Data leakage across tenants | JWT AuthN (v1.2) + per-record `access_scope` deny-by-default filter (v1.14) + per-domain pools (v1.0) |
| Unwarranted erasure | DSAR is Admin-only; purge is explicit + tombstoned + audited; `/purge` requires explicit ids/owner |
| Tampered audit | SHA-256 hash chain + `/audit/verify` + `/metrics` chain-ok gauge + DSAR certificates anchored to the chain |
| Unauthenticated access | Bearer token (opaque) or JWT/JWS + OIDC discovery (v1.2); loopback-first defaults |
| PII at rest | Not encrypted at rest — documented posture: full-disk encryption (LUKS/FileVault) is the operator's layer; write-time placeholder mode opt-in (v1.14) |
| Exfiltration | No outbound HTTP by default; the only outbound path is the opt-in Art 19 webhook |

## 6. Framework Maps

### 6.1 ISO/IEC 42001 (AI management systems) + NIST AI RMF + SOC 2

| Control area | brain-server evidence | ISO 42001 | NIST AI RMF | SOC 2 |
|---|---|---|---|---|
| Identity & access | JWT/JWS + OIDC/JWKS, opaque bearer, AuthZ matrix per route (handler-entry gates, test-pinned) | 8.3 | Govern 1.1 / Map 3 | CC6.1, CC6.6 |
| Change management | semver releases, schema-contract + route-coverage tests, migration rehearsal tool | 9.4 | Measure 4.1 | CC8.1 |
| Monitoring | hash-chained audit + `/metrics` + `/audit/verify` chain-ok | 9.3 | Measure 4.2 | CC7.1–CC7.4 |
| Logging | append-only hash chain; read-event audit (opt-in, v1.15) | 9.3 | Measure 4.4 | CC7.2 |
| Data lifecycle | proposal gate, decay, DSAR export/purge/certificate, tombstones, retention pruning | 9.2 | Govern 2.1 | CC6.1, CC9.1 |
| System ops | `/health` + capacity envelopes + `bench --envelope` ship gate | 8.4 | Measure 5.1 | CC7.3 |

### 6.2 Intent-Based Auditing 4/4

| Pillar | Evidence |
|---|---|
| Identity | JWT principal (`sub`/`tenant`/`scopes`), OIDC discovery, JWKS (v1.2) |
| Decision-path (memory reads/writes) | Read-event audit + `GET /recall/{trace_id}/trace` replay (v1.15) |
| Policy attribution | AuthZ matrix at handler entry (v1.12.1, test-pinned route-by-route) + record-level `access_scope` (v1.14) |
| Selective-forgetting | DSAR locate→export→purge→certificate + tombstone registry (v1.15) |

### 6.3 Jurisdiction Posture

- **Philippines (DPA RA 10173):** personal data stays on the host; the DSAR
  workflow + deletion certificate are the data subject's rights-of-erasure
  implementation; breach notification is operator-side (the server provides
  the audit evidence).
- **EU (GDPR):** Art 15/17 (DSAR + erasure certificate), Art 19 (onward-
  notification webhook), Art 22 (trace replay = meaningful information about
  the logic), Art 26(6) guidance (retention ≥180 days where required).
- **California (CCPA/CPRA + ADMT):** the Art 22 trace maps to the ADMT
  right-to-know logic explanation; `/export` is the data-portability response;
  `/tombstones` evidences completed deletion requests.
- **Residency:** loopback-first means the data physically never leaves the
  host; cross-border deployment is purely a deployment choice, not an
  architectural one.
- **EU CRA horizon:** the embedded-dependency story is documented in
  `SECURITY.md` (bundled SQLite 3.53.2, cargo-audit gate); no change to the
  data path.

### 6.4 EU AI Act Art 4 (AI literacy)

The deployer's literacy evidence for the memory component: the dashboard's
controls (`/proposals` review queue, `/quarantine`, `/audit`, `/tombstones`)
and the trace endpoint make the component's decisions inspectable —
"what informed this retrieval, and who approved this write" — which is the
operational substance of Art 4 literacy for a memory component. See
`docs/AI_LITERACY.md` for the operator-facing literacy playbook.

## 7. Machine-Readable Origin Metadata (Art 50 transparency bridge)

Every `knowledge` row carries an ingest path (`source`: manual | vault |
memory | markdown | structured) and, where present, `assertion_kind`
(stated/observed/inferred) + `confidence`. `/export` emits these fields with
the content, so a memory export can state *how a memory entered the system and
with what provenance* — the transparency bridge for model-vs-human origin.
(Forward link: UMP wire-format conformance is a later release.)

**Machine-readable disclosure.** The server also serves the Art 50 disclosure
itself at `GET /.well-known/ai-notice` (public, no auth) — a JSON document
with `art_50: true`, the human-readable disclosure that stored content may be
AI-generated, the `origin_metadata` fields a consumer can read per row
(`source` / `assertion_kind` / `confidence`), `effective_date: 2026-08-02`,
and `jurisdiction: EU AI Act Article 50 (Regulation (EU) 2024/1689)`. A
deployer can point a consumer or auditor at the URL and at an `/export` bundle
to close the model-origin transparency loop without pasting a policy.

## 8. Retention Classes

| Class | Contents | Default | Mechanism |
|---|---|---|---|
| Permanent | audit chain (excluding pruned window), tombstones, `dsar_requests`, proposals ledger | forever (personal use) | `BRAIN_AUDIT_RETENTION_DAYS` opt-in prune with chain re-anchor |
| Transient | read-event traces (`recall_traces`) | same window as the audit rows they key | pruned with their audit row |
| Content | knowledge chunks | until explicitly purged | `/purge` (ids/owner), DSAR, supersession (`valid_to`) |
| Placeholder map | `pii_map` (write-time redaction mode) | until operator purges | `/purge`, excluded from `/export` by default |

## Honest Ceilings

- Single-process audit chain (distributed audit = v2.1 multi-instance work).
- Read events default off in loopback mode (personal-use contract); JWT mode
  on. A loopback deployment must set `BRAIN_AUDIT_READ_EVENTS=on` explicitly
  to collect read traces.
- No PII encryption at rest — full-disk encryption is the operator's layer.
- DSAR export is brain-server JSON, not the UMP wire format.
- This file is a documented posture, not a certification.
- The trace endpoint serves recorded events only; there is no historical
  backfill for recalls that predate v1.15.0.
