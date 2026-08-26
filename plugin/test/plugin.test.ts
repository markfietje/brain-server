import type { OpenClawPluginApi } from "openclaw/plugin-sdk/plugin-entry";
/**
 * Brain Server plugin integration tests.
 *
 * NOTE: brain-server is architecturally distinct from memory-lancedb:
 *   - memory-lancedb is an IN-PROCESS plugin (LanceDB + OpenAI embeddings
 *     loaded directly into the host), so its tests must mock LanceDB tables
 *     and the OpenAI embeddings API.
 *   - brain-server is a THIN HTTP SHIM: all memory logic lives in a separate
 *     Rust process reached over loopback HTTP. So the only thing to mock here
 *     is `fetch` — standing in for the Rust server's `/recall`, `/ingest`,
 *     `/memory/{id}` endpoints exactly as defined in API_CONTRACT.md.
 *
 * What this suite verifies (brain-server-specific behavior):
 *   1. Deterministic recall: before_prompt_build fires every turn, issues ONE
 *      POST /recall, and injects prependContext. Zero decision tokens.
 *   2. Fail-open contract: a transport/HTTP failure in recall NEVER stalls the
 *      agent (returns undefined + warns). Auth failures are still surfaced by
 *      the explicit tools.
 *   3. Per-agent + chat-type gating (OWASP LLM06 / data-leakage prevention) —
 *      a capability brain-server adds that memory-lancedb does not have.
 *   4. autoCapture on agent_end stores capture-worthy user turns to /ingest.
 *   5. Tools surface distinct error codes (404 vs 500) instead of masking them
 *      — the behavior added so an agent gets actionable feedback.
 */
import { afterEach, describe, expect, test, vi } from "vitest";
import plugin from "../index.js";
import { MAX_HIT_CHARS } from "../src/tools.js";

/**
 * Structural mock of OpenClawPluginApi. We only import the SDK *type* for the
 * register-boundary cast — the mock object itself is a plain recorder. The real
 * `definePluginEntry` (imported by index.ts from the SDK) is what runs at
 * registration time; this object just captures what the plugin registers so we
 * can invoke it directly.
 */
type HookHandler = (...args: unknown[]) => unknown;
type MockApi = {
  pluginConfig: unknown;
  runtime: { config?: { current?: unknown } };
  logger: {
    info?: (...args: unknown[]) => void;
    warn?: (...args: unknown[]) => void;
    error?: (...args: unknown[]) => void;
    debug?: (...args: unknown[]) => void;
  };
  on: (name: string, handler: HookHandler, opts?: unknown) => void;
  registerTool: (tool: unknown, opts?: { name?: string }) => void;
  registerService: (s: { id: string; start?: () => unknown; stop?: () => void }) => void;
  registerMemoryCapability?: (cap: { promptBuilder: () => unknown[] }) => void;
  registerMemoryCorpusSupplement?: (supplement: unknown) => void;
};

function mockResponse(body: unknown, init: { status?: number } = {}) {
  const status = init.status ?? 200;
  const text = typeof body === "string" ? body : JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: "",
    text: async () => text,
  } as unknown as Response;
}

/** Register the plugin with a recording mock API; return captured registrations. */
function registerPlugin(pluginConfig: unknown) {
  const hooks = new Map<string, HookHandler>();
  const tools = new Map<string, { execute: (...args: unknown[]) => unknown }>();
  const services: Array<{ id: string; start?: () => unknown; stop?: () => void }> = [];
  const logger = { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() };

  const api: MockApi = {
    pluginConfig,
    runtime: {},
    logger,
    on: vi.fn((name: string, handler: HookHandler) => hooks.set(name, handler)),
    registerTool: vi.fn((tool, opts) => {
      const t = tool as { name?: string; execute?: (...a: unknown[]) => unknown };
      const name = opts?.name ?? t.name;
      if (name && t.execute) {
        tools.set(name, t as { execute: (...a: unknown[]) => unknown });
      }
    }),
    registerService: vi.fn((s) => services.push(s)),
    registerMemoryCapability: vi.fn(),
    registerMemoryCorpusSupplement: vi.fn(),
  };

  plugin.register(api as unknown as OpenClawPluginApi);
  return { hooks, tools, services, logger };
}

/** Typed projection of a registered hook handler used across these tests. */
type HookHandlerFn = (e: unknown, ctx: unknown) => Promise<unknown>;
const getHook = (hooks: Map<string, HookHandler>, name: string): HookHandlerFn =>
  hooks.get(name) as HookHandlerFn;

// ---------------------------------------------------------------------------

