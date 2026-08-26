/**
 * Team Bridge — v0.5.0.
 *
 * Mirrors OpenClaw agent activity onto brain-server's governed-workflow
 * surface so the console dashboards (Ops boards, Crew roster, run timelines,
 * Scoreboard) show the AI team exactly the way they show human teams.
 *
 * INTENT
 *   Peter Steinberger's point (Aug 2026): "cli is nice, having UI
 *   visualizations and your team where you work is nicer." A terminal-only
 *   agent is invisible work. This module makes OpenClaw agents visible by
 *   seating them in brain-server's existing governance surfaces:
 *
 *     Mesh cards      POST /ops/agents/cards        signed identity per agent
 *     Crew roster     crew_touch (rides every mutating tx server-side)
 *     Run timelines   POST /workflow/runs + /events (lineage, exactly-once)
 *     Scoreboard      close-out states aggregate into verified outcomes
 *
 * PRIVACY POSTURE
 *   - Off by default (`teamBridge: false`). Visibility is an operator choice,
 *     exactly like autoCapture.
   - Gated by the SAME per-agent allowlist as recall (least privilege).
 *   - Only a truncated intent label (first 200 chars, whitespace-collapsed)
 *     enters run state. Full prompts/messages NEVER leave the host process.
 *
 * FAILURE POSTURE
 *   - Every network call is fire-and-forget from the host's perspective:
 *     handlers resolve `undefined` even when transport rejects. The bridge
 *     can never stall or fail an agent turn — worst case, the dashboard is
 *     briefly stale.
 */
import type { OpenClawPluginApi } from "openclaw/plugin-sdk/plugin-entry";
import type { BrainClient } from "./brain-client.js";
import type { ResolvedBrainConfig } from "./config.js";
import { sanitizeForBlock } from "./format.js";

// --------------------------------------------------------------------------
// Pure helpers (exported for tests)
// --------------------------------------------------------------------------

/** Lowercase slug of an agent id, safe for mesh-card principals. */
export function principalFor(agentId: string): string {
  const slug = agentId
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/-{2,}/g, "-")
    .replace(/^-|-$/g, "");
  return `openclaw-${slug || "agent"}`.slice(0, 64);
}

/** Collapse whitespace/control chars and truncate — dashboard-safe intent. */
export function intentLabel(prompt: string, max = 200): string {
  const flat = prompt.replace(/[\u0000-\u001f\u007f]+/g, " ").replace(/\s+/g, " ").trim();
  // Truncate by CODE POINTS — naive .slice() can split an emoji surrogate
  // pair and produce a lone surrogate that survives into JSON.
  const cps = Array.from(flat);
  return cps.length <= max ? flat : `${cps.slice(0, max - 1).join("")}…`;
}

