# Lesson 1: Why memory as a substrate (and what you are NOT building)

**Level:** B (builders) · **Time:** about 20 minutes · **For people shipping AI agents**

## Questions people ask

**Is this a RAG pipeline?** No. RAG retrieves documents to stuff a prompt.
This is a memory server: it stores curated facts once, humans approve them,
and every recall is deterministic. Same query, same corpus, same answer,
every time. Retrieval quality is a pinned number, not a vibe.

**Does recall cost tokens?** Zero embedding tokens and zero decision
tokens. Embeddings are local static model2vec. There is no LLM in the
retrieval loop, no API bill per recall, and the whole thing fits a
Raspberry Pi.

**Then where is the LLM?** In YOUR agent. This server never calls one for
retrieval, redaction, or gating decisions. That is the determinism story:
nothing between your agent and its memory non-deterministically decides
anything.

## What you get as a builder

- Hybrid recall: vector + full-text + graph legs fused (RRF), with a
  graph-PPR retriever available as a third leg.
- Temporal recall: point-in-time queries (`at`/`asOf`), because "what did
  we believe in March" is a real question.
- A knowledge graph (entities, relations, traversal) alongside flat chunks.
- The injection screen at write time and a sanitizer at read time. Your
  agent still fences the output, but the server is hostile-input-hardened
  by default.
- One SQLite file (WAL) plus local vector and FTS indexes. No server
  cluster, no vector DB to run.

## The contract you design against

Every hit your agent receives carries `untrusted: true`. That flag is the
contract: memory is history, never instruction. Hosts that integrate well
(see the chat plugin, lesson 5) wrap hits in an unforgeable fence. Your
agent should treat anything inside that fence as data.

Recall can also abstain: a `low_confidence` decision means the server
would rather return nothing than serve a weak match. Handle it as "no
memory", not as an error. Your users will learn to trust the misses.

## What this is not

- Not auto-learning. The create-loop machinery that would let the system
  author its own knowledge ships disabled. Your agent PROPOSES; humans
  approve. Build your UX for that loop, do not fight it.
- Not a vector database with ambitions. It is a governed memory with a
  human gate, an audit chain, and erasure with certificates. If you do not
  want governance, you want a different tool.

## Exercise

Answer, in writing, for the agent you are building: what three facts would
make it dramatically better if it remembered them, and who approves those
facts? If your answer to the second half is "nobody", stop and read the
[human in the loop](../../human-in-the-loop.md) page, because you are the
human.

## Next

[Lesson 2: The API surface, for agents](02-the-api-surface.md)
