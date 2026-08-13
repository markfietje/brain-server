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
      .mockResolvedValue(mockResponse({ hits: [{ id: 1, content: "prefers Helix", score: 0.9 }] }));

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
    const body = JSON.parse(String(proposal[0][1]?.body));
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
      mockResponse({ id: 42, status: "pending", novelty: 1.0 }),
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
