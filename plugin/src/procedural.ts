/**
 * Brain Server — procedural memory surface (v0.4.0).
 *
 * Wraps brain-server's v1.10.0 "Procedural" endpoints: runbooks/playbooks with
 * ordered steps and deterministic decision-tree evaluation. Kept in its own
 * module (rather than brain-client.ts / tools.ts) because it is a distinct
 * memory domain and a distinct feature set — and to keep both of those files
 * under the oxlint max-lines budget.
 *
 * The client operations reuse {@link BrainClient.fetchJson} (the same transport
 * + typed error model) so there is one HTTP path, not two. Every operation is
 * deterministic — no LLM, no tokens.
 */
import type { OpenClawPluginApi } from "openclaw/plugin-sdk/plugin-entry";
import { Type, type Static } from "typebox";
import { BrainClient, BrainHttpError, describeBrainError } from "./brain-client.js";
import type { ResolvedBrainConfig } from "./config.js";
import { sanitizeForBlock } from "./format.js";

type LiveCfg = () => ResolvedBrainConfig;

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/** One ordered step within a procedure; `memoryKind` is `step` or `decision`. */
export type BrainProcedureStep = {
  id: number;
  stepIndex: number;
  content: string;
  memoryKind: string;
  title?: string;
};

/** Input for one step when creating a procedure (`POST /procedure`). */
export type BrainProcedureStepInput = {
  title: string;
  content: string;
  isDecision?: boolean;
};

/** Result of creating a procedure: the root id + the ordered step ids. */
export type BrainProcedureCreateResult = {
  id: number;
  status: string;
  stepIds: number[];
};

/** Ordered step chain for a procedure (`GET /procedure/{id}/steps`). */
export type BrainProcedureStepsResult = {
  procedureId: number;
  title?: string;
  content?: string;
  steps: BrainProcedureStep[];
};

/** Outcome of evaluating a stored decision rule. Deterministic — no LLM. */
export type BrainDecisionOutcome = {
  result: string;
  usedDefault: boolean;
  matchedCondition?: string;
  citation?: number;
};

// ---------------------------------------------------------------------------
// Wire shapes (server serializes snake_case) + mapper
// ---------------------------------------------------------------------------

type StepViewWire = {
  step_index: number;
  id: number;
  title?: string;
  content: string;
  memory_kind: string;
};

type ProcedureStepsWire = {
  procedure_id: number;
  title?: string;
  content?: string;
  steps: StepViewWire[];
};

type DecisionOutcomeWire = {
  result: string;
  matched_condition?: string;
  citation?: number;
  used_default: boolean;
};

/**
 * Map a wire step row to the camelCase {@link BrainProcedureStep}. A named
 * mapper so the conditional optional-field spread is not inside the
 * `.map(stepWireToView)` call site (oxc/no-map-spread).
 */
function stepWireToView(s: StepViewWire): BrainProcedureStep {
  return {
    id: s.id,
    stepIndex: s.step_index,
    content: s.content,
    memoryKind: s.memory_kind,
    ...(s.title !== undefined ? { title: s.title } : {}),
  };
}

// ---------------------------------------------------------------------------
// Client operations (reuse BrainClient.fetchJson transport)
// ---------------------------------------------------------------------------

/**
 * Create a procedure + its ordered steps in one transaction (`POST /procedure`).
 * Each step becomes a `step`-kind chunk linked to the root via `next_step`
 * edges. Screened server-side; no proposal variant, so this is a direct write.
 */
export async function createProcedure(
  client: BrainClient,
  params: {
    title: string;
    content: string;
    steps: BrainProcedureStepInput[];
    domain?: string;
    timeoutMs?: number;
  },
): Promise<BrainProcedureCreateResult> {
  const body = {
    title: params.title,
    content: params.content,
    // is_decision defaults to false server-side; sending it explicitly is
    // equivalent and keeps the map spread-free.
    steps: params.steps.map((s) => ({
      title: s.title,
      content: s.content,
      is_decision: s.isDecision === true,
    })),
    ...(params.domain ? { domain: params.domain } : {}),
  };
  const res = await client.fetchJson<{ id?: number; status?: string; step_ids?: number[] }>(
    "/procedure",
    "POST",
    body,
    params.timeoutMs,
  );
  return { id: res?.id ?? 0, status: res?.status ?? "created", stepIds: res?.step_ids ?? [] };
}

