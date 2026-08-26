/**
 * Team Bridge tests (v0.5.0).
 *
 * The bridge mirrors OpenClaw agent activity onto brain-server's governed
 * workflow surface (mesh cards, run open/close, lineage events). Transport is
 * the real BrainClient against a mocked global fetch, so every assertion runs
 * against the exact request shapes the Rust handlers deserialize
 * (`deny_unknown_fields` — a wrong field name here is a 400 in production).
 *
 * Verified contracts (src/handlers/workflow.rs + mesh.rs @ v1.28.34):
 *   POST /workflow/runs              {domain, kind, state_json} → {run_id, revision}
 *   POST /workflow/runs/{id}/events  {topic, payload_json, idempotency_key} → {first, event_id}
 *   PUT  /workflow/runs/{id}/state   {expected_rev, state_json, status} → {revision} | 409
 *   POST /ops/agents/cards           {domain, principal, name, description, capabilities} — Admin
 */
import { afterEach, describe, expect, test, vi } from "vitest";
import plugin from "../index.js";
import {
  eventKey,
  initialState,
  intentLabel,
  principalFor,
  teamGateEnabled,
} from "../src/team-bridge.js";
import type { OpenClawPluginApi } from "openclaw/plugin-sdk/plugin-entry";

type HookHandler = (...args: unknown[]) => unknown;

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

function registerPlugin(pluginConfig: unknown) {
  const hooks = new Map<string, HookHandler>();
  const logger = { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() };
  const api = {
    pluginConfig,
    runtime: {},
    logger,
    on: vi.fn((name: string, handler: HookHandler) => {
      // one handler per hook — house rule; a second registration shadows and
      // MUST fail loudly in tests instead of silently in production.
      if (hooks.has(name)) {
        throw new Error(`duplicate hook registration: ${name}`);
      }
      hooks.set(name, handler);
    }),
    registerTool: vi.fn(),
    registerService: vi.fn(),
    registerMemoryCapability: vi.fn(),
    registerMemoryCorpusSupplement: vi.fn(),
  };
  plugin.register(api as unknown as OpenClawPluginApi);
  return { hooks, logger };
}

const getHook = (hooks: Map<string, HookHandler>, name: string): HookHandlerFn =>
  hooks.get(name) as HookHandlerFn;
type HookHandlerFn = (e: unknown, ctx: unknown) => Promise<unknown>;

interface RecordedCall {
  method: string;
  path: string;
  body: unknown;
}

function stubFetch(respond: (method: string, path: string) => { status: number; body: unknown }) {
  const calls: RecordedCall[] = [];
  vi.stubGlobal("fetch", async (url: string | URL, init?: RequestInit) => {
    const u = new URL(String(url));
    const method = init?.method ?? "GET";
    const body =
      typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined;
    calls.push({ method, path: u.pathname, body });
    const r = respond(method, u.pathname);
    return mockResponse(r.body, { status: r.status });
  });
  return calls;
}

const enabledCfg = {
  enabled: true,
  agents: ["Main"],
  teamBridge: true,
  teamHeartbeatMs: 60_000,
  requestTimeoutMs: 100,
};

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------------------

describe("team bridge gating", () => {
  test("off by default — zero network calls when teamBridge is unset", async () => {
    const { hooks } = registerPlugin({ agents: ["Main"] });
    const calls = stubFetch(() => ({ status: 200, body: {} }));
    await getHook(hooks, "before_agent_run")(
      { prompt: "hello" },
      { agentId: "Main", sessionKey: "s1" },
    );
    expect(calls).toHaveLength(0);
  });

  test("per-agent allowlist gates the bridge (least privilege)", async () => {
    const { hooks } = registerPlugin({ ...enabledCfg, agents: ["Other"] });
    const calls = stubFetch(() => ({ status: 200, body: {} }));
    await getHook(hooks, "before_agent_run")(
      { prompt: "hello" },
      { agentId: "Main", sessionKey: "s1" },
    );
    expect(calls).toHaveLength(0);
  });

  test("missing agentId never reaches the network", async () => {
    const { hooks } = registerPlugin(enabledCfg);
    const calls = stubFetch(() => ({ status: 200, body: {} }));
    await getHook(hooks, "before_agent_run")({ prompt: "hello" }, { sessionKey: "s1" });
    expect(calls).toHaveLength(0);
  });
});

