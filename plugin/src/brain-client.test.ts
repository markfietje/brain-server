/**
 * BrainClient transport tests. Uses a mocked global `fetch` to exercise the
 * typed error model (BrainHttpError) introduced to let tools distinguish
 * 404/401/500/timeout/network/parse failures instead of collapsing them all
 * to a silent `undefined`.
 */
import { afterEach, describe, expect, test, vi } from "vitest";

import { BrainClient, BrainHttpError, describeBrainError } from "./brain-client.js";
import { resolveConfig } from "./config.js";

const cfg = () =>
  resolveConfig({
    baseUrl: "http://127.0.0.1:8765",
    requestTimeoutMs: 100,
  });

function mockResponse(body: unknown, init: { status?: number; statusText?: string } = {}) {
  const status = init.status ?? 200;
  const text = typeof body === "string" ? body : JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: init.statusText ?? "",
    text: async () => text,
  } as unknown as Response;
}

describe("BrainClient.recall", () => {
  afterEach(() => vi.restoreAllMocks());

  test("returns parsed hits on 200", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ hits: [{ id: 1, content: "x", score: 0.9 }], domain: "health" }),
    );
    const client = new BrainClient(cfg());
    const res = await client.recall({ query: "q", limit: 5 });
    expect(res.hits).toHaveLength(1);
    expect(res.domain).toBe("health");
  });

  test("empty 2xx body => empty hits, no throw", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("", { status: 204 }));
    const client = new BrainClient(cfg());
    const res = await client.recall({ query: "q", limit: 5 });
    expect(res.hits).toEqual([]);
  });

  test("500 => throws BrainHttpError(http, status=500)", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("boom", { status: 500 }));
    const client = new BrainClient(cfg());
    await expect(client.recall({ query: "q", limit: 5 })).rejects.toMatchObject({
      kind: "http",
      status: 500,
    });
  });

  test("401 => throws http error with status 401", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("unauthorized", { status: 401 }));
    const client = new BrainClient(cfg());
    await expect(client.recall({ query: "q", limit: 5 })).rejects.toMatchObject({
      kind: "http",
      status: 401,
    });
  });

  test("network failure => throws BrainHttpError(network)", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new TypeError("fetch failed"));
    const client = new BrainClient(cfg());
    await expect(client.recall({ query: "q", limit: 5 })).rejects.toMatchObject({
      kind: "network",
    });
  });

  test("malformed JSON => throws BrainHttpError(parse)", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("{not json", { status: 200 }));
    const client = new BrainClient(cfg());
    await expect(client.recall({ query: "q", limit: 5 })).rejects.toMatchObject({
      kind: "parse",
    });
  });
});

describe("BrainClient.forget — 404 vs error distinction", () => {
  afterEach(() => vi.restoreAllMocks());

  test("404 => resolves to null (not found), does NOT throw", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("not found", { status: 404 }));
    const client = new BrainClient(cfg());
    await expect(client.forget("123")).resolves.toBeNull();
  });

  test("200 => resolves to { deleted: true }", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ deleted: true }));
    const client = new BrainClient(cfg());
    await expect(client.forget("123")).resolves.toEqual({ deleted: true });
  });

  test("500 => throws (a real error is not masked as 'not found')", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse("boom", { status: 500 }));
    const client = new BrainClient(cfg());
    await expect(client.forget("123")).rejects.toMatchObject({ kind: "http", status: 500 });
  });
});

describe("BrainClient.store", () => {
  afterEach(() => vi.restoreAllMocks());

  test("201 => returns created status + id", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ id: 42, status: "created", domain: "health", entitiesAdded: 2 }),
    );
    const client = new BrainClient(cfg());
    const res = await client.store({ title: "t", content: "c" });
    expect(res.id).toBe(42);
    expect(res.status).toBe("created");
    expect(res.entitiesAdded).toBe(2);
  });

  test("omits entities/relations from body when empty", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      mockResponse({ id: 1, status: "created" }),
    );
    const client = new BrainClient(cfg());
    await client.store({ title: "t", content: "c" });
    const sentBody = JSON.parse((fetchMock.mock.calls[0]?.[1]?.body as string) ?? "{}");
    expect(sentBody).not.toHaveProperty("entities");
    expect(sentBody).not.toHaveProperty("relations");
  });
});

describe("BrainClient.health", () => {
  afterEach(() => vi.restoreAllMocks());

  test("returns true when /health reports status", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(mockResponse({ status: "ok" }));
    const client = new BrainClient(cfg());
    await expect(client.health()).resolves.toBe(true);
  });

  test("returns false on transport failure (never throws)", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new TypeError("nope"));
    const client = new BrainClient(cfg());
    await expect(client.health()).resolves.toBe(false);
  });
});

describe("describeBrainError", () => {
  test("http error includes status", () => {
    const msg = describeBrainError(new BrainHttpError("http", "boom", 500));
    expect(msg).toContain("500");
    expect(msg).toContain("boom");
  });

  test("timeout/network/parse are labeled", () => {
    expect(describeBrainError(new BrainHttpError("timeout", "slow"))).toContain("timed out");
    expect(describeBrainError(new BrainHttpError("network", "down"))).toContain("unreachable");
    expect(describeBrainError(new BrainHttpError("parse", "bad"))).toContain("malformed JSON");
  });

  test("non-BrainHttpError falls back to string coercion", () => {
    expect(describeBrainError(new Error("xyz"))).toContain("xyz");
  });
});
