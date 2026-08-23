# Dual-era MCP without the handshake tax

*2026. Two live MCP spec generations, one stdio binary — and neither generation pays for the other's ceremony.*

The Model Context Protocol ecosystem currently lives across two spec eras.
The 2026-07-28 revision made servers **stateless**: no `initialize` handshake,
per-request `_meta` carrying the protocol version, discovery via a plain
`server/discover` call. That's a genuine win — you can put a stateless MCP
endpoint behind any HTTP load balancer and stop caring which client holds
which session. But it has a migration cost most implementations handle badly:
**every mainstream SDK client still speaks the older dialect**, initializes
first, and sends bare tool calls afterwards. A server that enforces the new
rules unconditionally doesn't look modern — it looks broken to every client
that exists today. We shipped through exactly this failure mode and fixed it
by making the era a property of the *request*, not of the server.

## One binary, two dialects, zero configuration

`brain-server`'s MCP surface (`mcp`) is a small Rust binary — JSON-RPC 2.0 over
newline-delimited stdio, translating tool calls into authenticated HTTP
against the store. No MCP framework dependency; the protocol surface is small
enough to hold in your head and audit in an afternoon. It dispatches each
incoming line by shape:

- **A request whose params carry `_meta`** is treated as modern. The meta is
  validated strictly — a supported `protocolVersion` (`2026-07-28`, or
  `2025-11-25` for callers pinning the older revision) plus a
  `clientCapabilities` object — then dispatched on the stateless surface,
  answered with the `resultType: "complete"` envelope and `_meta.serverInfo`.
- **A bare `initialize`** selects legacy semantics *for that stdio process*:
  subsequent bare `tools/list` / `tools/call` requests dispatch without meta
  requirements, and responses keep the classic JSON-RPC shape the client's
  SDK expects. This is precisely what `@modelcontextprotocol/sdk` clients do
  today — they connect, handshake once, then call tools plainly.
- **Neither**: rejected with `-32602` naming what was missing. Ambiguity is
  refused, never guessed.

No flag chooses the era. The client's own behavior declares it, and mixed
fleets — last quarter's agent build next to this month's — work against the
same binary without an operator ever thinking about protocol revisions.

## Discovery that respects the cache

Both eras get the same capability document, and because that document is a
compile-time constant, the server advertises honest caching hints instead of
making every client re-fetch: `server/discover` carries a one-hour TTL,
`tools/list` five minutes, both `cacheScope: public`. Twelve tools today —
three memory verbs, nine UMP verbs — so the tool table is also static and the
TTL claim is truthful rather than aspirational. Statelessness plus cacheable
discovery is what makes the "no handshake tax" claim economic, not just
compatible: a fleet of agents can share one warm discovery document instead of
each connection re-learning the world.

## The security posture rides along

Serving two eras doubles the input grammar, so the boundary hardening matters
more, not less:

- Client-controlled strings reflected in errors — a hostile tool name or
  protocol version — are truncated and hex-escaped in **both**
  `error.message` and `error.data`. An MCP host injects these messages into
  the calling model's context; a raw echo would be a prompt-injection carrier
  aimed at your own agent.
- Unsupported versions answer with `-32022` and a `supported` array, so a
  mismatch is diagnosable in one round trip.
- Stdin lines are capped at 1 MiB before parsing — `read_line` grows without
  bound otherwise, and a hostile parent process shouldn't own your RSS.
- Auth inherits the store's bearer ladder (`BRAIN_TOKEN_FILE` → env → default
  install path), sending exactly one slot of a rotation file.

## Why this wins the review

- **Your integration matrix stops being a negotiation.** Old SDKs and new
  stateless callers interoperate today; the era question never reaches your
  ticket queue.
- **Stateless where it pays.** Discovery and listing are static and cached;
  the only per-session state is which dialect a stdio peer selected, and that
  dies with the pipe.
- **Auditable surface.** Hand-rolled means enumerable: two eras, twelve
  tools, three auth sources, every rejection reason pinned by test.

**The takeaway:** when a protocol you depend on revises itself, the winning
server posture isn't "upgrade everyone" (you can't) and isn't "freeze forever"
(you shouldn't). It's a dispatcher that reads the caller's era off the wire
and serves both faithfully — with the strictness turned up, not down, because
two grammars means twice the injection surface.

*The dispatcher is [`src/bin/mcp.rs`](https://github.com/markfietje/brain-server/blob/main/src/bin/mcp.rs);
tool-level docs live in the [MCP server guide](../mcp.md), and the OpenClaw
plugin wiring that uses it is covered in
[OpenClaw integration](../openclaw-integration.md).*
