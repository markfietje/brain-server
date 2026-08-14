# MCP Server (Model Context Protocol)

Brain Server ships a **Model Context Protocol (MCP)** server as a separate
binary, `mcp`. It speaks **JSON-RPC 2.0 over stdio** and translates MCP tool
calls into HTTP requests against a running brain-server — so any MCP-capable
host (Claude Desktop, IDEs, agent frameworks) can search, recall, and write to
the same memory the CLI and HTTP API use.

This page is verified against `src/bin/mcp.rs`.

## Why a separate binary

`mcp` is deliberately thin: it is a **protocol shim**, not a second
implementation. Every tool maps 1:1 onto the brain-server HTTP API. There is no
retrieval logic in the MCP binary — it forwards, so the honest guarantees of the
server (deterministic recall, no LLM in the loop, PII read-path masking, audit)
hold no matter how you reach the store.

## Building & running

`mcp` is built from the same `Cargo.toml` as the server (it is not
feature-gated).

```bash
cargo build --release
# against a running server on 127.0.0.1:8765 (the default base URL)
./target/release/mcp
```

The binary reads the same auth token resolution as the CLI
(`AUTH_TOKEN_FILE` → `AUTH_TOKEN` → default file) and forwards bearer auth on
each outgoing request. Point your MCP host at the `stdin`/`stdout` of the `mcp`
process.

## Protocol surface

- **Transport:** JSON-RPC 2.0 over stdio (line-delimited).
- **Dual-era negotiation.** The modern (final 2026-07-28) spec is
  **stateless** — per-request `protocolVersion` + `clientCapabilities`, no
  `initialize` handshake. For legacy (2025-11-25) clients, an `initialize`
  request selects the legacy semantics. The server name is `brain-server-mcp`;
  the version is `env!("CARGO_PKG_VERSION")`.
- **`tools/list`** is static and identical for every caller (compile-time
  constant — no external calls, no per-request query).
- **Errors:** unknown tool names / bad params come back as JSON-RPC errors with
  a `message` the host injects into the calling LLM's context, so a bad call is
  surfaceable rather than silently swallowed.

## Tools

The tool list (verified from `src/bin/mcp.rs` `method_tools_list`):

| Tool | Maps to | Purpose |
|---|---|---|
| `brain_search` | `POST /search` (hybrid) | Hybrid semantic + lexical search; `query`, `limit`, `phrases`, `exclude`, `code`, `sources`, `source`, `since`, `intent`, `provenance` |
| `brain_recall` | `POST /recall` | Deterministic end-to-end recall (embed → hybrid); alias of `brain_search`; adds `domain`, `min_relevance` etc. `limit` 1..100 |
| `brain_ingest` | `POST /ingest` | Write a memory; accepts `content`, optional `title`, `source`, explicit `entities[]`/`relations[]`, `domain` |
| `ump.capabilities` | `GET /ump/capabilities` | UMP 1.0 negotiation: conformance level, kinds, bindings, retrieval signals, `max_recall`, `writable`, `audit` |
| `ump.remember` | `POST /ump/remember` | Store a UMP memory record |
| `ump.get` | `GET /ump/memory/{id}` | Read one record by id (integrity re-verified; others' rows §2.7-redacted) |
| `ump.recall` | `POST /ump/recall` | Ranked recall with per-result signals (`filter.kind`, `filter.valid_at`) |
| `ump.revise` | `POST /ump/revise` | Patch a record; stored as a new revision, old chunk expired via supersession |
| `ump.forget` | `POST /ump/forget` | Soft (default) or hard erase (`hard: true` runs the v1.14 erase path) |
| `ump.feedback` | `POST /ump/feedback` | Record outcome feedback (`followed`/`overridden`/`ignored`/`contradicted`) |
| `ump.audit` | `POST /ump/audit` | Recent hash-chained audit rows |
| `ump.audit.verify` | `GET /ump/audit/verify` | Full audit-chain integrity verification |

There are **12 tools**: three `brain_*` retrieval/write tools and nine
`ump.*` governance/data tools.

### Example

A modern (stateless) tool call:

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}},"name":"brain_recall","arguments":{"query":"how do we onboard"}}}' \
  | ./target/release/mcp
```

A legacy client selects the handshake mode first:

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"host","version":"1.0"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | ./target/release/mcp
```

## Relation to the UMP and OpenClaw tools

`mcp` is one of three ways an agent reaches the store:

| Surface | Transport | Tools |
|---|---|---|
| **AMCP binary (`mcp`)** | JSON-RPC 2.0 / stdio | `brain_search`, `brain_recall`, `brain_ingest`, `ump.*` |
| **OpenClaw plugin** | loopback HTTP | `memory_recall`, `memory_store`, `memory_verify`, `memory_get`, `memory_graph_*`, `memory_procedure_*`, `memory_decision_evaluate` |
| **HTTP API** | HTTP/JSON | Everything in the [API reference](./api.md) |

The **UMP tools** (`ump.*`) expose the Universal Memory Protocol's
memory/capability surfaces over MCP; the **UMP document**
([./universal-memory-protocol.md](./universal-memory-protocol.md)) specifies the
contract those tools implement.

## Security notes

- The MCP binary is **clientside** — it performs no listening, no network binds;
  it only makes outbound HTTP calls to the configured server, inheriting the
  server's auth, PII redaction, and audit on every read/write.
- It applies the same token-file resolution and never logs the token.
- There is no separate credential; whoever can invoke the binary acts as the
  configured principal on the server.

## Next steps

- **[Universal Memory Protocol](./universal-memory-protocol.md)** — the `ump.*` contract.
- **[API reference](./api.md)** — every endpoint the tools forward to.
- **[OpenClaw integration](./openclaw-integration.md)** — the agent-facing plugin surface.