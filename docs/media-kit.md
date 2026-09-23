# Media kit

> **Status:** positioning + one-liners + sizing for a landing page, a PR
> pitch, or a journalist. Author-faithful to the product (not an external
> analyst's endorsement). Version-grounded: every technical claim maps to a
> shipped release in the [proof map](./trust/proof-map.md).

## Name / one-liner

- **Product:** Brain Server
- **One-line (technical):** "A local-first decision and memory substrate for AI
  agents — deterministic retrieval, human-gated state promotion, tamper-evident
  provenance, and structured decision traces."
- **One-line (primary positioning):** "Governed decision and memory substrate
  for AI agents — deterministic local recall, human-gated permanent state, and
  tamper-evident audit, with structured decision traces."
- **One-line (buyer):** "Agent memory and decisions you can verify, budget, and
  delete on request — no LLM per query, no data egress, no vendor lock-in."
- **Three-word elevator:** "Verifiable agent decisions."
- **One-line (contact-center / BPO support):** "Agent-assist memory and decision
  substrate that recalls past resolutions and policy, stays on-prem, and is
  yours to audit and erase — no per-query LLM, no vendor lock-in."

## Positioning statement

For teams building AI agents that must hold memory and make structured
decisions responsibly, Brain Server is a self-hosted substrate that makes both
recall and the decisions that depend on it deterministic, human-gated, and
tamper-evident — unlike cloud memory services that charge per query and keep
user data in a third-party datacenter.

Because it runs on the operator's own infrastructure with no LLM in the hot
recall path, it delivers **zero per-query cost, zero data egress, and an audit
trail a reviewer can verify live**.

## Who it's for

The same engine serves several audiences; see **[Who it's for — target
audiences](./audiences.md)** for the full map (each marked shipped vs.
roadmap).

- **AI-agent builders & OpenClaw users** — deterministic memory, zero token
  cost, in the memory slot.
- **BPOs & multi-client contact-center operators** — the v2.0 "Cortex" roadmap
  is explicitly call-center intelligence (multi-team tenancy, ticket-pattern
  resolution). The *controls* they need are shipped today (per-domain
  scoping, per-tenant audit, DSAR, PII containment, human-gated writes);
  multi-client tenancy on one shared backend is the documented v2.0 piece
  (true storage isolation is the separate `BRAIN_MULTI_DB` mode, not the
  default shim).
- **In-house contact & support centers** — agent-assist memory that recalls
  past resolutions and policy, supervised and audited, without fabricating
  answers (calibrated abstention + span verification).
- **Regulated enterprises** (finance, healthcare, legal, government) — memory
  that stays on-prem, is auditable to a chain, honors DSAR, and is explainable.
- **Edge / field / air-gapped deployments** — a single self-hosted runtime on 4 GB ARM.
- **Delivery partners** (SIs, MSPs, consultants) — a deployable, auditable
  memory layer with procurement-grade evidence (`RFP_RESPONSE_KIT.md`).

## The three pillars (press-ready)

1. **Recall that never has to think** — deterministic, reference-faithful
   retrieval (bi-temporal KG, submodular packing, PPR graph leg, hub
   dampening, calibrated abstention). No LLM decides, no token is spent.
2. **A write gate, not a write path** — memory is *proposed* and promoted only
   on human approval; an injection screen quarantines adversarial input.
3. **A chain, not a log** — every decision lands in a tamper-evident SHA-256
   chain; DSARs produce chain-verifiable deletion certificates; an OWASP 2026
   control matrix states every control as shipped or owned ceiling.

## Brain vs. the field (sizing, with honest ceilings)

| | Brain Server | Mem0-class (framework memory) | LangGraph-class (agent framework) | Plain RAG |
|---|---|---|---|---|
| Per-query cost | **$0** | LLM/embedding API | LLM/embedding API | LLM/embedding API |
| Where memory lives | **Your device** | vendor/cloud | vendor/cloud | your infra |
| Recall determinism | **Yes** | no | no | partial |
| Human write gate | **Default** | optional | no | no |
| Tamper-evident audit | **Yes (hash chain)** | no | no | no |
| DSAR deletion cert | **Yes** | partial | no | no |
| Standard wire | **UMP L3 + open HTTP + MCP** | proprietary/framework-bound | framework-bound | none |
| Zero LLM in loop | **Yes** | no | no | no |

*Honest ceilings we don't claim (each owned + versioned):* multi-team tenancy
(v2.0), per-tenant limits (v2.1), pricing/licensing (v2.2).
Retrieval is deterministic, not SOTA-generative; multi-hop graph quality is
corpus-bound; abstention is heuristic, not learned.

## Headline stats (verify in the proof map)

- **UMP 1.0** — 13/13 reference-suite checks (`@universalmemoryprotocol/core`, CI re-run): L3 signed on a keyed instance, L2 hash-only without a key.
- **`$0`** per query — no LLM/embedding API in recall or writes.
- **Small-device capable** — runs on a 4 GB ARM device (Jetson Nano / RPi 5); no power-draw figure is claimed (none measured).
- **`{"ok":true}`** in one command — `/audit/verify` proves the chain intact.
- **OWASP 2026** matrix — 100% control coverage (shipped or owned ceiling).

## Press contact / ask

For a reviewer: run the 3-minute [`reproduce.md`](./trust/reproduce.md) walk-
through to verify every security claim live against a throwaway instance —
"trust us" becomes "verify it." For a journalist: the honest-ceiling post
([`blog/07-honest-ceiling.md`](./blog/07-honest-ceiling.md)) is the story — a
memory store that tells you its limits.

## Logos / naming notes

Name has no built-in icon yet (operator step). The wordmark is "Brain Server";
the CLI/product family is `brain` / `brain-server` / `mcp`. Repository:
`markfietje/brain-server`.

## Author / contact

Maintained by **Mark Fietje**:

- LinkedIn: [linkedin.com/in/markfietje](https://www.linkedin.com/in/markfietje/)
