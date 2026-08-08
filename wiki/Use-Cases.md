# Use Cases

Brain Server is built for the edge — private, offline, deterministic, and free to run. Here are the concrete scenarios it's designed for, with a worked example for each.

## 1. An agent with memory that costs nothing to recall

**The problem.** Every turn of your agent, you want it to remember what it learned. Cloud memory services charge per read/write — an LLM or embedding API on every recall.

**The fix.** Brain Server uses a **static, local embedding model** and a deterministic pipeline. Recall is **0 decision tokens, 0 embedding tokens**. The agent calls `/recall`, gets the evidence, and moves on. No per-query cost, no data egress, no network latency.

**Worked example** — an OpenClaw agent that remembers across turns:

```bash
# Ingest a fact
curl -X POST http://localhost:8765/ingest/markdown \
  -d '{"title":"Client","content":"Acme Corp prefers [[uses::bignay}}."}'

# Recall it on a later turn
curl -X POST http://localhost:8765/recall -d '{"query":"what does acme prefer"}'
```

See the **[OpenClaw Integration](OpenClaw-Integration)** page for the plugin wiring.

## 2. A private health or business journal with point-in-time recall

**The problem.** You keep notes on health, business, or code — but notes that change over time are misleading. "Which medicine was I on in March?" needs temporal answers.

**The fix.** Every ingest stamps `observed_at` / `valid_from` / `valid_to`. Recall with `?at=<past>` returns the fact **as it was then**. Superseded facts are expired, not deleted.

```bash
curl -X POST http://localhost:8765/recall \
  -d '{"query":"current medication","at":"2025-03-01"}'
```

## 3. A domain-graphed memory that never leaks across topics

**The problem.** You keep health, business, and code notes in one place. You don't want a work question answered with a health fact.

**The fix.** Memories live in scoped **domains**, each with its own knowledge graph. Retrieval **auto-routes** by per-domain centroids and falls back across domains only on a miss — so one domain's memory never leaks into another's answers.

## 4. An agent that knows when it doesn't know

**The problem.** An agent that confidently returns a wrong memory is worse than one that says "I don't know."

**The fix.** **Calibrated abstention**: when retrieval quality is too low, `/recall` returns `{decision: "low_confidence", hits: []}` instead of top-1 garbage. `POST /verify` can double-check that a claim is literally supported by a chunk's text.

## 5. A memory that stays honest with human approval

**The problem.** Agents writing their own memories can inject noise or contradictions.

**The fix.** **Write-back gating**: `POST /ingest/proposal` scores a candidate but creates **no memory row**. It becomes memory only via human approval (`/proposals/{id}/approve`). Combined with **reviewable proposals** (duplicates, conflicts, stale sources, near-duplicates) and **prompt-injection quarantine**, the memory stays clean.

## 6. A compliant, auditable memory store

**The problem.** You need to answer "what did the system recall, and why?" — and honor erasure requests.

**The fix.** The **append-only SHA-256 audit chain** proves nothing was tampered with. **Recall traces** replay exactly what informed a retrieval. The **DSAR workflow** locates, exports, purges, and issues a chain-verifiable deletion certificate. See **[Governance & Compliance](Governance-and-Compliance)**.

## 7. An edge deployment that draws under 5 watts

**The problem.** You want memory on a Jetson Nano or Raspberry Pi, not in the cloud.

**The fix.** One Rust binary, embedded SQLite + sqlite-vec, int8-quantized vectors, ≤350 MB RSS on 4 GB ARM. No GPU, no embedding API, no Docker stack. Set `BRAIN_WORKER_THREADS=2` to trim RSS further.

## Next steps

- **[Quickstart](Quickstart)** — get running.
- **[OpenClaw Integration](OpenClaw-Integration)** — wire it into an agent.
- **[Features](Features)** — the full capability list.
