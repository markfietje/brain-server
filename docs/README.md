# Brain Server — Documentation

**The governed memory layer for AI agents — private, deterministic, and auditable.**

Brain Server is a **local-first semantic-memory and knowledge-graph server** that
gives an AI agent a durable second brain on hardware you own. It pairs a
**deterministic, zero-token retrieval pipeline** (no LLM decides whether to recall,
no embedding API is billed per query) with an **enterprise-grade governance layer**:
human-gated write-back, a tamper-evident append-only audit chain, prompt-injection
quarantine, per-domain knowledge graphs, legal-hold and retention controls, and
GDPR-grade export, purge, and DSAR subject workflows.

It is not "a RAG for a low-power device" — it is a **sovereign memory substrate** that
happens to run on a 4 GB ARM box drawing under 5 W. The same single Rust binary
powering an edge deployment carries the compliance and evidence machinery regulated
organizations (BPOs, finance, healthcare, legal, government) need, so the story you
sell is **privacy, control, and provenance** — not just efficiency.

- **Zero per-query cost** — static local embeddings; no cloud, no GPU, no token spend.
- **Zero data egress** — the agent's memory never leaves your device or datacenter.
- **Deterministic, explainable recall** — hybrid vector + lexical + graph, with
  per-hit provenance you can audit.
- **Human-gated memory** — nothing is written permanently without an operator's
  explicit approval; every decision lands in a hash-chained audit log.
- **Regulatory posture** — ISO 42001 / NIST AI RMF / SOC 2 controls, DSAR, retention,
  jurisdiction, legal holds, and a MemGhost (memory-poisoning) mitigation.

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

Brain Server is built to **stay current with the latest regulation** — the EU AI Act
(including the Art 4 AI-literacy obligation), GDPR Art 17 erasure and Art 12 response
deadlines, the Philippines Data Privacy Act, cross-border transfer rules, and
jurisdiction-aware DSAR handling. Compliance is enforcement, not documentation: legal
holds, retention windows, and per-jurisdiction deadlines are actual behaviors the
software applies, backed by a verifiable audit chain.

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
