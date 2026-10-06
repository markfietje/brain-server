# Lesson 6: The architecture and the loops, a builder's tour

**Level:** B · **Time:** about 30 minutes · **The map lesson**

## Questions people ask

**What is inside the binary?** One Rust server, strictly layered:
protocol adapters (HTTP handlers) that only parse, authorize, and shape;
service cores that own all SQL, bounds, and invariants; and domain
workflow modules beneath them. The layering is enforced by machine gates,
not convention: handlers containing SQL fail CI, full stop. The request
flow (auth middleware, rate limit outermost, one database transaction per
write with its audit row inside the transaction) is diagrammed on the
[architecture page](../../architecture.md).

**Where does the data live?** One SQLite file in WAL mode, plus three
index legs: a vector index (local static embeddings), full-text search
(FTS5), and the graph tables. Writes are transactions; every mutation
writes its hash-chained audit row INSIDE the same transaction, so a change
and its evidence commit together or not at all.

**What are "the loops"?** The governed workflow engines: create, solve,
evolve, deflect (plus the mesh of crew, channels, runs). A run walks steps
with gates, questions bound to the live question (a stale answer cannot
satisfy a new gate), CAS-guarded transitions, and handovers emitted from
the timeline. The one loop you must know by name is the CREATE loop: the
machinery by which the system would author its own knowledge. It ships
inert, promotion disabled at compile time, no env override. Your agent
files proposals into that world; humans are the promotion engine.

**How is determinism enforced?** Pinned evaluation floors on a frozen
corpus (recall quality floors in CI), a drift census that re-scores the
frozen corpus against a committed baseline and raises a finding on
breach, replay gates on delivery traces, and same-seed determinism tests
on concurrent clients. Quality here is a measured, versioned fact.

## The builder's checklist, assembled

1. Your agent recalls from `/recall` or UMP, treats every hit as
   `untrusted`, and fences memory in the prompt.
2. Your agent writes via `/ingest/proposal`. Humans approve. Your UX
   shows the queue as a feature, not an apology.
3. Your integration authenticates with the least credential (agent token
   or read-only capability).
4. Your CI carries canaries: hostile input must quarantine, clean input
   must recall, abstention must be handled.
5. Your memory boundary is fail-open at the chat layer and fail-closed at
   the truth layer.

## Capstone

Build the smallest complete loop, any language, an afternoon:

1. Boot a throwaway instance, review posture.
2. A "capture" path: take three sentences of user text, file each as a
   proposal, with the user as the human approver via the digest flow.
3. A "recall" path: query, render hits inside a fence you generate,
   handle one abstention gracefully.
4. A "rights" path: dry-run and export for the user's subject label.
5. One canary through the capture path, assert it quarantined.

That loop, five paths, is the whole product in miniature. Everything else
this server does exists to make those five paths trustworthy at scale.

## Course complete

Back to the [course index](../README.md), or read the
[architecture page](../../architecture.md) and the
[agent loop page](../../agentloop.md) with your new eyes.
