# Overview

**Brain Server** is a **local-first semantic-memory and knowledge-graph server for AI agents.** It gives an agent a second brain that lives on the operator's own device — private, offline-capable, and deterministic.

The core idea is simple: **recall that never has to think.** Instead of asking a language model whether to recall, and instead of paying an embedding API on every read and write, Brain Server uses a **static, local embedding model** (`model2vec` / `minishlab/potion-retrieval-32M`) and a **deterministic retrieval pipeline**. No LLM decides, no token is spent, no data leaves the device.

## Why it exists

Cloud memory services (Zep, Mem0, Letta Cloud) are powerful but carry three structural costs that don't fit every use case:

1. **Per-query cost** — an LLM or embedding API is charged on every read and write.
2. **Data egress** — the agent's memory lives in someone else's datacenter.
3. **Network latency** — recall waits on a round-trip to the cloud.

Brain Server inverts all three: **zero per-query cost, zero data egress, zero network latency on recall.** It is designed to run on a 4 GB ARM device (Jetson Nano, Raspberry Pi 5, a small mini PC) drawing under 5 watts.

## Who it is for

1. **Edge / privacy-first agent builders** — people who can't or won't use an embedding API, and want the memory to live on the device.
2. **OpenClaw users who want memory without token cost** — a deterministic drop-in for the `active-memory` sub-agent, in the same memory slot.
3. **Knowledge-workers who think in domains** — health, business, code, and more as separate brains that cross-reference on a miss.

## What's inside

- **Hybrid retrieval** — vector KNN + lexical FTS5 fused via Reciprocal Rank Fusion, with deterministic PRF expansion and full provenance.
- **Temporal evidence** — every ingest stamps `observed_at` / `valid_from` / `valid_to`; point-in-time recall returns the revision active at a timestamp.
- **Knowledge graph** — entities and relationships extracted from markdown, traversable and queryable, with faithful multi-hop explanations.
- **Governance** — append-only audit log, prompt-injection quarantine, write-back gating with human approval, GDPR export/purge/DSAR, and calibrated abstention.

## One-line positioning

> **Brain Server is the offline, deterministic, domain-graphed second brain for AI agents on the edge — zero embedding-API cost, zero decision tokens, one Rust binary.**

## Next steps

- Continue to the **[Quickstart](Quickstart)** to get running.
- Understand the internals in **[Architecture](Architecture)**.
- See everything it can do in **[Features](Features)**.
