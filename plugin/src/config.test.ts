import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, test } from "vitest";
import { assertSafeBaseUrl, DEFAULTS, resolveConfig } from "./config.js";

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
      untrustedOrigins: "label",
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
      teamBridge: DEFAULTS.teamBridge,
      teamDomain: DEFAULTS.defaultDomain,
      teamHeartbeatMs: DEFAULTS.teamHeartbeatMs,
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
      baseUrl: "  http://127.0.0.1:8765  ", // loopback: remote cleartext is refused (F-E4)
      authToken: "  secret  ",
      agents: ["main", "research"],
      autoRecall: false,
      autoRecallTopK: 10,
      defaultDomain: "  health  ",
    });
    // resolveConfig trims surrounding whitespace; trailing-slash stripping is
    // BrainClient's job (it normalizes on construction).
    expect(cfg.baseUrl).toBe("http://127.0.0.1:8765");
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

        // An unreadable configured file FAILS CLOSED — it must refuse,
        // never silently downgrade to a weaker rung.
        process.env.BRAIN_TOKEN_FILE = path.join(dir, "missing");
        expect(() => resolveConfig({ authToken: "cfg-token" })).toThrow(/unreadable/);
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

  test("a multi-line token file refuses naming the agent line", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "brain-cfg-"));
    const tokenFile = path.join(dir, "token");
    fs.writeFileSync(tokenFile, "operator-token\nagent-token\n");
    const prev = process.env.BRAIN_TOKEN_FILE;
    process.env.BRAIN_TOKEN_FILE = tokenFile;
    try {
      expect(() => resolveConfig({})).toThrow(/agent-token line/);
    } finally {
      if (prev === undefined) {
        delete process.env.BRAIN_TOKEN_FILE;
      } else {
        process.env.BRAIN_TOKEN_FILE = prev;
      }
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test("plugin_config_token_refuses_a_multi_token_value", () => {
    // The config rung is the one production rides in a default gateway
    // install (the env rungs are empty), so it carries the same refusal
    // the file and env rungs already enforce: a value with two
    // tokens/separators is the server's two-line token file pasted whole,
    // and its FIRST line is the privileged operator token.
    const prevFile = process.env.BRAIN_TOKEN_FILE;
    const prevVar = process.env.BRAIN_TOKEN;
    delete process.env.BRAIN_TOKEN_FILE;
    delete process.env.BRAIN_TOKEN;
    try {
      // Multi-token forms all refuse — newline, spaces, tabs, and leading
      // operator token before the agent one (the paste direction that
      // leaks operator authority, not just a malformed value).
      for (const bad of [
        "operator-token\nagent-token",
        "operator-token agent-token",
        "operator-token\tagent-token",
        "operator-token\r\nagent-token",
      ]) {
        expect(() => resolveConfig({ authToken: bad })).toThrow(
          /authToken holds more than one token/,
        );
      }

      // Anti-vacuity: a single-token config value still resolves and is
      // emitted trimmed — the refusal is about the multi-token shape, not
      // about disabling the rung.
      expect(resolveConfig({ authToken: "agent-token" }).authToken).toBe("agent-token");

      // Blank stays unset (an env placeholder that substituted to empty
      // must not become a bogus token).
      expect(resolveConfig({ authToken: "   " }).authToken).toBeUndefined();
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
  });
});

describe("assertSafeBaseUrl (F-E4 scheme gate)", () => {
  test("https passes", () => {
    expect(() => assertSafeBaseUrl("https://brain.example.com")).not.toThrow();
  });
  test("loopback http passes", () => {
    for (const u of ["http://127.0.0.1:8765", "http://localhost:8765", "http://[::1]:8765"]) {
      expect(() => assertSafeBaseUrl(u)).not.toThrow();
    }
  });
  test("remote cleartext throws", () => {
    expect(() => assertSafeBaseUrl("http://brain.example.com")).toThrow(/cleartext|loopback/);
    expect(() => assertSafeBaseUrl("ftp://x")).toThrow(/scheme/);
    expect(() => assertSafeBaseUrl("not a url")).toThrow(/valid URL/);
  });
});

