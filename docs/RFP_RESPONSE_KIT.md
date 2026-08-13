# RFP Response Kit — brain-server

**Applies to:** brain-server 1.20.25 · **Last updated:** 2026-08-13

A two-to-three page map from common enterprise RFP sections to the concrete
brain-server features that satisfy them, so a procurement response can cite
evidence instead of promises. Every claim below links to a real control,
route, or test in this repository. It is a *pointer* document: the technical
file (`COMPLIANCE.md`), threat model (`THREAT_MODEL.md`), security map
(`SECURITY.md`), SBOM (`cargo audit` / `Cargo.lock`), and audit chain
(`/audit/verify`) are the evidence base that backs each line.

> **How to use.** For each RFP section, take the mapped rows, verify the
> route is live (`curl http://127.0.0.1:8765/...`), and attach the named
> artifact. Do not copy claims you have not verified on your own deployment —
> the point of the kit is truthful, evidence-backed answers.

## 1. Security & Access Control

| RFP ask | brain-server answer | Evidence |
|---|---|---|
| Authentication | Opaque bearer token, or enterprise JWT/JWS + OIDC discovery + JWKS (`/.well-known/openid-configuration`, `/.well-known/jwks.json`) | `SECURITY.md`, v1.2 release |
| Authorization | Route-by-route AuthZ matrix enforced at handler entry, test-pinned; record-level `access_scope` deny-by-default filter in JWT mode | v1.12.1, `COMPLIANCE.md` §6.1 |
| Vulnerability management | `cargo audit` gate (0 vulnerabilities), bundled SQLite 3.53.2, semver releases | CI, `SECURITY.md`, v1.12.2 |
| Memory safety | Zero panics in production paths, `unsafe` blocks documented + counted in `/health`, fuzz + proptest suites | v1.3.0 "Bedrock" |
| Data residency | Loopback-first, single-host SQLite; data physically never leaves the host unless the operator chooses to | `COMPLIANCE.md` §1, §6.3 |

## 2. Privacy, Data Protection & Rights

| RFP ask | brain-server answer | Evidence |
|---|---|---|
| DSAR / right to erasure | Locate → export → purge → **deletion certificate** + tombstone registry (`/dsar`, `/tombstones`) | `COMPLIANCE.md` §4 |
| Right to explanation | Replayable recall trace (`GET /recall/{trace_id}/trace`) = Art 22 "meaningful information about the logic" | `COMPLIANCE.md` §3, §6.3 |
| Data portability | `/export` emits content + provenance (`source`/`assertion_kind`/`confidence`) | `COMPLIANCE.md` §7 |
| PII handling | Deterministic read-time output redaction (masked for principals without `pii:read`); no plaintext stored in a placeholder vault | v1.14, v1.20.19, `COMPLIANCE.md` §2 |
| Onward notification | Opt-in Art 19 HMAC-SHA256-signed webhook on purge | `COMPLIANCE.md` §4, v1.15 |
| Audit trail | Append-only SHA-256 hash chain, `/audit/verify`, `/metrics` chain-ok gauge | `COMPLIANCE.md` §3 |

## 3. AI Governance, Transparency & Safety

| RFP ask | brain-server answer | Evidence |
|---|---|---|
| Human-in-the-loop | Proposal gate: ingestion scores but **writes nothing until a human approves** (`/proposals`) | v1.14, `COMPLIANCE.md` §6.1 |
| Memory poisoning / prompt-injection defense | Quarantine + flagged-row exclusion, HITL gate, MemGhost mitigation | `docs/MEMGHOST_MITIGATION.md` |
| Origin transparency (Art 50) | Machine-readable `/.well-known/ai-notice` + per-row provenance | `COMPLIANCE.md` §7 |
| AI literacy (Art 4) | Operator playbook + inspectable dashboard/trace/DSAR controls | `docs/AI_LITERACY.md` |
| Explainable retrieval | Per-result provenance (vector/lexical/graph ranks, fused score) + trace replay | `/recall` provenance, v0.9.5/v1.15 |
| Calibrated abstention | Deterministic low-confidence abstention + `/verify` span check (no fabricated top-1) | v1.5.0, `docs/api.md` |
| Selective repair | Supersede/undo + near-duplicate + stale-source review, all operator-driven | v1.6/v1.8, MemSecBench "selective repair" lane |

