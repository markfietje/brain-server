# Brain Server — Who it's for (target audiences)

> **Meta description:** Brain Server is a local-first, offline, deterministic
> semantic-memory and knowledge-graph server for AI agents. Zero token cost,
> human-gated writes, GDPR/DSAR erasure, SHA-256 audit, and the current MCP
> 2026-07-28 stateless protocol — all in one self-hosted Rust binary.

Brain Server is a **local-first, offline, deterministic semantic-memory and
knowledge-graph server for AI agents**. This page maps the product's **shipped**
capabilities to the **concrete people and teams** who use them, so you can tell
at a glance whether it fits your job — and exactly what you'd get.

Every claim below is **reverse-checked against the current source** (v1.27.22): a
"Shipped" row names a real route, role preset, or test that exists in this
repository today. "Planned" means a documented roadmap ceiling. Nothing here is a
promise dressed as a feature — the honest ceiling is stated plainly at the end,
and so is the honest "when **not** to choose it."

---

## In one minute — is this you?

Answer these to self-select before reading the tables:

- **You build or run an AI agent and need it to remember** — you want
  conversation history, decisions, runbooks, and customer context recalled
  deterministically, not hallucinated. → **§3 AI / agent builders.**
- **You run customer support or a helpdesk** and want "how did we resolve this
  before?" answered from a grounded memory your team can review. → **§1 support
  & contact-center.**
- **You're in a regulated industry** (finance, healthcare, legal, government)
  where memory must stay in-house, be auditable, and be erasable on request. →
  **§2 enterprise & regulated.**
- **You deploy on thin or air-gapped hardware** (Jetson, Raspberry Pi, field
  ops) with no cloud dependency. → **§4 edge & field.**
- **You're an individual who wants a private second brain** that does temporal,
  point-in-time recall. → **§5 knowledge workers.**
- **You're an SI/MSP/consultant** standing up auditable memory layers for
  clients. → **§6 ecosystem & delivery partners.**

---

## The honest frame first (read this before the tables)

The shipped product is a **single-node, loopback-first memory server**. Today it
has **per-domain isolation**, per-tenant **audit**, **DSAR** (data-subject access
requests), **PII redaction**, a **human write-gate**, and **the current MCP
2026-07-28 stateless protocol**. What it does **not** have yet is **multi-team
tenancy** — running several client accounts as isolated tenants on one shared
backend. That is the documented **v2.0 "Cortex"** milestone (call-center
intelligence), so the *BPO* and *multi-client contact-center* rows below are the
roadmap the product is building toward, not its current single-node form.

In plain terms:

- **What it is:** your own private memory server for an AI agent — no cloud, no
  embedding API fees, no telemetry. One Rust binary (v1.27.22) + one SQLite file.
- **What it costs to run:** local static embeddings (model2vec), so recall costs
  **zero embedding tokens and zero decision tokens**; fits a Jetson/Raspberry Pi.
- **What it gives an agent:** deterministic hybrid recall (vector + full-text +
  graph), a knowledge graph, temporal evidence, and an audit trail — **without an
  LLM in the loop** making retrieval or redaction decisions.
- **What it enforces:** human-gated writes (proposals), prompt-injection
  quarantine, PII redaction on read, GDPR/DSAR erasure with certificates, and a
  SHA-256 hash-chained audit log.
- **The one big gap:** shared multi-tenant packaging. If you need several client
  accounts on one backend as hard-isolated tenants, that's v2.0. Until then each
  tenant gets its own domain on its own node.

---

## Shipped, in numbers (all source-checked)

| Capability | The real number |
|---|---|
| Self-contained deployment | **1 binary** + **1 SQLite file** (WAL), single process |
| Memory cost per recall | **0 embedding tokens, 0 decision tokens** (local static `model2vec`) |
| Retrieval quality gate | **r@5 = r@10 = 0.919, MRR 0.905, nDCG@10 0.909** on the frozen 37-query / 10-doc smoke set; CI pins floors r5/r10/mrr **≥ 0.85** |
| Audit integrity | **SHA-256 hash chain**, verifiable end-to-end via `/audit/verify` |
| Agent protocol | **UMP 1.0 conformance: L3** (13/13 checks), **MCP 2026-07-28 stateless**, OpenAPI |
| Human write-gate | Proposals: novelty/conflict/salience scored, **approved or rejected by a human** |
| Erasure | **DSAR** locate → export → purge → chain-verifiable certificate |
| Domain isolation | Per-domain graphs + auto-routing; registration capped at **256 domain DBs** |

