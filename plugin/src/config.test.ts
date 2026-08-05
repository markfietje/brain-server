import { describe, expect, test } from "vitest";
import { DEFAULTS, resolveConfig } from "./config.js";

describe("resolveConfig", () => {
  test("applies all defaults for an empty config", () => {
    const cfg = resolveConfig({});
    expect(cfg).toEqual({
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
  });

  test("security defaults: group/channel excluded, agents opt-in empty", () => {
    const cfg = resolveConfig({});
    // Data-leakage prevention: group/channel NOT in the default allowlist.
    expect(cfg.allowedChatTypes).toEqual(["direct", "explicit"]);
    // Least privilege: empty agents allowlist => disabled until an agent opts in.
    expect(cfg.agents).toEqual([]);
  });

  test("overrides provided values and trims whitespace", () => {
    const cfg = resolveConfig({
      baseUrl: "  http://example.com:8765  ",
      authToken: "  secret  ",
      agents: ["main", "research"],
      autoRecall: false,
      autoRecallTopK: 10,
      defaultDomain: "  health  ",
    });
    // resolveConfig trims surrounding whitespace; trailing-slash stripping is
    // BrainClient's job (it normalizes on construction).
    expect(cfg.baseUrl).toBe("http://example.com:8765");
    expect(cfg.authToken).toBe("secret");
    expect(cfg.agents).toEqual(["main", "research"]);
    expect(cfg.autoRecall).toBe(false);
    expect(cfg.autoRecallTopK).toBe(10);
    expect(cfg.defaultDomain).toBe("health");
  });

  test("falls back to defaults when values are blank/empty", () => {
    const cfg = resolveConfig({ baseUrl: "   ", defaultDomain: "" });
    expect(cfg.baseUrl).toBe(DEFAULTS.baseUrl);
    expect(cfg.defaultDomain).toBe(DEFAULTS.defaultDomain);
  });

  test("authToken blank string resolves to undefined (not emitted)", () => {
    const cfg = resolveConfig({ authToken: "   " });
    expect(cfg.authToken).toBeUndefined();
  });
});
