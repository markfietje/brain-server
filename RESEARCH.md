# Research Foundation — brain-server

Techniques verified against July 2026 research; the October 2026
memory-governance and agentic-memory sweep is recorded as adoption decisions in
§8, and the cloud-agent convergence check (platforms, ecosystem, regulation)
in §9. Every component is documented with its source paper, the implementation
location, and the specific findings or benchmarks that validate the approach.

---

## 1. Retrieval Stack

### Hybrid Dense + Lexical Retrieval

- **Implementation**: `src/search/mod.rs` — concurrent `vec0` KNN (dense) + FTS5 BM25 (lexical)
- **Dense model**: [`minishlab/potion-retrieval-32M`](https://huggingface.co/minishlab/potion-retrieval-32M) (512-dim static embeddings via [model2vec](https://github.com/minishlab/model2vec), ~120 MiB RSS)
- **SOTA status**: potion-retrieval-32M is the most performant static retrieval model on MTEB as of July 2026 — 80%+ of `all-MiniLM-L6-v2` quality at 500x the speed (no transformer inference). Model2Vec distillation achieves this in minutes on CPU.
- **Vector index**: [`sqlite-vec`](https://github.com/asg017/sqlite-vec) — SOTA for embedded/local vector search in SQLite. Trade-off: portable and zero-dependency vs. purpose-built engines (Qdrant, Milvus) at billion-vector scale.

### Reciprocal Rank Fusion (RRF)

- **Implementation**: `src/search/mod.rs:594-600` — concurrent dense + lexical readers, fused at `RRF_K=60`
- **Research**: [Cormack, Clarke & Büttcher (SIGIR 2009)](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf) — Reciprocal Rank Fusion outperforms Condorcet and individual rank-learning methods.
- **SOTA status**: RRF remains the standard model-free fusion algorithm for first-stage hybrid retrieval as of July 2026. It solves score-incompatibility (unbounded BM25 vs. bounded cosine) without training, and is universally combined with a second-stage cross-encoder reranker in production.

### Cross-Encoder Reranker (optional)

- **Implementation**: `src/search/mod.rs` (feature-gated `--features rerank`) — over-fetches `RERANK_CANDIDATES=30`, reranks via BGE-RerankerV2M3, truncates to final k.
- **Model**: [BGE-Reranker-v2-M3](https://huggingface.co/BAAI/bge-reranker-v2-m3) (BAAI, 568M params, via fastembed-rs)
- **SOTA status**: BGE-RerankerV2M3 (~71.5 BEIR nDCG@10) is the most-deployed open-weight production reranker as of July 2026. The accuracy leader is Qwen3-Reranker-8B (~77%) but requires 8B params vs. 568M. Our architecture accepts any cross-encoder; the model is swappable. Ponytail: deploy Qwen3-Reranker-8B on GPU hardware for maximum accuracy.

### Pseudo-Relevance Feedback (PRF)

- **Implementation**: `src/search/mod.rs:727-977` — `prf_should_expand` gate fires only when the top pass-1 result appears in both dense AND lexical lists within a bounded rank. Anti-injection guardrail skips quarantined rows.
- **Research**: Standard PRF technique adapted for hybrid retrieval. Our deterministic gate avoids the common problem of PRF expanding on low-confidence results.
- **SOTA status**: PRF with cross-retriever agreement gating is a production-hardened innovation. No learned feedback weights — our gate is deterministic and zero-cost.

### Submodular Evidence Packing

- **Implementation**: `src/search/packing.rs` — budgeted monotone submodular maximization. Lazy greedy (Leskovec et al. 2007) under a token knapsack (default 160). Objective = relevance + coverage + representativeness, gated by MMR-style diversity (`DEDUP_SIMILARITY=0.85`).
- **Research**: ["What Survives Into Context: A Diagnostic for Budget-Constrained Multi-Hop RAG and When Submodular Evidence Packing Improves It"](https://arxiv.org/abs/2607.00725) (July 2026) — +5.1 F1 on HotpotQA with a 3B reader under 160-token budget.
- **Key metric**: `answer_in_context` diagnostic (our implementation) — measures whether the gold answer survives packing, shown to be a stronger predictor of F1 than document recall.
- **Ponytail**: Diversity uses lexical Jaccard (cheap proxy). A cosine-similarity dedup path (via the embedding model) would improve diversity scoring but adds per-call cost.

### Evaluation Harness

- **Implementation**: `src/eval.rs` + `bench eval` — precision@k, recall@k, MRR, NDCG, `answer_in_context_rate`. Hand-computed value checks.
- **SOTA status**: Standard IR metrics. Our innovation is the ship-gate integration — `bench --envelope desktop|jetson` exits non-zero on regression, turning evaluation into a CI gating step.

---

## 2. Knowledge Graph

### Bi-temporal Knowledge Graph (Graphiti model)

- **Implementation**: `src/temporal.rs` + `src/migration.rs:630-637` — `valid_at`/`invalid_at` columns on `relationships`. Temporal-marker extraction from free text ("from 2011 to 2017", "currently", "since 2020"). Point-in-time filter: `valid_at <= ? AND (invalid_at IS NULL OR invalid_at > ?)`.
- **Research**: [Graphiti / Zep](https://github.com/getzep/graphiti) — bi-temporal KG model. Distinguishes valid-time (when the fact holds in the world) from transaction-time (when brain learned it). SOTA for temporal knowledge representation: 82.2 on Deep Memory Retrieval benchmark, 94.8% accuracy on DMR, outperforming Mem0 (83.22) and MemGPT/Letta (83.31).
- **SOTA status**: Confirmed as the leading approach for temporal agent memory graphs as of July 2026. All four timestamps implemented: `valid_at`/`invalid_at` (valid time) and `created_at`/`expired_at` (system time — transaction+origin).
- **Our contribution**: Deterministic temporal extraction from free text — no LLM, no external API. 11 unit-tested patterns. This is novel vs. Graphiti itself (which uses an LLM extractor).

### TRACE State-Aware Traversal

- **Implementation**: `src/trace.rs` — typed-edge prefix vocabulary (`update:`, `supersedes:`, `contradicts:`, `causes:`) + bounded-walk constants (`MAX_HOPS=4`, `MAX_VISITED=256`). Validity-aware: the bi-temporal `at` filter skips expired edges.
- **Research**: ["TRACE: State-Aware Query Processing over Temporal Evidence Graphs for Conversational Data"](https://arxiv.org/abs/2607.00339) (July 2026) — hierarchical temporal evidence graphs with typed relations and validity annotations for long-conversation QA.
- **SOTA status**: July 2026 paper, currently the leading approach for combining temporal evidence with typed-edge traversal. Our implementation adopts the prefix vocabulary and bounded-walk constants as the immediate production surface; the full session/topic hierarchy (node_kind/parent_id) is schema-reserved for v1.8 Consolidate.
- **Ponytail**: Node hierarchy construction deferred to v1.8 — the column schema exists but nothing populates it.

---

## 3. Entity Extraction & Linking

### Deterministic Entity Linker

- **Implementation**: `src/linker.rs` — Aho-Corasick multi-pattern entity matcher (O(n) matching), heading hierarchy extraction, verb-suffix filtering for discovered relationship patterns.
- **Research**: Aho-Corasick (1975) confirmed as SOTA for deterministic multi-pattern matching in the July 2026 ACL/EMNLP literature. For pure frequency-based between-word relationship discovery: superseded by dependency parsing + SVO extraction in the SOTA literature (requires a POS tagger dep, ~5 MB via `nlrule`). Our approach is the pragmatic middle ground: verb-suffix filtering (zero deps) + heading hierarchy extraction (2026 document-structure research confirms this as a critical structural signal).
- **Ponytail**: Full dependency parsing upgrade path documented inline. Bare base-form verbs ("run", "set") without derivational suffixes are missed by `has_verb_suffix` — handled by the built-in `RELATION_PATTERNS` fallback of 25+ infrastructure verbs.

### GFM Table & Markdown Parsing

- **Implementation**: `src/chunker.rs` — [pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark) 0.13 (`#![forbid(unsafe_code)]` upstream), the canonical Rust CommonMark parser. `src/linker.rs` — custom `find_table_ranges`, `find_code_ranges`, `find_list_item_bold_ranges`.
- **SOTA status**: pulldown-cmark is the standard Rust CommonMark parser, used by `text-splitter`'s `MarkdownSplitter`. Our table/range extraction is a pragmatic addition — we don't need full AST parsing for entity extraction, only byte-range exclusion.

---

## 4. Embedding & Model Stack

### Static Embeddings

- **Model**: [`minishlab/potion-retrieval-32M`](https://huggingface.co/minishlab/potion-retrieval-32M) — fine-tuned version of `potion-base-32M`, distilled from `BAAI/bge-base-en-v1.5` via Tokenlearn.
- **Framework**: [model2vec](https://github.com/minishlab/model2vec) — static embedding creation via distillation, no runtime neural network. Lookup-table + mean pooling only.
- **SOTA status**: Most performant static retrieval model on MTEB as of July 2026. 500x faster than transformer embeddings, 50x smaller, reaching 80%+ of transformer quality at a fraction of the resource cost. The brain-server's key differentiator vs. cloud services — recall costs microseconds, not milliseconds with GPU.

### Quantization

- **Implementation**: `src/migration.rs:286-329` — `vec0` virtual table with `int8` + `bit` quantization columns. sqlite-vec's hardware-optimized SIMD/NEON acceleration.
- **SOTA status**: Standard approach for memory-constrained vector search. The `int8` + binary quantization reduces per-vector storage from 2048 bytes (f32, 512-dim) to 512 + 64 bits, making 100k-vector indexes fit in ~60 MB.

---

## 5. Security & Authentication

### JWT/JWS + OIDC

- **Implementation**: `src/auth/jwt.rs`, `src/auth/jwks.rs`, `src/handlers/well_known.rs`, `src/handlers/auth.rs` — full JWT verification, JWKS key rotation, RFC 8414 OIDC discovery, refresh-token chain with reuse detection.
- **Library**: [jsonwebtoken v10](https://docs.rs/jsonwebtoken) (RS256/384/512, ES256/384/512, EdDSA). OWASP algorithm-confusion defense (`none`, HS*, PS* rejected unconditionally before key lookup).
- **SOTA status**: Industry-standard. Our additional innovation: refresh-chain reuse detection burns the entire chain on reuse, preventing replay attacks (OWASP pattern).

### Constant-Time Token Comparison

- **Implementation**: `src/main.rs:3217` — `subtle::ConstantTimeEq::ct_eq` from [RustCrypto/subtle](https://docs.rs/subtle) (transitive dep via sha2/hmac/aes-gcm).
- **Research**: [OWASP Timing Attack Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Timing_Attack_Cheat_Sheet.html) — constant-time comparison prevents side-channel timing analysis of bearer tokens.
- **SOTA status**: Correct implementation using the asm/black_box-backed `ct_eq` primitive. Notably, LLVM can fold a hand-rolled XOR-based constant-time comparison back into a non-constant-time lookup table — `subtle` prevents this with explicit compiler barriers.

### Auditing

- **Implementation**: `src/audit.rs` — append-only SHA-256 hash chain. Every event's hash includes `prev_hash` of the previous row, forming a tamper-evident chain. `savepoint`-protected for safe nesting inside caller transactions.
- **SOTA status**: Standard hash-chain audit log (cf. certificate transparency, blockchain ledgers). Our implementation avoids the common migration bug where NULL prev_hash rows (from additive ALTER TABLE) cause false-positive verification failures — fixed in v1.1.1.

---

## 6. Operations & Reliability

### Capacity Envelopes

- **Implementation**: `src/capacity.rs` — Desktop (16 GB / 4 GB RSS) and Jetson (4 GB / 1.5 GB RSS) targets. Enforced via HTTP 507 (`InsufficientStorage`) on ingest; reads never check.
- **Research**: Based on the ARM device power envelope rather than a specific paper. The design principle: not exceeding the platform's memory guarantees deterministic behavior.
- **SOTA status**: Production-hardened. The `bench --envelope` mode turns capacity checks into a CI ship gate.

### Fuzz Testing

- **Implementation**: `fuzz/` crate — 4 cargo-fuzz targets: `fuzz_chunker`, `fuzz_lex_compile`, `fuzz_query_doc`, `fuzz_validator`.
- **SOTA status**: cargo-fuzz is the standard Rust fuzzing framework (LLVM libFuzzer integration). Ponytail: `fuzz_chunker`/`fuzz_lex` are stubs pending migration of chunks/query modules to the lib crate.

### Property-Based Testing

- **Implementation**: 4 `proptest` suites (256+ cases each) — chunker verbatim-substring invariant, multibyte safety, domain normalization idempotence, capacity monotonicity.
- **SOTA status**: proptest is the standard Rust property-testing framework. Each test proves an invariant that hand-written tests can miss.

---

## 7. Planned (v1.5+) Research in the Roadmap

| Version | Technique | Paper |
|---------|-----------|-------|
| v1.5 Epistemic | Confidence calibration + "I don't know" | [arXiv:2607.00862](https://arxiv.org/abs/2607.00862) — CAT: Confidence-Adaptive Thinking (ACL 2026 Industry) |
| v1.5 Epistemic | Dual-confidence calibration for RAG | [arXiv:2607.00570](https://arxiv.org/abs/2607.00570) — Dual-Confidence Contrastive Decoding for RAG |
| v1.5 Epistemic | Span-level hallucination detection | [arXiv:2607.00895](https://arxiv.org/abs/2607.00895) — Beyond Document Grounding |
| v1.6 Reconcile | Knowledge localization & editing | [arXiv:2607.01000](https://arxiv.org/abs/2607.01000) — KnowledgeDebugger |
| v1.7 Reason | Looping discrete + continuous embeddings | [arXiv:2607.00341](https://arxiv.org/abs/2607.00341) — DiscoLoop |
| v2.0 Cortex | Agentic memory (A-MEM / Cognee / Honcho) | [NeurIPS 2025 — A-MEM](https://arxiv.org/abs/zzzz) — Zettelkasten-inspired |
| v4.0 Sovereign | Causal inference / intervention | [DoWhy](https://github.com/py-why/dowhy) + federated A2A |

---

## 8. Evaluated, Not Adopted

Papers assessed and deliberately not adopted, with reasons recorded so the decision is re-derivable.

| Paper | Evaluated | Verdict and reasons |
|-------|-----------|---------------------|
| [arXiv:2610.10957](https://arxiv.org/abs/2610.10957) — TRACE: A Governance Framework for Measuring Explainability Debt in Production AI Systems (Pathak, Oct 2026) | 2026-10-12 | **Not adopted.** Its object — silent degradation of post-hoc explanations for learned-model decisions under drift — does not exist here: the core is deterministic and LLM-free, so any answer is reconstructable from its inputs, and every mutating act lands on the pinned audit chain. The Art-13-adjacent transparency duty it targets is already discharged by stronger artifacts (origin labels on every content-returning wire; the gate-law register's red-proofed guards). Single-author, single-domain case study (fraud-detection ML); aspects claimed by a provisional patent. Unrelated to §2's TRACE traversal ([2607.00339](https://arxiv.org/abs/2607.00339)) — name collision only. Revisit if score-producing, user-facing judgment features ever land. |
| [arXiv:2610.10659](https://arxiv.org/abs/2610.10659) — Applying Security by Design at the Point of Execution: How Governed Security Requirements Affect the Security of AI-Generated Code (Farinha, Oct 2026) | 2026-10-12 | **Evaluated, not adopted — no gap; validates existing methodology.** Its mechanism (governed requirements delivered to a coding agent at the point of execution via an MCP selector, with requirement-to-code conformance tracing) is already practiced here in stronger form: AGENTS.md + per-round execution prompts are the point-of-execution delivery, and the gate-law register is the conformance trace (row + red-proof + class + owned `owed` column). The paper's own infrastructure is a thin, specialized instance of this server's product shape — governed memory recalled into agent context, with owner-origin provenance as the "governed" trust primitive and proposal-only agent writes guarding the KB; a use case, not a build item. Its honest ceiling (prompt-time delivery is advisory) is superseded by the repo's enforce-at-the-seam stance (derived origin, class refusals, digest-bound approvals). Recorded takeaway: ASVS/SSDF identifier mapping on gate-law register rows, a docs-only change if external auditability is ever wanted. |
| [arXiv:2610.10265](https://arxiv.org/abs/2610.10265) — Stale, Misattributed, or Late: Where Personal Memory Fails Before Generation (Deng & Sinchaisri, Oct 2026) | 2026-10-12 | **Validates the core model; not adopted as mechanism.** Its three pre-generation memory failure modes map onto existing named mechanisms: *stale* is the bi-temporal validity filter (§2 — "serving only the active value of each correctly keyed slot" is exactly the `valid_at`/`invalid_at` point-in-time law, and its finding that temporal validity is a property of memory *construction* is the supersession/consolidation seam); *misattributed* is the owner-bound record gate (R100/R101 — a fact about the wrong person is a foreign-row read, refused rather than retrieved); *late* is only indirectly bounded here (static-embedding recall posture + capacity envelopes; no per-request serving-deadline SLO is enforced). Recorded hook: the paper's controlled-revision methodology could extend `bench eval` with a stale-exposure-rate metric — deterministic, hand-checkable, in the existing harness's style. |
| [arXiv:2610.11188](https://arxiv.org/abs/2610.11188) — What to Admit and How to Present: Governing Persistent Memory in LLM Agents (Liu & Ding, Oct 2026) | 2026-10-12 | **Validates the two-seam design; no gap.** Its admission/presentation split — what enters the working context vs how it is expressed — is the split this repo already enforces as two different seams: admission = recall filters, the exclude posture, and quarantine gates; presentation = the read seam (labels on every content-returning wire, PII masking, `sanitize_stored`). Both of its designs translate adjudicated attributes into eligibility decisions via deterministic policies — the compiled-permit shape of `record_read_gate` — and its typed-unit decomposition (PS) is the chunk-kind tier that already exists. Its retraining-free, inference-time posture is this repo's "no LLM in the core" law stated as a research result. |
| [arXiv:2610.10629](https://arxiv.org/abs/2610.10629) — The Harness as the Only Mutable Surface: Compliance-Bounded Self-Evolution of LLM Agents in Credit Pipelines, with a Measured Admission Gate (Akhtyamov, Oct 2026) | 2026-10-12 | **Validates the proposal/review/audit design; no gap.** For memory content, its reviewable-self-evolution bound is this server's ordinary path: "every adaptation is a diff with a cause and a test attached" ≈ agent writes are proposal-only, disposal is a human review-queue act, approval is quorum-gated and digest-bound, and its hash-chained record-before-deployment gate is the audit chain every mutating act already lands on. AI-Act-lane alignment: the reviewable artefact a supervisor inspects is the register + chain, not the agent's judgment. Stated ceiling: the paper governs *harness* self-edits (instruction text, tool-call logic) — that surface lives in the agent host and plugin, outside this tree. |
| [arXiv:2610.09348](https://arxiv.org/abs/2610.09348) — Relevance Is Not Sufficiency: What Actually Closes the Evidence Gap in Long-Term Memory QA (Li & Li, Oct 2026) | 2026-10-12 | **Validates §1's coverage objective; one eval hook recorded.** Its claim — top-ranked memories can each be relevant while jointly omitting the complementary fact the answer requires — is the gap the submodular packing module already encodes (objective = relevance + coverage + representativeness, checked by `answer_in_context` rather than document recall); the paper grounds the distinction in legal-evidence scholarship and shows it bites on multi-session and temporal questions. Recorded hook: a sufficiency-gap metric (required-fact coverage, per its conformance framing) could extend `bench eval` beside precision/recall/NDCG — an operator decision, deterministic and hand-checkable. |
| Agentic-memory architecture cluster: [Hippocam](https://arxiv.org/abs/2610.12124) (intent-structured consolidation), [QGMem](https://arxiv.org/abs/2610.11920) (event-centric memory + query-aware graph augmentation), [HGP](https://arxiv.org/abs/2610.10071) (on-device typed hybrid graph), [Memento 3](https://arxiv.org/abs/2610.11794) (revisable rulebooks as semantic memory), [SkillForge](https://arxiv.org/abs/2610.09832) (fitness-driven skill lifecycle), [Memory Type Varies](https://arxiv.org/abs/2610.11573) (per-type strategies) — all Oct 2026 | 2026-10-12 | **Landscape row for v2.0 Cortex — nothing adoptable under the no-LLM law today.** The store-side substrate these architectures assume already exists here: typed memory (`memory_kinds` ≈ episodic/semantic/procedural/procedure), the bi-temporal typed-edge graph, supersession-based consolidation, and quarantine + decay as lifecycle states (SkillForge's trial/active/stable/retired, minus the learned fitness signal). The papers' routing classifiers, consolidation policies, and credit assignment are learned components that would live in the agent, not the store — the reverse of this server's boundary. Watch item: QGMem's finding that flat memory + query-time augmentation beats eager graph construction (irrelevant relations accumulate over long histories) is the argument to weigh when v1.8 Consolidate populates the reserved node hierarchy. |
| [arXiv:2608.12428](https://arxiv.org/abs/2608.12428) — MindMemOS: A Portable and Self-Evolving Memory Operating Layer for AI Agents (Liang, Cui et al., Huawei, Aug 2026) | 2026-10-12 | **Validates the substrate; refused as machinery; three hooks recorded** ([full read](./docs/research/22-post-convergence-watch-oct-2026.md)). Its entity–property–time graph with timeline links over successive entity–property records is the entities/relations graph + bi-temporal pair + supersession chains re-derived independently; its fusion is RRF over BM25+dense; its admission gate is deterministic (Add/Reinforce/Update/Merge/Skip) even with LLM candidate classification; its Dreaming consolidation is traceable merge/archive/supersedes preferring non-destructive updates — doc 12's posture at a self-reported 94.03 LOCOMO (baselines second-hand, see doc 13). Refused: LLM extraction/fusion/planning/evolution on write and recall — the paper's own bill (~5.5M tokens for schema evolution, 25–27M per evolved-skill agent run vs 10.4M without) is the no-LLM law's best external evidence. Hooks: Reinforce as a consolidation verb; a deterministic consolidation-compression KPI for `bench eval`; feedback scoping (task-temporary/scenario-specific/long-term) for the decay lane. Ships plugins for the agent-host family this repo's plugin mirrors into. |
| [arXiv:2609.32091](https://arxiv.org/abs/2609.32091) — Memory as Middleware for Self-Improving AI Agents (Jayaram, Isahagian et al., Sep 2026) | 2026-10-12 | **Validates the architectural position; machinery unevaluated, refused by default.** Its diagnosis — agents are "operationally amnesiac" and bespoke per-agent memory is the wrong fix — is an independent (IBM) argument for this repository's shape: one governed store behind plugin, bridge, and UMP. Abstract-level triage only; the middleware's internals were not read, and any learned component in the write/recall path is refused under the standing no-LLM law before evaluation. Surfaced by a targeted provenance query, outside the doc-20 lane window ([doc 21](./docs/research/21-cloud-agent-convergence-oct-2026.md)). |
| [arXiv:2609.36373](https://arxiv.org/abs/2609.36373) — Audience-Bound Persistent Memory: Authorization Across the Memory Lifecycle (Liu, Sep 2026) | 2026-10-12 | **Validates the taint-carriage law; designated watch item.** Each memory item carries the audience present at record time; derived items inherit the intersection of their sources' audiences — the authorization-across-derivation problem R116 shipped the enforcement half of (revise inherits the superseded row's origin; peers can label their own traffic, never assert owner-grade provenance). The refinement this store lacks: intersection semantics over multi-source derivations. The shipped posture is deliberately one boolean capture label, no lattice (R116 ponytail); this paper is the first serious argument for refining it and is the designated re-triage trigger at the next monthly sweep. Mapping: the `(owner, access_scope)` pair is already the audience fact; the paper adds set algebra over derivation, not a new column. |
| [arXiv:2609.34242](https://arxiv.org/abs/2609.34242) — Stashbird: Efficient Speaker-Indexed Memory for Conversational Agents (Biringa, Yannul et al., Sep 2026) | 2026-10-12 | **Validates provenance-linked derivation; no gap.** Links source episodes to derived memory state through explicit provenance, indexed by speaker — the per-principal attribution this store enforces on every row (`owner`, origin labels, per-edge capture markers) plus the `derived_from` evidence graph the DSAR purge and `dsar_locate` already walk. Its episodic/semantic organization is the chunk-kind tier that exists. |
| [arXiv:2609.27279](https://arxiv.org/abs/2609.27279) — EnSIMem: Entity-Structured Indexing for Long-Term Agent Memory (Meng, Fan et al., Sep 2026) | 2026-10-12 | **Validates the entity/evidence tier's problem statement; machinery not evaluated.** Its diagnosed failure modes — generic summaries and anonymous chunks losing the entity–property–evidence binding — are exactly what the deterministic linker + entity/relations graph exist against. Its indexing model is not assessed; the problem statement is the validation. |
| [arXiv:2609.25913](https://arxiv.org/abs/2609.25913) — EPGM: Execution Provenance for Budgeted Agent Memory Retrieval (Wang, Ju et al., Sep 2026) | 2026-10-12 | **Eval hook recorded; learned retriever refused.** Its claim — fixed-k retrieval rewards fragments while answers need the complete evidence set under a hard token budget — is the gap submodular packing encodes (coverage term + `answer_in_context` diagnostic). Recorded hook: an evidence-completeness metric in its style could extend `bench eval` beside precision/recall/NDCG — deterministic, hand-checkable, an operator decision. |
| [arXiv:2609.21081](https://arxiv.org/abs/2609.21081) — Loopjacking: Hijacking Human-in-the-Loop Approval (Kumar, Sep 2026) | 2026-10-12 | **Threat-input, full-read, and live-reproduced as a defense verification.** Its two variants — representation-based display mismatch, post-approval state substitution — map onto R117 (one function generates the display and its digest, so the approver sees the bytes the digest binds) and R102 (kind + status + digest consumed atomically in one CAS tx). The paper's tested-positive systems are harness-layer (fourteen vulnerable versions across Agno AgentOS and LangGraph Agent Server); its negative control behaves as this approve path behaves. **Live run 2026-10-12** against an isolated instance of the shipped binary (temp db, two-lane tokens, agent principal as maker): 14/14 checks held — canonical display binding (sha256(shown)==shown digest), raw-digest and substituted-digest refusals (409), fail-closed missing digest (400), maker-class consumption refused (403), promoted bytes == displayed bytes, replay 404, concurrent CAS one-200-one-404, no-bearer 401 ([full procedure](./docs/research/22-post-convergence-watch-oct-2026.md)). The composite drill candidate resolves: variant-1 already pinned, variant-2 unreachable (no pending-state mutation path). Surfaced by the oversight-integrity watch lane. |
| [arXiv:2609.26761](https://arxiv.org/abs/2609.26761) — A2M: Trace-Optimized Agent Hijacking in the MCP Ecosystem (Li, Wang et al., Sep 2026) | 2026-10-12 | **Threat-input for the bridge lane; posture already held.** Its two-stage attack optimizes third-party tool *metadata* to raise invocation probability, then hijacks through the invocation — the MCP spec's normative answer ("tool annotations are untrusted unless from a trusted server") is this repo's standing posture, and the plugin's tools are static and self-authored. Honest residual: third-party servers configured into the host's MCP config are the surface, and that config lives outside this tree. |

---

## 9. Ecosystem Posture, Verified (October 2026)

The cloud-agent convergence check ([full doc](./docs/research/21-cloud-agent-convergence-oct-2026.md)):
all claims verified against vendor documentation on **2026-10-12**, prompted by
a circulated strategy video and settled against primary sources, not the video.

- **The harness layer shipped the loop.** Claude Code "Cloud sessions" (GA
  2026-09-23): isolated sandbox per task, secure git proxy, domain allowlists.
  GitHub Copilot cloud agent: **Automations** — schedule or repo-event
  triggers — plus confidence-based holds for human review. Codex Cloud:
  per-task workspaces that outlive the operator's laptop, network presets,
  review-and-publish before any task runs. All three are execution harnesses;
  the trigger layer stays in the harness (host cron/webhooks), not the
  store — the boundary decision this table's components assume.
- **The memory primitive is client-side by design.** Anthropic's memory tool
  (`memory_20250818`) issues six file-commands against a `/memories` prefix
  that *the application* maps onto storage it controls — the platform ships
  the verb, the deployment owns the store. That is the slot this server
  occupies; none of the platform-native stores carries attribution, human
  gating, tamper-evident history, or certified erasure across agents.
- **The category is now crowded; the governance axis is not.** GitHub API,
  2026-10-12: mem0 67,005 stars, MCP servers 91,151, cognee 32,042 (pushed
  the day of the check), graphiti 31,654, letta 25,108. "Memory layer" is a
  funded category; what distinguishes stores is provability (provenance,
  gating, audit, erasure) — the last twenty rounds' subject matter.
- **The regulatory clock is running.** The remainder of the EU AI Act applies
  from 2026-08-02 (Arts. 102–110 from 2026-07-27); Art. 50(2) synthetic-content
  transparency 2026-12-02; GPAI grandfathering 2027-08-02; high-risk
  record-keeping (Art. 12) 2027-12-02 / 2028-08-02. Timeline page updated
  2026-08-31; primary-law verification remains the Standards Register's lane.

Displacement: none. The check's yield is positioning evidence plus two
operator-decision candidates (the audience-intersection re-triage of the
boolean origin label; an ops-surface rollup of per-principal proposal,
quarantine, and feedback outcomes), neither a component change.

---

## Summary: SOTA Compliance

| Component | Library / Technique | SOTA as of July 2026 |
|-----------|-------------------|-----------------------|
| Static embeddings | `potion-retrieval-32M` via model2vec | ✅ SOTA for static (500x faster than transformers) |
| Hybrid fusion | RRF (k=60) | ✅ Standard model-free fusion algorithm |
| Cross-encoder reranker | BGE-RerankerV2M3 | ✅ Most-deployed production reranker (Qwen3-8B is accuracy leader, ~5x params) |
| PRF query expansion | Cross-retriever agreement gate | ✅ Production-hardened innovation |
| Submodular packing | Lazy greedy (160-token budget) | ✅ July 2026 paper (+5.1 F1 HotpotQA) |
| Temporal KG | Graphiti bi-temporal model | ✅ SOTA (82.2 DMR) |
| Temporal traversal | TRACE typed-edge prefixes | ✅ July 2026 paper |
| Entity linking | Aho-Corasick + verb-suffix filter | ✅ SOTA deterministic (dependency parsing upgrade path documented) |
| Markdown parsing | pulldown-cmark 0.13 | ✅ Canonical CommonMark parser |
| Embedded vector DB | sqlite-vec | ✅ SOTA for embedded/local (vs. cloud-scale engines) |
| Auth | JWT/JWS + OIDC | ✅ Industry standard |
| Security | subtle::ConstantTimeEq | ✅ OWASP-recommended |
| Audit | SHA-256 hash chain | ✅ Standard tamper-evident log |
| Capacity | Desktop / Jetson envelopes | ✅ Production-hardened |
| Fuzz testing | cargo-fuzz (LLVM libFuzzer) | ✅ Standard Rust fuzzing |
| Property testing | proptest (256+ cases) | ✅ Standard Rust property testing |

**Verdict**: all 16 components in the table above use the standard or SOTA
approach for their domain, verified against July 2026 research — the table's
stated basis, not re-swept since. The October 2026 sweep (§8) assessed the
memory-governance, agentic-memory, and AI-Act lanes for adoption and displaced
no component; the convergence check (§9) verified the ecosystem posture those
components ship into — harness sandboxes, trigger-spawned loops, client-side
memory primitives — and displaced nothing either. The table's own lanes
(retrieval, embeddings, parsing, auth) have not been re-verified against
post-July literature. Ponytail ceilings are documented with upgrade paths for
future releases.
