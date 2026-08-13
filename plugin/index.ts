/**
 * Brain Server — OpenClaw memory plugin entry.
 *
 * THIN SHIM ARCHITECTURE (Rust engine + TS plugin):
 *   - The OpenClaw plugin SDK is TypeScript, so this module is TS. brain-server
 *     itself is Rust and stays the single source of truth for all memory logic
 *     (model2vec embeddings, sqlite-vec int8/binary search, per-domain KGs,
 *     centroid auto-routing). This plugin contains NO memory logic — it only
 *     implements the SDK contract (hooks, tools, config, gating) and delegates
 *     to the Rust server over loopback HTTP via BrainClient.
 *   - Deterministic recall: the `before_prompt_build` hook fires every turn,
 *     calls ONE endpoint (`/recall`), and injects the result. No LLM decides
 *     whether to recall (zero decision tokens); embeddings are local/static
 *     (zero embedding tokens). Only the capped returned snippets cost context.
 *
 * Verified contract (OpenClaw plugin SDK, 2026.6.x):
 *   - definePluginEntry({ id, name, description, configSchema, register(api) })
 *   - api.on("before_prompt_build", (event, ctx) => result|void, { timeoutMs })
 *       event: { prompt: string; messages: unknown[] }
 *       ctx:    { agentId?, sessionKey?, sessionId?, channel?, chatId?, trigger?, ... }
 *       result: { prependContext?, prependSystemContext?, appendContext?, ... }
 *   - api.registerTool(tool, { name })
 *   - api.registerService({ id, start(), stop() })
 *   - api.registerMemoryCapability({ promptBuilder })
 *   - api.resolvePath, api.logger, api.pluginConfig, api.runtime.config.current()
 */
import { definePluginEntry, type OpenClawPluginApi } from "openclaw/plugin-sdk/plugin-entry";
import { Type } from "typebox";
import { BrainClient, describeBrainError } from "./src/brain-client.js";
import { brainPluginConfigSchema, resolveConfig, type ResolvedBrainConfig } from "./src/config.js";
import {
  RECALL_ABSTENTION,
  STATIC_SYSTEM_GUIDANCE,
  formatRecallContext,
  latestUserText,
  looksCaptureWorthy,
  normalizeRecallQuery,
  sanitizeForBlock,
} from "./src/format.js";
import { deriveChatType, isRecallAllowed, type GateContext } from "./src/gating.js";

const PLUGIN_ID = "brain-server";

