/**
 * Brain Server plugin config — Typebox schema, resolved config type, defaults.
 *
 * Gating defaults follow OWASP LLM Top 10 (least privilege / LLM06) and the
 * Lakera AI Agent Security model (per-agent rating + Data Leakage Prevention):
 *   - memory is opt-in per agent (empty `agents` => disabled)
 *   - group/channel chats excluded by default to prevent private-memory leakage
 *
 * Server auth: the opaque bearer token lives on the server as `AUTH_TOKEN` (or
 * `AUTH_TOKEN_FILE`, preferred). This plugin's `authToken` config must match it
 * verbatim; both are sent as `Authorization: Bearer <token>`.
 */
import { readFileSync } from "node:fs";
import { Type, type Static } from "typebox";

export const brainConfigSchema = Type.Object({
  enabled: Type.Optional(Type.Boolean()),
  baseUrl: Type.Optional(Type.String({ default: "http://127.0.0.1:8765" })),
  authToken: Type.Optional(Type.String()),
  agents: Type.Optional(Type.Array(Type.String())),
  allowedChatTypes: Type.Optional(
    Type.Array(
      Type.Union([
        Type.Literal("direct"),
        Type.Literal("group"),
        Type.Literal("channel"),
        Type.Literal("explicit"),
      ]),
    ),
  ),
  allowedChatIds: Type.Optional(Type.Array(Type.String())),
  deniedChatIds: Type.Optional(Type.Array(Type.String())),

  autoRecall: Type.Optional(Type.Boolean()),
  autoCapture: Type.Optional(Type.Boolean()),
  // v1.20.1 "Shield" M2: how auto-captures enter the brain.
  //   "proposal" (default) — go through the server's human review queue
  //   (POST /ingest/proposal) and only become memory once a reviewer approves:
  //   nothing is trusted directly into long-term memory from an untrusted turn.
  //   "direct"            — store straight to memory (the v1.16.x behavior), gated
  //   only by the server-side injection screen.
  captureMode: Type.Optional(Type.Union([Type.Literal("proposal"), Type.Literal("direct")])),
  strictDomain: Type.Optional(Type.Boolean()),
  defaultDomain: Type.Optional(Type.String()),

  autoRecallTopK: Type.Optional(Type.Integer({ minimum: 1, maximum: 20 })),
  autoRecallTimeoutMs: Type.Optional(Type.Integer({ minimum: 250, maximum: 30_000 })),
  requestTimeoutMs: Type.Optional(Type.Integer({ minimum: 250, maximum: 30_000 })),
  minQueryLength: Type.Optional(Type.Integer({ minimum: 1, maximum: 200 })),
  recallMaxChars: Type.Optional(Type.Integer({ minimum: 40, maximum: 10_000 })),
  // v0.3.0: advanced auto-recall tuning. `autoRecallGraph` adds the server's
  // zero-token graph-PPR retriever as a third RRF leg; `autoRecallMaxContextTokens`
  // asks the server to submodularly pack injected memories to a token budget
  // (coverage/diversity) instead of always taking the top-K verbatim.
  autoRecallGraph: Type.Optional(Type.Boolean()),
  autoRecallMaxContextTokens: Type.Optional(Type.Integer({ minimum: 0, maximum: 8_000 })),
  // v0.3.0: expose the human review-queue tools (memory_proposal_list /
  // memory_proposal_decide). Off by default — promoting a capture to memory is
  // an operator action, so the agent must not gain it unless explicitly opted in.
  proposalTools: Type.Optional(Type.Boolean()),

  // v0.5.0 "Team Bridge": mirror OpenClaw agent activity onto brain-server's
  // governed-workflow dashboards (Mesh cards, Crew roster, run timelines,
  // Scoreboard). Off by default — visibility is an operator choice, exactly
  // like autoCapture. Gated by the same per-agent allowlist as recall.
  teamBridge: Type.Optional(Type.Boolean()),
  // Domain the mirrored runs open in. Defaults to defaultDomain when unset.
  teamDomain: Type.Optional(Type.String()),
  // Minimum gap between progress heartbeats for one session (server events
  // are exactly-once by idempotency key; this bounds chatter on quiet turns).
  teamHeartbeatMs: Type.Optional(
    Type.Integer({ minimum: 15_000, maximum: 600_000 }),
  ),
});

export type BrainConfig = Static<typeof brainConfigSchema>;

export const DEFAULTS = {
  enabled: true,
  baseUrl: "http://127.0.0.1:8765",
  autoRecall: true,
  autoCapture: false,
  captureMode: "proposal" as const,
  strictDomain: false,
  defaultDomain: "global",
  allowedChatTypes: ["direct", "explicit"] as const,
  autoRecallTopK: 3,
  autoRecallTimeoutMs: 5_000,
  requestTimeoutMs: 8_000,
  minQueryLength: 5,
  recallMaxChars: 1_000,
  autoRecallGraph: false,
  proposalTools: false,
  teamBridge: false,
  teamHeartbeatMs: 60_000,
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
  captureMode: "proposal" | "direct";
  strictDomain: boolean;
  defaultDomain: string;
  autoRecallTopK: number;
  autoRecallTimeoutMs: number;
  requestTimeoutMs: number;
  minQueryLength: number;
  recallMaxChars: number;
  autoRecallGraph: boolean;
  autoRecallMaxContextTokens?: number;
  proposalTools: boolean;
  teamBridge: boolean;
  teamDomain: string;
  teamHeartbeatMs: number;
};