> **Honest calibration on the numbers.** The retrieval figures above are a
> **directional signal on a small frozen smoke set**, not a large benchmark —
> the repo itself says so. They prove the recall pipeline is deterministic and
> gated; they do **not** claim a production-quality corpus score. Expand to
> ≥100 judged queries before treating any recall number as a floor for your
> workload.

---

## 1. Customer-support & contact-center operations

The v2.0 "Cortex" milestone is explicitly **call-center intelligence**. The
*controls* those teams need are largely **shipped today** (isolation, audit,
DSAR, PII, human-gated writes); the *shared-tenant packaging* is the planned part.

| Who you are | What you need | What Brain Server gives you | Status |
|---|---|---|---|
| **BPO (Business Process Outsourcer)** | Serve multiple client accounts with hard isolation; per-client agent-assist memory; per-client audit + DSAR; PII containment | Per-domain isolation, per-tenant audit chain, DSAR + deletion certificates, PII redaction, human write-gate | Controls **shipped**; **multi-client tenancy = v2.0 Cortex** (planned) |
| **In-house contact / call center** | One org, many teams; agent memory that recalls past resolutions, policies, customer context; supervision + audit | Deterministic recall, knowledge graphs, temporal evidence, HITL write gate, audit chain, reviewer-calibration strip | **Shipped** (single-org form); multi-team packaging in v2.0 |
| **Customer-support team / helpdesk** | Faster, grounded answers; "how did we resolve this before?"; no fabricated answers | Calibrated abstention, span verification (`/verify`), recall traces, resolution knowledge graph | **Shipped** |
| **Managed-service / shared-services support** | Standardized knowledge across internal teams with per-team scope | Domains + centroid routing, per-agent opt-in, chat-type gating | **Shipped** |

**Try it (10 minutes, single node):** `brain-server` + `brain ingest-dir` a
handful of past resolutions, then `brain recall "how did we fix the onboarding
issue"` and `brain get <id>` to pull the source chunk. Approve a captured fact
through the proposal queue to see the human write-gate in action.

## 2. Enterprise & regulated industries (sovereignty)

Brain Server is **self-hosted, offline-capable, and audited**, so it fits
organizations for whom memory must stay in-house and be provable.

| Who you are | What you need | What Brain Server gives you | Status |
|---|---|---|---|
| **Financial services** | PII containment, immutable audit, DSAR (GDPR/CCPA), no data egress | SHA-256 audit chain, read-time PII redaction, DSAR/certificates, loopback-only default | **Shipped** |
| **Healthcare & clinical** | Local records, on-prem, explainable recall, erasure | Local-first, `/verify` span check, DSAR, `/.well-known/ai-notice` | **Shipped** |
| **Legal & compliance** | Tamper-evident logs, Art 22 explainability, Art 50 origin | Hash chain + `/audit/verify`, replayable recall traces, origin metadata | **Shipped** |
| **Government / public sector** | Air-gapped or on-prem, procurement-grade evidence | Single binary, no telemetry, `RFP_RESPONSE_KIT.md`, threat model | **Shipped** |
| **Any regulated enterprise** | SOC 2 / ISO 42001 evidence base | Documented posture + evidence kit (`COMPLIANCE.md`) | **Shipped** (posture, not certification) |

**Try it:** run `/audit/verify` (returns `{ok: true}` if the chain is intact) and
run a DSAR **dry-run** (`POST /dsar {"dry_run": true}`) to see the locate/export
footprint with zero erasure. Both are live, audited endpoints.

## 3. AI / agent builders & platforms

The current primary audience — teams and individuals building agents that need
memory.