export default definePluginEntry({
  id: PLUGIN_ID,
  name: "Brain Server",
  description:
    "Local semantic-memory + knowledge-graph (Rust brain-server). Deterministic auto-recall, per-domain KGs, local static embeddings. Zero decision/embedding tokens.",
  configSchema: brainPluginConfigSchema,

  register(api: OpenClawPluginApi) {
    const cfg: ResolvedBrainConfig = resolveConfig(api.pluginConfig);
    const client = new BrainClient(cfg);

    // Live-config resolver: operators may change settings without a restart.
    const liveCfg = (): ResolvedBrainConfig => {
      try {
        const current = api.runtime.config?.current;
        if (!current) {
          return cfg;
        }
        // Plugin config is authoritative; re-resolve from the live object.
        return resolveConfig(api.pluginConfig);
      } catch {
        return cfg;
      }
    };

    api.logger.info(
      `${PLUGIN_ID}: registered (url: ${cfg.baseUrl}, autoRecall: ${cfg.autoRecall}, agents: ${cfg.agents.length})`,
    );

    // ------------------------------------------------------------------------
    // Memory slot capability — static, provider-cacheable system guidance.
    // promptBuilder returns a STATIC section (no per-query recall here — the
    // dynamic recall happens in before_prompt_build). Using prependSystemContext
    // for static guidance avoids per-turn token re-billing (prompt caching).
    // ------------------------------------------------------------------------
    api.registerMemoryCapability?.({
      promptBuilder: () => [STATIC_SYSTEM_GUIDANCE],
    });

    // ------------------------------------------------------------------------
    // Deterministic recall hook — the reason this costs zero decision tokens.
    // ------------------------------------------------------------------------
    api.on(
      "before_prompt_build",
      async (event, ctx) => {
        const c = liveCfg();
        if (!c.autoRecall || !c.enabled) {
          return undefined;
        }
        if (!event.prompt || event.prompt.length < c.minQueryLength) {
          return undefined;
        }

        const gate = mapCtx(ctx);
        const decision = isRecallAllowed(c, gate);
        if (!decision.allowed) {
          // Not an error — gated out by policy. Silent no-op.
          return undefined;
        }

        const querySource =
          latestUserText(Array.isArray(event.messages) ? event.messages : []) ?? event.prompt;
        const query = normalizeRecallQuery(querySource, c.recallMaxChars);
        if (query.length < c.minQueryLength) {
          return undefined;
        }

        try {
          const result = await client.recall({
            query,
            // Let the server auto-route via centroids; only force a domain if caller set one.
            ...(c.defaultDomain && c.defaultDomain !== "global" ? { domain: c.defaultDomain } : {}),
            ...(c.strictDomain ? { strictDomain: true } : {}),
            limit: c.autoRecallTopK,
            timeoutMs: c.autoRecallTimeoutMs,
          });
          if (!result.hits.length) {
            // Calibrated abstention (v1.5): the server returned no hits on
            // purpose because quality was too low. Fail-open — inject nothing.
            if (result.decision === "low_confidence") {
              api.logger.info?.(
                `${PLUGIN_ID}: recall abstained (low confidence) for "${query}"; skipping injection`,
              );
            }
            return undefined;
          }

          const block = formatRecallContext(result.hits);
          if (!block) {
            return undefined;
          }

          api.logger.info?.(
            `${PLUGIN_ID}: injecting ${result.hits.length} memories (domain=${result.domain ?? "auto"})`,
          );
          // Dynamic recall => prependContext (per-turn, NOT cached).
          // Static guidance is handled separately via registerMemoryCapability (cacheable).
          return { prependContext: block };
        } catch (err) {
          // FAIL-OPEN: never stall the agent on a memory error.
          api.logger.warn?.(`${PLUGIN_ID}: recall failed (${String(err)}); skipping injection`);
          return undefined;
        }
      },
      { timeoutMs: cfg.autoRecallTimeoutMs + 500 },
    );

    // ------------------------------------------------------------------------
    // autoCapture — store durable facts/decisions after a successful turn.
    // Off by default; gated by looksCaptureWorthy to avoid bloat.
    // ------------------------------------------------------------------------
    api.on("agent_end", async (event, ctx) => {
      const c = liveCfg();
      if (!c.autoCapture || !c.enabled) {
        return;
      }
      if (!event.success) {
        return;
      }
      const messages = event.messages;
      const gate = mapCtx(ctx);
      if (!isRecallAllowed(c, gate).allowed) {
        return;
      }

      try {
        const body = joinMessageTexts(messages).slice(0, 2000);
        for (const text of extractUserTexts(messages)) {
          if (!looksCaptureWorthy(text)) {
            continue;
          }
          // v1.20.1 "Shield" M2: route autoCapture through the human review
          // queue by default. `captureMode: "proposal"` submits to
          // POST /ingest/proposal (only becomes memory after a reviewer
          // approves); `captureMode: "direct"` keeps the old straight-to-
          // memory behavior (still screened by the server injection gate).
          if (c.captureMode === "direct") {
            await client.store({
              title: text.slice(0, 80),
              content: text,
              ...(c.defaultDomain && c.defaultDomain !== "global"
                ? { domain: c.defaultDomain }
                : {}),
              timeoutMs: c.requestTimeoutMs,
            });
          } else {
            await client.submitProposal({
              content: text,
              source: "agent_end",
              ...(body.length ? { sourcePrompt: body } : {}),
              timeoutMs: c.requestTimeoutMs,
            });
          }
        }
      } catch (err) {
        api.logger.warn?.(`${PLUGIN_ID}: capture failed (${String(err)})`);
      }
    });

    api.on("session_end", () => {
      // No per-session state to clean in this shim (cursors live server-side).
    });

    // ------------------------------------------------------------------------
    // Tools — explicit agent-callable. These supersede the legacy MCP skill.
    // ------------------------------------------------------------------------
    api.registerTool(
      {
        name: "memory_recall",
        label: "Memory Recall",
        description:
          "Search long-term memory. Use for past decisions, preferences, or previously discussed topics. Optionally scope by source or time, or override the semantic query.",
        parameters: Type.Object({
          query: Type.String({ description: "Search query" }),
          limit: Type.Optional(
            Type.Integer({ minimum: 1, maximum: 50, description: "Max results (default 5)" }),
          ),
          domain: Type.Optional(
            Type.String({ description: "Force a specific domain (auto-routing is default)" }),
          ),
          source: Type.Optional(Type.String({ description: "Filter by knowledge source id/name" })),
          since: Type.Optional(
            Type.String({
              description: "Only rows with created_at after this ISO-8601/RFC3339 time",
            }),
          ),
          lex: Type.Optional(
            Type.String({
              description: "Lexical (FTS5) query override: exact terms, phrases, -exclusions",
            }),
          ),
          vec: Type.Optional(Type.String({ description: "Semantic embedding-query override" })),
          hyde: Type.Optional(
            Type.String({ description: "Hypothetical-answer embedding override (beats vec)" }),
          ),
          intent: Type.Optional(
            Type.String({ description: "Free-form intent label, recorded for provenance" }),
          ),
        }),
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = (params ?? {}) as {
            query?: string;
            limit?: number;
            domain?: string;
            source?: string;
            since?: string;
            lex?: string;
            vec?: string;
            hyde?: string;
            intent?: string;
          };
          const query = normalizeRecallQuery(p.query ?? "", c.recallMaxChars);
          if (!query) {
            return {
              content: [{ type: "text" as const, text: "No query provided." }],
              details: { count: 0 },
            };
          }
          let result;
          try {
            result = await client.recall({
              query,
              ...(p.domain ? { domain: p.domain } : {}),
              limit: p.limit ?? 5,
              source: p.source,
              since: p.since,
              lex: p.lex,
              vec: p.vec,
              hyde: p.hyde,
              intent: p.intent,
              timeoutMs: c.requestTimeoutMs,
            });
          } catch (err) {
            // Tools surface the failure so the agent can react; recall errors
            // are not silent here (unlike the fail-open auto-recall hook).
            return {
              content: [
                { type: "text" as const, text: `Recall failed: ${describeBrainError(err)}` },
              ],
              details: { count: 0, error: describeBrainError(err) },
            };
          }
          if (result.decision === "low_confidence") {
            // Calibrated abstention (v1.5): empty by design, not a miss.
            return {
              content: [{ type: "text" as const, text: RECALL_ABSTENTION }],
              details: { count: 0, decision: result.decision },
            };
          }
          if (!result.hits.length) {
            return {
              content: [{ type: "text" as const, text: "No relevant memories found." }],
              details: { count: 0, decision: result.decision },
            };
          }
          return {
            content: [{ type: "text" as const, text: formatRecallContext(result.hits) }],
            details: {
              count: result.hits.length,
              decision: result.decision,
              domainsSearched: result.domainsSearched,
              // v1.20.25: if the runtime serializes tool `details` into the
              // model context, raw bidi/zero-width bytes would reach the model
              // verbatim — run every hit field through the same block boundary
              // the `content` path already uses (the v1.20.24 Sweep closed the
              // content seam; this closes the `details` seam).
              memories: result.hits.map((h) => ({
                ...h,
                title: h.title ? sanitizeForBlock(h.title) : h.title,
                content: sanitizeForBlock(h.content),
              })),
            },
          };
        },
      },
      { name: "memory_recall" },
    );

    api.registerTool(
      {
        name: "memory_store",
        label: "Memory Store",
        description:
          "Save a durable fact/decision to long-term memory. Optionally include entities/relations for the knowledge graph.",
        parameters: Type.Object({
          text: Type.String({ description: "Information to remember" }),
          title: Type.Optional(
            Type.String({ description: "Short title (default: first 80 chars)" }),
          ),
          domain: Type.Optional(Type.String()),
          entities: Type.Optional(
            Type.Array(
              Type.Object({
                name: Type.String(),
                type: Type.Optional(Type.String()),
              }),
            ),
          ),
          relations: Type.Optional(
            Type.Array(
              Type.Object({
                from: Type.String(),
                to: Type.String(),
                type: Type.String(),
              }),
            ),
          ),
        }),
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = (params ?? {}) as {
            text?: string;
            title?: string;
            domain?: string;
            entities?: unknown;
            relations?: unknown;
          };
          const text = (p.text ?? "").trim();
          if (!text) {
            return {
              content: [{ type: "text" as const, text: "No text provided." }],
              details: { stored: false },
            };
          }
          let res;
          try {
            // v1.20.25: the agent's write surface must respect the same
            // capture-mode rule as autoCapture. Default `captureMode:
            // "proposal"` queues the fact for HUMAN review (POST
            // /ingest/proposal) instead of writing it straight to memory — an
            // agent driving this tool can no longer persist arbitrary
            // instructions into long-term memory without a reviewer. Only an
            // operator who explicitly configured `captureMode: "direct"`
            // keeps the straight-to-memory path (still server-screened).
            if (c.captureMode === "direct") {
              res = await client.store({
                title: p.title?.trim() || text.slice(0, 80),
                content: text,
                ...(p.domain ? { domain: p.domain } : {}),
                ...(Array.isArray(p.entities) ? { entities: p.entities as never[] } : {}),
                ...(Array.isArray(p.relations) ? { relations: p.relations as never[] } : {}),
                timeoutMs: c.requestTimeoutMs,
              });
            } else {
              const prop = await client.submitProposal({
                content: text,
                source: "memory_store",
                ...(p.title?.trim() ? { sourcePrompt: p.title.trim() } : {}),
                timeoutMs: c.requestTimeoutMs,
              });
              res = { id: prop.id, status: prop.status };
            }
          } catch (err) {
            return {
              content: [
                { type: "text" as const, text: `Store failed: ${describeBrainError(err)}` },
              ],
              details: { stored: false, error: describeBrainError(err) },
            };
          }
          const pending = res.status === "pending";
          return {
            content: [
              {
                type: "text" as const,
                text: pending
                  ? `Submitted for review (proposal id: ${res.id}). It becomes memory only after a reviewer approves it.`
                  : `Memory ${res.status} (id: ${res.id}).`,
              },
            ],
            details: { stored: !pending, pending, id: res.id, status: res.status },
          };
        },
      },
      { name: "memory_store" },
    );

    // v1.20.25: the agent-facing `memory_forget` tool is REMOVED. An agent can
    // autonomously hard-delete long-term memory with no human gate — ambient
    // authority in the exact shape the mantra forbids ("memory you can see,
    // approve, and erase" — erase is a HUMAN action). The read-only recall/
    // get/verify/graph tools + the review-queued `memory_store` are the agent's
    // only surface. Erasure remains a human action via the operator console
    // and the `brain` CLI (server `DELETE /memory/{id}` is untouched).

    api.registerTool(
      {
        name: "memory_verify",
        label: "Memory Verify",
        description:
          "Deterministic span verification (no LLM): checks whether a claim is literally supported by a chunk's stored text. Use after recalling a fact to confirm the brain actually said it before acting on it.",
        parameters: Type.Object({
          chunk_id: Type.Integer({ description: "Chunk/memory id (from a recall hit)" }),
          claim: Type.String({ description: "The claim to verify against the chunk text" }),
        }),
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = (params ?? {}) as { chunk_id?: number; claim?: string };
          const claim = (p.claim ?? "").trim();
          if (!claim || typeof p.chunk_id !== "number") {
            return {
              content: [{ type: "text" as const, text: "Both chunk_id and claim are required." }],
              details: { verified: false },
            };
          }
          try {
            const res = await client.verify({
              chunkId: p.chunk_id,
              claim,
              timeoutMs: c.requestTimeoutMs,
            });
            const text = res.supported
              ? `Supported: the chunk ${res.chunkId} contains the claim (${res.matchRanges.length} match${res.matchRanges.length === 1 ? "" : "es"}).`
              : `Not supported: the chunk ${res.chunkId} does not contain the claim. Do not present it as memory-backed.`;
            return {
              content: [{ type: "text" as const, text }],
              details: { verified: res.supported, decision: res.decision, chunkId: res.chunkId },
            };
          } catch (err) {
            return {
              content: [
                { type: "text" as const, text: `Verify failed: ${describeBrainError(err)}` },
              ],
              details: { verified: false, error: describeBrainError(err) },
            };
          }
        },
      },
      { name: "memory_verify" },
    );

    api.registerTool(
      {
        name: "memory_get",
        label: "Memory Get",
        description:
          "Fetch the full stored text of a memory/chunk by id. Use to read the complete context behind a recalled snippet before relying on it.",
        parameters: Type.Object({
          id: Type.Integer({ description: "Chunk/memory id (from a recall hit)" }),
        }),
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = (params ?? {}) as { id?: number };
          if (typeof p.id !== "number") {
            return {
              content: [{ type: "text" as const, text: "id is required." }],
              details: { found: false },
            };
          }
          try {
            const chunk = await client.get(p.id, c.requestTimeoutMs);
            if (!chunk) {
              return {
                content: [{ type: "text" as const, text: `No memory with id ${p.id}.` }],
                details: { found: false, id: p.id },
              };
            }
            const title = chunk.title?.trim() ? `\n${sanitizeForBlock(chunk.title)}` : "";
            return {
              content: [
                {
                  type: "text" as const,
                  text: `Memory ${chunk.id}:${title}\n${sanitizeForBlock(chunk.content)}`,
                },
              ],
              details: { found: true, id: chunk.id, title: chunk.title ? sanitizeForBlock(chunk.title) : chunk.title },
            };
          } catch (err) {
            return {
              content: [{ type: "text" as const, text: `Get failed: ${describeBrainError(err)}` }],
              details: { found: false, error: describeBrainError(err) },
            };
          }
        },
      },
      { name: "memory_get" },
    );

    api.registerTool(
      {
        name: "memory_graph_entity",
        label: "Memory Graph Entity",
        description:
          "Look up an entity in the knowledge graph and its one-hop relations. Use to explore how a concept connects to others.",
        parameters: Type.Object({
          name: Type.String({ description: "Entity name (e.g. 'vitamin d3')" }),
        }),
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = (params ?? {}) as { name?: string };
          const name = (p.name ?? "").trim();
          if (!name) {
            return {
              content: [{ type: "text" as const, text: "name is required." }],
              details: { found: false },
            };
          }
          try {
            const entity = await client.graphEntity(name, c.requestTimeoutMs);
            if (!entity) {
              return {
                content: [
                  { type: "text" as const, text: `No entity named "${name}" in the graph.` },
                ],
                details: { found: false, name },
              };
            }
            const etype = entity.type ? ` (${sanitizeForBlock(entity.type).trim()})` : "";
            const rels =
              entity.relations.length === 0
                ? " (no relations)"
                : entity.relations
                    .map(
                      (r) =>
                        `\n  - ${r.direction === "out" ? "→" : "←"} ${sanitizeForBlock(r.relation_type).trim()} ${sanitizeForBlock(r.to_entity).trim()}`,
                    )
                    .join("");
            return {
              content: [{ type: "text" as const, text: `${sanitizeForBlock(entity.name).trim()}${etype}${rels}` }],
              details: {
                found: true,
                name: sanitizeForBlock(entity.name),
                relations: entity.relations.map((r) => ({
                  ...r,
                  to_entity: sanitizeForBlock(r.to_entity),
                  relation_type: sanitizeForBlock(r.relation_type),
                })),
              },
            };
          } catch (err) {
            return {
              content: [
                { type: "text" as const, text: `Graph lookup failed: ${describeBrainError(err)}` },
              ],
              details: { found: false, error: describeBrainError(err) },
            };
          }
        },
      },
      { name: "memory_graph_entity" },
    );

    // ------------------------------------------------------------------------
    // Service lifecycle
    // ------------------------------------------------------------------------
    api.registerService({
      id: PLUGIN_ID,
      async start() {
        const ok = await client.health(2_000);
        if (!ok) {
          api.logger.warn?.(
            `${PLUGIN_ID}: brain-server not reachable at ${cfg.baseUrl} (recall will fail-open until it is). Start the Rust service: brain-server`,
          );
        } else {
          api.logger.info?.(`${PLUGIN_ID}: connected to brain-server at ${cfg.baseUrl}`);
        }
      },
      stop() {
        api.logger.info?.(`${PLUGIN_ID}: stopped`);
      },
    });
  },
});

