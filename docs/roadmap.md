# Roadmap

Brain Server ships in small, verifiable, named releases. This page summarizes the
journey to the current version and where it is going. The authoritative plan is
[ROADMAP.md](https://github.com/markfietje/brain-server/blob/main/ROADMAP.md);
the full per-version record is
[CHANGELOG.md](./CHANGELOG.md).

---

## Current status

**v1.28.x — the current server line (1.28.34 "Goodwill").** Brain Server ships a
Dioxus GUI (web + desktop + iOS + Android from one Rust codebase) on top of a
mature server. The v1.27 line was the hardening + operator-surface line: fail-
closed erasure + fence-forgeability closes (1.27.19–1.27.21), the backup v3
envelope (1.27.17), per-IP rate limiting + fail-closed identity (1.27.16),
review-armour approvals that bind to the displayed bytes (1.27.12), role-gated
console views + a `--json` CLI envelope (1.27.20), and the i18n truth pass
(1.27.20). The v1.28 line shipped the governed loop from FirstLight through
complaints/aftersales/frontdesk: 1.28.15 "FirstLight" runs the loop for real
over new role-gated routes, 1.28.16 "Anvil" mediates every engine tool-effect
through one auditable hostcall door, 1.28.17 "Settle" turns the engine's
settlement guarantees into contract tests, **1.28.34 "Goodwill"** closes the
ISO 10002/10003 complaint lifecycle on workflow lineage events, and
**1.28.35 "Outreach"** adds consent-first proactive care.

The server core (retrieval, graph, governance) is stable and heavily tested
(800+ tests across the workspace).

---

## The path so far

| Line | Theme | What it delivered |
|---|---|---|
| **v0.9.x** | Foundations | Hybrid retrieval + RRF, Obsidian vault ingest, CommonMark chunker, sources/revisions, structured `QueryDoc`, evidence + provenance, connectors, capacity envelopes, migration rehearsal |
| **v1.0 "Domains"** | Multi-domain | Per-domain knowledge graphs, centroid auto-routing, cross-domain RRF, domain lifecycle |
| **v1.1–v1.2 "Harden + AuthN"** | Security | Audit hash chain, constant-time auth, JWT/JWS + AuthZ layer, OIDC/JWKS, revocation |
| **v1.3 "Bedrock"** | Memory safety | Panic elimination, `unsafe` audit, cargo-fuzz, proptests, configurable worker threads |
| **v1.4 "Calibrate"** | Retrieval quality | Bi-temporal edges, submodular evidence packing, typed-edge graphs, regression harness |
| **v1.5–v1.10** | Cognitive stack | Calibrated abstention, span verification, atomic supersession, faithful explanations, reviewable proposals, opt-in anticipation, ordered procedures |
| **v1.11–v1.12** | Graph retrieval | HippoRAG-2-style Personalized PageRank leg, noise-aware weights + hub dampening + complexity-gated rescue, AuthZ wiring completion |
| **v1.13–v1.14** | Route + Gate | Retrieval routing, write-back gating with human approval, decay, access scopes, PII controls, GDPR export/purge |
| **v1.15 "Observe"** | Compliance | Read-event audit, recall traces, DSAR workflow, deletion certificates, COMPLIANCE.md |
| **v1.16 "Client"** | The GUI | Dioxus control surface — connection machine, review, recall trace, DSAR, audit, security, styled dashboard, mobile-responsive + secure token storage |
| **v1.17 "Govern"** | Governance + UMP | Per-kind retention, Art 30, UMP 1.0 conformance through L3, eval ship-gate + SBOM |
| **v1.18 "Compliant"** | Accessibility | WCAG 2.2 AA + i18n + secret-safe console history + Art 50 origin marker |
| **v1.20 "Polish"** | Client + harden | System-following theme, offline queue, and the GhostJacking-hardening audit tail (SHA-256 digests, read-seam masking, cross-domain DSAR, reviewer calibration, read-path cost + FTS-vocabulary PRF weights) |
| **v1.21–v1.24** | Profiles → Connectors | Preset knob bundles + `brain setup`, legal hold + region + compliance pack, role postures + client-auditor domains, connector registry + translate template |
| **v1.25–v1.27** | PH-Compliant → Cascade | Breach workflow + transfer register + TIA/DPA, fail-closed erasure + fence forgeability, backup v3, console `--json`, and the graph edge-supersession + history fix (1.27.22) |

---

## Where it's going

| Milestone | Theme |
|---|---|
| **v2.0 "Cortex"** | Multi-team tenancy — the first externally-pilotable release (consumes the v1.2 AuthN/AuthZ foundation) |
| **v2.x** | Distributed revocation, limits/regions, federation |
| **v3.x** | Sovereign + survive (resilience), federated deployments |
| **v4.0** | Sovereign standard |

The v1.19–v1.27 intermediate milestones (profiles, regulated modes, roles,
connectors, BPO operations, and the hardening/correctness line through 1.27.22)
are complete. The plan is evidence-gated: work is only shipped
when it is verifiable and earned by a need, not speculation.

---

## Guiding principles

- **Evidence-gated, not roadmap-gated.** Features ship only when they are
  verifiable and justified. Several plan items are explicitly deferred rather than
  shipped for their own sake.
- **Deterministic by default.** No LLM in the retrieval hot path; no surprise
  token cost; no hidden personalization or push.
- **One binary, edge-first.** A single Rust binary with embedded SQLite, bounded
  memory, and no cloud dependency.
- **Honest ceilings.** Every release documents what it does **not** do, so claims
  never outrun implementation.

---

## Next steps

- [Overview](./overview.md) — what Brain Server is and who it is for.
- [API](./api.md) — the endpoint surface available today.
- The authoritative [ROADMAP.md](https://github.com/markfietje/brain-server/blob/main/ROADMAP.md) and [CHANGELOG.md](./CHANGELOG.md).
