# Governance & Compliance

This page explains the **governance layer** — the append-only audit log, the DSAR workflow, data retention — and how Brain Server maps to the compliance frameworks that procurement and regulators ask about. This is a **documented engineering posture, not a certification**: ISO/IEC 42001 / SOC 2 attestation are organization-level audits that happen outside this repository.

## The audit log: a tamper-evident hash chain

Every governance-relevant event is recorded in an **append-only audit log** built as a SHA-256 hash chain. Each row stores the hash of the previous row (`prev_hash` over `ts|kind|actor|target_hash|prev`), so **any modification or deletion is detectable**.

- `/audit/verify` recomputes and proves the chain is intact.
- `/metrics` reports a live `brain_audit_chain_ok` gauge.
- **Hash-only invariant:** the audit row stores hashes, never content. Raw queries and identifiers never appear (pinned by a regression test).

| Event kind | Recorded | Payload |
|---|---|---|
| Ingest (all paths) | always | hash-only target, kind, actor |
| AuthN/AuthZ | always | token-verified / rejected (reason) / authz-denied (principal/action/team/domain) |
| Webhook ingress | always | HMAC-verified |
| Reconcile / supersede / undo / DSAR | always | hash-only |
| **Read events** (`recall`, `search`, `get`, `multi-get`) | **opt-in (v1.15)** | caller principal, target refs, scope decision; recall also stores a replayable trace |

### Recall traces (v1.15) — "what informed this answer"

`/recall?trace=true` returns a `trace_id`; `GET /recall/{trace_id}/trace` replays the exact injected chunks, fused scores, abstention decision, access scope, principal, and domains searched. **The trace is the replayable artifact proving what informed a retrieval** — the log next to the answer.

## The DSAR workflow (GDPR Art 15/17/19 + CCPA)

`POST /dsar {subject, action: export|purge|both}` (Admin-only) runs five steps:

1. **Locate** — every `knowledge` row with `owner = subject`, plus all transitive `derived_from` descendants (bounded walk, depth 8).
2. **Export** — a portable JSON bundle of the located rows.
3. **Purge** — hard delete in one transaction across `knowledge`, `vec_knowledge`, `relationships`, `evidence_links`, and `proposals`; a tombstone row is left with the reason.
4. **Certificate** — `{subject, action, found_count, purged_ids, tombstone_root, certified_at, chain_head}`; re-fetchable via `GET /dsar/{id}/certificate` with a live chain check.
5. **Art 19 onward-notification** — an opt-in webhook POSTs `{subject, certified_at, certificate_id}` HMAC-SHA256-signed. Fail-soft — a webhook failure never rolls back the purge.

The deletion registry (`GET /tombstones`) is the queryable, hash-only, append-only tombstone view. **Nothing is ever deleted autonomously; every deletion is an explicit operator action.**

## Data retention classes

| Class | Contents | Default | Mechanism |
|---|---|---|---|
| Permanent | audit chain, tombstones, `dsar_requests`, proposals ledger | forever (personal use) | `BRAIN_AUDIT_RETENTION_DAYS` opt-in prune with chain re-anchor |
| Transient | read-event traces (`recall_traces`) | same window as the audit rows they key | pruned with their audit row |
| Content | knowledge chunks | until explicitly purged | `/purge`, DSAR, supersession (`valid_to`) |
| Placeholder map | `pii_map` (write-time redaction) | until operator purges | `/purge`, excluded from `/export` by default |

When `BRAIN_AUDIT_RETENTION_DAYS` is set, expired rows are pruned and the hash chain **re-anchored** — the oldest surviving row becomes the new genesis.

## Framework maps

### ISO/IEC 42001 + NIST AI RMF + SOC 2

| Control area | Brain Server evidence | ISO 42001 | NIST AI RMF | SOC 2 |
|---|---|---|---|---|
| Identity & access | JWT/JWS + OIDC/JWKS, opaque bearer, per-route AuthZ matrix (test-pinned) | 8.3 | Govern 1.1 / Map 3 | CC6.1, CC6.6 |
| Change management | semver releases, schema-contract + route-coverage tests, migration rehearsal tool | 9.4 | Measure 4.1 | CC8.1 |
| Monitoring | hash-chained audit + `/metrics` + `/audit/verify` chain-ok | 9.3 | Measure 4.2 | CC7.1–CC7.4 |
| Logging | append-only hash chain; read-event audit (opt-in) | 9.3 | Measure 4.4 | CC7.2 |
| Data lifecycle | proposal gate, decay, DSAR export/purge/certificate, tombstones, retention pruning | 9.2 | Govern 2.1 | CC6.1, CC9.1 |
| System ops | `/health` + capacity envelopes + `bench --envelope` ship gate | 8.4 | Measure 5.1 | CC7.3 |

### Intent-Based Auditing (4/4 pillars)

