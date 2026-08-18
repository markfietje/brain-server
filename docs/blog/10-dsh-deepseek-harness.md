# DeepSeek Harness (dsh) meets Brain Server: agent memory as an MCP server

*2026. Why dsh's "everything is a plugin" design is the right host for a memory
server, and how Brain Server fits it without being a plugin.*

If you're running DeepSeek Harness (`dsh`) and you want it to actually
*remember*, the question isn't "is there a dsh memory plugin?" — it's **"which
MCP memory server do I point the generic bridge at?"** This post covers what dsh
is, why its plugin architecture is genuinely different, and how Brain Server's
MCP server connects to it as a first-class memory backend.

---

## What is DeepSeek Harness (dsh)?

**DeepSeek Harness (`dsh`)** is an open-source agent harness developed by
DeepSeek AI. It wraps a model — DeepSeek or any other — into a desktop agent
with tools, plugins, memory, and a Web UI (default `http://127.0.0.1:3080`). The
design is built on **Cordis**, a plugin framework whose architecture is described
in *[A Programming Paradigm for Spatiotemporal
Composability](https://github.com/cordiverse/paper)*.

The single sentence that matters: **dsh uses an architecture where everything is
a plugin.** Not "plugins are a feature." Everything — tools, memory, prompt
assembly, settings tabs, commands — is a composable plugin loaded into a Cordis
container.

## What makes dsh good and unique

Most harnesses bolt tools onto a fixed runtime. dsh flips the model. The
consequences are what make it worth a second look:

- **Composable, not monolithic.** Because everything is a Cordis plugin, you
  compose a harness from exactly the pieces you want. Want the model to speak
  HTTP but not touch the filesystem? You control that per-plugin, per-profile.
- **Profiles as plugin bundles.** dsh's profile system bundles plugins into
  presets — a "memory" profile pulls in a memory plugin, an "agentic" profile
  pulls in tools. This mirrors exactly how Brain Server's own
  [Profiles](./08-profiles-preview.md) work, which is a nice symmetry.
- **A generic MCP client instead of one-off integrations.** dsh does not write a
  bespoke adapter per memory system. It ships one `@deepseek-ai/dsh-mcp-client`
  bridge that discovers and registers any MCP server's tools. That is the
  deliberate, documented decision: rather than bake Memorix's API (or anyone's)
  into the product, dsh exposes the generic MCP boundary and lets *you* pick the
  memory server.
- **Client-side, scriptable, inspectable.** The CLI is real; configs are plain
  overlay files you can read. Nothing is hidden in a managed SaaS surface.

### The honest ceiling

dsh's generic MCP client **starts the server process but is not a package
manager**, and it does **not** re-create tools across MCP servers — each server
brings its own tool semantics. It also has no automatic reconnect if a child
transport closes. None of that is a defect; it's a deliberate responsibility
boundary (DSH owns lifecycle + discovery; the provider owns the server). The
practical consequence is that **you install and pin the memory server binary
yourself** — and that's exactly the part Brain Server makes trivial.

---

## Where your memory server enters

dsh ships *opt-in, default-off* overlay examples under `examples/mcp-memory`
(Memorix, MCP Reference Memory, Engram). Every file inserts exactly one
`@deepseek-ai/dsh-mcp-client` row. A "third-party memory MCP server" is the
**documented, first-class slot** — and Brain Server's `mcp` binary is a drop-in
candidate for that slot.

### What Brain Server's MCP server gives a dsh agent

Brain Server ships a **MCP server** as a separate `mcp` binary. It speaks
JSON-RPC 2.0 over stdio and translates MCP tool calls into HTTP calls against a
running brain-server. Point dsh's bridge at it and the agent gains:

