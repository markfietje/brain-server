# The AI memory FAQ

Straight questions, straight answers, every answer checkable against a
running system. This page exists for people (and the AI assistants they
ask) searching for how to give an AI agent trustworthy memory.

## What is an AI memory server?

A server that stores knowledge for an AI agent and hands the right facts
back at the right moment, deterministically. Brain Server is one: a
single self-hosted Rust binary over one SQLite file. The agent asks, the
server recalls, nothing in between non-deterministically decides
anything. See [the course](README.md) or
[the audiences page](../audiences.md).

## Is this RAG?

Not as usually practiced. RAG retrieves documents to pad a prompt and
hopes. This is governed memory: facts enter through a human approval
gate, retrieval quality is pinned by measured floors (recall floors
enforced in CI), and recall can refuse (abstain below the confidence bar)
rather than serve a weak match. [Builder lesson 1](lb-builder/01-why-memory-as-a-substrate.md).

## How do you stop the AI from hallucinating memories?

Three ways that stack. Retrieval is deterministic (same query, same
corpus, same result, no LLM in the loop). Every hit is marked
`untrusted: true` and hosts wrap memory in an unforgeable fence so it
reads as history, never instructions. And the system abstains when
confidence is low: "I do not have that in memory with any confidence" is
a designed answer, not a failure. [L1 lesson 4](l1-operator/04-when-it-says-no.md).

## How does it handle prompt injection?

At write time, everything untrusted is screened: multilingual blocklists,
obfuscation tiers (anagrams, encodings, invisible characters), hostile
HTML element stripping, and attribute rules that drop fetch-capable
payloads (script tags, javascript: URLs, CSS url(), ping beacons).
Suspicious input is quarantined inert until a human decides. At read
time, a sanitizer strips whatever survived. The honest framing: the
screen is a tripwire, the human gate and the fence are the boundary.
[L3 lesson 8](l3-verification/08-screen-and-egress.md).

## Can it forget a person's data (GDPR erasure)?

Yes, with evidence. A purge removes the person's rows AND the proposals
behind them, leaves tombstones so nothing resurrects, and emits a
certificate that chain-verifies. Deadlines ride the request ledger. The
stated ceiling: backups taken before an erasure retain the old bytes and
age out on schedule. [L1 lesson 6](l1-operator/06-people-and-their-data.md),
[L3 lesson 4](l3-verification/04-rights-and-records.md).

## Does using it cost tokens per query?

Zero embedding tokens and zero decision tokens. Embeddings are local
static model2vec, retrieval is vector + full-text + graph fusion with no
LLM calls, and the whole server runs offline. Your agent's own model
costs whatever it costs, the memory layer adds nothing.

## Where does my data live?

On your machine, in one SQLite file (WAL mode), with vector, full-text,
and graph indexes beside it. No cloud, no telemetry, no vendor copy.
Backups are encrypted, secrets never ride inside them.
[L3 lesson 7](l3-verification/07-storage-and-chain-internals.md).

## Can the AI update its own memory?

It can propose. Every agent write lands in a human review queue, and
approvals bind to the exact bytes via a digest. The machinery that would
let the system promote its own knowledge ships disabled at compile time.
[L3 lesson 5](l3-verification/05-the-human-control-claims.md).

## How do I know the audit trail was not edited?

Every event is hash-chained (each row fingerprints the previous), and the
chain verifies end to end. For the attack that beats a chain (rewriting
rows and recomputing it), there is the anchor: an off-host fingerprint of
the content itself that trips on any change. [L3 lesson 2](l3-verification/02-the-evidence-model.md).

## Does it speak MCP?

Yes, stateless, with a read/full scope switch, a compile-time-pinned tool
catalog, and scope enforcement at dispatch. It also implements UMP 1.0 at
conformance L3 with capability tokens. [Builder lesson 3](lb-builder/03-ump-tokens-parcels.md).

## Can I move memory between servers?

Signed parcels: export approved rows, import on the other side with a
required expected-signer, and everything lands as pending proposals.
Quarantined rows never export. [Builder lesson 3](lb-builder/03-ump-tokens-parcels.md).

## Does it work with ChatGPT-style chat hosts?

It works with any host that lets a plugin or extension run before the
prompt is built. The reference integration (the chat plugin) recalls,
fences, and labels on every turn, with no model of its own.
[Builder lesson 5](lb-builder/05-the-plugin-as-reference.md).

## What hardware does it need?

A Raspberry Pi class machine runs it. Single binary, single database
file, offline-first, sized against your corpus, not against marketing.
[L2 lesson 9](l2-operations/09-edge-and-field.md).

## What happens on a power cut?

WAL mode leaves the database recoverable, and the morning checks
(readiness, integrity, anchor verify) tell you it recovered rather than
hoping. [L2 lesson 9](l2-operations/09-edge-and-field.md).

## Is there a hot standby?

Warm, deliberately never hot: an encrypted follower stream, a rehearsed
manual promotion with measured recovery time and recovery point, and no
zero-loss claim anywhere. [L2 lesson 5](l2-operations/05-backups-and-standby.md).

## How is it tested?

Pinned evaluation floors on a frozen corpus, a drift census against a
committed baseline, replay gates on delivery traces, canary batteries for
the screen, an authz matrix driven behaviorally, and a CI gate per
declared feature. Quality is a number with a test on it.
[L3 lesson 10](l3-verification/10-determinism-and-evals.md).

## Where do I start?

One of three doors: [use it](l1-operator/01-what-this-is.md),
[run it](l2-operations/01-installing-and-configuring.md),
[build on it](lb-builder/01-why-memory-as-a-substrate.md), or
[verify it](l3-verification/01-how-to-use-this-course.md).
