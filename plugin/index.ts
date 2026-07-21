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
import type {
  PluginHookAgentContext,
  PluginHookAgentEndEvent,
} from "openclaw/plugin-sdk/types";
import { Type } from "typebox";

import { BrainClient, describeBrainError } from "./src/brain-client.js";
import { brainPluginConfigSchema, resolveConfig, type ResolvedBrainConfig } from "./src/config.js";
import {
  deriveChatType,
  isRecallAllowed,
  type GateContext,
} from "./src/gating.js";
import {
  STATIC_SYSTEM_GUIDANCE,
  formatRecallContext,
  latestUserText,
  looksCaptureWorthy,
  normalizeRecallQuery,
} from "./src/format.js";

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
        if (!current) return cfg;
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
        if (!c.autoRecall || !c.enabled) return undefined;
        if (!event.prompt || event.prompt.length < c.minQueryLength) return undefined;

        const gate = mapCtx(ctx);
        const decision = isRecallAllowed(c, gate);
        if (!decision.allowed) {
          // Not an error — gated out by policy. Silent no-op.
          return undefined;
        }

        const querySource =
          latestUserText(Array.isArray(event.messages) ? event.messages : []) ?? event.prompt;
        const query = normalizeRecallQuery(querySource, c.recallMaxChars);
        if (query.length < c.minQueryLength) return undefined;

        try {
          const result = await client.recall({
            query,
            // Let the server auto-route via centroids; only force a domain if caller set one.
            ...(c.defaultDomain && c.defaultDomain !== "global" ? { domain: c.defaultDomain } : {}),
            ...(c.strictDomain ? { strictDomain: true } : {}),
            limit: c.autoRecallTopK,
            timeoutMs: c.autoRecallTimeoutMs,
          });
          if (!result.hits.length) return undefined;

          const block = formatRecallContext(result.hits);
          if (!block) return undefined;

          api.logger.info?.(`${PLUGIN_ID}: injecting ${result.hits.length} memories (domain=${result.domain ?? "auto"})`);
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
      if (!c.autoCapture || !c.enabled) return;
      if (!event.success) return;
      const messages = event.messages;
      const gate = mapCtx(ctx);
      if (!isRecallAllowed(c, gate).allowed) return;

      try {
        for (const text of extractUserTexts(messages)) {
          if (!looksCaptureWorthy(text)) continue;
          await client.store({
            title: text.slice(0, 80),
            content: text,
            ...(c.defaultDomain && c.defaultDomain !== "global" ? { domain: c.defaultDomain } : {}),
            timeoutMs: c.requestTimeoutMs,
          });
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
          "Search long-term memory. Use for past decisions, preferences, or previously discussed topics.",
        parameters: Type.Object({
          query: Type.String({ description: "Search query" }),
          limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 50, description: "Max results (default 5)" })),
          domain: Type.Optional(Type.String({ description: "Force a specific domain (auto-routing is default)" })),
        }),
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = (params ?? {}) as { query?: string; limit?: number; domain?: string };
          const query = normalizeRecallQuery(String(p.query ?? ""), c.recallMaxChars);
          if (!query) {
            return { content: [{ type: "text" as const, text: "No query provided." }], details: { count: 0 } };
          }
          let result;
          try {
            result = await client.recall({
              query,
              ...(p.domain ? { domain: p.domain } : {}),
              limit: p.limit ?? 5,
              timeoutMs: c.requestTimeoutMs,
            });
          } catch (err) {
            // Tools surface the failure so the agent can react; recall errors
            // are not silent here (unlike the fail-open auto-recall hook).
            return {
              content: [{ type: "text" as const, text: `Recall failed: ${describeBrainError(err)}` }],
              details: { count: 0, error: describeBrainError(err) },
            };
          }
          if (!result.hits.length) {
            return { content: [{ type: "text" as const, text: "No relevant memories found." }], details: { count: 0 } };
          }
          return {
            content: [{ type: "text" as const, text: formatRecallContext(result.hits) }],
            details: { count: result.hits.length, memories: result.hits },
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
          title: Type.Optional(Type.String({ description: "Short title (default: first 80 chars)" })),
          domain: Type.Optional(Type.String()),
          entities: Type.Optional(
            Type.Array(Type.Object({
              name: Type.String(),
              type: Type.Optional(Type.String()),
            })),
          ),
          relations: Type.Optional(
            Type.Array(Type.Object({
              from: Type.String(),
              to: Type.String(),
              type: Type.String(),
            })),
          ),
        }),
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = (params ?? {}) as { text?: string; title?: string; domain?: string; entities?: unknown; relations?: unknown };
          const text = String(p.text ?? "").trim();
          if (!text) {
            return { content: [{ type: "text" as const, text: "No text provided." }], details: { stored: false } };
          }
          let res;
          try {
            res = await client.store({
              title: p.title?.trim() || text.slice(0, 80),
              content: text,
              ...(p.domain ? { domain: p.domain } : {}),
              ...(Array.isArray(p.entities) ? { entities: p.entities as never[] } : {}),
              ...(Array.isArray(p.relations) ? { relations: p.relations as never[] } : {}),
              timeoutMs: c.requestTimeoutMs,
            });
          } catch (err) {
            return {
              content: [{ type: "text" as const, text: `Store failed: ${describeBrainError(err)}` }],
              details: { stored: false, error: describeBrainError(err) },
            };
          }
          return {
            content: [{ type: "text" as const, text: `Memory ${res.status} (id: ${res.id}).` }],
            details: { stored: true, id: res.id, status: res.status },
          };
        },
      },
      { name: "memory_store" },
    );

    api.registerTool(
      {
        name: "memory_forget",
        label: "Memory Forget",
        description: "Delete a memory by id.",
        parameters: Type.Object({ id: Type.String({ description: "Memory id" }) }),
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = (params ?? {}) as { id?: string };
          let res;
          try {
            res = await client.forget(String(p.id ?? ""), c.requestTimeoutMs);
          } catch (err) {
            return {
              content: [{ type: "text" as const, text: `Forget failed: ${describeBrainError(err)}` }],
              details: { deleted: false, error: describeBrainError(err) },
            };
          }
          // null = 404 not found; {deleted:false} would be an unexpected ok-without-delete.
          const deleted = res?.deleted ?? false;
          return {
            content: [{ type: "text" as const, text: deleted ? "Forgotten." : "Not found." }],
            details: { deleted },
          };
        },
      },
      { name: "memory_forget" },
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

/** Map the SDK hook context into the minimal GateContext used for gating. */
function mapCtx(ctx: PluginHookAgentContext | undefined): GateContext {
  if (!ctx) return {};
  const chatType = deriveChatType({
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
function extractUserTexts(messages: ReadonlyArray<unknown>): string[] {
  const out: string[] = [];
  for (const m of messages) {
    const msg = m as { role?: string; content?: unknown } | null;
    if (!msg || msg.role !== "user") continue;
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
