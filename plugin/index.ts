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
 *   - api.registerMemoryCorpusSupplement({ search, get })
 *   - api.resolvePath, api.logger, api.pluginConfig, api.runtime.config.current()
 */
import { definePluginEntry, type OpenClawPluginApi } from "openclaw/plugin-sdk/plugin-entry";
import { BrainClient, type BrainRecallResult } from "./src/brain-client.js";
import { brainPluginConfigSchema, resolveConfig, type ResolvedBrainConfig } from "./src/config.js";
import {
  STATIC_SYSTEM_GUIDANCE,
  formatRecallContext,
  latestUserText,
  looksCaptureWorthy,
  normalizeRecallQuery,
  sanitizeForBlock,
} from "./src/format.js";
import { deriveChatType, isRecallAllowed, type GateContext } from "./src/gating.js";
import { registerProceduralTools } from "./src/procedural.js";
import { attachTeamBridge, teamCloseOnEnd, teamPauseOnSessionEnd } from "./src/team-bridge.js";
import { registerBrainTools, MAX_HIT_CHARS } from "./src/tools.js";

const PLUGIN_ID = "brain-server";

// v1.20.29 "Bound" (F-6): per-session recall ceiling. Auto-recall + corpus
// search share a closure-scoped counter; once exceeded, further recalls return
// empty (no-op, not error) so an agent loop can't fan out unbounded POSTs in a
// single session. Reset on `session_end`. ponytail: client-side bound only —
// the server's per-IP limiter still keys on 127.0.0.1 (one shared bucket for
// all loopback clients); per-principal limiting is v2.1 (needs Redis).
const MAX_RECALLS_PER_TURN = 10;

