# 21 · The Cloud-Agent Convergence (October 2026): platforms shipped the loop, the literature shipped the label

*The problem.* A widely circulated cloud-agent strategy video ([Ondrej,
October 2026](https://www.youtube.com/watch?v=ZHOmBeMNbnc)) forecast that agent
work moves to trigger-spawned loops in isolated cloud sandboxes with the human
promoted to reviewer and fleet manager. The failure mode for this project is
neither dismissing the forecast nor absorbing it as fact: it is treating every
claim as a question about primary sources, then checking what the adjacent
literature says about the layer *this repository actually ships* — the memory
substrate those loops would read and write.

*The method.* Every platform claim below was verified against the vendor's own
documentation or blog on **2026-10-12**; nothing rests on the video, whose
claims served only as the prompt. Ecosystem metrics were pulled live from the
GitHub API the same day. The regulatory dates were read from the implementation
timeline maintained at
[artificialintelligenceact.eu](https://artificialintelligenceact.eu/implementation-timeline/)
(page updated 2026-08-31). The five papers were surfaced by a targeted arXiv
query (`"agent memory" AND provenance`) on 2026-10-12 — three predate the
standing lanes' sweep window ([doc 20](./20-agent-memory-oct-2026-sweep.md):
2026-09-26 through 2026-10-09), and two fall inside it but arrived via the
targeted query, not the lanes. All five are preprints triaged at abstract
level; none is peer-reviewed. Verdict vocabulary follows doc 20.

## What the platforms shipped (verified 2026-10-12)

| Platform | Verified facts | Source |
|----------|----------------|--------|
| Anthropic — Claude Code on the web | Launched 2025-10-20 as a research preview; GA as **"Cloud sessions"** 2026-09-23 for Pro/Max/Team/Enterprise. Every task runs in an isolated sandbox with filesystem and network restrictions; git rides a secure proxy; domain allowlists (e.g. package managers) are operator-configured; tasks run in parallel. | [claude.com/blog/claude-code-on-the-web](https://claude.com/blog/claude-code-on-the-web) |
| Microsoft/GitHub — Copilot cloud agent | **Automations**: the agent runs "on a schedule or in response to events in a repository" — trigger-spawned work, shipped. Confidence-based approval: it "explains each change, rates its confidence, and holds lower-confidence changes for your review." Supports MCP and a centralized agent-management page. | [docs.github.com — coding agent](https://docs.github.com/en/copilot/concepts/agents/coding-agent) |
| OpenAI — Codex Cloud | Each task gets its own workspace that "can keep working while your computer is asleep"; network-access presets, allowed domains, network secrets, OIDC connections; an explicit **review-and-publish** step gates environments before tasks start; a human inspects results and commits or opens the PR. | [learn.chatgpt.com/docs/cloud](https://learn.chatgpt.com/docs/cloud) |

Three readings, all boundary confirmations for this project:

1. **The loop, the trigger, and the sandbox are harness-layer facts.** All
   three products are execution harnesses. The trigger layer (schedule,
   webhook, repo event) lives in the harness — for this deployment, OpenClaw's
   cron and webhooks — exactly where the R97-era boundary decision placed it.
   Nothing here argues for triggers inside the store.
2. **The human gate survived automation at every platform.** PR review,
   confidence-based holds, review-and-publish: the durable pattern is
   "automate the triage, gate the promotion." That is the same act this server
   enforces for memory — proposal-only agent writes, digest-bound approval,
   quorum — except the platforms gate *code* and provide nothing equivalent
   for *shared memory* across agents.
3. **The memory primitive is deliberately client-side.** Anthropic's memory
   tool (`memory_20250818`) issues six commands (`view`, `create`,
   `str_replace`, `insert`, `delete`, `rename`) against a `/memories` prefix
   that *the application* maps onto storage it controls — the platform ships
   the verb, the deployment owns the store, including the path-traversal duty.
   That is the structural slot this repository occupies: the governed store
   behind the harness's memory calls, via the MCP bridge and UMP.

## The literature: five provenance papers, one month (all preprints)

- **[Audience-Bound Persistent Memory: Authorization Across the Memory
  Lifecycle](https://arxiv.org/abs/2609.36373)** (Sibo Liu, 2026-09-28) —
  **VALIDATES the taint-carriage law; one watch item.** Each memory item
  carries the audience present when it was recorded, and derived items inherit
  the **intersection of their sources' audiences** — the authorization-across-derivation
  problem R116 shipped the enforcement half of (revise inherits the superseded
  row's origin; a peer can label its own traffic but never assert owner-grade
  provenance). The refinement brain-server lacks: intersection semantics over
  multi-source derivations. The shipped posture is deliberately one boolean
  capture label, no lattice (R116 ponytail); this paper is the first serious
  argument for refining it, and is the designated re-triage trigger for that
  refusal at the next monthly sweep. Mapping note: the server's
  `(owner, access_scope)` pair is already the audience fact; what the paper
  adds is set algebra over derivation, not a new column.
- **[Memory as Middleware for Self-Improving AI
  Agents](https://arxiv.org/abs/2609.32091)** (Jayaram, Isahagian et al.,
  2026-09-25) — **VALIDATES the architectural position; machinery unevaluated.**
  Agents are "operationally amnesiac"; the dominant fix — bespoke memory
  hand-wired into one agent — is the thing to unbuild, and the proposal is a
  shared middleware layer. That is this repository's shape (one governed store
  behind plugin, bridge, and UMP) argued independently by IBM authors.
  Abstract-level triage only: the middleware's internals were not read, and
  under the no-LLM law any learned component in the write or recall path is
  refused by default. The framing is adopted; nothing else is.
- **[Stashbird: Efficient Speaker-Indexed Memory for Conversational
  Agents](https://arxiv.org/abs/2609.34242)** (Biringa, Yannul et al.,
  2026-09-28) — **VALIDATES provenance-linked derivation.** Links source
  episodes to derived memory state "through explicit provenance," with
  episodic records and semantic relations, indexed by speaker. The
  speaker-indexing is the per-principal attribution this server enforces on
  every row (`owner`, origin labels, per-edge capture markers); the
  source-to-derived links are the `derived_from` evidence graph the DSAR
  purge and `dsar_locate` already walk.
- **[EnSIMem: Entity-Structured Indexing for Long-Term Agent
  Memory](https://arxiv.org/abs/2609.27279)** (Meng, Fan et al., 2026-09-23) —
  **VALIDATES the entity/evidence tier.** Its diagnosed failure modes —
  generic summaries and anonymous chunks that lose the entity–property–evidence
  binding — are the failure modes the deterministic linker + entity/relations
  graph exist against. The machinery (its indexing model) is not evaluated
  here; the problem statement is the validation.
- **[EPGM: Execution Provenance for Budgeted Agent Memory
  Retrieval](https://arxiv.org/abs/2609.25913)** (Wang, Ju et al., 2026-09-22)
  — **EVAL-FIXTURE candidate.** Fixed-k retrieval rewards fragments; answers
  need the *complete* evidence set under a hard token budget. That is the gap
  submodular evidence packing encodes (objective = relevance + coverage +
  representativeness, checked by `answer_in_context` rather than document
  recall). Recorded hook: an evidence-completeness metric in the paper's
  style could extend `bench eval` beside precision/recall/NDCG — an operator
  decision, deterministic and hand-checkable. Its learned retriever is refused
  per the standing law.

## The ecosystem, measured (GitHub API, 2026-10-12)

| Repository | Stars | Created | Note |
|------------|-------|---------|------|
| [mem0ai/mem0](https://github.com/mem0ai/mem0) | 67,005 | 2023-06-20 | "The Memory Layer for AI Agents" |
| [modelcontextprotocol/servers](https://github.com/modelcontextprotocol/servers) | 91,151 | 2024-11-19 | The MCP reference servers |
| [topoteretes/cognee](https://github.com/topoteretes/cognee) | 32,042 | 2023-08-16 | Pushed 2026-10-12 (the day of this check) |
| [getzep/graphiti](https://github.com/getzep/graphiti) | 31,654 | 2024-08-08 | The bi-temporal KG model ([doc 01](./01-bi-temporal.md)) |
| [letta-ai/letta](https://github.com/letta-ai/letta) | 25,108 | 2023-10-11 | Formerly MemGPT |

"Memory layer" is now a category with multiple funded contenders, not a niche.
The differentiation axis is therefore not *having* memory but what the store
can prove: who wrote each row, whether a human gated the promotion, whether
erasure is certifiable, whether the audit trail survives an attacker with the
database file — the axis this repository has spent its last twenty rounds on,
and the axis the platform-native stores (ChatGPT memory, Claude `/memories`,
Codex environments) structurally do not carry across agents and vendors.

## The regulatory clock (verified against the implementation timeline)

The remainder of the [EU AI
Act](https://artificialintelligenceact.eu/implementation-timeline/) began
applying **2026-08-02** ("unless specified otherwise", Art. 113), with Arts.
102–110 from 2026-07-27. Still ahead: synthetic-content transparency (Art.
50(2)) and the Art. 5 additions pushed to **2026-12-02**; GPAI grandfathering
**2027-08-02**; high-risk obligations with record-keeping duties (Art. 12,
Chapter III) **2027-12-02** (Annex III) and **2028-08-02** (Annex I). GDPR
duties (DSAR, erasure) apply regardless. The audit chain with genesis/head
pins, the DSAR bundle with certified erasure, and the gate-law register are
ahead of that clock; [the Standards
Register](./19-standards-register.md) carries the framework rows.

## What this changes: nothing mechanical, on purpose

No component moves, no knob appears, no trigger lands in the store. The
convergence's yield is positioning evidence and two recorded candidates, both
operator decisions rather than paper-driven integrations: (1) the
audience-intersection re-triage of the boolean-label refusal, owned by the
monthly sweep ([doc 20](./20-agent-memory-oct-2026-sweep.md) method); (2) an
ops-surface rollup of per-principal proposal outcomes, quarantine hits, and
UMP feedback — the "evals for your fleet" job the platforms are teaching
operators to want, computable from tables this server already writes.

*The ceiling.* Star counts are adoption proxies, not benchmarks. Platform
documentation states marketing postures that can change without notice; the
verified facts are dated 2026-10-12 and are snapshots, not contracts. All five
papers are preprints assessed at abstract level — full texts were not read,
and abstracts can overstate; verdicts are relevance triage against shipped
behavior, not replication. Regulatory dates are as published on a
third-party-maintained timeline (updated 2026-08-31), not primary-law text.
No affiliation with any listed work, vendor, or platform.
