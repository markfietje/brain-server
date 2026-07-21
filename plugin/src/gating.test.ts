import { describe, expect, test } from "vitest";

import { DEFAULTS } from "./config.js";
import { deriveChatType, isRecallAllowed, type GateContext } from "./gating.js";
import type { ResolvedBrainConfig } from "./config.js";

const baseCfg = (): ResolvedBrainConfig => ({
  enabled: true,
  baseUrl: DEFAULTS.baseUrl,
  agents: [],
  allowedChatTypes: [...DEFAULTS.allowedChatTypes],
  allowedChatIds: [],
  deniedChatIds: [],
  autoRecall: true,
  autoCapture: false,
  strictDomain: false,
  defaultDomain: DEFAULTS.defaultDomain,
  autoRecallTopK: DEFAULTS.autoRecallTopK,
  autoRecallTimeoutMs: DEFAULTS.autoRecallTimeoutMs,
  requestTimeoutMs: DEFAULTS.requestTimeoutMs,
  minQueryLength: DEFAULTS.minQueryLength,
  recallMaxChars: DEFAULTS.recallMaxChars,
});

describe("isRecallAllowed", () => {
  test("disabled plugin => blocked", () => {
    const cfg = baseCfg();
    cfg.enabled = false;
    expect(isRecallAllowed(cfg, { agentId: "main" })).toEqual({
      allowed: false,
      reason: expect.any(String),
    });
  });

  test("empty agents allowlist => disabled for every agent (least privilege)", () => {
    const cfg = baseCfg(); // agents: []
    expect(isRecallAllowed(cfg, { agentId: "main" }).allowed).toBe(false);
  });

  test("agent in allowlist => allowed", () => {
    const cfg = baseCfg();
    cfg.agents = ["main"];
    expect(isRecallAllowed(cfg, { agentId: "main" }).allowed).toBe(true);
  });

  test("agent not in allowlist => blocked", () => {
    const cfg = baseCfg();
    cfg.agents = ["main"];
    const d = isRecallAllowed(cfg, { agentId: "other" });
    expect(d.allowed).toBe(false);
  });

  test("missing agentId when allowlist set => blocked", () => {
    const cfg = baseCfg();
    cfg.agents = ["main"];
    expect(isRecallAllowed(cfg, {}).allowed).toBe(false);
  });

  test("group chat type blocked by default (data-leakage prevention)", () => {
    const cfg = baseCfg();
    cfg.agents = ["main"];
    const d = isRecallAllowed(cfg, { agentId: "main", chatType: "group" });
    expect(d.allowed).toBe(false);
    expect(d.allowed === false && d.reason).toContain("data-leakage");
  });

  test("channel chat type blocked by default", () => {
    const cfg = baseCfg();
    cfg.agents = ["main"];
    expect(isRecallAllowed(cfg, { agentId: "main", chatType: "channel" }).allowed).toBe(false);
  });

  test("explicit opt-in to group chat type => allowed", () => {
    const cfg = baseCfg();
    cfg.agents = ["main"];
    cfg.allowedChatTypes = ["direct", "explicit", "group"];
    expect(isRecallAllowed(cfg, { agentId: "main", chatType: "group" }).allowed).toBe(true);
  });

  test("deniedChatIds wins over allow (deny precedence)", () => {
    const cfg = baseCfg();
    cfg.agents = ["main"];
    cfg.allowedChatIds = ["c1", "c2"];
    cfg.deniedChatIds = ["c1"];
    expect(isRecallAllowed(cfg, { agentId: "main", chatId: "c1" }).allowed).toBe(false);
    expect(isRecallAllowed(cfg, { agentId: "main", chatId: "c2" }).allowed).toBe(true);
  });

  test("allowedChatIds allowlist => chat not listed is blocked", () => {
    const cfg = baseCfg();
    cfg.agents = ["main"];
    cfg.allowedChatIds = ["c1"];
    expect(isRecallAllowed(cfg, { agentId: "main", chatId: "c2" }).allowed).toBe(false);
  });
});

describe("deriveChatType", () => {
  test("explicit/portal/webchat triggers map to explicit", () => {
    expect(deriveChatType({ trigger: "explicit" })).toBe("explicit");
    expect(deriveChatType({ trigger: "webchat" })).toBe("explicit");
    expect(deriveChatType({ trigger: "PORTAL" })).toBe("explicit");
  });

  test("non-local channel maps to group (conservative)", () => {
    expect(deriveChatType({ channel: "discord" })).toBe("group");
    expect(deriveChatType({ channel: "telegram" })).toBe("group");
  });

  test("group/channel triggers map accordingly", () => {
    expect(deriveChatType({ trigger: "group" })).toBe("group");
    expect(deriveChatType({ trigger: "channel" })).toBe("channel");
  });

  test("no channel and no multi-user trigger => direct", () => {
    expect(deriveChatType({})).toBe("direct");
  });

  test("fail-closed: unknown trigger with a channel is treated as group", () => {
    expect(deriveChatType({ trigger: "something-weird", channel: "x" })).toBe("group");
  });
});
