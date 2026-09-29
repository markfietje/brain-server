# 02 — System Architecture (complete)

## 2.1 One picture

```
HUMAN (operator / clinician / auditor)
  │ answers · approves digest · reviews quarantine · verifies chain
  ▼
SIX LOOPS (the architecture — see §2.2, authoritative plan named there)
  Create ──▶ Solve ──▶ Evolve ──▶ Deflect ──▶ Operate
  Discover    Find      Review      Reuse      Watch
  Test        Ask       Publish     Measure    Learn
  Add         Evidence  Supersede   Flag       Improve
      ▲___________ feedback: Operate→Evolve, Operate→Create, Deflect→Create
  Deliver (software: Scope · Design · Verify · Release · Operate) ──▶ all five
  ▼
SHELL — health-bridge (Dioxus 0.7, one codebase → web/desktop/mobile)
  queue (Docs to review) · training due (Academy) · unit dash (Dash) · verify (Comply)
  Two RBAC skins, same kernel: patient/family vs operator
  │
MODULES (private, per-deployment EULA) — each = tenant domain + adapter + rerank profile
  health-docs · health-comply · health-dash · health-force · health-academy · health-bridge
  + 4 care adapters (school/dpc/home/facility) + 10 funded packs (rx/dme/lab/dental/mind/hospice/rpm/isnp/dsnp/levelfunded)
  │
SEAMS (shared/, versioned, tested)
  classify L0/L1/L2 → quarantine │ hybrid RRF k=60 → rerank top-50 │ store sqlite↔postgres │ FHIR R4 (+R5 shims) │ audit/DSAR/hold/decay
  │
KERNEL (public MIT brain-server 1.28.90 — DO NOT FORK)
  Axum 0.8.9 · Tokio 1.53 full · rusqlite 0.40.1 (SQLite 3.53.2 bundled) · sqlite-vec 0.1.9 · r2d2 0.8.10
  FastEmbed 6.0.2 + ort rc.13 (neural-embed/rerank-tier, OFF by default) · model2vec-rs 0.2 (static default)
  Keyed hash-chained audit (SHA-256 legacy / HMAC-SHA256 keyed) · domains · DSAR · legal hold · retention · recall/graph-PPR · MCP+REST
  Profile release: opt 2, lto fat, codegen 1, strip, panic abort, overflow-checks (Cargo.toml)
```

This extends `ARCHITECTURE_steward.md` Layer 1–4 (spine → reducers → harness → workflow substance) to health: the same spine, new same-shaped crates per §I factory rule (own closed schema, same spine, one at a time).

## 2.2 The six loops (authoritative: `plans/PLAN_SIX_LOOPS_FINAL_ARCHITECTURE.md`)

**Status of the source: AGREED — "this is the architecture" (2026-09-27).** This
section carries the loop architecture; the plan remains the normative source and
this summary must not diverge from it. `r45_0_blueprint_names_all_six_loops` in
the kernel fails if a loop is dropped from here.

**5 knowledge loops** (Create, Solve, Evolve, Deflect, Operate) and **1 software
loop** (Deliver). They are not six interchangeable peers.

Knowledge has a lifecycle; software has a lifecycle. They are not the same
lifecycle but share the same shape (`Create → Use → Improve → Deliver`).

| Loop | Lifecycle | What it does | Standards it maps to |
|---|---|---|---|
| **Create** | Knowledge | Discover new knowledge, test it, add it to the base | SECI, GRAI, AKI, CKLT, NIST AI RMF |
| **Solve** | Knowledge | Find answers with sources, ask experts when unsure | KCS v6/2027, ISO 30401 |
| **Evolve** | Knowledge | Review, approve, and publish knowledge | KCS v6/2027, ISO 30401 |
| **Deflect** | Knowledge | Reuse what works, measure what sticks | KCS v6/2027, COPC 8.0, ISO 10002 |
| **Operate** | Knowledge | Watch outcomes, learn from results, improve | KCS v6/2027, ISO 30401, DORA |
| **Deliver** | Software | Build, test, release, and track software | SLSA v1.2, in-toto, ISO 29110, DORA, ITIL 4 |

