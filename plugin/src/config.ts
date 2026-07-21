/**
 * Brain Server plugin config — Typebox schema, resolved config type, defaults.
 *
 * Gating defaults follow OWASP LLM Top 10 (least privilege / LLM06) and the
 * Lakera AI Agent Security model (per-agent rating + Data Leakage Prevention):
 *   - memory is opt-in per agent (empty `agents` => disabled)
 *   - group/channel chats excluded by default to prevent private-memory leakage
 */
import { Type, type Static } from "typebox";

export const brainConfigSchema = Type.Object({
  enabled: Type.Optional(Type.Boolean()),
  baseUrl: Type.Optional(
    Type.String({ default: "http://127.0.0.1:8765" }),
  ),
  authToken: Type.Optional(Type.String()),
  agents: Type.Optional(Type.Array(Type.String())),
  allowedChatTypes: Type.Optional(
    Type.Array(Type.Union([
      Type.Literal("direct"),
      Type.Literal("group"),
      Type.Literal("channel"),
      Type.Literal("explicit"),
    ])),
  ),
  allowedChatIds: Type.Optional(Type.Array(Type.String())),
  deniedChatIds: Type.Optional(Type.Array(Type.String())),

  autoRecall: Type.Optional(Type.Boolean()),
  autoCapture: Type.Optional(Type.Boolean()),
  strictDomain: Type.Optional(Type.Boolean()),
  defaultDomain: Type.Optional(Type.String()),

  autoRecallTopK: Type.Optional(Type.Integer({ minimum: 1, maximum: 20 })),
  autoRecallTimeoutMs: Type.Optional(Type.Integer({ minimum: 250, maximum: 30_000 })),
  requestTimeoutMs: Type.Optional(Type.Integer({ minimum: 250, maximum: 30_000 })),
  minQueryLength: Type.Optional(Type.Integer({ minimum: 1, maximum: 200 })),
  recallMaxChars: Type.Optional(Type.Integer({ minimum: 40, maximum: 10_000 })),
});

export type BrainConfig = Static<typeof brainConfigSchema>;

export const DEFAULTS = {
  enabled: true,
  baseUrl: "http://127.0.0.1:8765",
  autoRecall: true,
  autoCapture: false,
  strictDomain: false,
  defaultDomain: "global",
  allowedChatTypes: ["direct", "explicit"] as const,
  autoRecallTopK: 3,
  autoRecallTimeoutMs: 5_000,
  requestTimeoutMs: 8_000,
  minQueryLength: 5,
  recallMaxChars: 1_000,
} as const;

export type ResolvedBrainConfig = {
  enabled: boolean;
  baseUrl: string;
  authToken?: string;
  agents: string[];
  allowedChatTypes: ReadonlyArray<"direct" | "group" | "channel" | "explicit">;
  allowedChatIds: string[];
  deniedChatIds: string[];
  autoRecall: boolean;
  autoCapture: boolean;
  strictDomain: boolean;
  defaultDomain: string;
  autoRecallTopK: number;
  autoRecallTimeoutMs: number;
  requestTimeoutMs: number;
  minQueryLength: number;
  recallMaxChars: number;
};

/** Resolve raw plugin config into a fully-populated, validated config. */
export function resolveConfig(raw: unknown): ResolvedBrainConfig {
  const cfg = (raw ?? {}) as Partial<BrainConfig>;
  const authToken = cfg.authToken?.trim() || undefined;
  return {
    enabled: cfg.enabled ?? DEFAULTS.enabled,
    baseUrl: (cfg.baseUrl && cfg.baseUrl.trim()) || DEFAULTS.baseUrl,
    // exactOptionalPropertyTypes: only emit the key when a token is present.
    ...(authToken !== undefined ? { authToken } : {}),
    agents: cfg.agents ?? [],
    allowedChatTypes: cfg.allowedChatTypes ?? DEFAULTS.allowedChatTypes,
    allowedChatIds: cfg.allowedChatIds ?? [],
    deniedChatIds: cfg.deniedChatIds ?? [],
    autoRecall: cfg.autoRecall ?? DEFAULTS.autoRecall,
    autoCapture: cfg.autoCapture ?? DEFAULTS.autoCapture,
    strictDomain: cfg.strictDomain ?? DEFAULTS.strictDomain,
    defaultDomain: cfg.defaultDomain?.trim() || DEFAULTS.defaultDomain,
    autoRecallTopK: cfg.autoRecallTopK ?? DEFAULTS.autoRecallTopK,
    autoRecallTimeoutMs: cfg.autoRecallTimeoutMs ?? DEFAULTS.autoRecallTimeoutMs,
    requestTimeoutMs: cfg.requestTimeoutMs ?? DEFAULTS.requestTimeoutMs,
    minQueryLength: cfg.minQueryLength ?? DEFAULTS.minQueryLength,
    recallMaxChars: cfg.recallMaxChars ?? DEFAULTS.recallMaxChars,
  };
}

/**
 * OpenClawPluginConfigSchema adapter. The host calls `safeParse()`/`validate()`
 * on the schema handed to definePluginEntry; a raw Typebox TObject has neither,
 * so we wrap resolveConfig (which already applies defaults + does the work the
 * manifest's JSON-schema configSchema documents). `data` is returned as the
 * resolved config so the host never needs to re-resolve.
 */
export const brainPluginConfigSchema = {
  safeParse(value: unknown) {
    try {
      return { success: true as const, data: resolveConfig(value) };
    } catch (err) {
      return { success: false as const, error: { issues: [{ path: [], message: String(err) }] } };
    }
  },
  validate(value: unknown) {
    try {
      resolveConfig(value);
      return { ok: true as const };
    } catch (err) {
      return { ok: false as const, errors: [String(err)] };
    }
  },
} as const;
