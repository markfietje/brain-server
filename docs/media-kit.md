# Media kit

> **Status:** positioning + one-liners + sizing for a landing page, a PR
> pitch, or a journalist. Author-faithful to the product (not an external
> analyst's endorsement). Version-grounded: every technical claim maps to a
> shipped release in the [proof map](./trust/proof-map.md).

## Name / one-liner

- **Product:** Brain Server
- **One-line (technical):** "A local-first semantic-memory and knowledge-graph
  server for AI agents — deterministic retrieval, a human-in-the-loop write
  gate, and a tamper-evident audit chain."
- **One-line (buyer):** "Agent memory you can verify, budget, and delete on
  request — no LLM per query, no data egress, no vendor lock-in."
- **Three-word elevator:** "Verifiable agent memory."

## Positioning statement

For teams building AI agents that must hold memory responsibly, Brain Server is
a self-hosted memory store that makes agent recall **deterministic, human-
gated, and tamper-evident** — unlike cloud memory services that charge per
query and keep user memory in a third-party datacenter. Because it runs on the
operator's own device with no LLM in the loop, it delivers **zero per-query
cost, zero data egress, and an audit trail a reviewer can verify live** — and,
unlike framework-bound memory layers, it is **standard-based** (UMP 1.0 / L3,
open HTTP, MCP) so it never locks you in.

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
(v2.0), per-tenant limits (v2.1), OTel/SSE ops line (v1.20.7/8), SOC 2 kit
(v1.20.10), pricing/licensing (v2.2 "Meridian"), use-case Profiles (v1.21.0).
Retrieval is deterministic, not SOTA-generative; multi-hop graph quality is
corpus-bound; abstention is heuristic, not learned.

## Headline stats (verify in the proof map)

- **UMP 1.0 / L3** conformance — reference-suite scored 13/13.
- **`$0`** per query — no LLM/embedding API in recall or writes.
- **< 5 W** — runs on a 4 GB ARM device (Jetson Nano / RPi 5).
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
