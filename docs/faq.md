# FAQ

Frequently asked questions about **Brain Server** — the local-first semantic-memory and knowledge-graph server for AI agents.

## General

**What is Brain Server?**
A local-first semantic-memory and knowledge-graph server for AI agents. It gives an agent a second brain that lives on the operator's own device — private, offline-capable, deterministic, and free to run.

**Is it really free?**
Yes — **zero per-query cost**. Recall uses a *static, local embedding model* and a deterministic pipeline. There is no LLM or embedding API charged on every read and write. Token accounting: **0 decision tokens, 0 embedding tokens.**

**Where does my data live?**
On your device. There is no cloud and no telemetry to third parties. Outbound HTTP is opt-in and off unless configured (an Art 19 DSAR webhook and an optional system-alert webhook).

**What does it run on?**
Anything Rust compiles to. It's designed for 4 GB ARM edge devices (Jetson Nano, Raspberry Pi 5, a mini PC) drawing **under 5 watts**, but it runs on any macOS/Linux host.

## Usage

**How do I install it?**
Build from source with `cargo build --release --features bench`, run `./target/release/brain-server`, and hit `http://localhost:8765`. See the **[Quickstart](./quickstart.md)**.

**How do I add memory?**
Ingest markdown with `POST /ingest/markdown`, structured data with `POST /ingest`, or memories with `POST /ingest/memory`. `[[relation::entity]]` links build the knowledge graph.

**How do I recall?**
Call `POST /recall` with a `QueryDoc`, or use `brain query "..."`. See **[Retrieval & Recall](./retrieval-and-recall.md)**.

**Is there a GUI?**
Yes — a Dioxus web + desktop + mobile app served at `/app`. See the **[Client GUI](./client-gui.md)**.

**Does it work with OpenClaw?**
Yes — Brain Server is the memory backend for OpenClaw via a `kind: "memory"` plugin. See the **[OpenClaw Integration](./openclaw-integration.md)** page.

## Capability

**Does it use an LLM?**
No. Retrieval, graph building, classification, and span verification are all **deterministic** — no LLM in the loop. Static embeddings via `model2vec`.

**Can it say "I don't know"?**
Yes. **Calibrated abstention**: when retrieval quality is too low, `/recall` returns `{decision: "low_confidence", hits: []}` instead of top-1 garbage.

**Can it forget?**
Yes, deliberately and auditably. `POST /purge` deletes by id/owner with a tombstone + audit row; the **DSAR workflow** locates, exports, purges, and issues a chain-verifiable deletion certificate. Nothing is deleted autonomously.

**Can I see why a result was returned?**
Yes. Every result carries provenance, and `/recall?trace=true` records a replayable decision path. See **[Retrieval & Recall](./retrieval-and-recall.md)**.

## Security & compliance

**How is it secured?**
Loopback-safe by default; two auth modes (opaque bearer or JWT/JWS); a deny-by-default AuthZ layer; an append-only SHA-256 audit chain. See **[Security](./security.md)**.

**Is it compliant?**
It maps to ISO/IEC 42001, NIST AI RMF, SOC 2, GDPR, CCPA/CPRA, and the Philippines DPA — as a **documented engineering posture**, not a certification. See **[Governance & Compliance](./compliance.md)**.

**Where do I report a vulnerability?**
Use the GitHub Security Advisories tab. Do **not** file public issues for security findings.

## Troubleshooting

**I get `exit 137` on first run (macOS).**
A `com.apple.provenance` xattr makes Gatekeeper SIGKILL freshly copied executables. Use `scripts/install-service.sh` — it strips the xattr. See **[Installation](./deployment.md)**.

**The server won't bind `0.0.0.0`.**
By design. Set `BIND_PUBLIC=1` to bind publicly. See **[Configuration](./configuration.md)**.

## Next steps

- **[Quickstart](./quickstart.md)** — get running.
- **[Glossary](./glossary.md)** — terminology.
- **[Contributing](./contributing.md)** — how to help.
