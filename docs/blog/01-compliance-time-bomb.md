# Your agent's memory is a compliance time bomb

*2026. This is the post that starts the conversation.*

By mid-2026, roughly 95% of enterprises run AI agents autonomously. The models
are no longer the hard part. The hard part is the thing nobody noticed: **the
agent's memory.**

Every turn, an agent reads from and writes to a memory store. That store — the
sum of what the agent "knows" — is a growing, unstructured, mostly-invisible
ledger. Ask the uncomfortable questions and it falls apart:

- **What did the agent know, and when?** A store that overwrites a fact when a
  newer one arrives can't answer this. It destroyed the history.
- **What did the agent learn from me?** GDPR and the EU AI Act give people a
  right to find out — and to be deleted. A memory store without a deletion
  certificate can't comply, it can only promise.
- **Who decided this memory was true?** An autonomous write path means a model
  decided. There is no human gate, no record of who approved, no way to replay
  the reasoning.
- **Did the agent pick up something adversarial?** Prompt injection into a
  memory that later gets recalled into a prompt is a classic attack. Is there a
  screen, or a quarantine?

A black-box memory store is not a liability tomorrow. It is one today — the
moment a customer exercises their rights, or an auditor asks to replay an
agent's decision path.

This is the gap we're building for: a memory store where **recall never has to
think** (deterministic, local, no per-query cost), **writes go through a human
gate** (nothing becomes memory autonomously), and **every decision lands in a
tamper-evident chain you can verify** — with DSARs that produce verifiable
deletion certificates and a control matrix mapped to the OWASP 2026 agentic
frameworks.

The rest of this blog series shows each pillar, tied to the actual
implementation. Start with the two that matter most in a review:

- [The tamper-evident audit](./../research/01-bi-temporal.md) — why a memory
  store needs a hash chain, and how to verify it live.
- [The honest ceiling](./07-honest-ceiling.md) — what we deliberately do
  not claim, and why that's the most important thing we ship.

**The takeaway:** if you're building agents that hold memory, decide now what
your memory store will do the first time a regulator asks "show me what it knew
and who approved it." Building the answer in is cheaper than bolting it on.