// ------------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------------

/**
 * Minimal projection of the SDK hook agent context used for gating. `api.on`
 * already types the handler params via `PluginHookHandlerMap`; this structural
 * type keeps the module from importing the plugin-internal hook types directly.
 */
type HookContextLike = {
  agentId?: string;
  chatId?: string;
  channelId?: string;
  chatType?: "direct" | "group" | "channel" | "explicit";
  channel?: string;
  trigger?: string;
};

/** Map the SDK hook context into the minimal GateContext used for gating. */
function mapCtx(ctx: HookContextLike | undefined): GateContext {
  if (!ctx) {
    return {};
  }
  // Prefer the gateway's already-classified chatType (e.g. telegram DM => "direct").
  // deriveChatType is a fail-closed fallback for contexts that omit it.
  const chatType =
    ctx.chatType ??
    deriveChatType({
      ...(ctx.channel !== undefined ? { channel: ctx.channel } : {}),
      ...(ctx.trigger !== undefined ? { trigger: ctx.trigger } : {}),
      ...(ctx.chatId !== undefined ? { chatId: ctx.chatId } : {}),
    });
  return {
    ...(ctx.agentId !== undefined ? { agentId: ctx.agentId } : {}),
    ...((ctx.chatId ?? ctx.channelId) !== undefined
      ? { chatId: (ctx.chatId ?? ctx.channelId) as string }
      : {}),
    ...(chatType ? { chatType } : {}),
  };
}

