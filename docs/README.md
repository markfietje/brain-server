# Brain Server — Documentation

**A local-first semantic-memory and knowledge-graph server for AI agents.** Runs on
a 4 GB ARM device drawing under 5 W — no GPU, no cloud, no per-query cost.

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
| [Research](./research/) | One scientific explainer per retrieval mechanism (reference → implementation → ceiling) |
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