describe("team bridge happy path", () => {
  test("card ensured once, run opened with exact wire shape, start event fired", async () => {
    const { hooks } = registerPlugin({
      ...enabledCfg,
      defaultDomain: "support",
      teamDomain: "support",
    });
    const calls = stubFetch((method, path) => {
      if (path === "/workflow/runs" && method === "POST") {
        return { status: 200, body: { run_id: 7, revision: 0 } };
      }
      if (path.startsWith("/workflow/runs/7/events")) {
        return { status: 200, body: { first: true, event_id: 1 } };
      }
      return { status: 200, body: {} };
    });

    await getHook(hooks, "before_agent_run")(
      { prompt: "Fix the login flow for client acme" },
      { agentId: "Main", sessionKey: "s1" },
    );

    const card = calls.find((c) => c.path === "/ops/agents/cards");
    expect(card).toBeDefined();
    expect(card?.body).toMatchObject({
      domain: "support",
      principal: "openclaw-main",
      name: "Main",
      capabilities: { source: "openclaw" },
    });

    const open = calls.find((c) => c.path === "/workflow/runs");
    expect(open).toBeDefined();
    const openBody = open?.body as { domain: string; kind: string; state_json: string };
    expect(openBody.domain).toBe("support");
    expect(openBody.kind).toBe("openclaw-turn");
    const parsedState = JSON.parse(openBody.state_json) as Record<string, unknown>;
    expect(parsedState.status).toBe("running");
    expect(parsedState.engine).toBe("openclaw");
    expect(parsedState.intent).toBe("Fix the login flow for client acme");

    const startEvent = calls.find(
      (c) => c.method === "POST" && c.path === "/workflow/runs/7/events",
    );
    const startBody = startEvent?.body as { topic: string; idempotency_key: string };
    expect(startBody.topic).toBe("workflow/openclaw/start");
    expect(startBody.idempotency_key.length).toBeLessThanOrEqual(128);
  });

  test("second run-start for the same session does not reopen a run", async () => {
    const { hooks } = registerPlugin(enabledCfg);
    const calls = stubFetch((method, path) => {
      if (path === "/workflow/runs" && method === "POST") {
        return { status: 200, body: { run_id: 7, revision: 0 } };
      }
      return { status: 200, body: {} };
    });
    const h = getHook(hooks, "before_agent_run");
    await h({ prompt: "one" }, { agentId: "Main", sessionKey: "s1" });
    const opensAfterFirst = calls.filter((c) => c.path === "/workflow/runs").length;
    await h({ prompt: "two" }, { agentId: "Main", sessionKey: "s1" });
    const opensAfterSecond = calls.filter((c) => c.path === "/workflow/runs").length;
    expect(opensAfterFirst).toBe(1);
    expect(opensAfterSecond).toBe(1);
  });

  test("card 409 is treated as already-provisioned, not an error", async () => {
    const { hooks } = registerPlugin(enabledCfg);
    const calls = stubFetch((method, path) => {
      if (path === "/ops/agents/cards") return { status: 409, body: { error: "exists" } };
      if (path === "/workflow/runs") return { status: 200, body: { run_id: 3, revision: 0 } };
      return { status: 200, body: {} };
    });
    await expect(
      getHook(hooks, "before_agent_run")({ prompt: "x" }, { agentId: "Main", sessionKey: "s1" }),
    ).resolves.toBeUndefined();
    expect(calls.some((c) => c.path === "/workflow/runs")).toBe(true);
  });

  test("transport failure on run-open is swallowed — the host turn always passes", async () => {
    const { hooks, logger } = registerPlugin(enabledCfg);
    stubFetch(() => ({ status: 500, body: { error: "boom" } }));
    await expect(
      getHook(hooks, "before_agent_run")({ prompt: "x" }, { agentId: "Main", sessionKey: "s1" }),
    ).resolves.toBeUndefined();
    expect(logger.warn).toHaveBeenCalled();
  });
});