/** Tiny stable hash so idempotency keys stay bounded for long session keys. */
export function sessionHash(sessionKey: string): string {
  let h = 5381;
  for (let i = 0; i < sessionKey.length; i++) {
    h = ((h << 5) + h + sessionKey.charCodeAt(i)) | 0;
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}

const KEY_MAX = 128;

/** Idempotency key builder; hard-bounded to the server's 128-char limit. */
export function eventKey(parts: ReadonlyArray<string>): string {
  const joined = parts.join(":");
  if (joined.length <= KEY_MAX) {
    return joined;
  }
  const tail = sessionHash(joined);
  return `${joined.slice(0, KEY_MAX - tail.length - 1)}~${tail}`;
}

export function initialState(args: {
  agentId: string;
  sessionKey: string;
  intent: string;
}): Record<string, unknown> {
  return {
    status: "running",
    engine: "openclaw",
    agent: args.agentId,
    session: sessionHash(args.sessionKey),
    intent: args.intent,
  };
}

export function endState(
  base: Record<string, unknown>,
  outcome: "done" | "failed" | "paused",
): Record<string, unknown> {
  return { ...base, status: outcome };
}

/** Least-privilege gate: enabled flag AND explicit per-agent opt-in. */
export function teamGateEnabled(
  cfg: { enabled: boolean; teamBridge: boolean; agents: string[] },
  agentId: string | undefined,
): boolean {
  return (
    cfg.enabled && cfg.teamBridge && agentId !== undefined && cfg.agents.includes(agentId)
  );
}

// --------------------------------------------------------------------------
// Bridge core
// --------------------------------------------------------------------------

interface RunHandle {
  runId: number;
  revision: number;
  seq: number;
  state: Record<string, unknown>;
}

export interface BridgeLogger {
  info?: (msg: string) => void;
  warn?: (msg: string) => void;
}

export interface TeamBridgeOptions {
  /** Injectable clock for deterministic heartbeat tests. */
  now?: () => number;
}

export class TeamBridge {
  private readonly runs = new Map<string, RunHandle>();
  private static readonly MAX_OPEN_RUNS = 256;
  private readonly cardsEnsured = new Set<string>();
  private readonly lastBeat = new Map<string, number>();
  private readonly now: () => number;

  constructor(
    private readonly client: BrainClient,
    private readonly cfg: () => {
      teamDomain: string;
      heartbeatMs: number;
      requestTimeoutMs: number;
      agents: string[];
    },
    private readonly log: BridgeLogger,
    opts?: TeamBridgeOptions,
  ) {
    this.now = opts?.now ?? Date.now;
  }

  get openRuns(): number {
    return this.runs.size;
  }

  private conf() {
    return this.cfg();
  }

  private key(ctx: { agentId?: string; sessionKey?: string }): string | undefined {
    if (!ctx.agentId || !ctx.sessionKey) {
      return undefined;
    }
    return `${ctx.agentId}\u0000${ctx.sessionKey}`;
  }

  private async postEvent(handle: RunHandle, topic: string, payload: object): Promise<void> {
    handle.seq += 1;
    const key = eventKey(["ocb", String(handle.runId), topic, String(handle.seq)]);
    await this.client.fetchJson<{ first?: boolean; event_id?: number }>(
      `/workflow/runs/${handle.runId}/events`,
      "POST",
      {
        topic,
        payload_json: JSON.stringify(payload),
        idempotency_key: key,
      },
      this.conf().requestTimeoutMs,
    );
  }

  /**
   * before_agent_run: ensure card (once per agent), open a governed run for
   * this session if one isn't already open, append the start lineage event.
   */
  async onAgentRunStart(ctx: {
    agentId?: string;
    sessionKey?: string;
    prompt?: string;
  }): Promise<void> {
    const key = this.key(ctx);
    const conf = this.conf();
    if (!key || !ctx.agentId || !ctx.sessionKey) {
      return;
    }
    if (this.runs.has(key)) {
      return; // run already open for this session — nothing to do
    }
    // Bound: a long-lived host that never fires session_end must not grow
    // this map forever. Oldest mirrored run is dropped first (its server-side
    // run stays active and is swept by the session_end pause path).
    while (this.runs.size >= TeamBridge.MAX_OPEN_RUNS) {
      const oldest = this.runs.keys().next().value;
      if (oldest === undefined) break;
      this.runs.delete(oldest);
      this.lastBeat.delete(oldest);
      this.log.warn?.(`team-bridge: open-run cap hit — dropped oldest mirror (${oldest.replace("\u0000", "/")})`);
    }

    const principal = principalFor(ctx.agentId);

    // Card: once per agent. 409 = someone else provisioned it — fine.
    if (!this.cardsEnsured.has(principal)) {
      try {
        await this.client.fetchJson(
          "/ops/agents/cards",
          "POST",
          {
            domain: conf.teamDomain,
            principal,
            name: ctx.agentId.slice(0, 64),
            description: "OpenClaw agent (auto-provisioned by the brain-server team bridge)",
            capabilities: { source: "openclaw" },
          },
          conf.requestTimeoutMs,
        );
        this.cardsEnsured.add(principal);
      } catch (err) {
        const status = (err as { status?: number }).status;
        if (status === 409) {
          this.cardsEnsured.add(principal);
        } else {
          // Card is optional visibility — a failure must not block the run.
          this.log.warn?.(`team-bridge: card ensure failed (${sanitizeForBlock(String(err))})`);
        }
      }
    }

    const intent = intentLabel(ctx.prompt ?? "", 200);
    const opened = await this.client.fetchJson<{ run_id?: number; revision?: number }>(
      "/workflow/runs",
      "POST",
      {
        domain: conf.teamDomain,
        kind: "openclaw-turn",
        state_json: JSON.stringify(
          initialState({ agentId: ctx.agentId, sessionKey: ctx.sessionKey, intent }),
        ),
      },
      conf.requestTimeoutMs,
    );
    if (!opened || typeof opened.run_id !== "number") {
      throw new Error("team-bridge: run open returned no run_id");
    }
    const handle: RunHandle = {
      runId: opened.run_id,
      revision: opened.revision ?? 0,
      seq: 0,
      state: initialState({ agentId: ctx.agentId, sessionKey: ctx.sessionKey, intent }),
    };
    this.runs.set(key, handle);
    // Seed the beat clock at open: the start event IS the first announcement,
    // so an immediate turn-one ping would be pure chatter.
    this.lastBeat.set(key, this.now());
    await this.postEvent(handle, "workflow/openclaw/start", { intent });
  }

  /** Throttled progress ping on prompt turns (bounded event chatter). */
  async onTurnBeat(ctx: { agentId?: string; sessionKey?: string }): Promise<void> {
    const key = this.key(ctx);
    if (!key) {
      return;
    }
    const handle = this.runs.get(key);
    if (!handle) {
      return;
    }
    const conf = this.conf();
    const last = this.lastBeat.get(key) ?? 0;
    if (this.now() - last < conf.heartbeatMs) {
      return;
    }
    this.lastBeat.set(key, this.now());
    await this.postEvent(handle, "workflow/openclaw/beat", {});
  }

  /** Close the run: completion lineage event + CAS to done/failed. */
  async onAgentEnd(
    ctx: { agentId?: string; sessionKey?: string },
    result: { success: boolean },
  ): Promise<void> {
    const key = this.key(ctx);
    if (!key) {
      return;
    }
    const handle = this.runs.get(key);
    if (!handle) {
      return;
    }
    const outcome = result.success ? "done" : "failed";
    await this.postEvent(handle, `workflow/openclaw/${outcome}`, {});
    await this.client.fetchJson(
      `/workflow/runs/${handle.runId}/state`,
      "PUT",
      {
        expected_rev: handle.revision,
        state_json: JSON.stringify(endState(handle.state, outcome)),
        status: outcome,
      },
      this.conf().requestTimeoutMs,
    );
    this.runs.delete(key);
    this.lastBeat.delete(key);
  }

  /** Session teardown: pause any still-open runs for this session. */
  async onSessionEnd(sessionKey?: string): Promise<void> {
    for (const [key, handle] of [...this.runs.entries()]) {
      if (sessionKey && !key.endsWith(`\u0000${sessionKey}`)) {
        continue;
      }
      try {
        await this.postEvent(handle, "workflow/openclaw/paused", {});
        await this.client.fetchJson(
          `/workflow/runs/${handle.runId}/state`,
          "PUT",
          {
            expected_rev: handle.revision,
            state_json: JSON.stringify(endState(handle.state, "paused")),
            status: "paused",
          },
          this.conf().requestTimeoutMs,
        );
      } catch (err) {
        this.log.warn?.(`team-bridge: pause failed (${sanitizeForBlock(String(err))})`);
      }
      this.runs.delete(key);
      this.lastBeat.delete(key);
    }
  }
}

// --------------------------------------------------------------------------
// Wiring
// --------------------------------------------------------------------------

/**
 * Subscribe the bridge to the OpenClaw lifecycle. Every handler is
 * fail-open: transport errors are logged and swallowed so the bridge can
 * never stall or fail an agent turn.
 *
 * The per-turn heartbeat is NOT registered here — the host plugin folds
 * `bridge.onTurnBeat()` into its existing `before_prompt_build` handler
 * (one handler per hook, house rule) and returns this bridge instance.
 */
export function attachTeamBridge(
  api: OpenClawPluginApi,
  client: BrainClient,
  liveCfg: () => ResolvedBrainConfig,
): TeamBridge {
  const PLUGIN_ID = "brain-server";
  const log: BridgeLogger = {
    info: (m) => api.logger.info?.(`${PLUGIN_ID}: ${m}`),
    warn: (m) => api.logger.warn?.(`${PLUGIN_ID}: ${m}`),
  };
  const bridge = new TeamBridge(
    client,
    () => {
      const c = liveCfg();
      return {
        teamDomain: c.teamDomain,
        heartbeatMs: c.teamHeartbeatMs,
        requestTimeoutMs: c.requestTimeoutMs,
        agents: c.agents,
      };
    },
    log,
  );

  const gated = (
    ctx: { agentId?: string } | undefined,
  ): boolean => {
    const c = liveCfg();
    return teamGateEnabled(c, ctx?.agentId);
  };

  api.on(
    "before_agent_run",
    async (event, ctx) => {
      if (!gated(ctx)) {
        return undefined;
      }
      try {
        await bridge.onAgentRunStart({
          ...(ctx?.agentId !== undefined ? { agentId: ctx.agentId } : {}),
          ...(ctx?.sessionKey !== undefined ? { sessionKey: ctx.sessionKey } : {}),
          ...(event?.prompt !== undefined ? { prompt: event.prompt } : {}),
        });
      } catch (err) {
        log.warn?.(`run start mirror failed (${String(err)})`);
      }
      // ALWAYS pass — the bridge is observation-only and must never gate.
      // Fail fast too: a hung brain-server must not hold the agent's turn
      // for a full request budget; visibility is worth ~2s at most.
      return undefined;
    },
    { timeoutMs: 2_000 },
  );

  // NOTE: agent_end / session_end / before_prompt_build are NOT registered
  // here — the host plugin already owns those hooks (one handler per hook,
  // house rule). It folds this bridge's close/pause/beat calls into its
  // existing handlers using the TeamBridge instance this function returns.

  log.info?.("team bridge attached (visibility mirrors onto governed workflow)");
  return bridge;
}

/**
 * Fold-point for the host plugin's existing `agent_end` handler: closes the
 * mirrored governed run (status done|failed) exactly once per session.
 */
export async function teamCloseOnEnd(
  bridge: TeamBridge,
  cfg: { enabled: boolean; teamBridge: boolean; agents: string[] },
  ctx: { agentId?: string; sessionKey?: string } | undefined,
  success: boolean,
): Promise<void> {
  if (!cfg.enabled || !cfg.teamBridge || !ctx?.agentId || !cfg.agents.includes(ctx.agentId)) {
    return;
  }
  await bridge.onAgentEnd(ctx, { success });
}

/**
 * Fold-point for the host plugin's existing `session_end` handler: pauses any
 * mirrored runs still open for the ending session.
 */
export async function teamPauseOnSessionEnd(
  bridge: TeamBridge,
  sessionKey?: string,
): Promise<void> {
  await bridge.onSessionEnd(sessionKey);
}