| Who you are | What you need | What Brain Server gives you | Status |
|---|---|---|---|
| **OpenClaw users** | Deterministic memory in the memory slot, zero token cost | Native `kind: "memory"` plugin (autoRecall / autoCapture / Proposal), plugin 0.4.7 | **Shipped** |
| **Agent / LLM developers** | A self-hosted memory store with standard contracts | Open HTTP API, **MCP** binary, OpenAPI, **UMP 1.0 L3** | **Shipped** |
| **MCP-adopting teams (2026)** | A memory backend that speaks the **current stateless MCP** | The `mcp` binary implements **MCP 2026-07-28**: stateless, `server/discover`, per-request `_meta`, `ttlMs`/`cacheScope` — no `initialize` handshake | **Shipped** |
| **Edge / privacy-first agent builders** | Memory on-device, no embedding API | Local static `model2vec`, offline, bounded RSS (default 512 MiB) | **Shipped** |
| **Agent platforms & ISVs** | A memory backend to embed without lock-in | Standard-based (UMP, MCP, open HTTP), self-hostable | **Shipped** |

**Try it:** `brain recall "…"` from the CLI, or point any MCP-capable host at the
`mcp` binary (it implements the 2026-07-28 stateless spec out of the box). See
[`docs/mcp.md`](./mcp.md) for the exact install + a working request.

## 4. Edge, field & hardware deployments

| Who you are | What you need | What Brain Server gives you | Status |
|---|---|---|---|
| **Retail / logistics field ops** | Offline memory on thin hardware | Single binary, low power, Jetson / Raspberry Pi | **Shipped** |
| **Industrial / remote / air-gapped sites** | No cloud dependency, deterministic | Local static embeddings, no data egress | **Shipped** |

## 5. Knowledge workers & individuals

| Who you are | What you need | What Brain Server gives you | Status |
|---|---|---|---|
| **Personal-knowledge (PKM) users** | A private second brain, temporal recall | Domains (health/business/code), point-in-time recall | **Shipped** |
| **Researchers & academics** | A reproducible memory/RAG substrate | Open source, benchmark harness, frozen judged corpus | **Shipped** |

## 6. Ecosystem & delivery partners

| Who you are | What you need | What Brain Server gives you | Status |
|---|---|---|---|
| **SIs / MSPs / consultants** | A deployable, auditable memory layer to stand up for clients | One binary, edge-ready, documented deployment + DSAR drills | **Shipped** |
| **Platform / tooling vendors** | An embeddable, standard memory contract | UMP 1.0 L3, MCP 2026-07-28, OpenAPI | **Shipped** |

---

## Why it's genuinely useful (the practical cases)

Beyond the tables, here is what Brain Server does that most "agent memory"
solutions don't — in terms a buyer can hand to a decision-maker:

- **Zero-cost recall.** Because embeddings are local/static and the recall
  decision is made in code (not by an LLM), every memory read costs **no
  embedding tokens and no decision tokens**. In an agent that recalls every
  turn, that's the difference between a memory feature you can afford to leave
  on and one you disable to save money.
- **No fabricated answers.** When retrieval quality is too low to support a
  claim, `/recall` returns `{decision: "low_confidence", hits: []}` instead of
  top-1 garbage. `/verify` does deterministic span checking — is a claim
  *literally* in a stored chunk? No LLM guessing.
- **Memory that can't leak instructions.** Every recalled block is wrapped in an
  untrusted sentinel fence; the invisible-Unicode/bidi smuggling set and
  markdown references are stripped on every read seam. A malicious stored chunk
  cannot smuggle a "system:" injection or exfiltrate context to the model.
- **Memory your reviewer can trust.** Writes go through a human-gated proposal
  queue by default; a reviewer sees novelty, conflict, salience, a PII-safe
  digest, and a calibration strip — not a rubber stamp.
- **Memory you can prove.** The audit log is a SHA-256 hash chain
  (`/audit/verify` returns `{ok: true}`), recall traces are replayable, DSAR
  produces deletion certificates. "Show me" replaces "trust me."
- **Speaks the 2026 standard.** The MCP server implements the **stateless
  MCP 2026-07-28** spec — `server/discover` instead of `initialize`, per-request
  `_meta`, `ttlMs`/`cacheScope` caching. It's ready for the current generation of
  MCP hosts out of the box.

