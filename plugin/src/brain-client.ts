/**
 * Thin typed HTTP client for the Rust brain-server.
 *
 * This module contains NO memory logic — it is a pure transport shim. Every
 * heavy operation (embedding, vector search, domain centroid routing, KG
 * traversal, quantization) lives in the Rust server. The plugin never sees a
 * vector, never loads a model, never touches SQLite.
 *
 * Security:
 *  - Requests carry the bearer token (constant-time compare happens server-side).
 *  - Every call has a hard timeout (AbortController) — never unbounded fetch.
 *  - Responses are validated against minimal shapes before use; unknown fields
 *    are ignored. Recalled text is treated as UNTRUSTED data by the caller.
 *
 * Error model:
 *  - Transport failures (non-2xx, timeout, network, bad JSON) throw typed
 *    errors carrying an actionable `kind` + HTTP status. This lets the tools
 *    surface the difference between a 404 (not found) and a 500 (server bug)
 *    to the agent, while the recall hook can still fail-open by catching.
 */
import type { ResolvedBrainConfig } from "./config.js";

/**
 * Typed transport error. `kind` is the actionable category; `status` is the
 * HTTP status when the server responded (undefined for timeout/network).
 */
export class BrainHttpError extends Error {
  readonly kind: "http" | "timeout" | "network" | "parse";
  readonly status?: number;

  constructor(kind: BrainHttpError["kind"], message: string, status?: number) {
    super(message);
    this.name = "BrainHttpError";
    this.kind = kind;
    // Only set status when provided so exactOptionalPropertyTypes holds.
    if (status !== undefined) {
      this.status = status;
    }
  }
}

/** Human-readable summary for tool output and logs. */
export function describeBrainError(err: unknown): string {
  if (err instanceof BrainHttpError) {
    switch (err.kind) {
      case "http":
        return err.status !== undefined
          ? `brain-server HTTP ${err.status}: ${err.message}`
          : `brain-server HTTP error: ${err.message}`;
      case "timeout":
        return `brain-server timed out: ${err.message}`;
      case "network":
        return `brain-server unreachable: ${err.message}`;
      case "parse":
        return `brain-server returned malformed JSON: ${err.message}`;
    }
  }
  return `brain-server error: ${String(err)}`;
}

/**
 * A recalled memory hit, aligned to the brain-server `RecallHit` wire shape
 * (API_CONTRACT.md §1). `untrusted` is always `true` from the server
 * (OWASP LLM01:2025); `conflict` marks chunks participating in a
 * `contradicts`/`supersedes` link with another current chunk; `evidence` and
 * `snippet` carry the verbatim snippet window. Recalled content is UNTRUSTED
 * and must never be executed as instructions (see format.ts banner).
 */
export type BrainRecallHit = {
  id: number | string;
  title?: string;
  content: string;
  score: number;
  domain?: string;
  source?: "vector" | "fts" | "both" | "graph";
  provenance?: unknown;
  evidence?: unknown;
  snippet?: string;
  untrusted?: boolean;
  conflict?: boolean;
};

/**
 * Result of `POST /recall`. `decision` reflects the server's calibrated
 * abstention (v1.5): `"low_confidence"` means retrieval quality was too low to
 * support a claim, so `hits` is empty by design — the agent should escalate
 * (ask the user) or fall back to web search rather than treat an empty result
 * as a plain "no memories".
 */
export type BrainRecallResult = {
  hits: BrainRecallHit[];
  decision: "ok" | "low_confidence";
  domain?: string;
  domainsSearched?: string[];
  telemetry?: unknown;
};

export type BrainStoreResult = {
  id: number | string;
  status: "created" | "duplicate";
  domain?: string;
  entitiesAdded?: number;
  relationsAdded?: number;
};

export type BrainRelation = {
  from: string;
  to: string;
  type: string;
};

export type BrainEntity = {
  name: string;
  type?: string;
};

/**
 * A fetched chunk (`GET /get/{id}`). Mirrors the server's column projection;
 * the meaningful fields for agent use are `content` (authoritative source text),
 * `title`, and `source_uri`.
 */
