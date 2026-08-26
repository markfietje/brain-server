# Editions

> **Status placeholder.** Pricing and licensing are a documented roadmap
> milestone (v2.2 "Meridian"); nothing here is a committed price. This page
> exists so the commercial question has a *documented* answer rather than an
> omission. The technical capability line is real and shipped; the commercial
> wrapper is not.

The capability is one self-hosted binary. Editions are a **packaging
distinction, not a feature fork** — the enterprise controls are already in the
code (JWT/JWS AuthN, deny-by-default AuthZ, per-tenant audit, DSAR, capability
tokens, Standard Webhooks).

| | OSS | Self-hosted Pro | Enterprise |
|---|---|---|---|
| The binary + CLI + MCP + OpenAPI | ✓ | ✓ | ✓ |
| Deterministic retrieval (all mechanisms) | ✓ | ✓ | ✓ |
| Human-in-the-loop write gate + screen | ✓ | ✓ | ✓ |
| Tamper-evident audit + `/audit/verify` | ✓ | ✓ | ✓ |
| JWT/JWS AuthN + AuthZ (v1.2) | ✓ | ✓ | ✓ |
| DSAR + deletion certificates + Art 50/19 | ✓ | ✓ | ✓ |
| Multi-team tenancy + per-tenant limits | — | — | v2.0/v2.1 |
| OTel/OTLP export + SSE alert feed | — | ✓ | ✓ |
| Use-case Profiles (presets) | ✓ | ✓ | ✓ |
| SOC 2 evidence kit + onboarding | — | — | ✓ |
| Support SLA | community | best-effort | contract |

Rows map to shipped releases:
- ✓ **shipped**: v1.2 AuthN, v1.14 gate, v1.15 DSAR/audit, v1.17 UMP L3, v1.18–1.20 console/hardening line — and since then the profiles/connectors/BPO-controls arc and the governed-loop line (workflow harness, lineage, scoreboard, complaints/aftersales/frontdesk) through **v1.28.34 "Goodwill"**.
- **v2.0/v2.1**: multi-team tenancy + per-tenant limits (roadmap, no code yet) — the enabler for **BPO / multi-client contact-center** deployments. The controls those buyers need (isolation, audit, DSAR, PII, human-gated writes) are shipped today; the shared-tenant packaging is the roadmap. See **[Who it's for — target audiences](../audiences.md)**.
- **OTel** shipped feature-gated in v1.20.7 (`--features otel`, opt-in via `BRAIN_OTEL_ENABLED`/`BRAIN_OTEL_ENDPOINT`), with the SSE alert feed shipped alongside it. See **[Observability](../observability.md)**.
- **SOC 2 evidence kit** shipped in v1.20.10 + the v1.20.12 trust tier.
- **Use-case Profiles** shipped in v1.21.0 and are part of the OSS line.

## The honest promise

Editions are about **operational posture and support**, not holding back
features an enterprise needs for compliance. The audit chain, DSAR, and the
OWASP 2026 matrix ship in the OSS line — because a memory store that only
becomes auditable after you pay for a license is not a memory store anyone
should adopt.
