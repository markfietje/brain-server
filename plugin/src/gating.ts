/**
 * Access gating for memory recall/capture.
 *
 * OWASP LLM Top 10 (LLM06 Excessive Agency, secure defaults) + Lakera AI Agent
 * Security (per-agent rating, Data Leakage Prevention):
 *   - Per-agent opt-in (empty allowlist => no agent may use memory).
 *   - Chat-type gating: direct + explicit by default; group/channel excluded
 *     unless explicitly enabled, to prevent private memory surfacing in shared
 *     contexts (data-leakage prevention).
 *   - Explicit per-chat allow/deny overrides (deny wins).
 *
 * Everything here is synchronous and pure — the security decision must be
 * deterministic and cheap.
 */
import type { ResolvedBrainConfig } from "./config.js";

/**
 * Minimal projection of PluginHookAgentContext used for gating. Kept structural
 * so this module does not import the full SDK type surface (testable in
 * isolation). The index.ts adapter maps the real ctx into this shape.
 */
export type GateContext = {
  agentId?: string;
  /** Logical chat kind derived from ctx.channel/trigger: direct|group|channel|explicit. */
  chatType?: "direct" | "group" | "channel" | "explicit";
  chatId?: string;
};

export type GateDecision = { allowed: true } | { allowed: false; reason: string };

export function isRecallAllowed(cfg: ResolvedBrainConfig, ctx: GateContext): GateDecision {
  if (!cfg.enabled) return { allowed: false, reason: "plugin disabled" };

  // Per-agent opt-in (least privilege). Empty allowlist => disabled until at
  // least one agent is listed (OWASP LLM06: memory is a granted capability).
  if (cfg.agents.length === 0) {
    return { allowed: false, reason: "no agents opted in; set config.agents to enable memory" };
  }
  if (!ctx.agentId || !cfg.agents.includes(ctx.agentId)) {
    return { allowed: false, reason: `agent ${ctx.agentId ?? "<none>"} not in allowlist` };
  }

  // Chat-type gating.
  if (ctx.chatType && !cfg.allowedChatTypes.includes(ctx.chatType)) {
    return { allowed: false, reason: `chat type ${ctx.chatType} not allowed (data-leakage guard)` };
  }

  // Explicit per-chat overrides (deny wins over allow).
  if (ctx.chatId) {
    if (cfg.deniedChatIds.includes(ctx.chatId)) {
      return { allowed: false, reason: `chat ${ctx.chatId} denied` };
    }
    if (cfg.allowedChatIds.length > 0 && !cfg.allowedChatIds.includes(ctx.chatId)) {
      return { allowed: false, reason: `chat ${ctx.chatId} not in allowlist` };
    }
  }

  return { allowed: true };
}

/**
 * Derive a logical chat type from the hook context. Channel-originated runs
 * (discord/slack/etc.) map to group/channel; direct/explicit portal sessions
 * map to direct/explicit. Conservative: unknown => treated as group (blocked
 * by default) rather than direct (allowed by default) — fail closed.
 */
export function deriveChatType(ctx: {
  channel?: string;
  trigger?: string;
  chatId?: string;
}): GateContext["chatType"] {
  const trigger = (ctx.trigger ?? "").toLowerCase();
  const channel = (ctx.channel ?? "").toLowerCase();
  // Explicit/portal/webchat sessions.
  if (trigger.includes("explicit") || trigger.includes("portal") || trigger.includes("webchat")) {
    return "explicit";
  }
  // Channel-originated.
  if (channel && channel !== "local") {
    // Heuristic: known multi-user channels => group; treat unknown channels as group too.
    return "group";
  }
  if (trigger.includes("group") || trigger.includes("channel")) {
    return trigger.includes("channel") ? "channel" : "group";
  }
  // No channel + no multi-user trigger => direct.
  if (!channel) return "direct";
  return "group";
}