/**
 * Resolve the auth token WITHOUT forcing it into the plaintext plugin config
 * (the remediation for the documented openclaw-config token leak): the
 * ladder mirrors the `brain` CLI —
 *   1. `BRAIN_TOKEN_FILE` — path to a 0600 token file (preferred; the token
 *      never appears in any config, env dump, or process listing argument)
 *   2. `BRAIN_TOKEN` — the token itself via environment
 *   3. `authToken` in the plugin config (legacy fallback; still plaintext
 *      wherever openclaw stores its config — migrate off it)
 * Config wins ONLY when no env source is set, so an operator rotating via
 * env does not fight a stale config value.
 *
 * Two-token pattern (Seatbelt): point this at the AGENT token (the second
 * whitespace-split line of the server's token file, provisioned by the
 * installer) — never the operator token. Under the server's
 * `BRAIN_WRITE_POSTURE=review`, agent writes land as pending proposals.
 */
function resolveAuthToken(cfg: Partial<BrainConfig>): string | undefined {
  const file = process.env.BRAIN_TOKEN_FILE?.trim();
  if (file) {
    try {
      const token = readFileSync(file, "utf8").trim();
      if (token) {
        return token;
      }
    } catch {
      // Unreadable token file: fall through to the next rung with a warning —
      // a broken ladder must not silently degrade to a weaker source.
      console.warn(`brain plugin: BRAIN_TOKEN_FILE unreadable (${file}); falling back`);
    }
  }
  const envToken = process.env.BRAIN_TOKEN?.trim();
  if (envToken) {
    return envToken;
  }
  return cfg.authToken?.trim() || undefined;
}

/** Resolve raw plugin config into a fully-populated, validated config. */
/**
 * Scheme gate (F-E4): `https:` always OK; `http:` only for loopback hosts —
 * the bearer token plus user-turn text must never transit remote cleartext.
 * Anything else throws at registration with a clear message.
 */
export function assertSafeBaseUrl(raw: string): void {
  let url: URL;
  try {
    url = new URL(raw);
  } catch (cause) {
    throw new Error(`brain-server plugin: baseUrl is not a valid URL: ${raw}`, {
      cause,
    });
  }
  if (url.protocol === "https:") return;
  if (url.protocol === "http:") {
    const host = url.hostname;
    if (host === "127.0.0.1" || host === "localhost" || host === "::1" || host === "[::1]") {
      return;
    }
    throw new Error(
      `brain-server plugin: refusing remote cleartext baseUrl '${raw}' — use https:// or a loopback host (the bearer token would transit in plaintext)`,
    );
  }
  throw new Error(`brain-server plugin: unsupported baseUrl scheme in '${raw}'`);
}

const TEAM_DOMAIN_RE = /^[a-z0-9][a-z0-9-]{0,62}$/;

/**
 * Mirrors the server's `valid_domain_label` (lowercase alnum, hyphens after
 * position one, <= 63). Validating HERE means a typo fails once at boot
 * instead of producing a 400 warn-line on every mirrored turn.
 */
function assertValidTeamDomain(raw: string): void {
  if (!TEAM_DOMAIN_RE.test(raw)) {
    throw new Error(
      `brain-server plugin: teamDomain '${raw}' invalid — use 1..=63 lowercase alnum/hyphen (server would reject every mirrored request)`,
    );
  }
}

export function resolveConfig(raw: unknown): ResolvedBrainConfig {
  const cfg = (raw ?? {}) as Partial<BrainConfig>;
  const authToken = resolveAuthToken(cfg);
  const baseUrl = (cfg.baseUrl && cfg.baseUrl.trim()) || DEFAULTS.baseUrl;
  const defaultDomain = cfg.defaultDomain?.trim() || DEFAULTS.defaultDomain;
  assertSafeBaseUrl(baseUrl);
  return {
    enabled: cfg.enabled ?? DEFAULTS.enabled,
    baseUrl,
    // exactOptionalPropertyTypes: only emit the key when a token is present.
    ...(authToken !== undefined ? { authToken } : {}),
    agents: cfg.agents ?? [],
    allowedChatTypes: cfg.allowedChatTypes ?? DEFAULTS.allowedChatTypes,
    allowedChatIds: cfg.allowedChatIds ?? [],
    deniedChatIds: cfg.deniedChatIds ?? [],
    autoRecall: cfg.autoRecall ?? DEFAULTS.autoRecall,
    autoCapture: cfg.autoCapture ?? DEFAULTS.autoCapture,
    captureMode: cfg.captureMode ?? DEFAULTS.captureMode,
    strictDomain: cfg.strictDomain ?? DEFAULTS.strictDomain,
    defaultDomain,
    autoRecallTopK: cfg.autoRecallTopK ?? DEFAULTS.autoRecallTopK,
    autoRecallTimeoutMs: cfg.autoRecallTimeoutMs ?? DEFAULTS.autoRecallTimeoutMs,
    requestTimeoutMs: cfg.requestTimeoutMs ?? DEFAULTS.requestTimeoutMs,
    minQueryLength: cfg.minQueryLength ?? DEFAULTS.minQueryLength,
    recallMaxChars: cfg.recallMaxChars ?? DEFAULTS.recallMaxChars,
    autoRecallGraph: cfg.autoRecallGraph ?? DEFAULTS.autoRecallGraph,
    ...(cfg.autoRecallMaxContextTokens !== undefined
      ? { autoRecallMaxContextTokens: cfg.autoRecallMaxContextTokens }
      : {}),
    proposalTools: cfg.proposalTools ?? DEFAULTS.proposalTools,
    teamBridge: cfg.teamBridge ?? DEFAULTS.teamBridge,
    ...(cfg.teamDomain?.trim()
      ? (() => {
          const td = cfg.teamDomain.trim();
          assertValidTeamDomain(td);
          return { teamDomain: td };
        })()
      : { teamDomain: defaultDomain }),
    teamHeartbeatMs: cfg.teamHeartbeatMs ?? DEFAULTS.teamHeartbeatMs,
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