export type BrainChunk = {
  id: number;
  title?: string;
  content: string;
  source?: string;
  document_id?: string;
  chunk_index?: number;
  heading_path?: string;
  line_start?: number;
  line_end?: number;
  created_at?: string;
  source_uri?: string;
  revision_id?: number;
};

/** Deterministic span-verification result (`POST /verify`, v1.5). */
export type BrainVerifyResult = {
  chunkId: number;
  supported: boolean;
  decision: "supported" | "unsupported_claim";
  matchRanges: Array<[number, number]>;
};

/** Knowledge-graph entity + one-hop relations (`GET /graph/entity/{name}`). */
export type BrainGraphEntity = {
  name: string;
  type?: string;
  relations: Array<{ to_entity: string; relation_type: string; direction: string }>;
};

// ---------------------------------------------------------------------------
// Wire shapes (server serializes snake_case — see API_CONTRACT.md)
// ---------------------------------------------------------------------------

/**
 * v1.13.3 "SourceFix": valid `source` values for /recall and GET /search.
 * - ingest kind: memory | markdown | structured | manual | vault (SQL filter)
 * - retrieval leg: vector | fts | graph (post-fusion filter)
 * - both: unrestricted. Unknown values are rejected with HTTP 422.
 */
export type BrainSourceFilter =
  | "memory"
  | "markdown"
  | "structured"
  | "manual"
  | "vault"
  | "vector"
  | "fts"
  | "graph"
  | "both";

type RecallRequestWire = {
  query: string;
  limit: number;
  provenance: boolean;
  domain?: string;
  strict?: boolean;
  /** Ingest kind, retrieval leg, or `both` (see {@link BrainSourceFilter}). */
  source?: string;
  since?: string;
  lex?: string;
  vec?: string;
  hyde?: string;
  intent?: string;
};

type RecallHitWire = {
  id: number;
  title?: string;
  content: string;
  score: number;
  domain?: string;
  source?: "vector" | "fts" | "both" | "graph";
  provenance?: unknown;
  evidence?: unknown;
  snippet?: string;
  untrusted?: boolean;
  conflict?: boolean;
};

type RecallResponseWire = {
  hits: RecallHitWire[];
  decision?: "ok" | "low_confidence";
  domain?: string;
  domains_searched?: string[];
  telemetry?: unknown;
};

type IngestResponseWire = {
  id: number;
  status?: "created" | "duplicate";
  domain?: string;
  entities_added?: number;
  relations_added?: number;
};

type VerifyResponseWire = {
  chunk_id: number;
  supported?: boolean;
  decision?: "supported" | "unsupported_claim";
  match_ranges?: Array<[number, number]>;
};

/** Liveness probe — used by the service start hook. */
export class BrainClient {
  private readonly baseUrl: string;
  private readonly token?: string;
  private readonly defaultTimeoutMs: number;

  constructor(cfg: ResolvedBrainConfig) {
    // Trim trailing slash so `${baseUrl}/path` is always well-formed.
    this.baseUrl = cfg.baseUrl.replace(/\/+$/, "");
    // exactOptionalPropertyTypes: only set when a token is configured.
    if (cfg.authToken !== undefined) {
      this.token = cfg.authToken;
    }
    this.defaultTimeoutMs = cfg.requestTimeoutMs;
  }

  /** Liveness probe — used by the service start hook. */
  async health(timeoutMs?: number): Promise<boolean> {
    try {
      const res = await this.fetchJson<{ status?: string }>("/health", "GET", undefined, timeoutMs);
      return Boolean(res && res.status);
    } catch {
      return false;
    }
  }

