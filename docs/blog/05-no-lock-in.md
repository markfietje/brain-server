# What Mem0's own docs say about lock-in

*2026. Framework-agnostic isn't a nice-to-have — it's the adoption bar.*

Agent-memory vendors are fond of telling you about integration counts. Mem0's
own positioning boasts dozens of LLM frameworks and vector stores supported.
Read closely and the message is: **a memory layer that locks you to one
framework or one vector store will not be adopted at scale.** That's a real
insight — and it's one we agree with, and act on in a way that doesn't create a
different lock-in.

## The two kinds of lock-in

1. **Framework lock-in:** "this memory only works inside my agent SDK." Adopt
   it and your memory is hostage to a framework choice you may reverse later.
2. **Service lock-in:** "your memory lives in my datacenter." Adopt it and your
   data — and your recall latency, and your bill — is hostage to a vendor's
   uptime, pricing, and compliance posture.

We avoid both, not by advertising more integrations, but by refusing to define
memory through a proprietary channel at all.

## Brain Server's no-lock-in answer

- **UMP 1.0 / L3 conformance** — a published memory-protocol standard, scored
  by the reference conformance suite (13/13, L3). Your memory is readable and
  writable through a standard wire, not a private API. Leave our product and the
  protocol — and your data — travel with you.
- **An open HTTP contract** — `GET /openapi.yaml` documents every route, served
  by the binary itself. Any client, any language, no SDK required.
- **MCP** — a stateless core implementing the Model Context Protocol, so it
  slots into the agent tools ecosystem without being bound to one runtime.
- **Local-first storage** — a single SQLite-family file on *your* device. There
  is no cloud side, no egress, no "your memory in our cluster." The ultimate
  anti-lock-in is that there's nothing to be locked into.

## The honest trade

No vendor lock-in means no vendor magic. The deterministic, no-LLM retrieval is
yours to run — which also means the *curation* and *evaluation* are yours too
(the corpus-quality ceiling in the PPR explainer is a real operator step, not a
marketing asterisk). We think that's the right trade: **portability and audit
over convenience.** A memory store you can leave is a memory store you can
trust; a memory store you can't leave is a dependency you'll be stuck defending.

**The takeaway:** when you evaluate agent memory, don't count integrations —
count *standards*. Ask: is there a published protocol? An open contract? A local
file I own? Those are the things that survive a framework migration, a vendor
pricing change, or a compliance deadline. That's what "no lock-in" actually
means, and it's the bar we hold ourselves to.

*See [`docs/trust/proof-map.md`](../trust/proof-map.md) for the UMP L3 +
capability-token rows and how to verify them live.*
