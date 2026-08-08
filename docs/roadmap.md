# Roadmap

Brain Server ships in small, verifiable, named releases. This page summarizes the
journey to the current version and where it is going. The authoritative plan is
[ROADMAP.md](../ROADMAP.md); the full per-version record is
[CHANGELOG.md](../CHANGELOG.md).

---

## Current status

**v1.16.x — the client control surface.** Brain Server now ships a Dioxus GUI
(web + desktop + iOS + Android from one Rust codebase) on top of a mature server.
The v1.16 line added: connection state machine with a false-offline guard,
honest-batch review, a recall decision-path viewer, DSAR certificate cards, an
auth-failure feed, audit filters + export, a shadcn/ui-styled sidebar dashboard,
JWT refresh lifecycle on the client, and mobile-responsive UX with secure keyring
token storage.

The server core (retrieval, graph, governance) is stable and heavily tested
(500+ tests).

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

---

## Where it's going

| Milestone | Theme |
|---|---|
| **v1.16.7 "Integrated"** | Final client integration polish |
| **v1.16.8 "Global"** | Client-wide global milestone |
| **v2.0 "Cortex"** | Multi-team tenancy — the first externally-pilotable release (consumes the v1.2 AuthN/AuthZ foundation) |
| **v2.x** | Distributed revocation, limits/regions, federation |
| **v3.x** | Sovereign + survive (resilience), federated deployments |
| **v4.0** | Sovereign standard |

The intermediate milestones (v1.17–v1.27) cover profiles, regulated modes, roles,
connectors, and BPO operations. The plan is evidence-gated: work is only shipped
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
- The authoritative [ROADMAP.md](../ROADMAP.md) and [CHANGELOG.md](../CHANGELOG.md).