  /**
   * End-to-end deterministic recall. The server embeds the query, auto-routes
   * to the nearest domain centroid(s), falls back across domains on miss, and
   * returns ready-to-inject snippets. ONE HTTP call per turn.
   *
   * `source`/`since`/`lex`/`vec`/`hyde`/`intent` are the structured-query
   * overrides ("power tools") exposed by `POST /recall` (API_CONTRACT.md §2).
   *
   * Throws BrainHttpError on transport failure; the caller decides whether to
   * fail-open (recall hook) or surface (tool).
   */
  async recall(params: {
    query: string;
    domain?: string;
    strictDomain?: boolean;
    limit: number;
    /** Ingest kind, retrieval leg, or `both` (see {@link BrainSourceFilter}). */
    source?: string;
    since?: string;
    lex?: string;
    vec?: string;
    hyde?: string;
    intent?: string;
    timeoutMs?: number;
  }): Promise<BrainRecallResult> {
    const body: RecallRequestWire = {
      query: params.query,
      limit: params.limit,
      provenance: true,
      ...(params.domain ? { domain: params.domain } : {}),
      ...(typeof params.strictDomain === "boolean" ? { strict: params.strictDomain } : {}),
      ...(params.source ? { source: params.source } : {}),
      ...(params.since ? { since: params.since } : {}),
      ...(params.lex ? { lex: params.lex } : {}),
      ...(params.vec ? { vec: params.vec } : {}),
      ...(params.hyde ? { hyde: params.hyde } : {}),
      ...(params.intent ? { intent: params.intent } : {}),
    };
    const res = await this.fetchJson<RecallResponseWire>(
      "/recall",
      "POST",
      body,
      params.timeoutMs ?? this.defaultTimeoutMs,
    );
    // The server serializes snake_case (`domains_searched`); map it to the
    // camelCase result shape the rest of the plugin consumes.
    return {
      hits: res?.hits ?? [],
      decision: res?.decision ?? "ok",
      ...(res?.domain !== undefined ? { domain: res.domain } : {}),
      ...(res?.domains_searched !== undefined ? { domainsSearched: res.domains_searched } : {}),
      ...(res?.telemetry !== undefined ? { telemetry: res.telemetry } : {}),
    };
  }

  /**
   * Structured store. The server trusts the caller's graph data (validated
   * server-side; names/length enforced). Used by autoCapture and the
   * `memory_store` tool. entities/relations are optional graph enrichment.
   */
  async store(params: {
    title: string;
    content: string;
    domain?: string;
    entities?: BrainEntity[];
    relations?: BrainRelation[];
    timeoutMs?: number;
  }): Promise<BrainStoreResult> {
    const body = {
      title: params.title,
      content: params.content,
      ...(params.domain ? { domain: params.domain } : {}),
      ...(params.entities?.length ? { entities: params.entities } : {}),
      ...(params.relations?.length ? { relations: params.relations } : {}),
    };
    const res = await this.fetchJson<IngestResponseWire>(
      "/ingest",
      "POST",
      body,
      params.timeoutMs ?? this.defaultTimeoutMs,
    );
    // The server serializes snake_case (`entities_added`/`relations_added`).
    return {
      id: res?.id ?? 0,
      status: res?.status ?? "created",
      ...(res?.domain !== undefined ? { domain: res.domain } : {}),
      ...(res?.entities_added !== undefined ? { entitiesAdded: res.entities_added } : {}),
      ...(res?.relations_added !== undefined ? { relationsAdded: res.relations_added } : {}),
    };
  }

  /**
   * Returns `{ deleted: true }` on success, `null` on 404 (not found), and
   * throws BrainHttpError on any other failure. Distinguishing 404 lets the
   * tool report "not found" without masking a real server error.
   */
  async forget(id: string | number, timeoutMs?: number): Promise<{ deleted: boolean } | null> {
    try {
      const res = await this.fetchJson<{ deleted?: boolean }>(
        `/memory/${encodeURIComponent(String(id))}`,
        "DELETE",
        undefined,
        timeoutMs,
      );
      return { deleted: Boolean(res?.deleted) };
    } catch (err) {
      if (err instanceof BrainHttpError && err.status === 404) {
        return null;
      }
      throw err;
    }
  }

  /**
   * Fetch a single chunk by id (`GET /get/{id}`). Returns `null` on 404. The
   * returned content is the authoritative source text — useful after a recall
   * so an agent can read the full context behind a cited memory.
   */
  async get(id: string | number, timeoutMs?: number): Promise<BrainChunk | null> {
    try {
      const res = await this.fetchJson<BrainChunk>(
        `/get/${encodeURIComponent(String(id))}`,
        "GET",
        undefined,
        timeoutMs,
      );
      return res ?? null;
    } catch (err) {
      if (err instanceof BrainHttpError && err.status === 404) {
        return null;
      }
      throw err;
    }
  }