describe("plugin registration", () => {
  test("registers the deterministic-recall hook and the memory-slot tools", () => {
    const { hooks, tools } = registerPlugin({ agents: ["main"] });
    // before_prompt_build is the entire reason this plugin exists.
    expect(hooks.has("before_prompt_build")).toBe(true);
    expect(hooks.has("agent_end")).toBe(true);
    expect(hooks.has("session_end")).toBe(true);
    // kind:"memory" slot contract (matches openclaw.plugin.json contracts.tools).
    expect(tools.has("memory_recall")).toBe(true);
    expect(tools.has("memory_store")).toBe(true);
    // v1.20.25: agent-facing hard-delete is removed — erasure is a human action.
    expect(tools.has("memory_forget")).toBe(false);
    // brain-server differentiated surfaces (span verify, fetch, knowledge graph).
    expect(tools.has("memory_verify")).toBe(true);
    expect(tools.has("memory_get")).toBe(true);
    expect(tools.has("memory_graph_entity")).toBe(true);
    // v0.3.0: graph traversal is always registered.
    expect(tools.has("memory_graph_traverse")).toBe(true);
    // v0.3.0: proposal review tools are gated behind config.proposalTools (off by default).
    expect(tools.has("memory_proposal_list")).toBe(false);
    expect(tools.has("memory_proposal_decide")).toBe(false);
    // v0.4.0: procedural tools (runbooks / decision trees) are always registered.
    expect(tools.has("memory_procedure_get")).toBe(true);
    expect(tools.has("memory_procedure_store")).toBe(true);
    expect(tools.has("memory_decision_evaluate")).toBe(true);
  });

  test("registers the proposal review tools when config.proposalTools is true", () => {
    const { tools } = registerPlugin({ agents: ["main"], proposalTools: true });
    expect(tools.has("memory_proposal_list")).toBe(true);
    expect(tools.has("memory_proposal_decide")).toBe(true);
  });

  test("registers a static memory capability (prompt-cached system guidance)", () => {
    const registerMemoryCapability = vi.fn();
    const api = {
      pluginConfig: { agents: ["main"] },
      runtime: {},
      logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
      on: vi.fn(),
      registerTool: vi.fn(),
      registerService: vi.fn(),
      registerMemoryCapability,
      registerMemoryCorpusSupplement: vi.fn(),
    } as unknown as MockApi;
    plugin.register(api as unknown as OpenClawPluginApi);
    expect(registerMemoryCapability).toHaveBeenCalledTimes(1);
    const cap = registerMemoryCapability.mock.calls[0]?.[0] as { promptBuilder: () => unknown[] };
    // Static guidance must mention treating memories as untrusted (LLM01/LLM02).
    const out = cap.promptBuilder();
    expect(String(out)).toContain("untrusted");
  });
});