| Pillar | Evidence |
|---|---|
| Identity | JWT principal (`sub`/`tenant`/`scopes`), OIDC discovery, JWKS |
| Decision-path (memory reads/writes) | Read-event audit + `GET /recall/{trace_id}/trace` replay |
| Policy attribution | AuthZ matrix at handler entry (test-pinned route-by-route) + record-level `access_scope` |
| Selective-forgetting | DSAR locate → export → purge → certificate + tombstone registry |

### Jurisdiction posture

- **Philippines (DPA RA 10173):** personal data stays on the host; the DSAR workflow + deletion certificate implement rights-of-erasure; breach notification is operator-side (the server provides the audit evidence).
- **EU (GDPR):** Art 15/17 (DSAR + erasure certificate), Art 19 (onward-notification webhook), Art 22 (trace replay = meaningful information about the logic), Art 26(6) guidance (retention ≥180 days where required).
- **California (CCPA/CPRA + ADMT):** the Art 22 trace maps to the ADMT right-to-know logic explanation; `/export` is the data-portability response; `/tombstones` evidences completed deletion requests.
- **Residency:** loopback-first means the data physically never leaves the host; cross-border deployment is purely a deployment choice, not an architectural one.
- **EU CRA horizon:** embedded-dependency story documented in `SECURITY.md` (bundled SQLite, cargo-audit gate).

### EU AI Act Art 4 (AI literacy)

The deployer's literacy evidence for the memory component: the dashboard's controls (`/proposals` review queue, `/quarantine`, `/audit`, `/tombstones`) and the trace endpoint make the component's decisions inspectable — *"what informed this retrieval, and who approved this write."*

### Machine-readable origin metadata (Art 50 transparency bridge)

Every `knowledge` row carries an ingest path (`source`: manual | vault | memory | markdown | structured) and, where present, `assertion_kind` (stated/observed/inferred) + `confidence`. `/export` emits these with the content, so a memory export can state *how a memory entered the system and with what provenance*.

## Compliance research & reference links

These are the authoritative sources behind the posture above.

### Regulation (primary texts)

- **EU AI Act** — full regulation, searchable: [artificialintelligenceact.eu](https://artificialintelligenceact.eu/) · official text on [EUR-Lex (Regulation (EU) 2024/1689)](https://eur-lex.europa.eu/eli/reg/2024/1689/oj). Articles referenced: **Art 12** (log recording), **Art 22** (transparency / information about the logic), **Art 26(6)** (retention guidance), **Art 4** (AI literacy), **Art 50** (transparency / origin metadata).
- **GDPR** — official text on [EUR-Lex (Regulation (EU) 2016/679)](https://eur-lex.europa.eu/eli/reg/2016/679/oj). Articles referenced: **Art 15** (access), **Art 17** (erasure), **Art 19** (notification), **Art 22** (automated decision-making).
- **CCPA / CPRA** — the California Privacy Protection Agency: [cppa.ca.gov](https://cppa.ca.gov/). The Automated Decisionmaking Technology (ADMT) regulations live here.
- **Philippines DPA (RA 10173)** — the National Privacy Commission: [privacy.gov.ph](https://www.privacy.gov.ph/).

### Standards & frameworks

- **ISO/IEC 42001** (AI management systems): [iso.org/standard/81230.html](https://www.iso.org/standard/81230.html).
- **NIST AI Risk Management Framework (AI RMF 1.0)**: [nist.gov/itl/ai-risk-management-framework](https://www.nist.gov/itl/ai-risk-management-framework).
- **AICPA SOC 2** (trust services criteria): [aicpa-cima.com SOC 2](https://www.aicpa-cima.com/topic/audit-assurance/audit-and-assurance-greater-than-soc-2).

### Guidance & security bodies

- **EDPB** (European Data Protection Board) — incl. the Coordinated Enforcement Framework on data deletion: [edpb.europa.eu](https://www.edpb.europa.eu/).
- **OWASP Top 10 for LLM Applications** — the source for the LLM01:2025 untrusted-evidence boundary: [owasp.org www-project-top-10-for-large-language-model-applications](https://owasp.org/www-project-top-10-for-large-language-model-applications/).
- **EU Cyber Resilience Act (CRA)** — the embedded-dependency horizon: [digital-strategy.ec.europa.eu CRA](https://digital-strategy.ec.europa.eu/en/library/cyber-resilience-act).
- **W3C WCAG 2.2** — accessibility for the client GUI: [w3.org/WAI/WCAG22](https://www.w3.org/WAI/WCAG22/).
- **RFC 9116** — `security.txt` disclosure: [rfc-editor.org/rfc/rfc9116](https://www.rfc-editor.org/rfc/rfc9116).

## Honest ceilings

- Single-process audit chain (distributed audit = v2.1 multi-instance work).
- Read events default off in loopback mode (personal-use contract); JWT mode on.
- No PII encryption at rest — full-disk encryption is the operator's layer.
- DSAR export is Brain Server JSON, not the UMP wire format.
- This is a documented posture, not a certification.

## Next steps

- **[Security](Security)** — the threat model and controls.
- **[Architecture](Architecture)** — where governance sits in the system.
- The full written contract: `COMPLIANCE.md`, `SECURITY.md`, `THREAT_MODEL.md` in the repository.