## 4. Operational Maturity

| RFP ask | brain-server answer | Evidence |
|---|---|---|
| Observability | `/health` (incl. hardening + capacity), `/metrics`, structured audit | `COMPLIANCE.md` §6.1 |
| Capacity / performance | Capacity envelopes (`/health`), `bench --envelope` ship gate | v0.9.9, `BENCHMARKS.md` |
| Disaster recovery | Pre-migration `VACUUM INTO` snapshots (chmod 0600), import/export, migration rehearsal tool | `docs/deployment.md`, v1.16.7 |
| Documentation | Wiki (22 pages) + `docs/` (public) + engineering docs (technical file, spec, contract) | `README.md` §Docs |

## 4.5 Competitive positioning — governance over leaderboard

> Use this when an RFP asks "how does your recall accuracy compare?" or a
evaluator quotes a competitor's LongMemEval/LoCoMo percentage. Do not
one-up the number; reframe the metric. This is the section that turns a
benchmark question into a production-readiness answer.

**The reframe (backed by a third party, not by us):** published agent-memory
benchmark scores (LongMemEval, LoCoMo) are recall benchmarks. They are
necessary but not sufficient for production. A 2026 independent critique of
the published numbers (memnode.dev) states that none of the leading scores
measure: **memory drift** when stored facts contradict later, **memory-
poisoning resistance**, **per-tenant isolation** under a shared backend,
**cost per recall** at production volume, or behavior at hundreds/thousands
of sessions — and that a 95% LongMemEval score does not mean the system is
operationally ready for multi-tenant production.

**Vendor-vs-independent inflation is real and visible:** a vendor's own
benchmark page (mem0.ai/research, 7 Aug 2026) reports 92.5 on LoCoMo / 94.4
on LongMemEval for its **managed platform**. That same vendor's own open,
reproducible paper lists **66.9%** on LoCoMo (arXiv:2504.19413), an
independent third-party table agrees (66.9%), and independent re-runs land
at 58–66% — a ~26-point gap between the marketing headline and what is
reproducible. When a vendor's headline and its reproducible number disagree,
only the reproducible number is trustworthy.

**How brain-server answers the recall question honestly:** we target the
production metrics the recall benchmarks leave out, and we ship the evidence
for them in-repo rather than as a one-line percentage:

| Production metric (what recall % misses) | brain-server evidence |
|---|---|
| Memory-poisoning resistance | Quarantine + flagged-row exclusion + HITL write gate | `docs/MEMGHOST_MITIGATION.md`, `COMPLIANCE.md` §6.1, OWASP ASI06 map |
| Per-tenant isolation | Record-level `access_scope` deny-by-default filter in JWT mode | `COMPLIANCE.md` §6.1, v1.12.1 |
| Memory drift / contradiction | Supersede + stale-source review, operator-driven | v1.6/v1.8, MemSecBench "selective repair" lane |
| Cost per recall | Zero-token local embeddings (minishlab/potion-32M), no inference on hot path | `README.md` §Architecture |
| At-scale behavior (100s–1000s sessions) | Capacity envelopes in `/health`; bench scaffold with frozen judged corpus | `BENCHMARKS.md` |
| Verifiability of the claim | Append-only SHA-256 audit chain, `/audit/verify` | `COMPLIANCE.md` §3 |

**One-liner for the written response:** "Raw recall % measures retrieval on a
curated test set; it does not measure poisoning resistance, drift, tenant
isolation, or cost. Those are the metrics that decide whether a memory system
survives production, and they are the metrics we publish evidence for."

## Honest ceilings (state these in your response)

- **Not a certification.** ISO/IEC 42001 / SOC 2 attestation are
  organization-level audits outside this repository — this is a documented
  engineering posture, not a certificate.
- **Single-process audit chain** (distributed audit is v2.1).
- **PII at rest is not encrypted** — full-disk encryption is the operator's
  layer (LUKS/FileVault), documented in `COMPLIANCE.md`.
- **Deterministic, not learned**: redaction is pattern-match, recall is
  heuristic + deterministic, no model inference on the hot path.
