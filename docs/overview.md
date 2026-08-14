# Overview

Brain Server is a **local-first semantic-memory and knowledge-graph server for AI
agents**. It gives an agent a second brain that lives on the operator's own
device — private, offline-capable, and deterministic.

The core idea is simple: **recall that never has to think.** Instead of asking a
language model whether to recall, and instead of paying an embedding API on every
read and write, Brain Server uses a **static, local embedding model** (`model2vec`
/ `minishlab/potion-retrieval-32M`) and a **deterministic retrieval pipeline**. No
LLM decides, no token is spent, no data leaves the device.

---

## Why it exists

Cloud memory services (Zep, Mem0, Letta Cloud) are powerful but carry three
structural costs that don't fit every use case:

1. **Per-query cost** — an LLM or embedding API is charged on every read and write.
2. **Data egress** — the agent's memory lives in someone else's datacenter.
3. **Network latency** — recall waits on a round-trip to the cloud.

Brain Server inverts all three: **zero per-query cost, zero data egress, zero
network latency on recall.** It is designed to run on a 4 GB ARM device (Jetson
Nano, Raspberry Pi 5, a small mini PC) drawing under 5 watts.

---

## Who it is for

1. **Edge / privacy-first agent builders** — people who can't or won't use an
   embedding API, and want the memory to live on the device.
2. **OpenClaw users who want memory without token cost** — a deterministic drop-in
   for the `active-memory` sub-agent, in the same memory slot.
3. **Knowledge-workers who think in domains** — health, business, code, and more as
   separate brains that cross-reference on a miss.

The full audience map — including **BPOs, in-house contact & support centers,
regulated enterprises (finance, healthcare, legal, government), edge/field
deployments, and delivery partners** — is in **[Who it's for — target audiences](./audiences.md)**,
with every segment marked shipped vs. planned (multi-client tenancy is the
v2.0 "Cortex" milestone).

---

## The five differentiators

### ① Zero-token, deterministic recall — no LLM in the loop
Every turn, the agent calls one `/recall` and gets the evidence to inject. No LLM
decides whether to recall, and no LLM extracts memories on write. Token accounting:
**0 decision tokens, 0 embedding tokens.** Only the capped returned snippets cost
context.

### ② Local static embeddings — offline, private, ~free on CPU
`potion-retrieval-32M` via `model2vec` is a **static** model — no transformer
forward pass, just token lookup. It runs in-process with no GPU and no network.
There is **no embedding API dependency**: embeddings are a local library call.

### ③ Per-domain knowledge graphs with automatic routing
Memories live in scoped domains (health, business, code, …), each with its own
entity/relationship graph. Routing between domains is **automatic** via per-domain
centroids — no manual tagging on ingest or query — with cross-domain fallback on a
miss.

### ④ Edge-first, memory-bounded, single binary
A **single Rust binary** with embedded SQLite + sqlite-vec. int8/binary vector
quantization (4–32× smaller), bounded connection pools, a configurable memory
ceiling (default 512 MiB, `CAPACITY_MAX_RSS_MIB`) on a
4 GB ARM device. No separate vector-DB process, no Python runtime, no Docker stack.

### ⑤ Native OpenClaw memory plugin
Ships as a `kind: "memory"` plugin occupying the memory slot, with per-agent opt-in
and group/channel exclusions for data-leakage prevention.

### ⑥ Human-gated write-back — meaningful control, not a rubber stamp
Nothing becomes permanent memory by default. A captured fragment is **scored, not
stored** (`POST /ingest/proposal`), and enters the store only after a human approves it —
optionally superseding the chunk it contradicts. The control room (Review panel, Memory
Operations panel with live SLA clocks + gate health, Agent Memory Register) is built to
make the operator a *critical evaluator*: raw evidence, sourcing prompt, and screen
verdict on every card, with every decision written to a tamper-evident audit chain. See
[**Human in the loop**](./human-in-the-loop.md).

---

## One-line positioning

> **Brain Server is the offline, deterministic, domain-graphed second brain for AI
> agents on the edge — zero embedding-API cost, zero decision tokens, one Rust
> binary, and a human gate on every write.**

---

## What's inside

- **Hybrid retrieval** — vector KNN + lexical FTS5 fused via Reciprocal Rank
  Fusion, with deterministic PRF expansion and full provenance.
- **Temporal evidence** — every ingest stamps `observed_at` / `valid_from` /
  `valid_to`; point-in-time recall returns the revision active at a timestamp.
- **Knowledge graph** — entities and relationships extracted from markdown,
  traversable and queryable, with faithful multi-hop explanations.
- **Governance** — append-only audit log, prompt-injection quarantine, write-back
  gating with human approval, GDPR export/purge/DSAR, and calibrated abstention.

See **[The memory lifecycle](./memory-lifecycle.md)** for the full end-to-end path a fact
takes from capture to storage, retention, recall, and erasure — and
[Human in the loop](./human-in-the-loop.md) for the review gate + erasure procedure.

Continue to the [Quickstart](./quickstart.md) to get running.