**Flow and feedback.** The five knowledge loops run in order —
`Create → Solve → Evolve → Deflect → Operate` — and the feedback edges are what
make them a system rather than a list: **Operate → Evolve** (outcomes feed back
into publication), **Operate → Create** (knowledge gaps trigger new creation),
**Deflect → Create** (high repeat rates trigger proposals), and **Deliver → all
five** (a new software version improves every loop).

**Why each exists.** KCS v6 defined Solve and Evolve, then added Deflect.
Deliver was added because knowledge needs software to be useful and software
needs the same governance. Operate was added because knowledge needs feedback
and outcomes must be attributable to specific knowledge. Create was added
because knowledge does not simply appear — it is discovered, tested, and
validated, which is distinct from managing what already exists.

**Where the loops meet this kernel.** Create lands through the screened write
path (`POST /ingest|/add`, quarantine-or-index in TX + audit row); Solve is
`POST /recall` (vector + BM25 → RRF k=60 → rerank, with provenance); Evolve is
the human promotion gate; Deflect is measured from the reuse and deflection
metrics; Operate is the feedback edge into Evolve; Deliver is the software
lifecycle the kernel itself is built under. The gate that Create and Evolve
share — every write reviewed by a human before it is memory — is the same gate
this round measured in `evals/R45_0_FPR_RESULTS_2026-09-28.md`.

## 2.3 Kernel surfaces (what modules call — no new kernel code)

* `POST /ingest|/ingest/markdown|/add` → screened write (L0 always-on) → quarantine or index (vec0/pgvector + FTS5/tsvector + graph) in TX + audit row.
* `POST /recall` → vector cosine + BM25 → RRF k=60 → rerank (if profile) → hits with score + provenance + `rerank_score` + untrusted fences.
* `POST /verify` → byte-exact claim check against stored text.
* `GET /audit/verify` → chain verification. `POST /dsar` (dry-run + purge + certificates + tombstones). Legal hold → 409 `legal_hold_active`. Retention/decay sweeps.
* `GET /ready` (+ JSON status), `GET /health/db` (WAL pending gauge), metrics (recall p95, rerank_ms, lock-wait µs).
* MCP server + `brain` CLI (`ingest-dir`, `eval --floor`, `key generate`, `doctor`).

## 2.4 Determinism (the design spine)

* Same inputs → same outcomes: closed schemas, integer ten-thousandths scoring, CAS-guarded writes (`BEGIN IMMEDIATE`, `state_revision`), RRF deterministic merge, bounded graph adjacency, in-TX audit rows, ordered fan-outs only (Loom invariant: no cross-chunk reduction).
* Model is advisory only: local ONNX/SLM suggestions validated against deterministic rubric; never overwrite a score alone. No external AI API (DPIA win).
* Measured (BENCHMARKS.md): eval floor 106 queries r@5 0.976 / r@10 0.991 / MRR 0.956 / nDCG 0.962 (static edge); Loom byte-identity (vec index sha256 identical across postures); recall p95 24–25 ms dev vault 8.6k docs.

## 2.5 Profiles (capability gates)

| Profile | Embed | Rerank | Store | Where |
|---------|-------|--------|-------|-------|
| edge (default) | potion 512-d static (model2vec) | off | SQLite | PH tablet, Jetson, brownout box |
| desktop | gte-base 768-d (neural-embed) | bge-reranker-v2-m3 (rerank-tier) | SQLite | Kabankalan office |
| enterprise | BGE-M3 1024-d + sparse/colbert heads (v1.30+) | mxbai-large-v1 premium / bge fallback | Postgres 18 + pgvector | Florida cluster |

Features are opt-in (`neural-embed`, `rerank-tier`, `injection-classifier`, `loom`, `otel`, `migrate`, `bench`) so the default build is unchanged and Jetson-safe.

## 2.6 Ports + extension points

* `BRAIN_STORE=sqlite|postgres` (env, not feature). `BRAIN_RERANK_MODEL_DIR` absolute-path only. `BRAIN_RERANK_TOP_N` (default 50). `BRAIN_LOOM=1` + non-jetson + feature (fail-closed parse). `BRAIN_OTEL_ENDPOINT` (default 127.0.0.1:4318). `BRAIN_SSE_REAUTH_SECS` (default 30, =0 restores admission-only, pinned).
* Engines compile against `brain-engine-sdk` (stable ABI), never against the server. New health crates follow the same rule.
