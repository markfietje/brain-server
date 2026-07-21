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
    if (status !== undefined) this.status = status;
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

export type BrainRecallHit = {
  id: number | string;
  title?: string;
  content: string;
  score: number;
  domain?: string;
  source?: "vector" | "fts" | "graph";
};

export type BrainRecallResult = {
  hits: BrainRecallHit[];
  domain?: string;
  domainsSearched?: string[];
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

/** Liveness probe — used by the service start hook. */
export class BrainClient {
  private readonly baseUrl: string;
  private readonly token?: string;
  private readonly defaultTimeoutMs: number;

  constructor(cfg: ResolvedBrainConfig) {
    // Trim trailing slash so `${baseUrl}/path` is always well-formed.
    this.baseUrl = cfg.baseUrl.replace(/\/+$/, "");
    // exactOptionalPropertyTypes: only set when a token is configured.
    if (cfg.authToken !== undefined) this.token = cfg.authToken;
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
   * Throws BrainHttpError on transport failure; the caller decides whether to
   * fail-open (recall hook) or surface (tool).
   */
  async recall(params: {
    query: string;
    domain?: string;
    strictDomain?: boolean;
    limit: number;
    timeoutMs?: number;
  }): Promise<BrainRecallResult> {
    const body = {
      query: params.query,
      limit: params.limit,
      ...(params.domain ? { domain: params.domain } : {}),
      ...(typeof params.strictDomain === "boolean" ? { strict: params.strictDomain } : {}),
      provenance: true,
    };
    const res = await this.fetchJson<BrainRecallResult>(
      "/recall",
      "POST",
      body,
      params.timeoutMs ?? this.defaultTimeoutMs,
    );
    return {
      hits: res?.hits ?? [],
      ...(res?.domain !== undefined ? { domain: res.domain } : {}),
      ...(res?.domainsSearched !== undefined ? { domainsSearched: res.domainsSearched } : {}),
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
    const res = await this.fetchJson<BrainStoreResult>(
      "/ingest",
      "POST",
      body,
      params.timeoutMs ?? this.defaultTimeoutMs,
    );
    return {
      id: res?.id ?? 0,
      status: res?.status ?? "created",
      ...(res?.domain !== undefined ? { domain: res.domain } : {}),
      ...(res?.entitiesAdded !== undefined ? { entitiesAdded: res.entitiesAdded } : {}),
      ...(res?.relationsAdded !== undefined ? { relationsAdded: res.relationsAdded } : {}),
    };
  }

  /**
   * Returns `{ deleted: true }` on success, `null` on 404 (not found), and
   * throws BrainHttpError on any other failure. Distinguishing 404 lets the
   * tool report "not found" without masking a real server error.
   */
  async forget(
    id: string | number,
    timeoutMs?: number,
  ): Promise<{ deleted: boolean } | null> {
    try {
      const res = await this.fetchJson<{ deleted?: boolean }>(
        `/memory/${encodeURIComponent(String(id))}`,
        "DELETE",
        undefined,
        timeoutMs,
      );
      return { deleted: Boolean(res?.deleted) };
    } catch (err) {
      if (err instanceof BrainHttpError && err.status === 404) return null;
      throw err;
    }
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
        if (text) detail = text.length > 500 ? `${text.slice(0, 500)}…` : text;
      } catch {
        // Body already consumed or unreadable; keep statusText.
      }
      throw new BrainHttpError("http", detail, res.status);
    }

    const text = await res.text();
    if (!text) return undefined;
    try {
      return JSON.parse(text) as T;
    } catch (err) {
      throw new BrainHttpError("parse", (err as Error)?.message ?? "invalid JSON");
    }
  }
}