describe("before_prompt_build — deterministic recall over POST /recall", () => {
  afterEach(() => vi.restoreAllMocks());

  test("issues exactly ONE /recall call and injects prependContext", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(
        mockResponse({ hits: [{ id: 1, content: "prefers Helix", score: 0.9, untrusted: true }] }),
      );

    const { hooks } = registerPlugin({ agents: ["main"] });
    const result = await getHook(hooks, "before_prompt_build")(
      {
        prompt: "what editor?",
        messages: [{ role: "user", content: "what editor should i use?" }],
      },
      { agentId: "main" },
    );

    // One HTTP call to the Rust server's /recall — that is the whole turn's cost.
    const recallCalls = fetchMock.mock.calls.filter((c) => (c[0] as string).endsWith("/recall"));
    expect(recallCalls).toHaveLength(1);
    expect(result).toEqual({ prependContext: expect.stringContaining("prefers Helix") });
    // Anti-injection banner rides along on every injected block.
    expect((result as { prependContext: string }).prependContext).toContain("UNTRUSTED");
  });

  test("autoRecallGraph flag is sent EXPLICITLY on /recall (S3-7 pin)", async () => {
    // The server default flipped to graph-on; omitting the flag on `false`
    // silently re-enabled the third leg for every plugin user. The param must
    // now always be present, with the configured value.
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));

    const off = registerPlugin({ agents: ["main"], autoRecallGraph: false });
    await getHook(off.hooks, "before_prompt_build")(
      { prompt: "query one", messages: [{ role: "user", content: "query one please" }] },
      { agentId: "main" },
    );
    let body = JSON.parse(
      fetchMock.mock.calls.find((c) => (c[0] as string).endsWith("/recall"))![1]?.body as string,
    );
    expect(body.graph).toBe(false);

    fetchMock.mockClear();
    const on = registerPlugin({ agents: ["main"], autoRecallGraph: true });
    await getHook(on.hooks, "before_prompt_build")(
      { prompt: "query two", messages: [{ role: "user", content: "query two please" }] },
      { agentId: "main" },
    );
    body = JSON.parse(
      fetchMock.mock.calls.find((c) => (c[0] as string).endsWith("/recall"))![1]?.body as string,
    );
    expect(body.graph).toBe(true);
  });

  test("empty hits => undefined (inject nothing, no banner)", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    const { hooks } = registerPlugin({ agents: ["main"] });
    const result = await getHook(hooks, "before_prompt_build")(
      { prompt: "hello", messages: [{ role: "user", content: "hello there" }] },
      { agentId: "main" },
    );
    expect(result).toBeUndefined();
  });

  test("FAILS OPEN on network error: undefined + warn, never throws", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new TypeError("connection refused"));
    const { hooks, logger } = registerPlugin({ agents: ["main"] });
    const result = await getHook(hooks, "before_prompt_build")(
      { prompt: "query", messages: [{ role: "user", content: "a real query here" }] },
      { agentId: "main" },
    );
    expect(result).toBeUndefined();
    expect(logger.warn).toHaveBeenCalled();
  });

  test("FAILS OPEN on HTTP 500: undefined (must not stall the agent)", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("boom", { status: 500 }));
    const { hooks } = registerPlugin({ agents: ["main"] });
    const result = await getHook(hooks, "before_prompt_build")(
      { prompt: "query", messages: [{ role: "user", content: "a real query here" }] },
      { agentId: "main" },
    );
    expect(result).toBeUndefined();
  });

  test("prompt shorter than minQueryLength => no /recall call", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    const { hooks } = registerPlugin({ agents: ["main"] });
    await getHook(hooks, "before_prompt_build")(
      { prompt: "hi", messages: [{ role: "user", content: "hi" }] },
      { agentId: "main" },
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("autoRecall:false => hook registered but never calls the server", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    const { hooks } = registerPlugin({ agents: ["main"], autoRecall: false });
    // Hook stays registered (SDK inert when disabled) but does no work.
    await getHook(hooks, "before_prompt_build")(
      { prompt: "a longer query", messages: [{ role: "user", content: "a longer query" }] },
      { agentId: "main" },
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("forwards defaultDomain to /recall only when set to a non-global domain", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    const { hooks } = registerPlugin({ agents: ["main"], defaultDomain: "health" });
    await getHook(hooks, "before_prompt_build")(
      { prompt: "a real query", messages: [{ role: "user", content: "a real query" }] },
      { agentId: "main" },
    );
    const body = JSON.parse((fetchMock.mock.calls[0]?.[1]?.body as string) ?? "{}");
    expect(body.domain).toBe("health");
  });
});

describe("live config — re-reads the plugin slice from the runtime snapshot", () => {
  afterEach(() => vi.restoreAllMocks());

  /** Register with a live runtime config snapshot the plugin can re-read each turn. */
  function registerWithLiveConfig(pluginConfig: unknown, liveEntryConfig: unknown) {
    const hooks = new Map<string, HookHandler>();
    const tools = new Map<string, { execute: (...args: unknown[]) => unknown }>();
    const services: Array<{ id: string; start?: () => unknown; stop?: () => void }> = [];
    const api: MockApi = {
      pluginConfig,
      runtime: {
        config: {
          current: () => ({
            plugins: { entries: { "brain-server": { config: liveEntryConfig } } },
          }),
        },
      },
      logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
      on: vi.fn((name: string, handler: HookHandler) => hooks.set(name, handler)),
      registerTool: vi.fn((tool, opts) => {
        const t = tool as { name?: string; execute?: (...a: unknown[]) => unknown };
        const name = opts?.name ?? t.name;
        if (name && t.execute) {
          tools.set(name, t as { execute: (...a: unknown[]) => unknown });
        }
      }),
      registerService: vi.fn((s) => services.push(s)),
      registerMemoryCapability: vi.fn(),
      registerMemoryCorpusSupplement: vi.fn(),
    };
    plugin.register(api as unknown as OpenClawPluginApi);
    return { hooks, tools, services };
  }

  test("a live override disabling autoRecall takes effect without re-registration", async () => {
    // Registered with autoRecall ON, but the runtime snapshot says it is now OFF.
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    const { hooks } = registerWithLiveConfig(
      { agents: ["main"], autoRecall: true },
      {
        agents: ["main"],
        autoRecall: false,
      },
    );
    await getHook(hooks, "before_prompt_build")(
      { prompt: "a real query", messages: [{ role: "user", content: "a real query" }] },
      { agentId: "main" },
    );
    // The live snapshot wins over the registration-time pluginConfig.
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("a live override enabling autoRecall takes effect even when registered OFF", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ hits: [{ id: 1, content: "prefers vim", score: 0.9, untrusted: true }] }),
    );
    const { hooks } = registerWithLiveConfig(
      { agents: ["main"], autoRecall: false },
      {
        agents: ["main"],
        autoRecall: true,
      },
    );
    const result = await getHook(hooks, "before_prompt_build")(
      {
        prompt: "what editor?",
        messages: [{ role: "user", content: "what editor should i use?" }],
      },
      { agentId: "main" },
    );
    expect(result).toEqual({ prependContext: expect.stringContaining("prefers vim") });
  });

  test("falls back to pluginConfig when the live snapshot has no brain-server entry", async () => {
    // liveEntryConfig is undefined: the live slice is absent, so the plugin must
    // fall back to the registration-time pluginConfig (autoRecall still on).
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(mockResponse({ hits: [{ id: 1, content: "ok", score: 0.5 }] }));
    const { hooks } = registerWithLiveConfig({ agents: ["main"], autoRecall: true }, undefined);
    await getHook(hooks, "before_prompt_build")(
      { prompt: "a real query", messages: [{ role: "user", content: "a real query" }] },
      { agentId: "main" },
    );
    expect(fetchMock).toHaveBeenCalled();
  });
});

