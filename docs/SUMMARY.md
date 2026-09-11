# brain-server

- [About brain-server](README.md)

# Getting started

- [Quickstart](quickstart.md)
- [Installation & configuration](deployment.md)
- [Docker deployment](docker.md)
- [Reverse-proxy SSO](proxy-sso.md)

# Core concepts

- [Overview](overview.md)
- [Who it's for — target audiences](audiences.md)
- [One brain for the whole team](team-workflow.md)
- [Human in the loop](human-in-the-loop.md)
- [The memory lifecycle](memory-lifecycle.md)
- [Architecture](architecture.md)
- [Connectors](connectors.md)
- [Custom CRM connectors](connector-crm-custom.md)

# Reference

- [API reference](api.md)
- [API contract](API_CONTRACT.md)
- [Features](features.md)
- [Use cases](use-cases.md)
- [Procedures & runbooks](runbooks.md)
- [Configuration](configuration.md)
- [Retrieval & recall](retrieval-and-recall.md)
- [Knowledge graph](knowledge-graph.md)
- [CLI reference](cli-reference.md)
- [Metrics dictionary](metrics.md)
- [KB deflection loop](kb-deflection.md)
- [Keystone worked example](keystone-worked-example.md)
- [Engine SDK](engine-sdk.md)
- [OpenClaw integration](openclaw-integration.md)
- [MCP server](mcp.md)
- [Client GUI](client-gui.md)
- [Client console](client-complete-console.md)
- [Dioxus WASM-split research](dioxus-wasm-split-research.md)
- [WFM seam (workload, coverage, shifts)](wfm-seam.md)
- [Universal Memory Protocol](universal-memory-protocol.md)
- [Technical specification](SPECS.md)
- [Glossary](glossary.md)
- [FAQ](faq.md)

# Operations

- [Security policy](security.md)
- [Threat model](THREAT_MODEL.md)
- [Compliance posture](compliance.md)
- [Warm standby](standby.md)
- [Valet reminders](valet.md)
- [Signed parcels](parcels.md)
- [Principal kill-switch](kill-switch.md)
- [Records pack (Art30/RoPA/breach/transfers)](records-pack.md)
- [Observability](observability.md)

# Trust & verification

- [Proof map](./trust/proof-map.md)
- [Reproduce on a throwaway instance](./trust/reproduce.md)
- [WCAG 2.2 AA checklist](./trust/wcag22-aa-checklist.md)
- [ACR / VPAT](./trust/acr-vpat.md)
- [Headroom proof (durability)](HEADROOM_PROOF_20260905.md)
- [Throughput proof (concurrency)](THROUGHPUT_PROOF_20260905.md)
- [Loom proof (parallelism)](LOOM_PROOF_20260906.md)
- [Meridian proof (content hygiene)](MERIDIAN_PROOF_20260907.md)

# Regulation & trust

- [OWASP 2026 compliance matrix](OWASP_AGENTIC_2026.md)
- [Memory-poisoning mitigation (MemGhost / ASI06)](MEMGHOST_MITIGATION.md)
- [AI literacy playbook (EU AI Act Art 4)](AI_LITERACY.md)
- [California ADMT transparency](admt.md)
- [US state AI map (operator runbook)](US_STATE_MAP.md)
- [CRA evidentiary kit](cra.md)
- [CRA reporting runbook](cra-reporting-runbook.md)
- [CRA 30-minute DSAR drill](CRA_DSAR_DRILL_20260808.md)
- [Crypto inventory](crypto-inventory.md)
- [Risk register](risk-register.md)
- [RFP response kit](RFP_RESPONSE_KIT.md)
- [Contact center standards alignment](CONTACT_CENTER_STANDARDS.md)

# Research

- [Research index](./research/index.md)
- [Bi-temporal Knowledge Graph](./research/01-bi-temporal.md)
- [Submodular Evidence Packing](./research/02-submodular-packing.md)
- [TRACE Typed Edges + Explanation Paths](./research/03-trace-edges.md)
- [Personalized PageRank Graph Retrieval](./research/04-ppr-graph.md)
- [Noise-Aware Graph + Hub Dampening](./research/05-hub-dampening.md)
- [Calibrated Abstention + Span Verification](./research/06-abstention-verify.md)
- [The PRF Gate + Evidence-Faithful Snippet](./research/07-prf-evidence.md)
- [Hybrid Fusion: RRF over BM25 + quantized vectors](./research/08-hybrid-fusion.md)
- [Opt-in Anticipation (the Suggest surface)](./research/09-anticipation.md)
- [Structure-Aware Markdown Chunking](./research/10-chunking.md)
- [Centroid Domain Auto-Routing](./research/11-domain-routing.md)
- [Deterministic Consolidation](./research/12-consolidation.md)
- [Benchmark landscape 2026](./research/13-benchmark-landscape-2026.md)

# Blog

- [Blog index](./blog/index.md)
- [Your agent's memory is a compliance time bomb](./blog/01-compliance-time-bomb.md)
- [Human-in-the-loop, not "ask the model nicely"](./blog/02-human-gate.md)
- [Tamper-evident audit: why your memory store needs a hash chain](./blog/03-tamper-evident-audit.md)
- [Reference-faithful retrieval, no LLM in the loop](./blog/04-reference-faithful.md)
- [What Mem0's own docs say about lock-in](./blog/05-no-lock-in.md)
- [OWASP 2026: our control matrix is the sales doc](./blog/06-owasp-matrix.md)
- [The honest ceiling](./blog/07-honest-ceiling.md)
- [From twelve products to one (a preview of Profiles)](./blog/08-profiles-preview.md)
- [Agent memory for a contact center](./blog/09-contact-center-vertex.md)
- [DeepSeek Harness (dsh) meets Brain Server](./blog/10-dsh-deepseek-harness.md)
- [The loop runs: what it means for an engine to ask permission](./blog/11-the-loop-runs.md)
- [The 500 that proved the audit chain works](./blog/12-the-500-that-proved-the-chain.md)
- [Dual-era MCP without the handshake tax](./blog/13-dual-era-mcp.md)
- [Four copies of sha256_hex: when a machine audits the docs](./blog/14-four-copies-of-sha256-hex.md)
- [Prompt injection made stateful — and the memory layer built for it](./blog/15-prompt-injection-made-stateful.md)
- [Two people have to say yes](./blog/16-two-people-have-to-say-yes.md)
- [The redirect that never happens](./blog/17-the-redirect-that-never-happens.md)
- [Signatures with a stated ceiling](./blog/18-signatures-with-a-stated-ceiling.md)
- [Visible mixing beats pretend isolation](./blog/19-visible-mixing-beats-pretend-isolation.md)
- [Why I built the governance layer](./blog/20-why-i-built-the-governance-layer.md)
- [The week runtime enforcement got a standard](./blog/21-runtime-enforcement-got-a-standard.md)

# Product site

- [Product landing](./product-site/index.md)
- [About & Contact](./product-site/about.md)
- [Editions](./product-site/editions.md)
- [Install](./product-site/install.md)
- [Quickstart](./product-site/quickstart.md)

# Project

- [Changelog](CHANGELOG.md)
- [Release history](roadmap-and-release-history.md)
- [Roadmap](roadmap.md)
- [Benchmarks](BENCHMARKS.md)
- [Audit register](AUDIT.md)
- [Agent history](AGENTS_HISTORY.md)
- [Release checklist](release-checklist.md)
- [Media kit](media-kit.md)
- [Contributing](CONTRIBUTING.md)
- [Code of conduct](CODE_OF_CONDUCT.md)
