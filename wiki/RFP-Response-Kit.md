# RFP Response Kit

**Applies to:** Brain Server 1.16.7 · **Last updated:** 2026-08-08

A map from common enterprise RFP sections to the concrete Brain Server features that satisfy them, so a procurement response can cite **evidence instead of promises**. Every claim below points at a real control, route, or test. It is a *pointer* document — the technical file (`COMPLIANCE.md`), threat model, security map, SBOM (`cargo audit` / `Cargo.lock`), and audit chain (`/audit/verify`) are the evidence base behind each line.

> **How to use.** For each RFP section, take the mapped rows, verify the route is live (`curl http://127.0.0.1:8765/...`), and attach the named artifact. Do not copy claims you have not verified on your own deployment — the point of the kit is truthful, evidence-backed answers.

## 1. Security & Access Control

| RFP ask | Brain Server answer | Evidence |
|---|---|---|
| Authentication | Opaque bearer token, or enterprise JWT/JWS + OIDC discovery + JWKS (`/.well-known/openid-configuration`, `/.well-known/jwks.json`) | [Security](Security) page |
| Authorization | Route-by-route AuthZ matrix enforced at handler entry, test-pinned; record-level `access_scope` deny-by-default filter in JWT mode | [Governance & Compliance](Governance-and-Compliance) |
| Vulnerability management | `cargo audit` gate (0 vulnerabilities), bundled SQLite 3.53.2, semver releases | CI, [Security](Security) |
| Memory safety | Zero panics in production paths, `unsafe` blocks documented + counted in `/health`, fuzz + proptest suites | v1.3.0 "Bedrock" |
| Data residency | Loopback-first, single-host SQLite; data physically never leaves the host unless the operator chooses to | [Governance & Compliance](Governance-and-Compliance) |

## 2. Privacy, Data Protection & Rights

| RFP ask | Brain Server answer | Evidence |
|---|---|---|
| DSAR / right to erasure | Locate → export → purge → **deletion certificate** + tombstone registry (`/dsar`, `/tombstones`) | [Governance & Compliance](Governance-and-Compliance) |
| Right to explanation | Replayable recall trace (`GET /recall/{trace_id}/trace`) = Art 22 "meaningful information about the logic" | [Governance & Compliance](Governance-and-Compliance) |
| Data portability | `/export` emits content + provenance (`source` / `assertion_kind` / `confidence`) | [Governance & Compliance](Governance-and-Compliance) |
| PII handling | Opt-in write-time placeholder redaction (`BRAIN_REDACT_PII=1`), conservative pattern match; read redaction on output | v1.14 |
| Onward notification | Opt-in Art 19 HMAC-SHA256-signed webhook on purge | [Governance & Compliance](Governance-and-Compliance) |
| Audit trail | Append-only SHA-256 hash chain, `/audit/verify`, `/metrics` chain-ok gauge | [Governance & Compliance](Governance-and-Compliance) |

## 3. AI Governance, Transparency & Safety

| RFP ask | Brain Server answer | Evidence |
|---|---|---|
| Human-in-the-loop | Proposal gate: ingestion scores but **writes nothing until a human approves** (`/proposals`) | v1.14, [Governance & Compliance](Governance-and-Compliance) |
| Memory poisoning / prompt-injection defense | Quarantine + flagged-row exclusion, HITL gate, MemGhost mitigation | [MemGhost Mitigation](MemGhost-Mitigation) |
| Origin transparency (Art 50) | Machine-readable `/.well-known/ai-notice` + per-row provenance | [Governance & Compliance](Governance-and-Compliance) |
| AI literacy (Art 4) | Operator playbook + inspectable dashboard/trace/DSAR controls | [AI Literacy](AI-Literacy) |
| Explainable retrieval | Per-result provenance (vector/lexical/graph ranks, fused score) + trace replay | `/recall` provenance |
| Calibrated abstention | Deterministic low-confidence abstention + `/verify` span check (no fabricated top-1) | v1.5.0 |
| Selective repair | Supersede/undo + near-duplicate + stale-source review, all operator-driven | v1.6 / v1.8 |

## 4. Operational Maturity

| RFP ask | Brain Server answer | Evidence |
|---|---|---|
| Observability | `/health` (incl. hardening + capacity), `/metrics`, structured audit | [Governance & Compliance](Governance-and-Compliance) |
| Capacity / performance | Capacity envelopes (`/health`), `bench --envelope` ship gate | v0.9.9 |
| Disaster recovery | Pre-migration `VACUUM INTO` snapshots (chmod 0600), import/export, migration rehearsal tool | [Deployment](Deployment) |
| Documentation | This wiki + `docs/` + engineering docs (technical file, spec, contract) | [Home](Home) |

## Honest ceilings (state these in your response)

- **Not a certification.** ISO/IEC 42001 / SOC 2 attestation are organization-level audits outside this repository — this is a documented engineering posture, not a certificate.
- **Single-process audit chain** (distributed audit is v2.1).
- **PII at rest is not encrypted** — full-disk encryption is the operator's layer (LUKS/FileVault).
- **Deterministic, not learned:** redaction is pattern-match, recall is heuristic + deterministic, no model inference on the hot path.