describe("gating — per-agent + chat-type (brain-server-specific, not in lancedb)", () => {
  afterEach(() => vi.restoreAllMocks());

  test("group chat is blocked even with autoRecall on (data-leakage prevention)", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    const { hooks } = registerPlugin({ agents: ["main"] });
    const result = await getHook(hooks, "before_prompt_build")(
      { prompt: "a real query", messages: [{ role: "user", content: "a real query" }] },
      { agentId: "main", channel: "discord", chatId: "c1" },
    );
    expect(result).toBeUndefined();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("agent not in allowlist => blocked (least privilege)", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    const { hooks } = registerPlugin({ agents: ["main"] });
    const result = await getHook(hooks, "before_prompt_build")(
      { prompt: "a real query", messages: [{ role: "user", content: "a real query" }] },
      { agentId: "other-agent" },
    );
    expect(result).toBeUndefined();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("empty agents allowlist => disabled for all (secure default)", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    // agents omitted => defaults to [] => memory disabled until an agent opts in.
    const { hooks } = registerPlugin({});
    const result = await getHook(hooks, "before_prompt_build")(
      { prompt: "a real query", messages: [{ role: "user", content: "a real query" }] },
      { agentId: "main" },
    );
    expect(result).toBeUndefined();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe("agent_end — autoCapture to POST /ingest", () => {
  afterEach(() => vi.restoreAllMocks());

  test("stores capture-worthy user text on a successful turn (direct mode /ingest)", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(mockResponse({ id: 9, status: "created" }));
    const { hooks } = registerPlugin({
      agents: ["main"],
      autoCapture: true,
      captureMode: "direct",
    });

    await getHook(hooks, "agent_end")(
      {
        success: true,
        messages: [
          { role: "user", content: "I decided to use Helix as my primary editor going forward" },
          { role: "assistant", content: "Noted." },
        ],
      },
      { agentId: "main" },
    );

    const ingestCalls = fetchMock.mock.calls.filter((c) => (c[0] as string).endsWith("/ingest"));
    expect(ingestCalls.length).toBeGreaterThanOrEqual(1);
  });

  test("v1.20.1 default routes autoCapture to the proposal queue, not /ingest", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(mockResponse({ id: 42, status: "pending" }));
    // captureMode omitted => defaults to "proposal".
    const { hooks } = registerPlugin({ agents: ["main"], autoCapture: true });

    await getHook(hooks, "agent_end")(
      {
        success: true,
        messages: [
          { role: "user", content: "I decided to use Helix as my primary editor going forward" },
          { role: "assistant", content: "Noted." },
        ],
      },
      { agentId: "main" },
    );

    const direct = fetchMock.mock.calls.filter((c) => (c[0] as string).endsWith("/ingest"));
    const proposal = fetchMock.mock.calls.filter((c) =>
      (c[0] as string).endsWith("/ingest/proposal"),
    );
    expect(direct.length).toBe(0);
    expect(proposal.length).toBeGreaterThanOrEqual(1);
    const body = JSON.parse(proposal[0]![1]?.body as string);
    expect(body.content).toContain("Helix");
    expect(body.source_prompt).toContain("Noted.");
  });

  test("skips capture on a failed turn (success:false)", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ id: 1 }));
    const { hooks } = registerPlugin({ agents: ["main"], autoCapture: true });
    await getHook(hooks, "agent_end")(
      {
        success: false,
        messages: [{ role: "user", content: "I decided to remember this important fact" }],
      },
      { agentId: "main" },
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("autoCapture off => no /ingest call even with worthy text", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ id: 1 }));
    const { hooks } = registerPlugin({ agents: ["main"], autoCapture: false });
    await getHook(hooks, "agent_end")(
      {
        success: true,
        messages: [{ role: "user", content: "I decided to remember this important fact today" }],
      },
      { agentId: "main" },
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe("tools — error surfacing (404 vs 500, brain-server-specific)", () => {
  afterEach(() => vi.restoreAllMocks());

  test("memory_recall surfaces a 500 to the agent instead of an empty result", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("boom", { status: 500 }));
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools.get("memory_recall")!.execute("call-1", { query: "anything" });
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("Recall failed");
    expect(text).toContain("500");
  });

  test("memory_forget is not an agent tool (erasure is human-only)", async () => {
    const { tools } = registerPlugin({ agents: ["main"] });
    expect(tools.has("memory_forget")).toBe(false);
  });

  test("memory_store queues a proposal for human review by default (captureMode: proposal)", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ id: 42, status: "pending", novelty: 1 }),
    );
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools.get("memory_store")!.execute("call-1", { text: "a durable fact" });
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    const details = (res as { details: { pending: boolean; id: number; status: string } }).details;
    // The agent's write is queued for HUMAN review, never written straight to memory.
    expect(details.pending).toBe(true);
    expect(details.status).toBe("pending");
    expect(details.id).toBe(42);
    expect(text).toContain("Submitted for review");
  });

  test("memory_store with captureMode:direct writes straight to memory", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ id: 7, status: "created", entities_added: 2 }),
    );
    const { tools } = registerPlugin({ agents: ["main"], captureMode: "direct" });
    const res = await tools.get("memory_store")!.execute("call-1", { text: "a durable fact" });
    const details = (res as { details: { pending: boolean; status: string } }).details;
    expect(details.pending).toBe(false);
    expect(details.status).toBe("created");
  });

  test("memory_recall surfaces calibrated abstention (low_confidence) to the agent", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ hits: [], decision: "low_confidence" }),
    );
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools.get("memory_recall")!.execute("call-1", { query: "vague query" });
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("low confidence");
    expect(text).toContain("clarify");
  });

  test("memory_verify reports supported vs unsupported claim", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ chunk_id: 7, supported: true, decision: "supported", match_ranges: [[0, 5]] }),
    );
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools.get("memory_verify")!.execute("call-1", {
      chunk_id: 7,
      claim: "vitamin",
    });
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("Supported");
    expect((res as { details: { verified: boolean } }).details.verified).toBe(true);
  });

  test("memory_get returns the full chunk text", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ id: 7, title: "Bignay", content: "an antioxidant fruit" }),
    );
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools.get("memory_get")!.execute("call-1", { id: 7 });
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("an antioxidant fruit");
    expect((res as { details: { found: boolean } }).details.found).toBe(true);
  });

  test("memory_graph_entity returns entity relations", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({
        name: "bignay",
        type: "fruit",
        relations: [{ to_entity: "blueberry", relation_type: "alternative_to", direction: "in" }],
      }),
    );
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools.get("memory_graph_entity")!.execute("call-1", { name: "bignay" });
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("alternative_to");
    expect((res as { details: { found: boolean } }).details.found).toBe(true);
  });
});