export default definePluginEntry({
  id: PLUGIN_ID,
  name: "Brain Server",
  description:
    "Local semantic-memory + knowledge-graph (Rust brain-server). Deterministic auto-recall, per-domain KGs, local static embeddings. Zero decision/embedding tokens.",
  configSchema: brainPluginConfigSchema,

  register(api: OpenClawPluginApi) {
    const cfg: ResolvedBrainConfig = resolveConfig(api.pluginConfig);
    const client = new BrainClient(cfg);

    // Live-config resolver: re-read this plugin's config slice from the current
    // runtime snapshot (api.runtime.config.current()) so operator changes take
    // effect without a restart. api.pluginConfig is the registration-time
    // snapshot (set once in buildPluginApi), so only the live snapshot reflects
    // post-registration config mutations. Falls back to cfg if the runtime is
    // unavailable or the slice is absent.
    const liveCfg = (): ResolvedBrainConfig => {
      try {
        const live = api.runtime.config.current().plugins?.entries?.[PLUGIN_ID]?.config;
        return resolveConfig(live ?? api.pluginConfig);
      } catch {
        return cfg;
      }
    };

    // ------------------------------------------------------------------
    // v1.20.29 "Bound" (F-6): shared inflight de-dup + per-session cap.
    // The three recall call sites (auto-recall hook, corpus `search`, and the
    // `memory_recall` tool) previously shared no guard — same-turn duplicates
    // and agent loops fanned out unbounded POSTs. This closure-scoped gate is
    // shared by the hook + corpus search (the two automated paths); the tool
    // stays explicit (an agent-callable surface, different semantics).
    //   - `inflight`: same query-same-turn collapses to ONE server POST (the
    //     second caller awaits the first's Promise).
    //   - `sessionRecallCount`: bounds total automated recalls per session;
    //     reset on `session_end`. Exceeding `MAX_RECALLS_PER_TURN` returns an
    //     empty-hits result (no-op, never an error).
    // ponytail: client-side bound only; the server's per-IP limiter still keys
    // on 127.0.0.1 (one shared bucket). Per-principal limiting is v2.1.
    // ------------------------------------------------------------------
    // v0.5.0 team bridge — mirror agent activity onto brain-server's governed
    // workflow dashboards (Mesh cards, Crew roster, run timelines, Scoreboard).
    // Off by default (teamBridge: false); observation-only, fail-open, gated by
    // the same per-agent allowlist as recall. See src/team-bridge.ts. The
    // per-turn heartbeat rides the existing before_prompt_build handler below
    // (one handler per hook — house rule), so attach happens BEFORE it.
    const teamBridge = attachTeamBridge(api, client, liveCfg);
    const teamBeat = (ctx: HookContextLike | undefined): void => {
      const c = liveCfg();
      if (!c.teamBridge || !c.enabled || !ctx?.agentId || !c.agents.includes(ctx.agentId)) {
        return;
      }
      void teamBridge.onTurnBeat(ctx).catch(() => {
        // heartbeat loss is harmless by definition
      });
    };

    const inflight = new Map<string, Promise<BrainRecallResult>>();
    let sessionRecallCount = 0;
    const boundedRecall = (
      queryKey: string,
      run: () => Promise<BrainRecallResult>,
    ): Promise<BrainRecallResult> => {
      // Cap exceeded → no-op empty result (fail-closed, not an error).
      if (sessionRecallCount >= MAX_RECALLS_PER_TURN) {
        return Promise.resolve({ hits: [], decision: "ok" });
      }
      // Same query already inflight this turn → await the shared POST.
      const existing = inflight.get(queryKey);
      if (existing) {
        return existing;
      }
      sessionRecallCount += 1;
      const p = run().finally(() => {
        // Evict once resolved so the NEXT turn's same query fires fresh.
        inflight.delete(queryKey);
      });
      inflight.set(queryKey, p);
      return p;
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
    api.registerMemoryCapability({
      promptBuilder: () => [STATIC_SYSTEM_GUIDANCE],
    });

    // ------------------------------------------------------------------------
    // v0.3.0: expose brain-server's index as a NON-exclusive unified search
    // corpus (api.registerMemoryCorpusSupplement). This composes with the
    // built-in memory slot rather than competing for it: the stock
    // `memory_search`/`memory_get` tools gain brain-server hits alongside
    // memory-core, gated by the same `agents` allowlist + chat-type policy as
    // auto-recall. Fail-open — corpus search never stalls the host on a server error.
    // ------------------------------------------------------------------------
    api.registerMemoryCorpusSupplement({
      search: async ({ query, maxResults, agentId, sandboxed }) => {
        // Reuse the auto-recall gate so the corpus honors the same least-privilege
        // policy (per-agent opt-in + chat-type leakage prevention).
        const allowed = isRecallAllowed(liveCfg(), {
          ...(agentId !== undefined ? { agentId } : {}),
          // Unified memory search has no single chat context; treat it as the
          // operator's direct surface (allowed by default) unless sandboxed.
          ...(sandboxed ? { chatType: "group" as const } : {}),
        });
        if (!allowed.allowed) {
          return [];
        }
        try {
          const result = await boundedRecall(query, () =>
            client.recall({
              query,
              limit: Math.min(maxResults ?? 5, 20),
              timeoutMs: liveCfg().requestTimeoutMs,
            }),
          );
          if (!result.hits.length) {
            return [];
          }
          return result.hits.map(hitToCorpusResult);
        } catch {
          return [];
        }
      },
      get: async ({ lookup }) => {
        try {
          const id = lookup.replace(/^.*\//, "");
          const chunk = await client.get(id, liveCfg().requestTimeoutMs);
          if (!chunk) {
            return null;
          }
          return {
            corpus: "brain-server",
            path: `/memory/${String(chunk.id)}`,
            ...(chunk.title ? { title: sanitizeForBlock(chunk.title) } : {}),
            content: sanitizeForBlock(chunk.content),
            fromLine: 1,
            lineCount: 1,
            ...(chunk.source_uri ? { sourcePath: chunk.source_uri } : {}),
          };
        } catch {
          return null;
        }
      },
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

        // v0.5.0 team bridge heartbeat (throttled inside the bridge).
        teamBeat(ctx);

        const querySource =
          latestUserText(Array.isArray(event.messages) ? event.messages : []) ?? event.prompt;
        const query = normalizeRecallQuery(querySource, c.recallMaxChars);
        if (query.length < c.minQueryLength) {
          return undefined;
        }

        try {
          const result = await boundedRecall(query, () =>
            client.recall({
              query,
              // Let the server auto-route via centroids; only force a domain if caller set one.
              ...(c.defaultDomain && c.defaultDomain !== "global"
                ? { domain: c.defaultDomain }
                : {}),
              ...(c.strictDomain ? { strictDomain: true } : {}),
              limit: c.autoRecallTopK,
              // v0.3.0: optional graph-PPR third leg + token-budgeted
              // packing. S3-7 (pass-3 audit): ALWAYS send the flag
              // explicitly — the server default flipped to graph-on
              // (00a79fe), so omitting it on `false` silently enabled the
              // leg for every plugin user. The plugin's documented default
              // stays opt-in (`autoRecallGraph: false`).
              graph: c.autoRecallGraph,
              ...(c.autoRecallMaxContextTokens !== undefined
                ? { maxContextTokens: c.autoRecallMaxContextTokens }
                : {}),
              timeoutMs: c.autoRecallTimeoutMs,
            }),
          );
          if (!result.hits.length) {
            // Calibrated abstention (v1.5): the server returned no hits on
            // purpose because quality was too low. Fail-open — inject nothing.
            // The log carries the query LENGTH only — openclaw's log is
            // persistent and the query is user text (privacy: never log it).
            if (result.decision === "low_confidence") {
              api.logger.info?.(
                `${PLUGIN_ID}: recall abstained (low confidence, query ${query.length} chars); skipping injection`,
              );
            }
            return undefined;
          }

          // v1.20.29 "Bound": per-hit body cap (caller-side, before formatting)
          // so a single huge chunk can't dominate the injected context.
          // format.ts is untouched (Release C owns it); the cap is applied here.
          const hits = result.hits.map((h) =>
            h.content.length > MAX_HIT_CHARS
              ? { ...h, content: h.content.slice(0, MAX_HIT_CHARS) }
              : h,
          );
          const block = formatRecallContext(hits);
          if (!block) {
            return undefined;
          }

          api.logger.info?.(
            `${PLUGIN_ID}: injecting ${hits.length} memories (domain=${result.domain ?? "auto"})`,
          );
          // Dynamic recall => prependContext (per-turn, NOT cached).
          // Static guidance is handled separately via registerMemoryCapability (cacheable).
          return { prependContext: block };
        } catch (err) {
          // FAIL-OPEN: never stall the agent on a memory error.
          api.logger.warn?.(
            `${PLUGIN_ID}: recall failed (${sanitizeForBlock(String(err))}); skipping injection`,
          );
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

      // v0.5.0 team bridge: close the mirrored governed run first — the run
      // outcome is recorded regardless of whether autoCapture is enabled.
      try {
        await teamCloseOnEnd(teamBridge, c, ctx, event.success === true);
      } catch (err) {
        api.logger.warn?.(`${PLUGIN_ID}: team run close failed (${sanitizeForBlock(String(err))})`);
      }

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
        api.logger.warn?.(`${PLUGIN_ID}: capture failed (${sanitizeForBlock(String(err))})`);
      }
    });

    api.on("session_end", (_event, ctx) => {
      // v1.20.29 "Bound": reset the per-session recall counter + drop any
      // stranded inflight entries so a new session starts unbounded. (Cursors
      // still live server-side — this is the amplification bound only.)
      sessionRecallCount = 0;
      inflight.clear();
      // v0.5.0 team bridge: pause mirrored runs still open for this session.
      const sessionKey = (ctx as { sessionKey?: string } | undefined)?.sessionKey;
      void teamPauseOnSessionEnd(teamBridge, sessionKey).catch(() => {});
    });

    // ------------------------------------------------------------------------
    // Tools — explicit agent-callable surfaces. Defined in src/tools.ts.
    // ------------------------------------------------------------------------
    registerBrainTools(api, client, liveCfg, cfg.proposalTools);

    // v0.4.0 procedural memory (runbooks / decision trees) — src/procedural.ts.
    registerProceduralTools(api, client, liveCfg);

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

/**
 * A unified-search corpus result this plugin contributes to memory_search.
 * Optional fields are populated only when the hit carries them (built
 * field-by-field, not spread, to satisfy oxc/no-map-spread).
 */
type CorpusResult = {
  corpus: "brain-server";
  path: string;
  score: number;
  snippet: string;
  id: string;
  title?: string;
  kind?: string;
  source?: string;
};

type CorpusHit = {
  id: number | string;
  title?: string;
  content: string;
  score: number;
  domain?: string;
};

/** Map a recall hit into a corpus-search result (non-exclusive unified search). */
function hitToCorpusResult(hit: CorpusHit): CorpusResult {
  return {
    corpus: "brain-server",
    path: `/memory/${String(hit.id)}`,
    ...(hit.title ? { title: sanitizeForBlock(hit.title) } : {}),
    score: hit.score,
    snippet: sanitizeForBlock(hit.content),
    id: String(hit.id),
    ...(hit.domain ? { kind: hit.domain, source: hit.domain } : {}),
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
