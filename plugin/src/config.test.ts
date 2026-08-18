import fs from "node:fs";
import os from "node:os";
import path from "node:path";
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
      captureMode: "proposal",
      strictDomain: false,
      defaultDomain: DEFAULTS.defaultDomain,
      autoRecallTopK: DEFAULTS.autoRecallTopK,
      autoRecallTimeoutMs: DEFAULTS.autoRecallTimeoutMs,
      requestTimeoutMs: DEFAULTS.requestTimeoutMs,
      minQueryLength: DEFAULTS.minQueryLength,
      recallMaxChars: DEFAULTS.recallMaxChars,
      autoRecallGraph: DEFAULTS.autoRecallGraph,
      proposalTools: DEFAULTS.proposalTools,
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

  test("token ladder: env file beats env var beats config (no plaintext store required)", () => {
    // The remediation for the documented openclaw-config token leak: the
    // plugin must never FORCE the token into the plaintext plugin config.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "brain-cfg-"));
    const tokenFile = path.join(dir, "token");
    try {
      const prevFile = process.env.BRAIN_TOKEN_FILE;
      const prevVar = process.env.BRAIN_TOKEN;
      try {
        // 3. Config-only (legacy fallback) still works. Clear any env source
        // with `delete` — `= undefined` stringifies to "undefined" on some
        // Node versions and would leak a bogus token into the ladder.
        delete process.env.BRAIN_TOKEN_FILE;
        delete process.env.BRAIN_TOKEN;
        expect(resolveConfig({ authToken: "cfg-token" }).authToken).toBe("cfg-token");

        // 2. BRAIN_TOKEN beats the config value.
        process.env.BRAIN_TOKEN = "env-token";
        expect(resolveConfig({ authToken: "cfg-token" }).authToken).toBe("env-token");

        // 1. BRAIN_TOKEN_FILE beats both.
        fs.writeFileSync(tokenFile, "file-token\n");
        process.env.BRAIN_TOKEN_FILE = tokenFile;
        expect(resolveConfig({ authToken: "cfg-token" }).authToken).toBe("file-token");

        // An unreadable file degrades loudly to the next rung, never silently
        // to a weaker source without notice.
        process.env.BRAIN_TOKEN_FILE = path.join(dir, "missing");
        expect(resolveConfig({ authToken: "cfg-token" }).authToken).toBe("env-token");
      } finally {
        if (prevFile === undefined) {
          delete process.env.BRAIN_TOKEN_FILE;
        } else {
          process.env.BRAIN_TOKEN_FILE = prevFile;
        }
        if (prevVar === undefined) {
          delete process.env.BRAIN_TOKEN;
        } else {
          process.env.BRAIN_TOKEN = prevVar;
        }
      }
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