describe("v0.3.0 — graph traverse, proposal review, advanced recall, corpus supplement", () => {
  afterEach(() => vi.restoreAllMocks());

  test("memory_graph_traverse forwards start/maxDepth/kind and maps the response", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({
        traversal: [
          {
            entity: "insulin resistance",
            depth: 1,
            path: "1",
            edge_path: "causes",
            from_entity: "metabolic syndrome",
            domain: "health",
          },
          {
            entity: "type 2 diabetes",
            depth: 2,
            path: "1->2",
            edge_path: "causes|causes",
            from_entity: "metabolic syndrome",
            domain: "health",
          },
        ],
        visited: 2,
      }),
    );
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools
      .get("memory_graph_traverse")!
      .execute("call-1", { start: "metabolic syndrome", maxDepth: 2, kind: "causes:" });
    const url = (fetchMock.mock.calls[0]?.[0] as string) ?? "";
    expect(url).toContain("/graph/traverse");
    expect(url).toContain("start=metabolic+syndrome");
    expect(url).toContain("max_depth=2");
    expect(url).toContain("kind=causes%3A");
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("type 2 diabetes");
    expect((res as { details: { found: boolean; visited: number } }).details).toMatchObject({
      found: true,
      visited: 2,
    });
  });

  test("memory_graph_traverse empty traversal => not found", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ traversal: [], visited: 0 }));
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools.get("memory_graph_traverse")!.execute("call-1", { start: "nothing" });
    expect((res as { details: { found: boolean } }).details.found).toBe(false);
  });

  test("memory_proposal_list is gated off unless proposalTools:true", () => {
    const { tools } = registerPlugin({ agents: ["main"] });
    expect(tools.has("memory_proposal_list")).toBe(false);
  });

  test("memory_proposal_list lists pending proposals", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse([
        {
          id: 42,
          kind: "fact",
          content: "prefers dark mode",
          novelty: 0.8,
          conflict_with: null,
          salience: 0.5,
          created_at: 1700000000,
          screen_verdict: "clean",
          expires_at: 1700604800,
          warn_secs: 3600,
          critical_secs: 300,
        },
      ]),
    );
    const { tools } = registerPlugin({ agents: ["main"], proposalTools: true });
    const res = await tools.get("memory_proposal_list")!.execute("call-1", { status: "pending" });
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("#42");
    expect(text).toContain("prefers dark mode");
    expect((res as { details: { count: number } }).details.count).toBe(1);
  });

  test("memory_proposal_decide approve promotes and forwards supersedes", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(
        mockResponse({ proposal_id: 42, chunk_id: 99, status: "approved", superseded: 7 }),
      );
    const { tools } = registerPlugin({ agents: ["main"], proposalTools: true });
    const res = await tools
      .get("memory_proposal_decide")!
      .execute("call-1", { id: 42, decision: "approve", supersedes: 7 });
    const url = (fetchMock.mock.calls[0]?.[0] as string) ?? "";
    expect(url).toContain("/proposals/42/approve");
    expect(url).toContain("supersedes=7");
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("chunk #99");
    expect((res as { details: { decided: boolean } }).details.decided).toBe(true);
  });

  test("memory_proposal_decide reject drops the proposal", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(mockResponse({ proposal_id: 42, status: "rejected" }));
    const { tools } = registerPlugin({ agents: ["main"], proposalTools: true });
    const res = await tools
      .get("memory_proposal_decide")!
      .execute("call-1", { id: 42, decision: "reject" });
    const url = (fetchMock.mock.calls[0]?.[0] as string) ?? "";
    expect(url).toContain("/proposals/42/reject");
    expect((res as { details: { decided: boolean } }).details.decided).toBe(true);
  });

  test("memory_recall forwards the v0.3.0 advanced params", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(
        mockResponse({ hits: [{ id: 1, content: "x", score: 0.5 }], decision: "ok" }),
      );
    const { tools } = registerPlugin({ agents: ["main"] });
    await tools.get("memory_recall")!.execute("call-1", {
      query: "what was true then",
      at: "2024-01-01",
      memoryKind: "fact",
      minRelevance: "high",
      graph: true,
      maxContextTokens: 1000,
    });
    const body = JSON.parse((fetchMock.mock.calls[0]?.[1]?.body as string) ?? "{}");
    expect(body.at).toBe("2024-01-01");
    expect(body.memory_kind).toBe("fact");
    expect(body.min_relevance).toBe("high");
    expect(body.graph).toBe(true);
    expect(body.max_context_tokens).toBe(1000);
  });

  test("auto-recall forwards autoRecallGraph + autoRecallMaxContextTokens", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    const { hooks } = registerPlugin({
      agents: ["main"],
      autoRecallGraph: true,
      autoRecallMaxContextTokens: 2048,
    });
    await getHook(hooks, "before_prompt_build")(
      { prompt: "a real query", messages: [{ role: "user", content: "a real query" }] },
      { agentId: "main" },
    );
    const body = JSON.parse((fetchMock.mock.calls[0]?.[1]?.body as string) ?? "{}");
    expect(body.graph).toBe(true);
    expect(body.max_context_tokens).toBe(2048);
  });

  test("registers a memory corpus supplement whose search maps recall hits", async () => {
    let supplement:
      | {
          search(p: {
            query: string;
            maxResults?: number;
            agentId?: string;
            sandboxed?: boolean;
          }): Promise<unknown[]>;
        }
      | undefined;
    const api = {
      pluginConfig: { agents: ["main"] },
      runtime: {},
      logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
      on: vi.fn(),
      registerTool: vi.fn(),
      registerService: vi.fn(),
      registerMemoryCapability: vi.fn(),
      registerMemoryCorpusSupplement: vi.fn((s) => {
        supplement = s;
      }),
    } as unknown as MockApi;
    plugin.register(api as unknown as OpenClawPluginApi);
    expect(api.registerMemoryCorpusSupplement).toHaveBeenCalledTimes(1);
    expect(supplement).toBeDefined();
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({
        hits: [{ id: 9, content: "a fact", title: "t", domain: "health", score: 0.7 }],
      }),
    );
    const out = (await supplement!.search({ query: "fact", agentId: "main" })) as Array<{
      corpus: string;
      snippet: string;
      id: string;
    }>;
    expect(out.length).toBe(1);
    expect(out[0]?.corpus).toBe("brain-server");
    expect(out[0]?.id).toBe("9");
    expect(out[0]?.snippet).toContain("a fact");
  });

  test("corpus supplement search honors the agent allowlist (empty => none)", async () => {
    let supplement:
      | { search(p: { query: string; agentId?: string }): Promise<unknown[]> }
      | undefined;
    const api = {
      // No agents opted in => gate denies everyone.
      pluginConfig: { agents: [] },
      runtime: {},
      logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
      on: vi.fn(),
      registerTool: vi.fn(),
      registerService: vi.fn(),
      registerMemoryCapability: vi.fn(),
      registerMemoryCorpusSupplement: vi.fn((s) => {
        supplement = s;
      }),
    } as unknown as MockApi;
    plugin.register(api as unknown as OpenClawPluginApi);
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ hits: [] }));
    const out = await supplement!.search({ query: "anything", agentId: "main" });
    expect(out).toEqual([]);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe("v0.4.0 — procedural memory (runbooks, decision trees)", () => {
  afterEach(() => vi.restoreAllMocks());

  test("memory_procedure_get renders the ordered step chain", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({
        procedure_id: 7,
        title: "AI readiness",
        content: "Overview",
        steps: [
          {
            step_index: 0,
            id: 8,
            title: "Inventory",
            content: "list software",
            memory_kind: "step",
          },
          { step_index: 1, id: 9, title: "Decide tier", content: "{}", memory_kind: "decision" },
        ],
      }),
    );
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools.get("memory_procedure_get")!.execute("call-1", { id: 7 });
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("Runbook #7");
    expect(text).toContain("Inventory");
    expect(text).toContain("[decision]");
    expect((res as { details: { found: boolean; stepCount: number } }).details).toMatchObject({
      found: true,
      stepCount: 2,
    });
  });

  test("memory_procedure_get 404 => not found", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("no procedure", { status: 404 }));
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools.get("memory_procedure_get")!.execute("call-1", { id: 99 });
    expect((res as { details: { found: boolean } }).details.found).toBe(false);
  });

  test("memory_procedure_store forwards title/content/steps + is_decision", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(mockResponse({ id: 7, status: "created", step_ids: [8, 9] }));
    const { tools } = registerPlugin({ agents: ["main"] });
    expect(tools.has("memory_procedure_store")).toBe(true);
    const res = await tools.get("memory_procedure_store")!.execute("call-1", {
      title: "Onboarding",
      content: "overview",
      steps: [{ title: "s1", content: "do x", isDecision: true }],
    });
    const body = JSON.parse((fetchMock.mock.calls[0]?.[1]?.body as string) ?? "{}");
    expect(body.title).toBe("Onboarding");
    expect(body.steps[0].is_decision).toBe(true);
    expect(
      (res as { details: { stored: boolean; id: number; stepIds: number[] } }).details,
    ).toMatchObject({
      stored: true,
      id: 7,
      stepIds: [8, 9],
    });
  });

  test("memory_decision_evaluate forwards variables and maps a matched branch", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({
        result: "enterprise",
        matched_condition: "employee_count >= 50",
        used_default: false,
      }),
    );
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools
      .get("memory_decision_evaluate")!
      .execute("call-1", { id: 5, variables: { employee_count: 75 } });
    const url = (fetchMock.mock.calls[0]?.[0] as string) ?? "";
    expect(url).toContain("/decision/5/evaluate");
    const body = JSON.parse((fetchMock.mock.calls[0]?.[1]?.body as string) ?? "{}");
    expect(body.variables.employee_count).toBe(75);
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("enterprise");
    expect(text).toContain("employee_count >= 50");
  });

  test("memory_decision_evaluate default branch renders as default", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ result: "small-business", used_default: true }),
    );
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools
      .get("memory_decision_evaluate")!
      .execute("call-1", { id: 5, variables: {} });
    const text = (res as { content: Array<{ text: string }> }).content[0]?.text ?? "";
    expect(text).toContain("small-business");
    expect(text).toContain("default");
  });

  test("memory_decision_evaluate 404 => not found", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("no rule", { status: 404 }));
    const { tools } = registerPlugin({ agents: ["main"] });
    const res = await tools
      .get("memory_decision_evaluate")!
      .execute("call-1", { id: 5, variables: {} });
    expect((res as { details: { evaluated: boolean } }).details.evaluated).toBe(false);
  });
});