/** Ordered step chain for a procedure (`GET /procedure/{id}/steps`); null on 404. */
export async function procedureSteps(
  client: BrainClient,
  id: number | string,
  timeoutMs?: number,
): Promise<BrainProcedureStepsResult | null> {
  try {
    const res = await client.fetchJson<ProcedureStepsWire>(
      `/procedure/${encodeURIComponent(String(id))}/steps`,
      "GET",
      undefined,
      timeoutMs,
    );
    if (!res) {
      return null;
    }
    return {
      procedureId: res.procedure_id,
      ...(res.title !== undefined ? { title: res.title } : {}),
      ...(res.content !== undefined ? { content: res.content } : {}),
      steps: (res.steps ?? []).map(stepWireToView),
    };
  } catch (err) {
    if (err instanceof BrainHttpError && err.status === 404) {
      return null;
    }
    throw err;
  }
}

/** Evaluate a stored decision rule (`POST /decision/{id}/evaluate`); null on 404. */
export async function evaluateDecision(
  client: BrainClient,
  params: { id: number; variables: Record<string, number>; timeoutMs?: number },
): Promise<BrainDecisionOutcome | null> {
  try {
    const res = await client.fetchJson<DecisionOutcomeWire>(
      `/decision/${encodeURIComponent(String(params.id))}/evaluate`,
      "POST",
      { variables: params.variables },
      params.timeoutMs,
    );
    if (!res) {
      return null;
    }
    return {
      result: res.result ?? "",
      usedDefault: res.used_default ?? true,
      ...(res.matched_condition !== undefined ? { matchedCondition: res.matched_condition } : {}),
      ...(res.citation !== undefined ? { citation: res.citation } : {}),
    };
  } catch (err) {
    if (err instanceof BrainHttpError && err.status === 404) {
      return null;
    }
    throw err;
  }
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

/**
 * Register the procedural-memory tools. All three are always registered for any
 * allowlisted agent; `memory_procedure_store` is a direct write (the server has
 * no proposal variant for procedures), gated instead by the server's Write
 * authz + injection screen and the plugin's per-agent `agents` allowlist.
 */
export function registerProceduralTools(
  api: OpenClawPluginApi,
  client: BrainClient,
  liveCfg: LiveCfg,
): void {
  const memoryProcedureGetParamsSchema = Type.Object({
    id: Type.Integer({
      description:
        "Procedure/runbook id (from a recall hit with memoryKind=procedure, or a prior memory_procedure_store).",
    }),
  });
  type MemoryProcedureGetParams = Partial<Static<typeof memoryProcedureGetParamsSchema>>;

  api.registerTool(
    {
      name: "memory_procedure_get",
      label: "Memory Procedure Get",
      description:
        "Fetch the ordered steps of a stored runbook/procedure. Use to follow a troubleshooting playbook or implementation guide step-by-step. Pair with memory_recall (memoryKind=procedure) to find a runbook first.",
      parameters: memoryProcedureGetParamsSchema,
      async execute(_toolCallId, params) {
        const c = liveCfg();
        const p = (params ?? {}) as MemoryProcedureGetParams;
        if (typeof p.id !== "number") {
          return {
            content: [{ type: "text" as const, text: "id is required." }],
            details: { found: false },
          };
        }
        try {
          const proc = await procedureSteps(client, p.id, c.requestTimeoutMs);
          if (!proc) {
            return {
              content: [{ type: "text" as const, text: `No procedure with id ${p.id}.` }],
              details: { found: false, id: p.id },
            };
          }
          const head = `Runbook #${proc.procedureId}${proc.title ? `: ${sanitizeForBlock(proc.title).trim()}` : ""}${proc.content ? `\n${sanitizeForBlock(proc.content)}` : ""}`;
          const body =
            proc.steps.length === 0
              ? "\n(no steps)"
              : proc.steps
                  .map(
                    (s) =>
                      `\n${s.stepIndex + 1}. [${s.memoryKind}] ${s.title ? `${sanitizeForBlock(s.title).trim()} — ` : ""}${sanitizeForBlock(s.content)}`,
                  )
                  .join("");
          return {
            content: [{ type: "text" as const, text: `${head}\nSteps:${body}` }],
            details: { found: true, procedureId: proc.procedureId, stepCount: proc.steps.length },
          };
        } catch (err) {
          return {
            content: [
              { type: "text" as const, text: `Procedure fetch failed: ${describeBrainError(err)}` },
            ],
            details: { found: false, error: describeBrainError(err) },
          };
        }
      },
    },
    { name: "memory_procedure_get" },
  );

  const memoryProcedureStoreParamsSchema = Type.Object({
    title: Type.String({ description: "Runbook title (e.g. 'New-hire laptop setup')." }),
    content: Type.String({ description: "Overview / when-to-use for the runbook." }),
    steps: Type.Array(
      Type.Object({
        title: Type.String({ description: "Step title." }),
        content: Type.String({
          description: "Step instructions (or decision-rule JSON if isDecision).",
        }),
        isDecision: Type.Optional(
          Type.Boolean({
            description:
              "Mark this step as a decision rule; evaluate later via memory_decision_evaluate.",
          }),
        ),
      }),
      { description: "Ordered steps. Server caps at 100." },
    ),
    domain: Type.Optional(Type.String()),
  });
  type MemoryProcedureStoreParams = Partial<Static<typeof memoryProcedureStoreParamsSchema>>;

  api.registerTool(
    {
      name: "memory_procedure_store",
      label: "Memory Procedure Store",
      description:
        "Create a runbook/procedure with ordered steps (knowledge base / troubleshooting playbook). Writes directly to memory (server-screened; no proposal review).",
      parameters: memoryProcedureStoreParamsSchema,
      async execute(_toolCallId, params) {
        const c = liveCfg();
        const p = (params ?? {}) as MemoryProcedureStoreParams;
        const title = (p.title ?? "").trim();
        const content = (p.content ?? "").trim();
        if (!title || !content) {
          return {
            content: [{ type: "text" as const, text: "Both title and content are required." }],
            details: { stored: false },
          };
        }
        const steps = Array.isArray(p.steps) ? p.steps : [];
        try {
          const res = await createProcedure(client, {
            title,
            content,
            // No conditional spread: isDecision defaults to false and the
            // server treats missing/false identically (#[serde(default)]).
            steps: steps.map((s) => ({
              title: (s.title ?? "").trim(),
              content: (s.content ?? "").trim(),
              isDecision: s.isDecision === true,
            })),
            ...(p.domain ? { domain: p.domain } : {}),
            timeoutMs: c.requestTimeoutMs,
          });
          return {
            content: [
              {
                type: "text" as const,
                text: `Created runbook #${res.id} with ${res.stepIds.length} step(s).`,
              },
            ],
            details: { stored: true, id: res.id, stepIds: res.stepIds },
          };
        } catch (err) {
          return {
            content: [
              { type: "text" as const, text: `Procedure store failed: ${describeBrainError(err)}` },
            ],
            details: { stored: false, error: describeBrainError(err) },
          };
        }
      },
    },
    { name: "memory_procedure_store" },
  );

  const memoryDecisionEvaluateParamsSchema = Type.Object({
    id: Type.Integer({
      description:
        "Decision-rule chunk id (a step stored with isDecision, or any decision-kind chunk).",
    }),
    variables: Type.Record(Type.String(), Type.Number(), {
      description:
        'Numeric input variables, e.g. {"employee_count": 25}. First matching branch wins.',
    }),
  });
  type MemoryDecisionEvaluateParams = Partial<Static<typeof memoryDecisionEvaluateParamsSchema>>;

  api.registerTool(
    {
      name: "memory_decision_evaluate",
      label: "Memory Decision Evaluate",
      description:
        "Deterministically evaluate a stored decision rule (no LLM): returns the branch matching the supplied numeric variables, or the default. Use for troubleshooting decision trees inside a runbook.",
      parameters: memoryDecisionEvaluateParamsSchema,
      async execute(_toolCallId, params) {
        const c = liveCfg();
        const p = (params ?? {}) as MemoryDecisionEvaluateParams;
        if (typeof p.id !== "number") {
          return {
            content: [{ type: "text" as const, text: "id is required." }],
            details: { evaluated: false },
          };
        }
        const variables =
          p.variables && typeof p.variables === "object"
            ? (p.variables as Record<string, number>)
            : {};
        try {
          const out = await evaluateDecision(client, {
            id: p.id,
            variables,
            timeoutMs: c.requestTimeoutMs,
          });
          if (!out) {
            return {
              content: [{ type: "text" as const, text: `No decision rule with id ${p.id}.` }],
              details: { evaluated: false, id: p.id },
            };
          }
          const text = out.usedDefault
            ? `Decision #${p.id}: ${out.result} (default — no branch matched).`
            : `Decision #${p.id}: ${out.result} (matched: ${out.matchedCondition ?? "?"}).`;
          return {
            content: [{ type: "text" as const, text }],
            details: { evaluated: true, ...out },
          };
        } catch (err) {
          return {
            content: [
              {
                type: "text" as const,
                text: `Decision evaluate failed: ${describeBrainError(err)}`,
              },
            ],
            details: { evaluated: false, error: describeBrainError(err) },
          };
        }
      },
    },
    { name: "memory_decision_evaluate" },
  );
}