/** Extract user-authored text blocks from a messages array (defensive). */
/// v1.20.1 "Shield" M2: the full-turn text that fed an auto-capture, for the
/// proposal's `source_prompt` (the exact capture trigger, not a summary).
/// Same shape tolerance as `extractUserTexts` — string or parts-array content.
function joinMessageTexts(messages: ReadonlyArray<unknown>): string {
  const parts: string[] = [];
  for (const m of messages) {
    const msg = m as { content?: unknown } | null;
    if (!msg) {
      continue;
    }
    const content = msg.content;
    if (typeof content === "string") {
      parts.push(content);
    } else if (Array.isArray(content)) {
      for (const block of content) {
        if (typeof block === "object" && block !== null && "text" in block) {
          parts.push(String((block as { text: unknown }).text));
        }
      }
    }
  }
  return parts.join("\n");
}

function extractUserTexts(messages: ReadonlyArray<unknown>): string[] {
  const out: string[] = [];
  for (const m of messages) {
    const msg = m as { role?: string; content?: unknown } | null;
    if (!msg || msg.role !== "user") {
      continue;
    }
    const content = msg.content;
    if (typeof content === "string") {
      out.push(content);
    } else if (Array.isArray(content)) {
      for (const block of content) {
        if (typeof block === "object" && block !== null && "text" in block) {
          out.push(String((block as { text: unknown }).text));
        }
      }
    }
  }
  return out;
}
