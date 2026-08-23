# @markfietje/brain-server-openclaw

OpenClaw **memory plugin** for the Rust [brain-server](https://github.com/markfietje/brain-server). Deterministic
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
- **Provenance-labeled recall (v1.27.12 / plugin 0.4.3)** — each hit renders a
  deterministic `[src: · mk: · lb: · reg:]` line (source / memory kind / lawful
  basis / region) inside the `UNTRUSTED` fence; labels run through
  `sanitizeForBlock`, so recalled content can neither forge its attribution nor
  break the fence markers.
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
  // Token resolution: BRAIN_TOKEN_FILE (path to a 0600 token) > BRAIN_TOKEN > authToken (legacy)
  "authToken": "<AUTH_TOKEN>", // legacy fallback — prefer an env source; never required
  "agents": ["main"], // per-agent opt-in; empty = disabled
  "allowedChatTypes": ["direct", "explicit"],
  "autoRecall": true, // deterministic per-turn recall
  "autoCapture": false, // store durable facts after a turn
  "captureMode": "proposal", // route captures through human review (default)
  "strictDomain": false, // false = cross-domain fallback on miss
  "autoRecallTopK": 3,
  "autoRecallTimeoutMs": 5000,
  "autoRecallGraph": false, // opt IN to the graph-PPR third recall leg (sent explicitly: false disables it, whatever the server default)
  "proposalTools": false, // expose proposal review tools to the agent
}
```

> **Token resolution (0.4.5):** the plugin never writes a token. It resolves the
> bearer via a ladder mirroring the `brain` CLI — `BRAIN_TOKEN_FILE` (a 0600 file;
> the token never appears in config or env dumps) → `BRAIN_TOKEN` (env) →
> `authToken` in the plugin config (legacy fallback). Config wins only when no env
> source is set, so rotating via env never fights a stale stored value; an
> unreadable token file degrades loudly to the next rung, never silently to a
> weaker source.
>
> **Two-token pattern (Seatbelt):** the server token file is whitespace-split.
> Line 1 is the OPERATOR token (approvals, erasure — it must never enter
> openclaw config). Line 2, provisioned by `scripts/install-service.sh` as the
> agent token, is what this plugin should carry: point `BRAIN_TOKEN_FILE` at a
> 0600 file holding it. Paired with the server's `BRAIN_WRITE_POSTURE=review`,
> the agent's writes land as proposals for operator approval — agents propose,
> operators dispose.

## Tools

| Tool                       | Purpose                                                                                                                                                                                                                     |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `memory_recall`            | Hybrid semantic + lexical recall. Power overrides `domain`/`source`/`since`/`lex`/`vec`/`hyde`/`intent`; advanced `at`/`asOf`/`memoryKind`/`minRelevance`/`graph`/`maxContextTokens`. Surfaces `low_confidence` abstention. |
| `memory_store`             | Save a durable fact, optionally with `entities[]`/`relations[]` for the KG. Default `captureMode: "proposal"` → human review.                                                                                               |
| `memory_verify`            | Deterministic span verification (no LLM): is a claim literally supported by a chunk's text? Use before acting on a recalled fact.                                                                                           |
| `memory_get`               | Fetch the full stored text behind a recalled snippet by id.                                                                                                                                                                 |
| `memory_graph_entity`      | Look up an entity and its one-hop knowledge-graph relations.                                                                                                                                                                |
| `memory_graph_traverse`    | Multi-hop KG traversal (causal subgraphs `kind="causes:"`, bi-temporal `at`, explained paths).                                                                                                                              |
| `memory_proposal_list`     | List captures awaiting human review. Gated behind `proposalTools`.                                                                                                                                                          |
| `memory_proposal_decide`   | Approve/reject a captured proposal. Gated behind `proposalTools`.                                                                                                                                                           |
| `memory_procedure_get`     | Fetch the ordered steps of a runbook/procedure.                                                                                                                                                                             |
| `memory_procedure_store`   | Create a runbook/procedure with ordered steps (direct write, server-screened).                                                                                                                                              |
| `memory_decision_evaluate` | Deterministically evaluate a stored decision rule against numeric variables (no LLM).                                                                                                                                       |

> No `memory_forget` tool — erasure is a human action (operator console / HTTP API), removed
> v1.20.25. The plugin also feeds brain-server hits into the stock `memory_search`/`memory_get`
> via `registerMemoryCorpusSupplement` (non-exclusive unified search). See
> [`docs/openclaw-integration.md`](https://github.com/markfietje/brain-server/blob/main/docs/openclaw-integration.md)
> for the procedural-memory scenarios (runbooks, decision trees).

## Files

| File                   | Purpose                                                            |
| ---------------------- | ------------------------------------------------------------------ |
| `index.ts`             | Plugin entry: `definePluginEntry`, hooks, tools, service           |
| `src/config.ts`        | Typebox schema + resolved config + defaults                        |
| `src/brain-client.ts`  | Thin typed HTTP client → Rust brain-server (no logic)              |
| `src/gating.ts`        | OWASP/Lakera access gating (per-agent + chat-type)                 |
| `src/format.ts`        | Recall formatting + anti-injection banner + capture heuristics     |
| `openclaw.plugin.json` | Manifest (`kind: "memory"`, contracts, config)                     |
| `package.json`         | Package metadata, min host version, plugin API compat              |
| `test/plugin.test.ts`  | Integration: hook/tool flow against a mocked Rust server (`fetch`) |
| `src/*.test.ts`        | Unit tests: config, gating, format, brain-client transport         |

## Testing

The plugin is a **thin HTTP shim**, not an in-process plugin like
`memory-lancedb`. So tests mock only `fetch` (standing in for the Rust
server's `/recall`, `/ingest`, `/memory/{id}`, `/verify`, `/get/{id}`,
`/graph/entity/{name}`), never LanceDB or an embedding provider.

Run from the openclaw repo root:

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
- **Wire-contract alignment** — snake_case responses (`domains_searched`,
  `entities_added`) parse into the plugin's camelCase shapes.
- **Error surfacing** — tools report 404 vs 500 distinctly to the agent.

## Why not a skill or a recall sub-agent?

- A **skill** is LLM-mediated: the model decides when to recall (tokens,
  unreliability).
- `active-memory` runs a **blocking sub-agent** (a second LLM) to predict recall
  (tokens, non-determinism).
- This plugin does **deterministic injection** in plugin code — strictly better.

For the full wire contract (request/response shapes, field bounds, error
envelope), see the brain-server repo's `API_CONTRACT.md` and `GET /openapi.yaml`.

## Changelog

See [`CHANGELOG.md`](./CHANGELOG.md). Released from the openclaw repo at
`extensions/brain-server`; mirror under `plugin/` in the brain-server repo.
