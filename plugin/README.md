# @markfietje/brain-server-openclaw

OpenClaw **memory plugin** for the Rust [brain-server](../). Deterministic
auto-recall, per-domain knowledge graphs, local static embeddings — over
loopback HTTP. **Zero decision/embedding tokens.**

> This is a **thin TypeScript shim**. All memory logic (model2vec embeddings,
> sqlite-vec int8/binary search, per-domain KGs, centroid auto-routing, hybrid
> FTS+vector recall, calibrated abstention, span verification) lives in the
> **Rust brain-server**. This plugin implements the OpenClaw SDK contract
> (hooks, tools, config, gating) and delegates everything else over HTTP. It
> never loads a model, never sees a vector, never touches SQLite.

## How it works

```
OpenClaw host
  │ before_prompt_build hook (deterministic, every turn)
  ▼
this plugin (TS)  ──POST /recall (loopback)──►  brain-server (Rust)
  │                                               │ model2vec (local, static)
  { prependContext }                              │ sqlite-vec int8 + FTS5
                                                  │ per-domain KGs + centroid routing
```

- **Deterministic recall:** `before_prompt_build` fires every turn → one `/recall`
  call → server embeds the query, auto-routes to the nearest domain centroid(s),
  falls back across domains on miss, and returns capped snippets. No LLM decides
  whether to recall.
- **Token accounting:** 0 decision tokens, 0 embedding tokens (local static
  model2vec). Only the capped returned snippets (~3) cost context. Static
  guidance goes to the provider-cacheable system prompt (`prependSystemContext`).
- **Calibrated abstention:** when retrieval quality is too low to support a
  claim, the server returns `decision: "low_confidence"` with no hits. The
  auto-recall hook fails open (injects nothing); the `memory_recall` tool tells
  the agent to clarify or fall back to web search instead of presenting a
  fabricated answer.

## Security defaults (OWASP LLM Top 10 + Lakera)

- **Per-agent opt-in** — empty `agents` list ⇒ disabled. Memory is a capability
  an agent must be granted (LLM06 least privilege).
- **Chat-type gating** — `direct` + `explicit` by default; `group`/`channel`
  excluded to prevent private-memory **data leakage** in shared contexts.
- **Recalled content = untrusted** — anti-injection banner on every block; the
  server also marks every hit `untrusted: true` (OWASP LLM01:2025). Memories are
  rendered as numbered citations, never executed as instructions; contested
  (`conflict`) hits are flagged.
- **Fail-open** on recall errors (never stall the agent); **fail-closed** on auth.

## Install

```bash
# 1. Run the Rust brain-server (loopback :8765), with auth if configured
AUTH_TOKEN=<your-token> brain-server &

# 2. Install the plugin into OpenClaw
openclaw plugins install @markfietje/brain-server-openclaw

# 3. Occupy the memory slot
#    in openclaw config: plugins.slots.memory = "brain-server"
```

Restart the gateway after installing. Min host version: `2026.5.31`.

## Config

```jsonc
{
  "baseUrl": "http://127.0.0.1:8765",
  "authToken": "<AUTH_TOKEN>",            // must match the server's AUTH_TOKEN / AUTH_TOKEN_FILE
  "agents": ["main"],                     // per-agent opt-in; empty = disabled
  "allowedChatTypes": ["direct", "explicit"],
  "autoRecall": true,                     // deterministic per-turn recall
  "autoCapture": false,                   // store durable facts after a turn
  "strictDomain": false,                  // false = cross-domain fallback on miss
  "autoRecallTopK": 3,
  "autoRecallTimeoutMs": 5000
}
```

## Tools

| Tool | Purpose |
| ---- | ------- |
| `memory_recall` | Hybrid semantic + lexical recall. Optional power-tools: `domain`, `source`, `since`, `lex`, `vec`, `hyde`, `intent`. Returns numbered untrusted citations; surfaces `low_confidence` abstention. |
| `memory_store` | Save a durable fact, optionally with `entities[]`/`relations[]` for the knowledge graph. |
| `memory_forget` | Delete a memory by id (404 → "Not found"). |
| `memory_verify` | Deterministic span verification (no LLM): is a claim literally supported by a chunk's text? Use before acting on a recalled fact. |
| `memory_get` | Fetch the full stored text behind a recalled snippet by id. |
| `memory_graph_entity` | Look up an entity and its one-hop knowledge-graph relations. |

## Files

| File | Purpose |
|------|---------|
| `index.ts` | Plugin entry: `definePluginEntry`, hooks, tools, service |
| `src/config.ts` | Typebox schema + resolved config + defaults |
| `src/brain-client.ts` | Thin typed HTTP client → Rust brain-server (no logic) |
| `src/gating.ts` | OWASP/Lakera access gating (per-agent + chat-type) |
| `src/format.ts` | Recall formatting + anti-injection banner + capture heuristics |
| `openclaw.plugin.json` | Manifest (`kind: "memory"`, contracts, config) |
| `package.json` | Package metadata, min host version, plugin API compat |
| `test/plugin.test.ts` | Integration: hook/tool flow against a mocked Rust server (`fetch`) |
| `src/*.test.ts` | Unit tests: config, gating, format, brain-client transport |

## Testing

The plugin is a **thin HTTP shim**, not an in-process plugin like
`memory-lancedb`. So tests mock only `fetch` (standing in for the Rust
server's `/recall`, `/ingest`, `/memory/{id}`, `/verify`, `/get/{id}`,
`/graph/entity/{name}`), never LanceDB or an embedding provider.

Tests run inside the **OpenClaw workspace** (where `@openclaw/plugin-sdk`
resolves as a `workspace:*` dependency). Keep this directory in sync with the
bundled copy at openclaw's `extensions/brain-server/`, then run from the
openclaw repo root:

```bash
pnpm test extensions/brain-server
```

What the suite covers (brain-server-specific):
- **Deterministic recall** — `before_prompt_build` issues exactly ONE
  `POST /recall` and injects `prependContext`.
- **Fail-open contract** — network/HTTP-500 failures never stall the agent;
  `low_confidence` abstention injects nothing.
- **Per-agent + chat-type gating** — group blocked, empty-agents disabled
  (OWASP LLM06; a capability `memory-lancedb` does not have).
- **Error surfacing** — tools report 404 vs 500 distinctly to the agent.
- **Wire-contract alignment** — snake_case responses (`domains_searched`,
  `entities_added`) parse into the plugin's camelCase shapes.

## Why not a skill or a recall sub-agent?

- A **skill** is LLM-mediated: the model decides when to recall (tokens,
  unreliability).
- `active-memory` runs a **blocking sub-agent** (a second LLM) to predict recall
  (tokens, non-determinism).
- This plugin does **deterministic injection** in plugin code — strictly better.

See [../PLUGIN_INTEGRATION.md](../PLUGIN_INTEGRATION.md) for the full contract.
