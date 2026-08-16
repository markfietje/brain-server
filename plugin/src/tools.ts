/**
 * Brain Server — agent-callable memory tools.
 *
 * Pure tool definitions extracted from the plugin entry so index.ts stays under
 * the oxlint max-lines budget. Each tool is a thin adapter over BrainClient;
 * the Rust server owns all memory logic. `liveCfg` is read fresh on every call
 * so operator config changes take effect without re-registration.
 *
 * Tool param schemas are the single source of truth: the runtime validates
 * incoming params against `parameters` before invoking `execute`, so handlers
 * narrow (unknown) params to the Static-derived type rather than re-validating.
 */
import type { OpenClawPluginApi } from "openclaw/plugin-sdk/plugin-entry";
import { Check } from "typebox/value";
import { Type, type Static, type TObject } from "typebox";
import {
  BrainClient,
  describeBrainError,
  type BrainApproveResult,
  type BrainProposal,
  type BrainRecallHit,
  type BrainRejectResult,
  type BrainTraverseResult,
} from "./brain-client.js";
import type { ResolvedBrainConfig } from "./config.js";
import {
  RECALL_ABSTENTION,
  formatRecallContext,
  normalizeRecallQuery,
  sanitizeForBlock,
} from "./format.js";

type LiveCfg = () => ResolvedBrainConfig;

// v1.20.29 "Bound" (F-7): replace the raw `as X` casts at handler dispatch
// with a Typebox `Check` narrowing guard. `Check` is a type predicate
// (`value is Static<S>`), so no `as` is needed once it passes. The SDK runtime
// validates `params` against `parameters` before invoking `execute`, so this is
// defense-in-depth at the plugin seam — a loose host runtime can no longer
// forward out-of-range args to the Rust server (which remains the authority).
// ponytail: does NOT add a full Zod layer (typebox is the existing dep; reuse
// it). On schema failure the params collapse to `{}` and the handler's
// existing `?? default` branches take over (graceful, fail-closed).
function checkedParams<S extends TObject>(schema: S, raw: unknown): Partial<Static<S>> {
  return Check(schema, raw) ? raw : {};
}

// v1.20.29 "Bound": per-hit body cap applied on the CALLER side (here + the
// auto-recall hook in index.ts), so `format.ts` (Release C) stays untouched.
// Caps the largest single field a recall hit can contribute to injected
// context; applied BEFORE `formatRecallContext`, not inside it.
export const MAX_HIT_CHARS = 1000;

/** Clamp each hit's `content` to `MAX_HIT_CHARS`. Non-mutating — returns a new
 *  array (only copies hits that actually need truncation) so the shared result
 *  from the dedup map is never mutated in place. */
function clampHitBodies(hits: ReadonlyArray<BrainRecallHit>): BrainRecallHit[] {
  return hits.map((h) =>
    h.content.length > MAX_HIT_CHARS ? { ...h, content: h.content.slice(0, MAX_HIT_CHARS) } : h,
  );
}

/**
 * Sanitized mirror of a recall hit for the tool `details` seam. Built
 * field-by-field (not spread) so bidi/zero-width bytes never reach the model
 * if the runtime serializes `details` into context, and to satisfy
 * oxc/no-map-spread inside the `.map(sanitizeHit)` call site.
 */
type SanitizedHit = {
  id: number | string;
  title?: string;
  content: string;
  score: number;
  domain?: string;
  source?: string;
  provenance?: unknown;
  evidence?: unknown;
  snippet?: string;
  untrusted?: boolean;
  conflict?: boolean;
};

function sanitizeHit(h: BrainRecallHitLike): SanitizedHit {
  return {
    id: h.id,
    ...(h.title !== undefined ? { title: sanitizeForBlock(h.title) } : {}),
    content: sanitizeForBlock(h.content),
    score: h.score,
    ...(h.domain !== undefined ? { domain: h.domain } : {}),
    ...(h.source !== undefined ? { source: h.source } : {}),
    ...(h.provenance !== undefined ? { provenance: h.provenance } : {}),
    ...(h.evidence !== undefined ? { evidence: h.evidence } : {}),
    // v1.27.14 "Fencepost2" (F-01): `snippet` carries the same smuggling class
    // as title/content — run it through the shared block boundary too (it was
    // the one field the detail seam passed raw).
    ...(h.snippet !== undefined ? { snippet: sanitizeForBlock(h.snippet) } : {}),
    ...(h.untrusted !== undefined ? { untrusted: h.untrusted } : {}),
    ...(h.conflict !== undefined ? { conflict: h.conflict } : {}),
  };
}

