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
| [Deployment](./deployment.md) | Service install, configuration, backup/restore, operational health |
| [Security](./security.md) | Threat model, authentication modes, and the controls that protect data |
| [Compliance](./compliance.md) | ISO 42001 / NIST AI RMF / SOC 2 posture, DSAR, retention, jurisdiction |
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