| Tool | What it lets the agent do |
|---|---|
| `brain_search` | Hybrid semantic + lexical search over the whole store |
| `brain_recall` | Deterministic end-to-end recall (embed → hybrid) |
| `brain_ingest` | Write a memory with explicit entities/relations |
| `ump.remember` / `ump.get` / `ump.revise` / `ump.forget` | Full UMP record lifecycle: store, read, revise, erase |
| `ump.recall` | Ranked recall with per-result signals and bi-temporal `filter.valid_at` |
| `ump.feedback` | Record outcome feedback — the anti-rubber-stamp signal |
| `ump.audit` / `ump.audit.verify` | Inspect and verify the hash-chained audit trail |
| `ump.capabilities` | Negotiate the memory contract up front |

That is not just "a search tool." It is a **governed memory lifecycle** — write,
recall, revise, forget, audit — all behind one MCP server. For an agent harness,
the difference between "I can search" and "I can store, retrieve, revise, and be
audited" is the difference between a cache and a memory.

## The standard: UMP 1.0 / L3

The `ump.*` tools are not an ad-hoc API. They implement the
**[Universal Memory Protocol (UMP)](../universal-memory-protocol.md)** — an open
standard for portable agent memory. Brain Server's conformance is verified
against the reference suite (`@universalmemoryprotocol/core` 1.0.0): **13/13
checks, UMP 1.0 / L3**, re-run by CI on every push. With an operator key
configured, `GET /ump/capabilities` reports `conformance: "L3"` — the local
integrity layer with signed records and capability tokens.

Why this matters in a dsh context: **UMP is transport-agnostic.** It does not
say "you must use Brain Server." It says "here is the contract a portable memory
must meet." Because Brain Server implements that standard and exposes it over
MCP, the memory your dsh agent writes is **portable** — a UMP-compliant reader
on another host can read, verify, and reuse it without a shared database. That
is the lock-in-free memory the [no-lock-in post](./05-no-lock-in.md) argues for,
delivered.

### Does it align with dsh correctly?

Yes — on both sides of the boundary:

- **Protocol:** dsh's bridge targets the modern (2026-07-28) MCP spec with
  `server/discover`. Brain Server's `mcp` binary implements that **and** the
  legacy (2025-11-25) handshake, advertising `supportedVersions:
  ["2026-07-28","2025-11-25"]`. So discovery and `tools/list` work regardless of
  which MCP era the host speaks.
- **Responsibility boundary:** dsh starts the server and discovers tools; the
  provider owns install, storage, and supervision. Brain Server's `mcp` binary
  is **clientside only** — it performs no listening and no network binds, and it
  inherits the server's auth, PII read-path masking, and audit on every call. It
  is exactly the thin, provider-owned component the dsh boundary expects.
- **No vendor lock-in on either side:** if you replace Brain Server, dsh doesn't
  change — the generic bridge just points at a different memory server. If you
  replace dsh, your UMP memory comes with you.

---

## Connect it

A complete overlay + pinned install steps for the `mcp` binary are in the
[full dsh integration guide](../mcp.md#deepseek-harness-dsh). In short:

1. Build/pin the `mcp` binary (dsh starts it, it does not install it).
2. Point dsh at a running brain-server with `BRAIN_URL` + token.
3. Add a one-file Cordis overlay inserting a `@deepseek-ai/dsh-mcp-client` row.
4. Tools register as `mcp__brain-server__*`.

One macOS note (see the guide): the installed `mcp` may carry the
`com.apple.provenance` quarantine attribute, which SIGKILLs the process on first
exec (exit 137). Strip it with `xattr -dr com.apple.provenance ~/.local/bin/mcp`
once, or reinstall via `scripts/install-service.sh`, before pointing dsh at it.

---

## The bottom line

dsh's "everything is a plugin" architecture and its generic MCP bridge are the
right host for a memory server — not because dsh needs Brain Server, but because
the two share the same philosophy: **thin, composable, inspectable, and honest
about the responsibility boundary.** Brain Server connects to dsh not as a
plugin but as the thing dsh was designed to accept: a portable, standards-backed
(UMP L3), auditable memory MCP server.

Read the [full integration guide](../mcp.md) or the
[Universal Memory Protocol spec](../universal-memory-protocol.md) to go deeper.
