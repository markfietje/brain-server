# Roadmap & Release History

Brain Server ships on a strict linear release chain. This page is the roadmap summary and the release history. The authoritative version of both lives in `ROADMAP.md` and `CHANGELOG.md` in the repository.

## Current status

- **Latest server version:** 1.20.29 "Bound" (2026-08-14) — the ATLAS-hardening tail closed: SSRF-egress refusals (Tourniquet), quarantine-taint provenance (Fencepost), and the fail-closed loopback bind guard (Bound).
- **Latest client version:** 1.20.25 "Consolidate" (2026-08-13) — ships alongside the server (read-seam rendering, DSAR/purge surfaces).
- **Latest plugin version:** 0.4.2 "Bound" (2026-08-14) — recall amplification bound + schema-clamped tool params.
- **Next milestone:** v2.0.0 "Cortex".
- **v2.0.0 "Cortex"** (multi-team tenancy) is the first externally-pilotable release — it consumes the v1.2 AuthN/AuthZ foundation.

## The release line (v0.9 → v1.17)

| Release | Name | What shipped |
|---|---|---|
| v0.9.1 | Recall | Hybrid retrieval (vector + FTS + RRF), PRF expansion, provenance |
| v0.9.2 | Connect | Obsidian vault ingestion |
| v0.9.4 | Sources | Source lifecycle + reconcile |
| v0.9.5 | Inspect | Structured query contract + evidence |
| v0.9.6 | Bridge | Connectors + GitHub backfill |
| v0.9.9 | Qualify | Capacity envelopes + migration rehearsal |
| v1.0.0 | Domains | Multi-domain foundation |
| v1.1.x | Harden | Audit chain fixes + constant-time hardening |
| v1.2.0 | AuthN | JWT/JWS + OIDC/JWKS + AuthZ |
| v1.3.0 | Bedrock | Memory-safety hardening |
| v1.4.0 | Calibrate | Bi-temporal edges + submodular packing + TRACE + eval harness |
| v1.4.1 | Link | Deterministic entity linker upgrade |
| v1.5.0 | Epistemic | Calibrated abstention + span verification |
| v1.6.0 | Reconcile | Atomic supersession + consistency check |
| v1.7.0 | Explain | Faithful path explanations |
| v1.8.0 | Maintain | Reviewable proposals + undo |
| v1.9.0 | Suggest | Opt-in anticipation + false-positive metric |
| v1.9.1 | Harden | Bug-fix audit |
| v1.10.0 | Procedural | Ordered procedures + classification + decision rules |
| v1.11.0 | Associate | HippoRAG-2-style PPR graph leg |
| v1.12.x | Discern / Harden | Noise-aware graph retrieval + AuthZ wiring |
| v1.13.x | Route / Recall-fix | Domain routing + routing hotfix |
| v1.14.0 | Gate | Human-in-the-loop write-back + trust surfaces |
| v1.15.0 | Observe | Read-event audit + recall trace + DSAR + COMPLIANCE.md |
| v1.16.0 | Client | The Dioxus control surface (web + desktop + mobile) |
| v1.16.1–1.16.8 | Serve / Styled / Secure / Mobile / Integrated / Global | Serving + CSP, design-system restyle, JWT lifecycle, responsive UX, deep links + PWA, i18n + themes |
| v1.17.0 | Mobile | Portable refresh + deep links + offline connect + store readiness |
| v1.17.1 | Govern | Per-kind retention + Art 30 + UMP wire adapter + eval ship-gate |
| v1.17.3 | UMP Rollout | Full UMP 1.0 conformance through L3 (HTTP ops + MCP tools + file binding + identity/capability tokens) |
| v1.17.4 | UMP Conformance | Reference-suite wire fixes (did:key + integrity block) → L3 |
| v1.17.5 | Eval Fix | `brain eval` revived + Round-21 CI gates + SBOM |
| v1.17.6 | Complete 1/3 | Command palette v2 + Overview home |
| v1.17.7 | Complete 2/3 | Graph panel + Create workspace |
| v1.17.8 | Complete 3/3 | Data & Rights + UMP + System panels + Try-it console |
| v1.18.0 | Compliant | `?` keyboard help on Review (WCAG 3.2.6) + a `client-gate` CI job |
| v1.18.1 | Harden | Console history persists (secret-safe) + measured client bundle |
| v1.18.2 | Transparency | Art 50 `knowledge.origin` marker + `/export` provenance |

## Milestone themes

- **v1.16.x "Integrated"** — client polish: PWA, deep links, command palette, responsive mobile, paginated audit.
- **v1.17.x "Govern" → "Complete"** — governance server releases (retention, Art 30, UMP conformance) then the full operator console that surfaces them (12 panels).
- **v1.18.x "Compliant" → "Transparency"** — WCAG 2.2 AA + i18n + privacy hardening, secret-safe console history, and the Art 50 origin marker + export provenance.
- **v2.0.0 "Cortex"** — multi-team tenancy, ready, consuming the v1.2 AuthN/AuthZ foundation.
- **v2.1+ "Limits" / "Regions"** — distributed revocation, scaling.
- **v3.x "Survive" / "Sovereign"** — federated, sovereign deployments.
- **v4.0 "Standard"** — standards conformance.

## How releases are governed

Since v1.5, feature releases are scoped to an **evidence-gated roadmap** (`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md`). The rule: ship only what is evidenced and low-risk; forbid autonomous consolidation, unsolicited push, hidden personalization, and synthetic content. Light cuts are preferred over ambitious-but-unverifiable features.

## Next steps

- **[Features](./features.md)** — everything current releases can do.
- **[Governance & Compliance](./compliance.md)** — the standards work ahead.
- The full history: `CHANGELOG.md` and `ROADMAP.md` in the repository.
