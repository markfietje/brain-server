# Roadmap & Release History

Brain Server ships on a strict linear release chain. This page is the roadmap summary and the release history. The authoritative version of both lives in `ROADMAP.md` and `CHANGELOG.md` in the repository.

## Current status

- **Latest version:** 1.16.7 "Integrated" (2026-08-08) — client-only: deep links, PWA, paginated audit, command palette, recall debounce, drawer focus trap, aria-live, RTL.
- **Server + API contract:** stable at 1.16.2–1.16.7 (the only server change since 1.16.2 is the additive `offset` param on `/audit`).
- **Next milestone:** v1.16.8 "Global".
- **v2.0.0 "Cortex"** (multi-team tenancy) is the first externally-pilotable release — it consumes the v1.2 AuthN/AuthZ foundation.

## The release line (v0.9 → v1.16)

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
| v1.16.1–1.16.7 | Serve / Styled / Secure / Mobile / Integrated | Serving + CSP, design-system restyle, JWT lifecycle, responsive UX, deep links + PWA |

## Milestone themes

- **v1.16.x "Integrated"** — client polish: PWA, deep links, command palette, responsive mobile, paginated audit.
- **v2.0.0 "Cortex"** — multi-team tenancy, ready, consuming the v1.2 AuthN/AuthZ foundation.
- **v2.1+ "Limits" / "Regions"** — distributed revocation, scaling.
- **v3.x "Survive" / "Sovereign"** — federated, sovereign deployments.
- **v4.0 "Standard"** — standards conformance.

## How releases are governed

Since v1.5, feature releases are scoped to an **evidence-gated roadmap** (`IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md`). The rule: ship only what is evidenced and low-risk; forbid autonomous consolidation, unsolicited push, hidden personalization, and synthetic content. Light cuts are preferred over ambitious-but-unverifiable features.

## Next steps

- **[Features](Features)** — everything current releases can do.
- **[Governance & Compliance](Governance-and-Compliance)** — the standards work ahead.
- The full history: `CHANGELOG.md` and `ROADMAP.md` in the repository.
