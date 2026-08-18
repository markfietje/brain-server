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

## Install & requirements

The `mcp` binary ships from the same `Cargo.toml` as the server — build it once
and it lives next to the other binaries:

```bash
cargo build --release --bin mcp
```

What you need to run it:

1. **A running brain-server** on loopback (default `http://127.0.0.1:8765`).
   Override the base URL with `BRAIN_URL` if the server is elsewhere. The MCP
   binary is **clientside only** — it makes outbound HTTP calls to the server
   and performs no listening/binds itself.
2. **Auth (only if the server requires a bearer).** The token resolves via the
   CLI ladder, in order: `BRAIN_TOKEN_FILE` (path to a 0600 secret file) →
   `BRAIN_TOKEN` (env) → `~/.config/brain-server/auth-token` (the default
   install path written by `scripts/install-service.sh`). If none resolve, the
   binary connects unauthenticated (the server's loopback-only default).
3. **An MCP-capable host** (Claude Desktop, an IDE, an agent framework). Point
   it at the `stdin`/`stdout` of the `mcp` process — it's a stdio server, so
   there is nothing to install into the OS; the host spawns it.

You can smoke-test it from a shell (a modern, stateless request is the example
further down): pipe one JSON-RPC line into `./target/release/mcp` and read the
JSON-RPC response on stdout.

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

## DeepSeek Harness (dsh)

DeepSeek Harness (`dsh`) uses an **everything-is-a-plugin** architecture built on
Cordis. Rather than ship one bespoke adapter per memory system, it exposes a
generic **MCP client bridge** (`@deepseek-ai/dsh-mcp-client`) and lets you pick
the memory server — the documented slot for a "third-party memory MCP server"
(its own `examples/mcp-memory` ship Memorix, MCP Reference Memory, and Engram
this way). Brain Server's `mcp` binary is a drop-in for that slot.

**Alignment with dsh's expectations**

- **Protocol.** dsh's bridge targets the modern (2026-07-28) MCP spec with
  `server/discover`. `mcp` implements that *and* the legacy (2025-11-25)
  handshake, advertising `supportedVersions: ["2026-07-28","2025-11-25"]`, so
  discovery and `tools/list` work under either era. Tools register in dsh as
  `mcp__brain-server__<tool>`.
- **Responsibility boundary.** dsh starts the server process and discovers tools;
  the provider owns install, storage, and supervision. `mcp` is clientside only
  (no listening, no network binds) and inherits the server's auth, PII masking,
  and audit — exactly the thin, provider-owned component dsh expects.
- **Standard.** The `ump.*` tools implement the
  [Universal Memory Protocol](./universal-memory-protocol.md) at **UMP 1.0 / L3**
  (13/13 reference checks, CI-pinned), so dsh-written memory is portable and
  verifiable, not locked to this store.

### Pinned install

dsh starts the binary but is **not a package manager** — you must install and
pin `mcp` yourself:

```bash
# 1. Build the MCP binary from this repo (same Cargo.toml as the server).
cargo build --release --bin mcp

# 2. Install next to the other binaries.
install -m 0755 target/release/mcp ~/.local/bin/mcp

# 3. macOS only: strip the Gatekeeper provenance xattr that SIGKILLs (exit 137)
#    on first exec of a freshly-copied executable, or reinstall via
#    scripts/install-service.sh.
xattr -dr com.apple.provenance ~/.local/bin/mcp 2>/dev/null || true

# 4. Confirm it answers the modern handshake before wiring into dsh.
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}' \
  | ~/.local/bin/mcp
```

### dsh overlay

dsh wires a memory server in with a one-file Cordis overlay that inserts a single
`@deepseek-ai/dsh-mcp-client` row (the shape dsh ships for its own memory
examples). Save as e.g. `brain-server.cordis.yml` and select it via
`--config`:

```yaml
# brain-server.cordis.yml — one memory MCP server for a running brain-server.
- insert:
    - id: memory-brain-server
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: brain-server
        transport: stdio
        command: mcp                    # or an absolute path to the pinned binary
        args: []
        cwd: !!js process.cwd()
        # env is inherited from the ambient environment (dsh scrubs DSH_* and
        # credential-shaped vars). Add overrides only as needed:
        #   BRAIN_URL: http://127.0.0.1:8765   # default; set if server is elsewhere
        #   BRAIN_TOKEN_FILE: /path/to/0600-secret   # or BRAIN_TOKEN
```

Prerequisites before it will discover tools: a **running brain-server** on
`BRAIN_URL` (default `http://127.0.0.1:8765`), and if it requires auth, a bearer
resolvable via the CLI ladder (`BRAIN_TOKEN_FILE` → `BRAIN_TOKEN` →
`~/.config/brain-server/auth-token`). With the server reachable, dsh discovers
the 12 tools (`brain_search`, `brain_recall`, `brain_ingest` + nine `ump.*`) and
registers them as `mcp__brain-server__*`.

## Next steps

- **[Universal Memory Protocol](./universal-memory-protocol.md)** — the `ump.*` contract.
- **[API reference](./api.md)** — every endpoint the tools forward to.
- **[OpenClaw integration](./openclaw-integration.md)** — the agent-facing plugin surface.
- **[DeepSeek Harness (dsh) and Brain Server](./blog/10-dsh-deepseek-harness.md)** — the background post.