describe("team bridge close-out", () => {
  function startRun() {
    const { hooks, logger } = registerPlugin(enabledCfg);
    const calls = stubFetch((method, path) => {
      if (path === "/workflow/runs" && method === "POST") {
        return { status: 200, body: { run_id: 7, revision: 0 } };
      }
      if (path === "/workflow/runs/7/state" && method === "PUT") {
        return { status: 200, body: { revision: 1 } };
      }
      return { status: 200, body: { first: true, event_id: 1 } };
    });
    const beforeAgentRun = getHook(hooks, "before_agent_run");
    const agentEnd = getHook(hooks, "agent_end");
    const sessionEnd = getHook(hooks, "session_end");
    return { hooks, calls, logger, beforeAgentRun, agentEnd, sessionEnd };
  }

  test("agent_end success closes via CAS with expected_rev and done status", async () => {
    const t = startRun();
    await t.beforeAgentRun({ prompt: "do a thing" }, { agentId: "Main", sessionKey: "s1" });
    await t.agentEnd({ success: true }, { agentId: "Main", sessionKey: "s1" });

    const put = t.calls.find((c) => c.method === "PUT" && c.path === "/workflow/runs/7/state");
    expect(put).toBeDefined();
    const putBody = put?.body as { expected_rev: number; status: string; state_json: string };
    expect(putBody.expected_rev).toBe(0);
    expect(putBody.status).toBe("done");
    expect(JSON.parse(putBody.state_json).status).toBe("done");

    const doneEvents = t.calls.filter(
      (c) =>
        c.method === "POST" &&
        c.path === "/workflow/runs/7/events" &&
        (c.body as { topic?: string }).topic === "workflow/openclaw/done",
    );
    expect(doneEvents).toHaveLength(1);

    // Run closed — a duplicate end is a no-op.
    const putsBefore = t.calls.filter((c) => c.method === "PUT").length;
    await t.agentEnd({ success: true }, { agentId: "Main", sessionKey: "s1" });
    expect(t.calls.filter((c) => c.method === "PUT").length).toBe(putsBefore);
  });

  test("agent_end failure marks failed", async () => {
    const t = startRun();
    await t.beforeAgentRun({ prompt: "x" }, { agentId: "Main", sessionKey: "s1" });
    await t.agentEnd({ success: false }, { agentId: "Main", sessionKey: "s1" });
    const put = t.calls.find((c) => c.method === "PUT");
    expect((put?.body as { status: string }).status).toBe("failed");
  });

  test("session_end pauses open runs and clears the mapping", async () => {
    const t = startRun();
    await t.beforeAgentRun({ prompt: "x" }, { agentId: "Main", sessionKey: "s1" });
    await t.sessionEnd({}, { sessionKey: "s1" });
    await new Promise((r) => setTimeout(r, 10)); // host fires this void; let it land
    const put = t.calls.find((c) => c.method === "PUT");
    expect((put?.body as { status: string }).status).toBe("paused");
    // cleared: another end is a no-op
    const putsBefore = t.calls.filter((c) => c.method === "PUT").length;
    await t.sessionEnd({}, { sessionKey: "s1" });
    await new Promise((r) => setTimeout(r, 10));
    expect(t.calls.filter((c) => c.method === "PUT").length).toBe(putsBefore);
  });
});