---

## When **not** to choose it (the honest other side)

Being direct saves everyone a wasted proof-of-concept:

- **You need shared multi-tenant SaaS** — several customer accounts on one
  hosted backend with per-tenant limits and billing. Brain Server is single-node
  and per-tenant-isolation is **per-domain on separate nodes** until v2.0.
- **You want a hosted, managed memory API** with no ops. This is self-hosted; you
  run the binary and the SQLite file.
- **You need semantic quality on a huge corpus today.** The shipped recall
  figures are validated on a **small smoke set** — a production-sized judged
  corpus is a roadmap item, not a current guarantee.
- **You want the model to judge relevance or summarize.** Brain Server is
  deliberately deterministic — no LLM in the retrieval or redaction path. If you
  want learned re-ranking, that's a different design.
- **You require an SOC 2 / ISO 42001 attestation certificate.** The repo ships a
  documented engineering posture, not an org-level certification.

---

## Frequently asked questions

**Is Brain Server free / self-hosted?**
Open source, MIT-licensed, self-hosted. One Rust binary + one SQLite file; no
cloud dependency and no telemetry.

**Does using it cost tokens?**
No. Embeddings are local/static (`model2vec`) and retrieval/redaction decisions
are deterministic code — recall costs **zero embedding and decision tokens**. The
only context cost is the capped snippets injected into a turn.

**How does it stop an agent from fabricating answers?**
`/recall` returns `{decision: "low_confidence", hits: []}` when retrieval quality
is too low, and `/verify` does deterministic span verification (is the claim
literally in a stored chunk?).

**How do I make sure my data can be erased on request?**
`POST /dsar` runs locate → export → purge and issues a chain-verifiable deletion
certificate. A `dry_run` shows the footprint without erasing anything.

**What MCP standard does it speak?**
The `mcp` binary implements the **current MCP 2026-07-28 stateless** spec:
`server/discover`, per-request `_meta`, `ttlMs`/`cacheScope`, no `initialize`
handshake. It also speaks **UMP 1.0 (L3 conformance, 13/13 checks)** and plain
OpenAPI over HTTP.

**Is it multi-tenant?**
Not yet. Per-domain isolation is shipped; **multi-team tenancy is the v2.0
"Cortex" roadmap** milestone.

---

## The honest ceiling (state this in any pitch)

- **Multi-client / multi-team tenancy is v2.0 "Cortex", not today.** A BPO
  running several client accounts as isolated tenants on one shared backend gets
  the *controls* (isolation, audit, DSAR, PII) shipped now, but the shared-tenant
  packaging and per-tenant limits are the documented v2.0/v2.1 roadmap. Until
  then, per-client isolation is per-domain on separate nodes.
- **Not a certification.** SOC 2 / ISO 42001 attestation are organization-level
  audits outside this repo; `COMPLIANCE.md` is a documented engineering posture.
- **PII at rest is not encrypted** — full-disk encryption is the operator's
  layer (LUKS/FileVault).
- **Deterministic, not learned** — recall and redaction are heuristic /
  deterministic, not model-inference.

---

## Next steps

- **[Overview](./overview.md)** — what it is and the five differentiators.
- **[Use cases](./use-cases.md)** — worked technical scenarios.
- **[RFP Response Kit](./RFP_RESPONSE_KIT.md)** — evidence-backed answers for procurement.
- **[Media kit](./media-kit.md)** — positioning + one-liners for press/marketing.
- **[Human in the loop](./human-in-the-loop.md)** — the operator's field manual,
  incl. **§7 the erasure procedure** (the documented, audited path a BPO/QA/Admin
  follows to delete memory).
- **[MCP](./mcp.md)** — the current stateless MCP server + install.
- **[OpenClaw integration](./openclaw-integration.md)** — the plugin (0.4.7) and
  its token-resolution ladder.
- **[Roadmap](./roadmap.md)** — the v2.0 "Cortex" trajectory this map points at.
- **[BENCHMARKS](./BENCHMARKS.md)** — the recall numbers behind the "in numbers"
  table, with their honest calibration caveats.