  /**
   * Deterministic span verification (`POST /verify`, v1.5): checks whether a
   * `claim` is literally supported by a chunk's stored text (case-insensitive
   * substring match). This is the hallucination-resistance primitive — an agent
   * that recalled a fact can confirm "the brain said X" before acting on it.
   */
  async verify(params: {
    chunkId: number;
    claim: string;
    timeoutMs?: number;
  }): Promise<BrainVerifyResult> {
    const res = await this.fetchJson<VerifyResponseWire>(
      "/verify",
      "POST",
      { chunk_id: params.chunkId, claim: params.claim },
      params.timeoutMs ?? this.defaultTimeoutMs,
    );
    return {
      chunkId: res?.chunk_id ?? params.chunkId,
      supported: Boolean(res?.supported),
      decision: res?.decision ?? "unsupported_claim",
      matchRanges: res?.match_ranges ?? [],
    };
  }

  /**
   * Knowledge-graph entity + one-hop relations (`GET /graph/entity/{name}`).
   * Returns `null` when the entity is unknown (the server answers `200` with an
   * `error` field for a missing entity). Name is normalized server-side.
   */
  async graphEntity(name: string, timeoutMs?: number): Promise<BrainGraphEntity | null> {
    const res = await this.fetchJson<BrainGraphEntity & { error?: string }>(
      `/graph/entity/${encodeURIComponent(name)}`,
      "GET",
      undefined,
      timeoutMs,
    );
    if (!res || res.error) {
      return null;
    }
    return {
      name: res.name,
      // exactOptionalPropertyTypes: omit `type` when absent, not set undefined.
      ...(res.type !== undefined ? { type: res.type } : {}),
      relations: res.relations ?? [],
    };
  }

  // --------------------------------------------------------------------------

  /**
   * Core transport. Throws BrainHttpError on every failure mode so callers can
   * distinguish 404/401/500/timeout/network/parse and act accordingly. An empty
   * 2xx body is a valid "no content" result and yields `undefined`.
   */
  private async fetchJson<T>(
    path: string,
    method: string,
    body: unknown,
    timeoutMs?: number,
  ): Promise<T | undefined> {
    const controller = new AbortController();
    const timer = setTimeout(
      () => controller.abort(new Error("brain-server request timed out")),
      timeoutMs ?? this.defaultTimeoutMs,
    );
    let res: Response;
    try {
      res = await fetch(`${this.baseUrl}${path}`, {
        method,
        signal: controller.signal,
        headers: {
          Accept: "application/json",
          ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
          ...(this.token ? { Authorization: `Bearer ${this.token}` } : {}),
        },
        // RequestInit.body is BodyInit | null (not undefined) under TS lib.
        body: body !== undefined ? JSON.stringify(body) : null,
      });
    } catch (err) {
      // AbortController fires with the reason passed to .abort(); everything
      // else is a DNS/TCP/connection failure.
      if (controller.signal.aborted) {
        throw new BrainHttpError("timeout", (err as Error)?.message ?? "timed out");
      }
      throw new BrainHttpError("network", (err as Error)?.message ?? "fetch failed");
    } finally {
      clearTimeout(timer);
    }

    if (!res.ok) {
      // Surface the status + a best-effort message from the server body.
      let detail = res.statusText;
      try {
        const text = await res.text();
        if (text) {
          detail = text.length > 500 ? `${text.slice(0, 500)}…` : text;
        }
      } catch {
        // Body already consumed or unreadable; keep statusText.
      }
      throw new BrainHttpError("http", detail, res.status);
    }

    const text = await res.text();
    if (!text) {
      return undefined;
    }
    try {
      return JSON.parse(text) as T;
    } catch (err) {
      throw new BrainHttpError("parse", (err as Error)?.message ?? "invalid JSON");
    }
  }
}