type BrainRecallHitLike = {
  id: number | string;
  title?: string;
  content: string;
  score: number;
  domain?: string;
  source?: string;
  provenance?: unknown;
  evidence?: unknown;
  snippet?: string;
  untrusted?: boolean;
  conflict?: boolean;
};

/**
 * Register the eight agent-callable memory tools. Gated tools
 * (memory_proposal_*) register only when `proposalTools` is true — promoting a
 * capture to memory is an operator action, so the agent gains them only on
 * explicit opt-in.
 */
export function registerBrainTools(
  api: OpenClawPluginApi,
  client: BrainClient,
  liveCfg: LiveCfg,
  proposalTools: boolean,
): void {
  const memoryRecallParamsSchema = Type.Object({
    query: Type.String({ description: "Search query" }),
    limit: Type.Optional(
      Type.Integer({ minimum: 1, maximum: 50, description: "Max results (default 5)" }),
    ),
    domain: Type.Optional(
      Type.String({ description: "Force a specific domain (auto-routing is default)" }),
    ),
    source: Type.Optional(Type.String({ description: "Filter by knowledge source id/name" })),
    since: Type.Optional(
      Type.String({
        description: "Only rows with created_at after this ISO-8601/RFC3339 time",
      }),
    ),
    lex: Type.Optional(
      Type.String({
        description: "Lexical (FTS5) query override: exact terms, phrases, -exclusions",
      }),
    ),
    vec: Type.Optional(Type.String({ description: "Semantic embedding-query override" })),
    hyde: Type.Optional(
      Type.String({ description: "Hypothetical-answer embedding override (beats vec)" }),
    ),
    intent: Type.Optional(
      Type.String({ description: "Free-form intent label, recorded for provenance" }),
    ),
    // v0.3.0 advanced recall surface (temporal + graph + packing).
    at: Type.Optional(
      Type.String({
        description:
          "Bi-temporal valid-time point-in-time (RFC3339/YYYY-MM-DD): only facts whose [valid_from, valid_to) contains it.",
      }),
    ),
    asOf: Type.Optional(
      Type.String({
        description: "Revision (transaction-time) point-in-time; hits tagged lifecycle:historical.",
      }),
    ),
    memoryKind: Type.Optional(
      Type.String({
        description: "Filter by knowledge node_kind: fact|procedure|step|decision|episodic.",
      }),
    ),
    minRelevance: Type.Optional(
      Type.String({
        description: "Drop lower relevance tiers post-fusion: high|medium|low.",
      }),
    ),
    includeDecayed: Type.Optional(
      Type.Boolean({ description: "Include decayed chunks, tagged decayed:true on the hit." }),
    ),
    graph: Type.Optional(
      Type.Boolean({
        description: "Enable graph-PPR as a third RRF retrieval leg (zero-token, deterministic).",
      }),
    ),
    maxContextTokens: Type.Optional(
      Type.Integer({
        minimum: 0,
        // v1.20.29 "Bound" (F-7): match the auto-recall ceiling (config.ts
        // autoRecallMaxContextTokens max = 8_000) so the tool can't request a
        // 32k-token pack the operator never intended to allow through the
        // auto-recall path. The server's validate_recall is still the authority.
        maximum: 8_000,
        description:
          "Submodular evidence-packing token budget: re-rank + truncate for coverage/diversity.",
      }),
    ),
  });

  api.registerTool(
    {
      name: "memory_recall",
      label: "Memory Recall",
      description:
        "Search long-term memory. Use for past decisions, preferences, or previously discussed topics. Optionally scope by source or time, or override the semantic query.",
      parameters: memoryRecallParamsSchema,
      async execute(_toolCallId, params) {
        const c = liveCfg();
        // v1.20.29 "Bound": `Check` narrows params to the schema type (no `as`).
        const p = checkedParams(memoryRecallParamsSchema, params ?? {});
        const query = normalizeRecallQuery(p.query ?? "", c.recallMaxChars);
        if (!query) {
          return {
            content: [{ type: "text" as const, text: "No query provided." }],
            details: { count: 0 },
          };
        }
        let result;
        try {
          result = await client.recall({
            query,
            ...(p.domain ? { domain: p.domain } : {}),
            limit: p.limit ?? 5,
            source: p.source,
            since: p.since,
            lex: p.lex,
            vec: p.vec,
            hyde: p.hyde,
            intent: p.intent,
            ...(p.at ? { at: p.at } : {}),
            ...(p.asOf ? { asOf: p.asOf } : {}),
            ...(p.memoryKind ? { memoryKind: p.memoryKind } : {}),
            ...(p.minRelevance ? { minRelevance: p.minRelevance } : {}),
            ...(typeof p.includeDecayed === "boolean" ? { includeDecayed: p.includeDecayed } : {}),
            ...(typeof p.graph === "boolean" ? { graph: p.graph } : {}),
            ...(typeof p.maxContextTokens === "number"
              ? { maxContextTokens: p.maxContextTokens }
              : {}),
            timeoutMs: c.requestTimeoutMs,
          });
        } catch (err) {
          // Tools surface the failure so the agent can react; recall errors
          // are not silent here (unlike the fail-open auto-recall hook).
          return {
            content: [{ type: "text" as const, text: `Recall failed: ${describeBrainError(err)}` }],
            details: { count: 0, error: describeBrainError(err) },
          };
        }
        if (result.decision === "low_confidence") {
          // Calibrated abstention (v1.5): empty by design, not a miss.
          return {
            content: [{ type: "text" as const, text: RECALL_ABSTENTION }],
            details: { count: 0, decision: result.decision },
          };
        }
        if (!result.hits.length) {
          return {
            content: [{ type: "text" as const, text: "No relevant memories found." }],
            details: { count: 0, decision: result.decision },
          };
        }
        // v1.20.29 "Bound": per-hit body cap (caller-side, before formatting)
        // so a single huge chunk can't dominate the injected context. format.ts
        // is untouched (Release C owns it); the cap is applied here + the hook.
        const hits = clampHitBodies(result.hits);
        return {
          content: [{ type: "text" as const, text: formatRecallContext(hits) }],
          details: {
            count: hits.length,
            decision: result.decision,
            domainsSearched: result.domainsSearched,
            // v1.20.25: if the runtime serializes tool `details` into the
            // model context, raw bidi/zero-width bytes would reach the model
            // verbatim — run every hit field through the same block boundary
            // the `content` path already uses.
            memories: hits.map(sanitizeHit),
          },
        };
      },
    },
    { name: "memory_recall" },
  );

  const memoryStoreParamsSchema = Type.Object({
    text: Type.String({ description: "Information to remember" }),
    title: Type.Optional(Type.String({ description: "Short title (default: first 80 chars)" })),
    domain: Type.Optional(Type.String()),
    entities: Type.Optional(
      Type.Array(
        Type.Object({
          name: Type.String(),
          type: Type.Optional(Type.String()),
        }),
      ),
    ),
    relations: Type.Optional(
      Type.Array(
        Type.Object({
          from: Type.String(),
          to: Type.String(),
          type: Type.String(),
        }),
      ),
    ),
  });

  api.registerTool(
    {
      name: "memory_store",
      label: "Memory Store",
      description:
        "Save a durable fact/decision to long-term memory. Optionally include entities/relations for the knowledge graph.",
      parameters: memoryStoreParamsSchema,
      async execute(_toolCallId, params) {
        const c = liveCfg();
        const p = checkedParams(memoryStoreParamsSchema, params ?? {});
        const text = (p.text ?? "").trim();
        if (!text) {
          return {
            content: [{ type: "text" as const, text: "No text provided." }],
            details: { stored: false },
          };
        }
        let res;
        try {
          // v1.20.25: the agent's write surface must respect the same
          // capture-mode rule as autoCapture. Default `captureMode:
          // "proposal"` queues the fact for HUMAN review (POST
          // /ingest/proposal) instead of writing it straight to memory — an
          // agent driving this tool can no longer persist arbitrary
          // instructions into long-term memory without a reviewer. Only an
          // operator who explicitly configured `captureMode: "direct"`
          // keeps the straight-to-memory path (still server-screened).
          if (c.captureMode === "direct") {
            res = await client.store({
              title: p.title?.trim() || text.slice(0, 80),
              content: text,
              ...(p.domain ? { domain: p.domain } : {}),
              ...(Array.isArray(p.entities) ? { entities: p.entities } : {}),
              ...(Array.isArray(p.relations) ? { relations: p.relations } : {}),
              timeoutMs: c.requestTimeoutMs,
            });
          } else {
            const prop = await client.submitProposal({
              content: text,
              source: "memory_store",
              ...(p.title?.trim() ? { sourcePrompt: p.title.trim() } : {}),
              timeoutMs: c.requestTimeoutMs,
            });
            res = { id: prop.id, status: prop.status };
          }
        } catch (err) {
          return {
            content: [{ type: "text" as const, text: `Store failed: ${describeBrainError(err)}` }],
            details: { stored: false, error: describeBrainError(err) },
          };
        }
        const pending = res.status === "pending";
        return {
          content: [
            {
              type: "text" as const,
              text: pending
                ? `Submitted for review (proposal id: ${res.id}). It becomes memory only after a reviewer approves it.`
                : `Memory ${res.status} (id: ${res.id}).`,
            },
          ],
          details: { stored: !pending, pending, id: res.id, status: res.status },
        };
      },
    },
    { name: "memory_store" },
  );

  // v1.20.25: the agent-facing `memory_forget` tool is REMOVED. An agent can
  // autonomously hard-delete long-term memory with no human gate — ambient
  // authority in the exact shape the mantra forbids ("memory you can see,
  // approve, and erase" — erase is a HUMAN action). The read-only recall/
  // get/verify/graph tools + the review-queued `memory_store` are the agent's
  // only surface. Erasure remains a human action via the operator console
  // and the `brain` CLI (server `DELETE /memory/{id}` is untouched).

  const memoryVerifyParamsSchema = Type.Object({
    chunk_id: Type.Integer({ description: "Chunk/memory id (from a recall hit)" }),
    claim: Type.String({ description: "The claim to verify against the chunk text" }),
  });

  api.registerTool(
    {
      name: "memory_verify",
      label: "Memory Verify",
      description:
        "Deterministic span verification (no LLM): checks whether a claim is literally supported by a chunk's stored text. Use after recalling a fact to confirm the brain actually said it before acting on it.",
      parameters: memoryVerifyParamsSchema,
      async execute(_toolCallId, params) {
        const c = liveCfg();
        const p = checkedParams(memoryVerifyParamsSchema, params ?? {});
        const claim = (p.claim ?? "").trim();
        if (!claim || typeof p.chunk_id !== "number") {
          return {
            content: [{ type: "text" as const, text: "Both chunk_id and claim are required." }],
            details: { verified: false },
          };
        }
        try {
          const res = await client.verify({
            chunkId: p.chunk_id,
            claim,
            timeoutMs: c.requestTimeoutMs,
          });
          const text = res.supported
            ? `Supported: the chunk ${res.chunkId} contains the claim (${res.matchRanges.length} match${res.matchRanges.length === 1 ? "" : "es"}).`
            : `Not supported: the chunk ${res.chunkId} does not contain the claim. Do not present it as memory-backed.`;
          return {
            content: [{ type: "text" as const, text }],
            details: { verified: res.supported, decision: res.decision, chunkId: res.chunkId },
          };
        } catch (err) {
          return {
            content: [{ type: "text" as const, text: `Verify failed: ${describeBrainError(err)}` }],
            details: { verified: false, error: describeBrainError(err) },
          };
        }
      },
    },
    { name: "memory_verify" },
  );

  const memoryGetParamsSchema = Type.Object({
    id: Type.Integer({ description: "Chunk/memory id (from a recall hit)" }),
  });

  api.registerTool(
    {
      name: "memory_get",
      label: "Memory Get",
      description:
        "Fetch the full stored text of a memory/chunk by id. Use to read the complete context behind a recalled snippet before relying on it.",
      parameters: memoryGetParamsSchema,
      async execute(_toolCallId, params) {
        const c = liveCfg();
        const p = checkedParams(memoryGetParamsSchema, params ?? {});
        if (typeof p.id !== "number") {
          return {
            content: [{ type: "text" as const, text: "id is required." }],
            details: { found: false },
          };
        }
        try {
          const chunk = await client.get(p.id, c.requestTimeoutMs);
          if (!chunk) {
            return {
              content: [{ type: "text" as const, text: `No memory with id ${p.id}.` }],
              details: { found: false, id: p.id },
            };
          }
          const title = chunk.title?.trim() ? `\n${sanitizeForBlock(chunk.title)}` : "";
          return {
            content: [
              {
                type: "text" as const,
                text: `Memory ${chunk.id}:${title}\n${sanitizeForBlock(chunk.content)}`,
              },
            ],
            details: {
              found: true,
              id: chunk.id,
              title: chunk.title ? sanitizeForBlock(chunk.title) : chunk.title,
            },
          };
        } catch (err) {
          return {
            content: [{ type: "text" as const, text: `Get failed: ${describeBrainError(err)}` }],
            details: { found: false, error: describeBrainError(err) },
          };
        }
      },
    },
    { name: "memory_get" },
  );

  const memoryGraphEntityParamsSchema = Type.Object({
    name: Type.String({ description: "Entity name (e.g. 'vitamin d3')" }),
  });

  api.registerTool(
    {
      name: "memory_graph_entity",
      label: "Memory Graph Entity",
      description:
        "Look up an entity in the knowledge graph and its one-hop relations. Use to explore how a concept connects to others.",
      parameters: memoryGraphEntityParamsSchema,
      async execute(_toolCallId, params) {
        const c = liveCfg();
        const p = checkedParams(memoryGraphEntityParamsSchema, params ?? {});
        const name = (p.name ?? "").trim();
        if (!name) {
          return {
            content: [{ type: "text" as const, text: "name is required." }],
            details: { found: false },
          };
        }
        try {
          const entity = await client.graphEntity(name, c.requestTimeoutMs);
          if (!entity) {
            return {
              content: [{ type: "text" as const, text: `No entity named "${name}" in the graph.` }],
              details: { found: false, name },
            };
          }
          const etype = entity.type ? ` (${sanitizeForBlock(entity.type).trim()})` : "";
          const rels =
            entity.relations.length === 0
              ? " (no relations)"
              : entity.relations
                  .map(
                    (r) =>
                      `\n  - ${r.direction === "out" ? "→" : "←"} ${sanitizeForBlock(r.relation_type).trim()} ${sanitizeForBlock(r.to_entity).trim()}`,
                  )
                  .join("");
          return {
            content: [
              {
                type: "text" as const,
                text: `${sanitizeForBlock(entity.name).trim()}${etype}${rels}`,
              },
            ],
            details: {
              found: true,
              name: sanitizeForBlock(entity.name),
              relations: entity.relations.map((r) => ({
                to_entity: sanitizeForBlock(r.to_entity),
                relation_type: sanitizeForBlock(r.relation_type),
                direction: r.direction,
              })),
            },
          };
        } catch (err) {
          return {
            content: [
              { type: "text" as const, text: `Graph lookup failed: ${describeBrainError(err)}` },
            ],
            details: { found: false, error: describeBrainError(err) },
          };
        }
      },
    },
    { name: "memory_graph_entity" },
  );

  const memoryGraphTraverseParamsSchema = Type.Object({
    start: Type.String({
      description: "Entity name to traverse from (e.g. 'insulin resistance').",
    }),
    maxDepth: Type.Optional(
      Type.Integer({
        minimum: 1,
        maximum: 4,
        description: "Max hops (server hard-caps at 4). Default 2.",
      }),
    ),
    crossDomain: Type.Optional(
      Type.Boolean({
        description:
          "Walk across every domain pool. Default false (stay in the auto-routed domain).",
      }),
    ),
    at: Type.Optional(
      Type.String({
        description:
          "Bi-temporal point-in-time: only edges valid at this instant (RFC3339/YYYY-MM-DD).",
      }),
    ),
    kind: Type.Optional(
      Type.String({
        description:
          "Edge-type filter. Exact match, or prefix match if it ends with ':' (e.g. 'causes:' for the causal subgraph).",
      }),
    ),
    explain: Type.Optional(
      Type.Boolean({
        description:
          "Add structured hop chains (paths). Intermediate node ids need memory_get to resolve names.",
      }),
    ),
  });

  api.registerTool(
    {
      name: "memory_graph_traverse",
      label: "Memory Graph Traverse",
      description:
        "Multi-hop knowledge-graph traversal from a start entity. Use for reasoning over how concepts connect: causal subgraphs (kind='causes:'), multi-hop relations, and point-in-time walks. Bounded to 4 hops / 256 nodes by the server.",
      parameters: memoryGraphTraverseParamsSchema,
      async execute(_toolCallId, params) {
        const c = liveCfg();
        const p = checkedParams(memoryGraphTraverseParamsSchema, params ?? {});
        const start = (p.start ?? "").trim();
        if (!start) {
          return {
            content: [{ type: "text" as const, text: "start is required." }],
            details: { found: false },
          };
        }
        try {
          const res: BrainTraverseResult = await client.graphTraverse({
            start,
            ...(p.maxDepth !== undefined ? { maxDepth: p.maxDepth } : {}),
            ...(typeof p.crossDomain === "boolean" ? { crossDomain: p.crossDomain } : {}),
            ...(p.at ? { at: p.at } : {}),
            ...(p.kind ? { kind: p.kind } : {}),
            ...(typeof p.explain === "boolean" ? { explain: p.explain } : {}),
            timeoutMs: c.requestTimeoutMs,
          });
          if (res.traversal.length === 0) {
            return {
              content: [{ type: "text" as const, text: `No graph paths from "${start}".` }],
              details: { found: false, visited: res.visited },
            };
          }
          const rows = res.traversal
            .map(
              (r) =>
                `${"  ".repeat(Math.max(0, r.depth - 1))}${r.depth}. ${sanitizeForBlock(r.entity).trim()} [${r.edgePath || "-"}] (domain: ${r.domain})`,
            )
            .join("\n");
          return {
            content: [
              {
                type: "text" as const,
                text: `Traverse from "${start}" (visited ${res.visited}):\n${rows}`,
              },
            ],
            details: {
              found: true,
              visited: res.visited,
              traversal: res.traversal.map((r) => ({
                entity: sanitizeForBlock(r.entity),
                depth: r.depth,
                path: r.path,
                edgePath: r.edgePath,
                fromEntity: r.fromEntity,
                domain: r.domain,
              })),
              ...(res.paths ? { paths: res.paths } : {}),
            },
          };
        } catch (err) {
          return {
            content: [
              { type: "text" as const, text: `Traverse failed: ${describeBrainError(err)}` },
            ],
            details: { found: false, error: describeBrainError(err) },
          };
        }
      },
    },
    { name: "memory_graph_traverse" },
  );

  // v0.3.0: human review-queue tools. These close the loop on the default
  // `captureMode: "proposal"`: a captured memory is invisible to recall
  // until a reviewer approves it here. Default OFF — promoting a capture to
  // memory is an operator action, so the agent gains it only when an operator
  // explicitly sets config.proposalTools = true.
  if (proposalTools) {
    const memoryProposalListParamsSchema = Type.Object({
      status: Type.Optional(
        Type.Union([Type.Literal("pending"), Type.Literal("approved"), Type.Literal("rejected")]),
      ),
      limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 200 })),
    });

    api.registerTool(
      {
        name: "memory_proposal_list",
        label: "Memory Proposal List",
        description:
          "List captured facts awaiting human review (default status: pending). Use to see what autoCapture or memory_store queued before it becomes memory.",
        parameters: memoryProposalListParamsSchema,
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = checkedParams(memoryProposalListParamsSchema, params ?? {});
          try {
            const proposals: BrainProposal[] = await client.listProposals({
              ...(p.status ? { status: p.status } : {}),
              ...(p.limit ? { limit: p.limit } : {}),
              timeoutMs: c.requestTimeoutMs,
            });
            if (proposals.length === 0) {
              return {
                content: [{ type: "text" as const, text: "No proposals in this state." }],
                details: { count: 0 },
              };
            }
            const lines = proposals
              .map((pr) => {
                const conflict =
                  pr.conflictWith !== undefined && pr.conflictWith !== null
                    ? ` conflits_with=${pr.conflictWith}`
                    : "";
                return `#${pr.id} [${pr.kind}] (novelty=${pr.novelty.toFixed(2)}, salience=${pr.salience.toFixed(2)}${conflict}, verdict=${pr.screenVerdict}): ${sanitizeForBlock(pr.content)}`;
              })
              .join("\n");
            return {
              content: [{ type: "text" as const, text: lines }],
              details: { count: proposals.length, proposals },
            };
          } catch (err) {
            return {
              content: [
                { type: "text" as const, text: `Proposal list failed: ${describeBrainError(err)}` },
              ],
              details: { count: 0, error: describeBrainError(err) },
            };
          }
        },
      },
      { name: "memory_proposal_list" },
    );

    const memoryProposalDecideParamsSchema = Type.Object({
      id: Type.Integer({ description: "Proposal id from memory_proposal_list." }),
      decision: Type.Union([Type.Literal("approve"), Type.Literal("reject")]),
      supersedes: Type.Optional(
        Type.Integer({
          description:
            "On approve: atomically expire this older chunk id in the same transaction (supersession).",
        }),
      ),
    });

    api.registerTool(
      {
        name: "memory_proposal_decide",
        label: "Memory Proposal Decide",
        description:
          "Approve or reject a captured proposal. Approving promotes it to memory (recallable); rejecting drops it (audited). This is the human-review gate for captureMode 'proposal'.",
        parameters: memoryProposalDecideParamsSchema,
        async execute(_toolCallId, params) {
          const c = liveCfg();
          const p = checkedParams(memoryProposalDecideParamsSchema, params ?? {});
          if (typeof p.id !== "number" || (p.decision !== "approve" && p.decision !== "reject")) {
            return {
              content: [
                {
                  type: "text" as const,
                  text: "Both id and decision (approve|reject) are required.",
                },
              ],
              details: { decided: false },
            };
          }
          try {
            if (p.decision === "approve") {
              const res: BrainApproveResult = await client.approveProposal({
                id: p.id,
                ...(p.supersedes !== undefined ? { supersedes: p.supersedes } : {}),
                timeoutMs: c.requestTimeoutMs,
              });
              return {
                content: [
                  {
                    type: "text" as const,
                    text: `Approved proposal #${res.proposalId} -> chunk #${res.chunkId}.${res.superseded !== null ? ` Superseded chunk #${res.superseded}.` : ""}`,
                  },
                ],
                details: { decided: true, ...res },
              };
            }
            const res: BrainRejectResult = await client.rejectProposal({
              id: p.id,
              timeoutMs: c.requestTimeoutMs,
            });
            return {
              content: [{ type: "text" as const, text: `Rejected proposal #${res.proposalId}.` }],
              details: { decided: true, ...res },
            };
          } catch (err) {
            return {
              content: [
                { type: "text" as const, text: `Decide failed: ${describeBrainError(err)}` },
              ],
              details: { decided: false, error: describeBrainError(err) },
            };
          }
        },
      },
      { name: "memory_proposal_decide" },
    );
  }
}