describe("untrustedOrigins (v0.6.0 Origin)", () => {
  test("defaults to label; exclude and label resolve verbatim", () => {
    expect(resolveConfig({}).untrustedOrigins).toBe("label");
    expect(resolveConfig({ untrustedOrigins: "exclude" } as never).untrustedOrigins).toBe(
      "exclude",
    );
    expect(resolveConfig({ untrustedOrigins: "label" } as never).untrustedOrigins).toBe("label");
  });

  test("an invalid value REFUSES to boot — degrading to label is fail-UNSAFE", () => {
    // The old tolerant fallback read as fail-safe, but for a typo of
    // "exclude" it silently switched the posture DOWN to label —
    // re-injecting exactly the channel-captured hits the operator wanted
    // dropped. A typo the operator must fix beats a posture they never
    // chose. The manifest schema is the first gate; this resolver is the
    // second for configs that bypass it (hand-edited jsonc).
    expect(() => resolveConfig({ untrustedOrigins: "purge" } as never)).toThrow(
      /untrustedOrigins.*must be/,
    );
  });
});

// ── boundary typecheck (ninth-pass remediation) ─────────────────────────────
// One type error used to defeat whole controls: a string `agents` turned
// the allowlist gates into SUBSTRING matching (`.includes` on a string
// admits any sub-agent), and a string `autoRecallTopK` failed every
// recall comparison. These pin the refusal — and, anti-vacuity, that a
// fully-valid typed config still resolves.
describe("config boundary typecheck", () => {
  test("a string agents allowlist refuses to boot (no substring admit)", () => {
    // `agents: "ops-agent-1"` is the exact ninth-pass mutant: `.includes`
    // on the string would admit `ops`, `ops-agent`, `1`, …
    expect(() => resolveConfig({ agents: "ops-agent-1" } as never)).toThrow(
      /agents.*array of strings.*string/,
    );
    expect(() => resolveConfig({ allowedChatIds: "teamchat" } as never)).toThrow(
      /allowedChatIds.*array of strings/,
    );
    // Mixed arrays refuse too — one non-string element re-opens the hole.
    expect(() => resolveConfig({ agents: ["ok", 7] } as never)).toThrow(/agents.*array of strings/);
  });

  test("string numerics and booleans refuse to boot", () => {
    expect(() => resolveConfig({ autoRecallTopK: "5" } as never)).toThrow(
      /autoRecallTopK.*integer/,
    );
    expect(() => resolveConfig({ enabled: "yes" } as never)).toThrow(/enabled.*boolean.*string/);
    // Out-of-range integers are NOT the type gate's business (ranges are
    // the manifest schema's): the resolver must not learn to refuse values
    // the schema layer owns — tests exercise sub-second heartbeats, for
    // one. Types refuse; ranges delegate.
    expect(resolveConfig({ autoRecallTopK: 99 } as never).autoRecallTopK).toBe(99);
  });

  test("a fully-typed config still resolves (anti-vacuity)", () => {
    const cfg = resolveConfig({
      enabled: true,
      agents: ["ops-agent-1"],
      allowedChatTypes: ["direct", "explicit"],
      allowedChatIds: ["chat-1"],
      deniedChatIds: ["chat-2"],
      untrustedOrigins: "exclude",
      captureMode: "proposal",
      autoRecallTopK: 7,
      teamHeartbeatMs: 30_000,
      autoRecallMaxContextTokens: 2_000,
    });
    expect(cfg.agents).toEqual(["ops-agent-1"]);
    expect(cfg.untrustedOrigins).toBe("exclude");
    expect(cfg.autoRecallTopK).toBe(7);
  });
});
