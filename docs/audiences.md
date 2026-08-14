# Who it's for — target audiences

Brain Server is a **local-first, offline, deterministic semantic-memory and
knowledge-graph server for AI agents**. This page maps the product's shipped
capabilities to the **concrete customer segments** it serves, so the right
audience can find the right answer. It is written the same way the product is
built: every claim is either **shipped** (real control, route, or test in this
repo) or explicitly **planned** (a documented roadmap ceiling) — never a
promise dressed as a feature.

> **How to read the map.** Each row names who you are, what you need, what
> Brain Server gives you, and whether it's shipped today or queued in the
> roadmap. The **[Proof map](./trust/proof-map.md)** and
> **[RFP kit](./RFP_RESPONSE_KIT.md)** cite the specific control behind each
> shipped claim.

---

## The honest frame first

The shipped product is a **single-node, loopback-first** memory server. It has
**per-domain** isolation, per-tenant **audit**, **DSAR**, PII redaction, and a
human write-gate today. What it does **not** have yet is **multi-team tenancy**
— running several client accounts as isolated tenants on one shared backend.
That is the documented **v2.0 "Cortex"** milestone (call-center intelligence),
so the *BPO* and *multi-client contact-center* rows below are the roadmap the
product is building toward, not the current single-node form. Buyers should
read each row's "Status" column.

---

## 1. Customer-support & contact-center operations

The product's v2.0 "Cortex" milestone is explicitly **call-center
intelligence**: multi-team tenancy (consuming the shipped v1.2 AuthZ),
ticket-pattern knowledge, resolution reinforcement, and cross-domain skill
seeding. The controls those teams need are largely shipped *today* (isolation,
audit, DSAR, PII, human-gated writes); the shared-tenant packaging is the
planned part.

| Segment | What they need | Brain Server | Status |
|---|---|---|---|
| **BPOs (Business Process Outsourcers)** | Serve multiple client accounts on one platform with hard isolation; agent-assist memory per client; audit + DSAR per client; PII containment | Per-domain isolation, per-tenant audit chain, DSAR + deletion certificates, PII redaction, human-gated writes | Controls shipped; **multi-client tenancy = v2.0 Cortex** (planned) |
| **In-house contact / call centers** | One org, many teams; an agent memory that recalls past resolutions, policies, and customer context; supervision + audit | Deterministic recall, knowledge graphs, temporal evidence, HITL write gate, audit chain, calibration strip | Shipped (single-org form); multi-team packaging in v2.0 |
| **Customer-support teams & helpdesk** | Faster, grounded answers; "how did we resolve this before?" memory; no fabricated answers | Calibrated abstention, span verification, recall traces, resolution KG | Shipped |
| **Managed-service / shared-services support** | Standardized knowledge across internal teams with per-team scope | Domains + centroid routing, per-agent opt-in, chat-type gating | Shipped |

## 2. Enterprise & regulated industries (sovereignty)

Because Brain Server is **self-hosted, offline-capable, and audited**, it fits
organizations for whom memory *must* stay in-house and be provable.

| Segment | What they need | Brain Server | Status |
|---|---|---|---|
| **Financial services** | PII containment, immutable audit, DSAR (GDPR/CCPA), no data egress | SHA-256 audit chain, read-time PII redaction, DSAR/certificates, loopback-only default | Shipped |
| **Healthcare & clinical** | Local records, on-prem, explainable recall, erasure | Local-first, `/verify` span check, DSAR, `/.well-known/ai-notice` | Shipped |
| **Legal & compliance** | Tamper-evident logs, Art 22 explainability, Art 50 origin | Hash chain + `/audit/verify`, replayable recall traces, origin metadata | Shipped |
| **Government / public sector** | Air-gapped or on-prem, procurement-grade evidence | Single binary, no telemetry, `RFP_RESPONSE_KIT.md`, threat model | Shipped |
| **Any regulated enterprise** | SOC 2 / ISO 42001 evidence base | Documented posture + evidence kit (see `COMPLIANCE.md`) | Shipped (posture, not certification) |

## 3. AI / agent builders & platforms

The current primary audience — teams and individuals building agents that need
memory.

| Segment | What they need | Brain Server | Status |
|---|---|---|---|
| **OpenClaw users** | Deterministic memory in the memory slot, zero token cost | Native `kind: "memory"` plugin (autoRecall/autoCapture/Proposal) | Shipped |
| **Agent / LLM developers** | A self-hosted memory store with standard contracts | Open HTTP, MCP binary, OpenAPI, **UMP 1.0 L3** | Shipped |
| **Edge / privacy-first agent builders** | Memory on-device, no embedding API | Local static `model2vec`, offline, bounded RSS (default 512 MiB) | Shipped |
| **Agent platforms & ISVs** | A memory backend to embed without lock-in | Standard-based (UMP, MCP, open HTTP), self-hostable | Shipped |

## 4. Edge, field & hardware deployments

| Segment | What they need | Brain Server | Status |
|---|---|---|---|
| **Retail / logistics field ops** | Offline memory on thin hardware | Single binary, <5 W, Jetson/Raspberry Pi | Shipped |
| **Industrial / remote / air-gapped sites** | No cloud dependency, deterministic | Local static embeddings, no data egress | Shipped |

## 5. Knowledge workers & individuals

| Segment | What they need | Brain Server | Status |
|---|---|---|---|
| **Personal-knowledge / PKM users** | A private second brain, temporal recall | Domains (health/business/code), point-in-time recall | Shipped |
| **Researchers & academics** | A reproducible memory/RAG substrate | Open source, benchmark harness, frozen judged corpus | Shipped |

## 6. Ecosystem & delivery partners

| Segment | What they need | Brain Server | Status |
|---|---|---|---|
| **SIs / MSPs / consultants** | A deployable, auditable memory layer to stand up for clients | One binary, edge-ready, documented deployment + DSAR drills | Shipped |
| **Platform / tooling vendors** | An embeddable, standard memory contract | UMP 1.0 L3, MCP, OpenAPI | Shipped |

---

## The honest ceiling (state this in any pitch)

- **Multi-client / multi-team tenancy is v2.0 "Cortex", not today.** A BPO
  running several client accounts as isolated tenants on one shared backend
  gets the *controls* (isolation, audit, DSAR, PII) shipped now, but the
  shared-tenant packaging and per-tenant limits are the documented v2.0/v2.1
  roadmap. Until then, per-client isolation is per-domain on separate nodes.
- **Not a certification.** SOC 2 / ISO 42001 attestation are organization-level
  audits outside this repo; `COMPLIANCE.md` is a documented engineering posture.
- **PII at rest is not encrypted** — full-disk encryption is the operator's
  layer (LUKS/FileVault).
- **Deterministic, not learned** — recall and redaction are heuristic/
  deterministic, not model-inference.

---

## Next steps

- **[Overview](./overview.md)** — what it is and the five differentiators.
- **[Use cases](./use-cases.md)** — worked technical scenarios.
- **[RFP Response Kit](./RFP_RESPONSE_KIT.md)** — evidence-backed answers for procurement.
- **[Media kit](./media-kit.md)** — positioning + one-liners for press/marketing.
- **[Human in the loop](./human-in-the-loop.md)** — the operator's field manual, incl. **§7 the erasure procedure** (the documented, audited path a BPO/QA/Admin follows to delete memory).
- **[Roadmap](./roadmap.md)** — the v2.0 "Cortex" trajectory this map points at.
