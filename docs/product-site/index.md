# Brain Server

**Local-first semantic memory + knowledge graph for AI agents. Deterministic,
privacy-preserving, human-auditable.**

Brain Server gives an agent a second brain that lives on the operator's own
device. Recall never has to think: a static, local embedding model plus a
deterministic retrieval pipeline answer the question *without* an LLM deciding,
*without* an embedding API on every read and write, and *without* data leaving
the machine.

The one-line framing for 2026: **your agent's memory is a compliance time bomb.
Brain Server is the tamper-evident, human-gated memory store that defuses it.**

---

## The three pillars

1. **Deterministic, reference-faithful retrieval** — no LLM in the loop, no
   per-query cost, no data egress. The retrieval stack implements published
   research *deterministically* (bi-temporal knowledge graphs, submodular
   evidence packing, TRACE edges, Personalized PageRank graph leg, GAAMA hub
   dampening, calibrated abstention). See [`docs/research/`](../research/).
2. **Human-in-the-loop write gate** — nothing becomes memory autonomously. A
   candidate is *proposed*, scored deterministically, and promoted only when a
   human approves. The injection screen (blocklist + optional local classifier)
   quarantines adversarial input before it reaches the gate. See v1.14–v1.20.
3. **Tamper-evident audit** — every decision (and, opt-in, every read) lands in
   a SHA-256 hash chain you can verify end to end. DSARs produce chain-
   verifiable deletion certificates. Every security/compliance claim in the
   docs is *reproducible live*, not asserted. See
   [`docs/trust/proof-map.md`](../trust/proof-map.md).

## What it is not

- Not an LLM — it stores and recalls, it does not generate.
- Not a SaaS lock-in — one self-hosted binary, zero telemetry, no vendor.
- Not a black box — every mechanism has a documented, deterministic
  implementation and an honest ceiling.

## Who it is for

- Developers building agents that need memory their users can trust, audit,
  and delete on request.
- Operators who must answer "what did the agent know, when, and why?" for a
  SOC 2 / GDPR / EU AI Act review.
- Teams that refuse to pay an embedding API on every read/write and refuse to
  ship user memory to a third-party datacenter.

---

Continue to [Quickstart](./quickstart.md) or [Install](./install.md). For the
self-serve evaluation story, see [Editions](./editions.md).

For the narrative — the why / who-it's-for / market-shift stories — see the
[blog](../blog/) (one post per hard-won mechanism, each tied to its research or
trust source) and the [media kit](../media-kit.md) (positioning, one-liners, and
a Brain-vs-the-field sizing table with honest ceilings).
