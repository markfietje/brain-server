# The builder track: memory for your AI agent

For people building AI agents, assistants, and tool integrations
(the audiences page's agent builders). You get the full API and protocol
surface, the security posture you inherit for free, the reference
integration to copy, and the architecture map. Six lessons, about two
hours, one capstone loop.

1. [Why memory as a substrate](01-why-memory-as-a-substrate.md)
2. [The API surface, for agents](02-the-api-surface.md)
3. [UMP, capability tokens, and parcels](03-ump-tokens-parcels.md)
4. [The injection screen, from the builder's side](04-the-screen-for-builders.md)
5. [The chat plugin, read as a reference integration](05-the-plugin-as-reference.md)
6. [The architecture and the loops, a builder's tour](06-architecture-and-loops.md)

**Questions people ask**

**Is this an alternative to RAG?** It is the governed layer under your
agent: deterministic hybrid recall over human-approved facts, zero tokens
per recall, local embeddings. [Lesson 1](01-why-memory-as-a-substrate.md).
**Does my agent need the cloud?** No. Single binary, single SQLite file,
runs offline on a Raspberry Pi. [Lesson 6](06-architecture-and-loops.md).
**Can the agent write its own memory?** It proposes; humans approve, and
the self-promotion machinery ships disabled. [Lesson 2](02-the-api-surface.md).

Related: [UMP](../../universal-memory-protocol.md),
[API reference](../../api.md),
[The agent loop](../../agentloop.md),
[Architecture](../../architecture.md).
