# Brain Server

**Your team's memory, on your machine. Zero dollars per query.**

Paste a resolution once, recall it the same way every time. A person has to approve before anything becomes permanent. One Rust binary, no cloud, no embedding API.

<p align="center">

[![Version](https://img.shields.io/badge/version-1.28.69-blue.svg)](#)
[![Docs](https://img.shields.io/badge/docs-brain--server-1f6feb.svg)](https://markfietje.github.io/brain-server/)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg?logo=rust)](#)
[![License: MIT](https://img.shields.io/github/license/markfietje/brain-server.svg)](#)
[![Cost](https://img.shields.io/badge/cost-%240%20per%20query-success.svg)](#)
[![Tests](https://img.shields.io/badge/tests-1408%20passed-brightgreen.svg)](#)

</p>

<p align="center">
Web + desktop + mobile GUI (Dioxus) · OpenAI-compatible embeddings · MCP server · OpenClaw plugin
</p>

## Try it in 30 seconds

A fresh `docker compose up` starts empty. Add one fact, then recall it:

```bash
docker compose up -d
curl http://127.0.0.1:8765/health

# Add one piece of knowledge
curl -X POST http://127.0.0.1:8765/ingest/markdown \
  -d '{"title":"Bignay","content":"Bignay is alternative to blueberry."}'

# Recall it
curl -X POST http://127.0.0.1:8765/recall \
  -d '{"query":"blueberry alternative","provenance":true}'
```

You get `decision: ok` with sources, or `low_confidence` when it is not sure. No guess. Fresh install has no data until you add some.

Prefer a local build?

```bash
cargo build --release --features bench,migrate,compliance-pack
./target/release/brain-server
# listens on 127.0.0.1:8765, data at ~/.openclaw/workspace/brain.db
```

## Why this instead of a cloud memory service

| Cloud memory | Brain Server |
|---|---|
| Pay per read and write | Zero per query. Local static embeddings. |
| Data in someone else's datacenter | Data stays in SQLite on your box. No telemetry. |
| Extra round trip | Sub 50ms p99 recall on loopback. |

## What it does

* **Same answer every time.** Hybrid retrieval. Vector KNN plus FTS5 via Reciprocal Rank Fusion. Deterministic.
* **Knows when it does not know.** Abstains with `low_confidence` instead of making something up. `POST /verify` checks a claim against the stored text.
* **Human has to approve.** Proposals are scored, a person approves, the approval locks to the exact bytes shown (`content_digest`). Stale approvals get a 409.
* **One daemon.** `brain-server` bundles SQLite plus sqlite-vec. Rust, Axum, Tokio. Runs anywhere Rust builds. CLI `brain`, MCP, bench in the same repo.
* **Plays with your agents.** OpenAI compatible `POST /v1/embeddings`, MCP tools `brain_search`, `brain_recall`, UMP 1.0 L3 (13 of 13), native OpenClaw plugin.

More in `docs/` if you want the deep dive: graph hops, bi-temporal `valid_from` to `valid_to`, audit chain, DSAR, case loop.

## Proof, not promises

* UMP 1.0 L3, 13 of 13 conformance checks, pinned in CI
* 1,408 tests passed, `cargo fmt` and `clippy -D warnings` clean
* Append only SHA-256 audit chain, `GET /audit/verify` to check it
* Maps to ISO 42001, NIST AI RMF, SOC 2, GDPR. See `COMPLIANCE.md`

Try the MCP server in two seconds:

```bash
curl -L -o mcp https://github.com/markfietje/brain-server/releases/latest/download/mcp-darwin-arm64
chmod +x mcp
echo '{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}' | ./mcp
```

## Docs

| | |
|---|---|
| Docs site | [markfietje.github.io/brain-server](https://markfietje.github.io/brain-server/) |
| API | `GET /openapi.yaml` at runtime and `API_CONTRACT.md` |
| Security | `SECURITY.md`, `THREAT_MODEL.md` |
| Compliance | `COMPLIANCE.md`, `docs/RFP_RESPONSE_KIT.md` |
| Trust map | `docs/trust/proof-map.md` - every claim traced to release and curl |
| Roadmap | `ROADMAP.md` |

## License

MIT 2026 Mark Fietje. See `LICENSE`. Issues welcome on GitHub.
