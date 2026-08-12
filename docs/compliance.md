# Compliance

Brain Server is a single-node, loopback-first memory component for an AI system.
This page summarizes its compliance posture for buyers and procurement. It is a
**documented engineering posture, not a certification** — ISO/IEC 42001 and SOC 2
attestation are organization-level audits outside this repository. The full
buyer-facing technical file is [COMPLIANCE.md](./compliance.md).

---

## What the system is

brain-server stores knowledge chunks, their embeddings, a lexical index, and a
knowledge graph, and serves deterministic retrieval (`/recall`, `/search`). All
data stays on the host (SQLite); there is **no cloud, no telemetry to third
parties, and no data egress by default**.

**Data flows (loopback unless stated):**

```
client ── ingest ──► /ingest, /ingest/memory, /ingest/markdown ──► SQLite
client ── recall ──► /recall ──► embed → hybrid (vec0 + FTS5, RRF) → rank
                        └──► audit read-event (opt-in) ──► audit_events (hash chain)
operator ── DSAR ──► /dsar ──► locate → export → purge → tombstone → certificate
                          └──► Art 19 webhook (opt-in, outbound, HMAC-signed)
```

**Purpose limitation.** The system stores only what the client sends it. There is
no web crawler, no email, no location, no biometric collection. Ingestion paths
are explicit client calls; nothing is inferred or scraped.

---

## Data minimization

- Stores exactly the content it is given, chunked for retrieval. No enrichment,
  inference, or profiling.
- `POST /ingest` trusts the client's declared entities/relations — the client
  controls the graph schema.
- `BRAIN_REDACT_PII=1` (opt-in) swaps detected PII (email / phone / card) for
  placeholders at write time — conservative pattern matching, "control, not a
  classifier".
- Read-event auditing is **off by default in loopback**, **on by default in JWT
  mode**; sampling via `BRAIN_AUDIT_READ_SAMPLE_RATE`.

---

## Logging (EU AI Act Art 12 / Art 26(6) posture)

The audit is an append-only, tamper-evident SHA-256 hash chain. Every row links to
its predecessor; `/audit/verify` proves integrity; `/metrics` reports
`brain_audit_chain_ok`. Retention is configurable via `BRAIN_AUDIT_RETENTION_DAYS`
(deployers: ≥180 days per AI Act Art 26(6) guidance).

| Event class | Recorded |
|---|---|
| Ingest / write | Hash-chained audit row |
| Auth denial | Hash-chained audit row |
| Read (recall/search/get) | Opt-in hash-chained row (no content, no raw query) |
| Purge / DSAR | Tombstone + audit + deletion certificate |

---

## Erasure (GDPR / CCPA / PH DPA)

- **`GET /export`** — portable JSON export of a subject's data.
- **`POST /purge`** — hard, explicit, audited deletion (by id or owner) with a
  tombstone.
- **`POST /dsar`** — locate → export → purge → **chain-verifiable deletion
  certificate** (found / purged / tombstone root / chain head / certified_at).
- **`GET /tombstones`** — queryable deletion registry.
- **Art 19 onward notification** — opt-in HMAC-SHA256-signed webhook on purge.

---

## Framework mapping

| Framework | Posture |
|---|---|
| **ISO/IEC 42001** | AI management-system posture documented; algorithmic-risk controls (abstention, human-in-the-loop write-back) |
| **NIST AI RMF** | Govern / Map / Measure / Manage controls across the retrieval lifecycle |
| **SOC 2** | Audit log, access control, encryption-at-rest (backup), change control |
| **EU AI Act** | Art 12/26(6) logging posture; Art 50 origin metadata note + `/.well-known/ai-notice` disclosure; Art 4 literacy playbook ([AI_LITERACY.md](./AI_LITERACY.md)) |
| **GDPR / CCPA / PH DPA** | Data portability, erasure, DSAR workflow, jurisdiction posture |

The full, row-by-row mapping with the intent-based-auditing coverage and the
jurisdiction table is in [COMPLIANCE.md](./compliance.md).

---

## What certification is NOT claimed

This document describes an engineered control posture. ISO 42001 / SOC 2
attestation require organization-level audits (policy, third-party pen tests,
monitoring) that this repository does not and cannot certify. Buyers should treat
these docs as the technical evidence base an audit would start from, not as an
audit result.

---

## Next steps

- [Security](./security.md) — the controls behind these postures.
- [Deployment](./deployment.md) — configuring audit retention, redaction, and the
  DSAR webhook.