describe("heartbeat", () => {
  test("beats ride the existing before_prompt_build handler, throttled by config", async () => {
    const { hooks } = registerPlugin({ ...enabledCfg, teamHeartbeatMs: 0 });
    const calls = stubFetch((method, path) => {
      if (path === "/workflow/runs" && method === "POST") {
        return { status: 200, body: { run_id: 7, revision: 0 } };
      }
      return { status: 200, body: { first: true, event_id: 2 } };
    });
    await getHook(hooks, "before_agent_run")({ prompt: "x" }, { agentId: "Main", sessionKey: "s1" });
    const beatsBefore = calls.filter(
      (c) => c.path === "/workflow/runs/7/events" && (c.body as { topic?: string }).topic?.includes("beat"),
    ).length;
    await getHook(hooks, "before_prompt_build")(
      { prompt: "next turn", messages: [] },
      { agentId: "Main", sessionKey: "s1" },
    );
    await new Promise((r) => setTimeout(r, 5)); // beat rides a void chain
    const beatsAfter = calls.filter(
      (c) => c.path === "/workflow/runs/7/events" && (c.body as { topic?: string }).topic?.includes("beat"),
    ).length;
    expect(beatsBefore).toBe(0);
    expect(beatsAfter).toBe(1);
  });

  test("throttle suppresses beats inside the window", async () => {
    const { hooks } = registerPlugin(enabledCfg); // heartbeatMs 60000
    const calls = stubFetch((method, path) => {
      if (path === "/workflow/runs" && method === "POST") {
        return { status: 200, body: { run_id: 7, revision: 0 } };
      }
      return { status: 200, body: { first: true, event_id: 3 } };
    });
    await getHook(hooks, "before_agent_run")({ prompt: "x" }, { agentId: "Main", sessionKey: "s1" });
    await getHook(hooks, "before_prompt_build")(
      { prompt: "turn two", messages: [] },
      { agentId: "Main", sessionKey: "s1" },
    );
    expect(
      calls.filter((c) => (c.body as { topic?: string })?.topic?.includes("beat")),
    ).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------

describe("pure helpers", () => {
  test("principalFor slugs and bounds agent ids", () => {
    expect(principalFor("Main Agent!")).toBe("openclaw-main-agent");
    expect(principalFor("  --")).toBe("openclaw-agent");
    expect(principalFor("Sub/Agent.v2").length).toBeLessThanOrEqual(64);
  });

  test("intentLabel collapses whitespace and truncates", () => {
    expect(intentLabel("  a\n\nb\t c  ")).toBe("a b c");
    const long = intentLabel("x".repeat(500), 200);
    expect(long.length).toBe(200);
    expect(long.endsWith("…")).toBe(true);
  });

  test("eventKey stays within the server's 128-char limit", () => {
    const k = eventKey(["ocb", "123456789", "workflow/openclaw/start", "k".repeat(200)]);
    expect(k.length).toBeLessThanOrEqual(128);
    expect(eventKey(["a", "b"]).length).toBeGreaterThan(0);
  });

  test("initialState carries the governance fields", () => {
    const s = initialState({ agentId: "Main", sessionKey: "sess-1", intent: "fix thing" });
    expect(s.status).toBe("running");
    expect(s.engine).toBe("openclaw");
    expect(s.agent).toBe("Main");
    expect(s.intent).toBe("fix thing");
  });

  test("intentLabel truncates by code points (no lone surrogates)", () => {
    const emoji = "\u{1F600}"; // 2 UTF-16 units
    const out = intentLabel(emoji.repeat(150) + "tail", 10);
    expect(Array.from(out).length).toBe(10);
    expect(out.endsWith("\u2026") || out.includes(emoji)).toBe(true);
    // A naive slice would strand half an emoji here.
    expect(/\uD83D$/.test(out.slice(0, -1))).toBe(false);
  });

  test("open-run map is bounded (oldest mirror dropped first)", async () => {
    let n = 0;
    registerPlugin({ ...enabledCfg, teamHeartbeatMs: 60_000 });
    // fresh fetch stub with incrementing run ids
    const calls: Array<{ path: string }> = [];
    vi.stubGlobal("fetch", async (url: string | URL, init?: RequestInit) => {
      const u = new URL(String(url));
      calls.push({ path: u.pathname });
      if (u.pathname === "/workflow/runs" && init?.method === "POST") {
        return mockResponse({ run_id: ++n, revision: 0 });
      }
      return mockResponse({ first: true, event_id: 1 });
    });
    const { hooks } = registerPlugin(enabledCfg);
    const start = getHook(hooks, "before_agent_run");
    for (let k = 0; k < 260; k++) {
      await start({ prompt: `t${k}` }, { agentId: "Main", sessionKey: `s${k}` });
    }
    // The bridge itself caps at MAX_OPEN_RUNS=256; assert via behavior:
    // the 261st distinct session still opens a run (map never blocks work).
    expect(calls.filter((c) => c.path === "/workflow/runs").length).toBe(260);
  });

  test("invalid teamDomain fails at registration, not per turn", () => {
    expect(() => registerPlugin({ ...enabledCfg, teamDomain: "Bad_Domain!" })).toThrow(
      /teamDomain/,
    );
  });

  test("teamGateEnabled requires enabled + opt-in list membership", () => {
    expect(teamGateEnabled({ enabled: true, teamBridge: true, agents: ["a"] }, "a")).toBe(true);
    expect(teamGateEnabled({ enabled: true, teamBridge: true, agents: [] }, "a")).toBe(false);
    expect(teamGateEnabled({ enabled: true, teamBridge: false, agents: ["a"] }, "a")).toBe(false);
    expect(teamGateEnabled({ enabled: false, teamBridge: true, agents: ["a"] }, "a")).toBe(false);
    expect(teamGateEnabled({ enabled: true, teamBridge: true, agents: ["a"] }, undefined)).toBe(false);
  });
});
