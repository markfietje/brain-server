# Brain Server — Documentation

**The governed memory layer for AI agents — deployed on your infrastructure, audited to the letter.**

Brain Server is a **self-hosted semantic-memory and knowledge-graph engine** that gives
your AI agents a durable, deterministic second brain — **entirely on hardware your
organization controls**. Where every other memory framework puts an LLM or an embedding
API between you and every read and write (metered per query, egressing your data to a
vendor's datacenter), Brain Server does recall with **zero token cost, zero data egress,
and zero network latency** — while wrapping that retrieval in a governance layer built
for regulated production: human-gated write-back, a tamper-evident append-only audit
chain, prompt-injection quarantine, per-domain knowledge graphs, legal-hold and
retention controls, and GDPR-grade export, purge, and DSAR subject workflows.

This is not a toy or a "local RAG." It is the compliance-grade memory substrate that
enterprises — **BPOs, in-house contact and support centers, healthcare providers,
financial institutions, legal, and government** — deploy when memory *must* be private,
explainable, and provably audited. Backed by an **Enterprise edition** that meets
procurement where it lives: enterprise JWT/JWS authentication with OIDC discovery and
JWKS, deny-by-default authorization, per-tenant capability tokens, OTel observability,
and a SOC 2 evidence kit with contract-level support. See **[Editions](./product-site/editions.md)**.

- **Enterprise-authenticated, least-privilege, by default** — enterprise JWT/JWS +
  OIDC/JWKS, deny-by-default multi-role authorization, per-tenant capability tokens.
- **Zero per-query cost** — static local embeddings; no cloud, no GPU, no token spend.
- **Zero data egress** — the agent's memory never leaves your tenant boundary.
- **Deterministic, explainable recall** — hybrid vector + lexical + graph, with
  per-hit provenance and a replayable trace for every retrieval.
- **Human-gated memory** — nothing enters permanent memory without an operator's
  explicit approval; every decision lands in a hash-chained audit log.
- **Regulatory posture** — ISO 42001 / NIST AI RMF / SOC 2, HIPAA, GDPR & EU AI Act,
  DSAR, retention, jurisdiction, legal holds, and a MemGhost (memory-poisoning)
  mitigation. Compliance is shipped behavior, not a brochure.

## Retrieval models

Recall runs on **local embeddings with zero token cost** — nothing is sent to an
embedding API in any profile. By default (`MODEL_PROFILE=edge-default`) that's the
static `minishlab/potion-retrieval-32M` model (512-d, `model2vec`, no transformer
forward pass — ideal for Jetson/RPi/edge). Opt-in retrieval profiles swap in larger
**local** models without changing the API:

| Profile | Embedding model | Dim |
|---|---|---|
| `enterprise` | `BAAI/bge-m3` | 1024 |
| `desktop` | `Alibaba-NLP/gte-base-en-v1.5` | 768 |
| `compact` (was `multilingual`) | `minishlab/potion-base-2M` | 512 |

> The old `multilingual` label was wrong — `potion-base-2M` is an **English**
> model (distilled from `BAAI/bge-base-en-v1.5`), not multilingual. Renamed to
> **`compact`** (the smallest, fastest static model); `MODEL_PROFILE=multilingual`
> still resolves to the same profile for backward compatibility.

An optional cross-encoder **rerank tier** (armed on `enterprise` / `desktop` /
`quality-local`) refines the fused order with `mixedbread-ai/mxbai-rerank-large-v1`
(fallback `BAAI/bge-reranker-v2-m3`). All models run locally — no cloud, no GPU
required, no token spend. See [Configuration](./configuration.md) for the full
profile matrix and the `BRAIN_RERANK_*` variables.

## Minimum hardware requirements

The retrieval profile you pick drives the hardware you need. **compact** uses a
static 512-d model (no transformer forward pass — it will run on a Raspberry Pi
or a Jetson); **desktop** and **enterprise** load a neural embedding model via
ONNX (FastEmbed), which needs real RAM and CPU. All figures are honest
**minimums for a single host running the server only**, and already include
headroom for the operating system, your agent application, and background
services — not a bare-bones, swap-thrashing floor. They assume a modern 64-bit
CPU (ARM64 or x86_64) with no GPU anywhere in the path.

| | `compact` (edge) | `desktop` | `enterprise` |
|---|---|---|---|
| Embedding model | `minishlab/potion-base-2M` (512-d, static) | `Alibaba-NLP/gte-base-en-v1.5` (768-d, ONNX) | `BAAI/bge-m3` (1024-d, ONNX) |
| RAM | **2 GB** | **8 GB** | **16 GB** |
| CPU | 2 cores | 4 cores | 8 cores |
| Free disk (server + DB + model cache) | **4 GB** | **8 GB** | **12 GB** |
| Device example | Raspberry Pi 4 / Jetson Nano | x86_64 mini-PC or Mac | server-class x86_64 / Mac |
| Typical process RSS | ~200 MB | ~0.8–1 GB | ~1 GB |
| OS headroom (included above) | Linux on 4 GB is comfortable | comfortable | comfortable |

Why the jumps look large next to the modest RSS figures: the ONNX embedder
**warms up its working set at boot** (never in the request path), and the
measured RSS is the server process alone. Add the OS, an agent process that
queries it, and occasional embedding bursts, and the real-world floor is what
the table states. On constrained ARM edge hardware, set
`BRAIN_WORKER_THREADS=2` and the RSS ceiling is bounded (`CAPACITY_MAX_RSS_MIB`,
default 512 MiB on a 4 GB device). See [Deployment — edge](./deployment.md#edge-deployment-jetson-nano--raspberry-pi)
and [Configuration](./configuration.md) for the knobs.

## Who it is for

- **Anyone who wants their agent's memory private** — your conversation history and
  working knowledge stay on your own device, never in a vendor's datacenter.
- **Knowledge workers** — health, business, code, and more kept as separate brains
  (domains) that cross-reference on a miss.
- **Healthcare professionals & hospitals** — patient-adjacent working memory under
  strict access, retention, and audit control.
- **Contact / call centers & BPOs** — governed, domain-scoped agent memory with a
  reviewer in the loop so nothing is written without human approval.

## Law-following by design

Brain Server is built to **stay current with the latest regulation**. It turns
compliance into shipped behavior — not a brochure: a jurisdiction table computes
data-subject response deadlines, legal holds freeze records against every erasure
path, retention windows are applied per domain and kind, cross-border transfer
mechanisms are validated at registration, and every write, approval, and erasure
lands on a tamper-evident SHA-256 audit chain you can verify. The inventory below
groups the instruments by region and sector; the full row-by-row control map lives
in [COMPLIANCE.md](https://github.com/markfietje/brain-server/blob/main/COMPLIANCE.md)
and [COMPLIANCE_PH.md](https://github.com/markfietje/brain-server/blob/main/COMPLIANCE_PH.md).
**This is a documented engineering posture, not a certification** — ISO/IEC 42001 and
SOC 2 attestation are organization-level audits outside this repository.

### Europe
- **EU AI Act — Regulation (EU) 2024/1689.** Art 4 AI-literacy playbook
  (`docs/AI_LITERACY.md`, served at `/.well-known/ai-literacy`); Art 12 / Art 26(6)
  logging posture with a configurable retention window (deployers set ≥180 days);
  Art 22 meaningful-information trace replay (`/recall/{id}/trace`); Art 50 model-vs-human
  provenance + machine-readable `/.well-known/ai-notice`. GPAI obligations now fully
  enforceable from **2 Aug 2026** (Regulation (EU) 2026/1744); penalty tiers tracked
  exactly — Art 99(2) €35M/7% for prohibited practices & GPAI provider duties, Art 99(3)
  €15M/3% for the Art 50 transparency line, up to €7.5M/1% for general infractions.
- **GDPR — Regulation (EU) 2016/679.** Art 15 access, Art 17 erasure (the DSAR
  locate→export→purge path, human-executed, audited), Art 19 onward-notification
  (HMAC-signed webhook), Art 12 response deadline clock, Art 22 logic-explanation
  trace, Art 26(6) retention guidance, Art 30 register (`GET /art30`), Art 28 DPA.
- **EU Standard Contractual Clauses 2021** and **EU-U.S. Data Privacy Framework**
  (adequacy, live since 10 Jul 2023) — both mechanisms in the validated transfer register.

### UK
- **UK GDPR** + **ICO International Data Transfer Agreement (IDTA) / Addendum** —
  the UK's standard clauses, distinct from the EU SCCs and treated independently
  (the UK's DPF adequacy extension is a separate instrument from the EU's).

### United States
- **California Consumer Privacy Act / California Privacy Rights Act (CCPA/CPRA) +
  California's Automated Decision-Making Technology Regulation (ADMT).** Data portability
  (`/export`), erasure, and a logic-explanation trace that folds into the
  right-to-know / ADMT disclosure expectations.
- **HIPAA (45 CFR Part 164)** — Security Rule + §164.502(g). Access + audit + integrity
  + minimum-necessary controls, PHI tokenization via strict-mode masking, legal
  hold for litigation/breach deferral, and storage-limitation reporting.
- **SOX (17 CFR §229 / PCAOB AS 2201).** Immutable audit trail, supersede-not-delete,
  records preservation, and erasure refusal under legal hold.

### Philippines (home jurisdiction)
- **RA 10173 — Data Privacy Act of 2012** + **NPC advisories** (2024-04 AI; 2026-01
  data scraping) + **EO 119** (2026, government-data residency). Data subject rights
  through the DSAR surface, 72-hour breach-notification workflow (DPO-gated), lawful-basis
  provenance for scraped data, and a pre-filled PIA template. **HB 7396** (a risk-based
  AI bill) is pending, not enacted — the profile/retention/role primitives are structured
  to absorb it, with no pre-implementation.

### APAC (cross-border register + provenance)
- **Singapore — Personal Data Protection Act 2012** (incl. the 2026 Amendment
  Regulations aligning APEC CBPR / Global CBPR cross-border systems).
- **Australia — Privacy Act 1988 / Australian Privacy Principles** (incl. the
  automated-decision transparency obligations starting 10 Dec 2026) and the
  **Japan — Act on the Protection of Personal Information (APPI)**, both surfaced
  through jurisdiction-aware DSAR handling, plus the cross-border transfer register
  (`scc-eu-2021`, `uk-idta`, `dpf-us`, `cbpr`, `bcr`, `adequacy`).

### Sector & frameworks the buyer will ask about
- **FedRAMP / FISMA (NIST 800-53 control posture)** — AC, AU, SC-7/SC-28, SI-12, and
  IR families mapped to shipped evidence.
- **ISO/IEC 42001, NIST AI RMF, SOC 2** — documented control-by-control posture across
  identity, change management, monitoring, logging, and data lifecycle.
- **EU Cyber Resilience Act (Art 13/14)** — a CycloneDX SBOM ships with every release
  for supply-chain evidence.
- **OWASP ASI06 (Memory & Context Poisoning)** — provenance at write time, a human
  approval gate, hash-chained memory-change audit, and a tombstone path — the controls
  the MemGhost / GhostWriter disclosures found missing.

Compliance is enforced, not documented: the same single binary that serves recall
applies legal holds, retention windows, region residency stamps, and jurisdiction-aware
deadlines — all of it auditable. For the honest ceilings (single-node audit chain, no
PII-at-rest encryption without operator full-disk encryption, posture-not-certification),
see [COMPLIANCE.md](https://github.com/markfietje/brain-server/blob/main/COMPLIANCE.md).

This directory is the public, informational documentation for Brain Server. For
the technical contract and engineering records, see the linked files in the repo
root.

---

## Documentation map

| Document | What it is |
|---|---|
| [Overview](./overview.md) | What Brain Server is, who it is for, and the five differentiators |
| [Quickstart](./quickstart.md) | Build, run, and make your first recall in minutes |
| [Architecture](./architecture.md) | How recall, ingest, the knowledge graph, and governance fit together |
| [Human in the loop](./human-in-the-loop.md) | Meaningful human control: what reaches a human, and how to evaluate it |
| [Deployment](./deployment.md) | Service install, configuration, backup/restore, operational health |
| [Docker](./docker.md) | Container image, compose, offline model bake, container ops |
| [Proxy SSO](./proxy-sso.md) | Reverse-proxy SSO (OAuth2-Proxy / Caddy / Authentik) in front of the server |
| [Security](./security.md) | Threat model, authentication modes, and the controls that protect data |
| [MemGhost mitigation](./MEMGHOST_MITIGATION.md) | How brain-server neutralizes the memory-poisoning attack (arXiv 2607.05189) |
| [AI literacy (Art 4)](./AI_LITERACY.md) | Operator playbook for the EU AI Act Art 4 literacy obligation |
| [RFP response kit](./RFP_RESPONSE_KIT.md) | Map brain-server features to common enterprise RFP sections |
| [Compliance](./compliance.md) | ISO 42001 / NIST AI RMF / SOC 2 posture, DSAR, retention, jurisdiction |
| [Product site](./product-site/index.md) | Buyer-facing landing, install, quickstart, editions |
| [Research](./research/index.md) | One scientific explainer per retrieval mechanism (reference → implementation → ceiling) |
| [Blog](./blog/index.md) | One technical-buyer post per hard-won mechanism, each tied to its research/trust source |
| [Media kit](./media-kit.md) | Positioning, one-liners, and a Brain-vs-Mem0/LangGraph/RAG sizing table with honest ceilings |
| [Trust / proof map](./trust/proof-map.md) | Every security/compliance claim → shipped release → live `curl`/`brain` proof |
| [API](./api.md) | Endpoint reference and links to the full contract |
| [Roadmap](./roadmap.md) | The shipped release history and the path forward |

---

## Linked engineering documents (repo root)

These are the source-of-truth technical records referenced throughout this guide:

- **README** — quick start, feature overview, endpoint table, CLI, configuration.
- **API_CONTRACT.md** — the versioned HTTP contract, query semantics, error codes.
- **openapi.yaml** — the machine-readable OpenAPI 3.0 contract (`GET /openapi.yaml` at runtime).
- **SPECS.md** — the technical specification.
- **SECURITY.md** / **THREAT_MODEL.md** — security posture and threat analysis.
- **COMPLIANCE.md** — compliance mapping and governance controls.
- **BENCHMARKS.md** — measured latency / recall / RSS figures.
- **ROADMAP.md** — the full release chain and plan.
- **CHANGELOG.md** — per-version release notes.
