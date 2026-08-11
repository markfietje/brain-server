# Brain Server — Local-First Semantic Memory & Knowledge-Graph Server for AI Agents

**Brain Server** is a **local-first semantic-memory and knowledge-graph server for AI agents.** It gives your agent a second brain that lives on *your* device — private, offline-capable, deterministic, and free to run. Built in Rust, it runs on a 4 GB ARM device (Jetson Nano, Raspberry Pi 5, a small mini PC) drawing **under 5 watts**, with **no GPU, no cloud, and no per-query cost**.

> **One-line positioning:** Brain Server is the offline, deterministic, domain-graphed second brain for AI agents on the edge — zero embedding-API cost, zero decision tokens, one Rust binary.

## Why Brain Server exists

Cloud memory services (Zep, Mem0, Letta Cloud) are powerful, but they carry three structural costs that don't fit every use case:

1. **Per-query cost** — an LLM or embedding API is charged on every read and write.
2. **Data egress** — the agent's memory lives in someone else's datacenter.
3. **Network latency** — recall waits on a round-trip to the cloud.

Brain Server inverts all three: **zero per-query cost, zero data egress, zero network latency on recall.** Recall "never has to think" — a static, local embedding model and a deterministic retrieval pipeline do the work without spending a single token.

## What it does at a glance

| Capability | What you get |
|---|---|
| **Hybrid retrieval** | Vector KNN + lexical FTS5 fused via Reciprocal Rank Fusion, with deterministic PRF expansion and full provenance |
| **Knowledge graph** | Entities + relationships extracted from markdown, traversable with faithful multi-hop explanations |
| **Temporal evidence** | Every ingest is time-stamped; point-in-time recall returns the fact as it was at any moment |
| **Governance** | Append-only SHA-256 audit chain, prompt-injection quarantine, human-gated write-back, GDPR export/purge/DSAR |
| **Deterministic honesty** | Calibrated abstention (says "I don't know"), claim span verification, reviewable proposals |
| **Client control surface** | A Dioxus web + desktop + mobile GUI — 12 panels covering review, recall, graph, create, subjects, security, audit, data, UMP, system console, and health, plus a ⌘K command palette |

## Who it is for

1. **Edge / privacy-first agent builders** — people who can't or won't use an embedding API, and want memory on the device.
2. **OpenClaw users who want memory without token cost** — a deterministic drop-in for the `active-memory` sub-agent, in the same memory slot.
3. **Knowledge-workers who think in domains** — health, business, code, and more as separate brains that cross-reference on a miss.

## Quick links

- **[Quickstart](Quickstart)** — get running in minutes
- **[Architecture](Architecture)** — how the pieces fit together
- **[Universal Memory Protocol](Universal-Memory-Protocol)**: portable memory for AI agents, implemented end to end
- **[API Reference](API-Reference)** — the full HTTP contract
- **[Security](Security)** — authentication, access control, data protections
- **[Governance & Compliance](Governance-and-Compliance)** — audit, DSAR, GDPR, ISO 42001 / NIST / SOC 2
- **[AI Literacy](AI-Literacy)** — the Art 4 operator playbook
- **[RFP Response Kit](RFP-Response-Kit)** — features mapped to enterprise RFP sections
- **[Features](Features)** — everything it can do
- **[Client GUI](Client-GUI)** — the visual control surface
- **[Complete Operator Console](Client-Complete-Console)** — the 12-panel v1.17.6→v1.17.8 client line
- **[Roadmap & Release History](Roadmap-and-Release-History)** — the version line
- **[FAQ](FAQ)** — common questions

## The five differentiators

**① Zero-token, deterministic recall — no LLM in the loop.** Every turn, the agent calls one `/recall` and gets the evidence to inject. Token accounting: **0 decision tokens, 0 embedding tokens.**

**② Local static embeddings — offline, private, ~free on CPU.** `potion-retrieval-32M` via `model2vec` is a *static* model — no transformer forward pass, just token lookup, in-process with no GPU and no network.

**③ Per-domain knowledge graphs with automatic routing.** Memories live in scoped domains (health, business, code…), each with its own graph. Routing between domains is automatic via per-domain centroids, with cross-domain fallback on a miss.

**④ Edge-first, memory-bounded, single binary.** One Rust binary with embedded SQLite + sqlite-vec, int8/binary vector quantization (4–32× smaller), bounded connection pools, ≤350 MB RSS on a 4 GB ARM device.

**⑤ Native OpenClaw memory plugin.** Ships as a `kind: "memory"` plugin occupying the memory slot, with per-agent opt-in and group/channel exclusions.

## Project facts

| | |
|---|---|
| **Language / stack** | Rust · Axum · rusqlite (WAL) · r2d2 · tokio |
| **Embedding model** | `minishlab/potion-retrieval-32M` (512-dim, static, ~120 MiB RSS) |
| **License** | MIT |
| **Latest version** | Server 1.20.1 "Shield" · Client 1.20.0 "Polish" |
| **Source** | [github.com/markfietje/brain-server](https://github.com/markfietje/brain-server) |

## Get started

```bash
cargo build --release --features bench
./target/release/brain-server
curl http://localhost:8765/health
```

See the **[Quickstart](Quickstart)** for the full walkthrough, or dive into **[Architecture](Architecture)** to understand how recall works under the hood.