// v1.20.29 "Bound" — request-amplification bound + param clamp + body cap.
//   - F-6: inflight de-dup collapses same-query same-turn recalls to ONE POST;
//     a per-session cap no-ops beyond MAX_RECALLS_PER_TURN.
//   - F-7: `memory_recall.maxContextTokens` schema max is now 8_000 (was 32k),
//     and the per-hit body cap (MAX_HIT_CHARS) truncates huge chunks before
//     formatting. format.ts is untouched (Release C owns it).
describe('v1.20.29 "Bound" — inflight de-dup, per-session cap, body + token clamp', () => {
  afterEach(() => vi.restoreAllMocks());

  test("inflight dedup collapses same-query same-turn to one POST", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(
      // Delay so both callers are inflight at once (the dedup only collapses
      // concurrent calls; sequential calls resolve + evict before the next).
      () =>
        new Promise<Response>((resolve) => {
          setTimeout(() => {
            resolve(
              mockResponse({
                hits: [{ id: 1, content: "prefers Helix", score: 0.9, untrusted: true }],
              }),
            );
          }, 10);
        }),
    );
    const { hooks } = registerPlugin({ agents: ["main"] });
    const fire = () =>
      getHook(hooks, "before_prompt_build")(
        {
          prompt: "what editor?",
          messages: [{ role: "user", content: "what editor should i use?" }],
        },
        { agentId: "main" },
      );
    // Two CONCURRENT same-query recalls in one turn.
    const [a, b] = await Promise.all([fire(), fire()]);
    const recallCalls = fetchMock.mock.calls.filter((c) => (c[0] as string).endsWith("/recall"));
    expect(recallCalls).toHaveLength(1); // collapsed to a single server POST
    expect(a).toEqual({ prependContext: expect.stringContaining("prefers Helix") });
    expect(b).toEqual({ prependContext: expect.stringContaining("prefers Helix") });
  });

  test("per-session cap returns empty (no-op) after the ceiling", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(mockResponse({ hits: [{ id: 1, content: "a fact", score: 0.5 }] }));
    const { hooks } = registerPlugin({ agents: ["main"] });
    const fire = (i: number) =>
      getHook(hooks, "before_prompt_build")(
        { prompt: `query number ${i}`, messages: [{ role: "user", content: `query number ${i}` }] },
        { agentId: "main" },
      );
    // Distinct queries => no dedup => each consumes one slot of the cap.
    for (let i = 0; i < 10; i++) {
      await fire(i);
    }
    const callsBeforeCap = fetchMock.mock.calls.filter((c) =>
      (c[0] as string).endsWith("/recall"),
    ).length;
    expect(callsBeforeCap).toBe(10);
    // 11th distinct query => cap exceeded => no-op (undefined, no POST).
    const over = await fire(100);
    expect(over).toBeUndefined();
    const callsAfterCap = fetchMock.mock.calls.filter((c) =>
      (c[0] as string).endsWith("/recall"),
    ).length;
    expect(callsAfterCap).toBe(10); // the 11th never reached the server
  });

  test("hit body clamped + memory_recall maxContextTokens schema is 8000", async () => {
    // (a) Per-hit body cap: a hit whose content far exceeds MAX_HIT_CHARS is
    // truncated to MAX_HIT_CHARS on the caller side before formatting.
    const longContent = "A".repeat(MAX_HIT_CHARS * 4);
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(
        mockResponse({ hits: [{ id: 1, content: longContent, score: 0.9, untrusted: true }] }),
      );
    const { hooks } = registerPlugin({ agents: ["main"] });
    const out = (await getHook(hooks, "before_prompt_build")(
      { prompt: "a real query", messages: [{ role: "user", content: "a real query" }] },
      { agentId: "main" },
    )) as { prependContext: string } | undefined;
    expect(out).toBeDefined();
    // The injected block contains at most MAX_HIT_CHARS of the hit's content
    // (plus the anti-injection banner — so it's bounded but >= the truncated body).
    expect(out!.prependContext).toContain("A".repeat(MAX_HIT_CHARS));
    expect(out!.prependContext).not.toContain("A".repeat(MAX_HIT_CHARS + 1));

    // (b) Schema max for maxContextTokens is now 8_000 (not 32_000): a recall
    //     at the 8k ceiling is forwarded; a value over the new ceiling is
    //     dropped by the Typebox Check guard (params collapse to {} => no query).
    const { tools } = registerPlugin({ agents: ["main"] });
    fetchMock.mockClear();
    fetchMock.mockResolvedValue(mockResponse({ hits: [{ id: 1, content: "x", score: 0.5 }] }));
    await tools.get("memory_recall")!.execute("c1", { query: "what", maxContextTokens: 8_000 });
    const atCeiling = JSON.parse((fetchMock.mock.calls[0]?.[1]?.body as string) ?? "{}");
    expect(atCeiling.max_context_tokens).toBe(8_000);

    fetchMock.mockClear();
    const overCeiling = await tools
      .get("memory_recall")!
      .execute("c2", { query: "what", maxContextTokens: 32_000 });
    // Check failed (32k > 8k max) => params => {} => "No query provided." + no POST.
    expect((overCeiling as { content: { text: string }[] }).content[0]!.text).toContain(
      "No query provided",
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
