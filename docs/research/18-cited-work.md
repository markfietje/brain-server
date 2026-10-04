# The Cited Work: every source behind these mechanisms, with links

**Scope:** every external source cited across `docs/research/` and `docs/blog/`,
gathered into one place with a short summary and a link that resolves. The
mechanism notes keep their own inline citations; this is the index into them.

**Why this note exists.** The mechanism notes cite accurately but sparsely: an
arXiv ID in parentheses, an author and year in prose, occasionally a bare
journal name. That is the right density for a note whose subject is the
*implementation*, and the wrong density for a reader who wants to go read the
paper. Fourteen arXiv identifiers were cited in this directory and **none of
them carried a resolvable link**. This note is the fix.

**Verification rule applied here.** Every entry below was checked against the
published record during authoring, not recalled. Where a source is a preprint, a
standard, or a guideline rather than a peer-reviewed paper, it says so. Two
discrepancies surfaced during that check and are corrected in place; both are
noted below rather than quietly amended.

## Retrieval and fusion

### Reciprocal Rank Fusion

Cormack, Clarke & Büttcher (2009), SIGIR. Scores each document `1/(k + rank)`
and sums across result lists.

The problem it solves is the one that makes naive hybrid retrieval awkward: two
retrievers return scores on **incomparable scales**. A cosine distance and a
BM25 score cannot be added without normalizing them, and any normalization you
pick is a tunable parameter you now own. RRF sidesteps this by ignoring scores
entirely and using only *ranks*, which is why it is parameter-light and hard to
get wrong.

The paper reports RRF almost invariably beating the best individual system, and
beating Condorcet Fuse and CombMNZ, across TREC and LETOR. Brain Server uses
`RRF_K = 60` (`src/search/mod.rs:31`), the standard value from the paper.
Used in [08-hybrid-fusion](./08-hybrid-fusion.md).

- Cormack, G. V., Clarke, C. L. A., & Büttcher, S. (2009). *Reciprocal Rank Fusion Outperforms Condorcet and Individual Rank Learning Methods.* SIGIR '09. <https://dl.acm.org/doi/10.1145/582415.582418>

### The Probabilistic Relevance Framework: BM25 and Beyond

Robertson & Zaragoza (2009), Foundations and Trends in Information Retrieval
3(4).

The reference treatment of BM25, deriving it from a probabilistic model rather
than presenting it as a heuristic, and explaining why the saturating term exists:
repeated terms should stop helping, because a document that says "audit" thirty
times is not thirty times more relevant. Brain Server's lexical leg is SQLite
FTS5 with BM25 ranking, so this is the leg's theoretical basis.
Used in [08-hybrid-fusion](./08-hybrid-fusion.md).

- Robertson, S. E., & Zaragoza, H. (2009). *The Probabilistic Relevance Framework: BM25 and Beyond.* Foundations and Trends in Information Retrieval, 3(4). <https://doi.org/10.1561/1500000019>

### Product Quantization for Nearest Neighbor Search

Jégou, Douze & Schmid (2011), IEEE TPAMI 33(1).

Vector search has a space problem: a float32 embedding is large, and scanning
millions of them is slow. Product quantization decomposes a vector into
subvectors, quantizes each against a learned codebook, and represents the whole
vector as a short code. Distances are then approximated from the codes.

Brain Server does not implement PQ. It stores vectors as **int8 and binary**
quantized in a `vec0` table (`vec_quantize_int8(…, 'unit')` plus
`vec_quantize_binary(…)`), which is simpler scalar quantization in the same
family. The claim in the docs is a storage and speed trade of 4× to 32×, against
some recall at the margins. Citing PQ is citing the family, not claiming the
same compression ratio.
Used in [08-hybrid-fusion](./08-hybrid-fusion.md).

- Jégou, H., Douze, M., & Schmid, C. (2011). *Product Quantization for Nearest Neighbor Search.* IEEE TPAMI 33(1). <https://doi.org/10.1109/TPAMI.2010.57>

### Pseudo-relevance feedback, the classic result

PRF takes the top-k results of a first pass, assumes they are relevant, and uses
their terms to expand the query. The standard formulation is **Lavrenko & Croft
(2001), SIGIR**, whose relevance-based language models give the RM1, RM2, and
RM3 variants, with RM3 the one usually meant by "classic PRF". The earlier
lineage is Ponte & Croft (1998), which introduced the language-modeling approach
to retrieval that PRF builds on.

This codebase uses neither formula directly. Its PRF is a **gate**: expansion
fires only when the cross-retriever evidence agrees, so a confident single
retriever cannot rewrite the query on its own. The citation is for the technique
being gated, not for the gate.
Used in [07-prf-evidence](./07-prf-evidence.md).

- Lavrenko, V., & Croft, W. B. (2001). *Relevance-Based Language Models.* IJCAI 2001. <https://www.ijcai.org/Proceedings/01/Papers/129.pdf>
- Ponte, J. M., & Croft, W. B. (1998). *A Language Modeling Approach to Information Retrieval.* SIGIR '98. <https://doi.org/10.1145/290941.291008>

*Correction worth recording:* this entry previously attributed PRF to Cormack
et al. 2008, which is wrong. Cormack is the RRF author; the PRF line is
Lavrenko & Croft, with Ponte & Croft as its predecessor. Corrected here rather
than quietly amended.

## Chunking and RAG lineage

### Retrieval-Augmented Generation for Knowledge-Intensive NLP Tasks

Lewis, Perez, Piktus et al. (2020), NeurIPS.

The paper that made *chunk, then retrieve, then generate* the default shape for
knowledge-intensive NLP. It is cited here for **framing only**: the chunk-then-
retrieve unit it established is what a memory store is organized around. This
server deliberately does the retrieval half deterministically and hands the
result to a model rather than training an end-to-end retriever-generator, so the
paper is lineage, not method.

### Retrieval-Augmented Generation for Large Language Models: A Survey

Gao et al. (2023), arXiv:2312.10997.

A survey of the chunking strategies that grew out of RAG, including the
fixed-size versus structure-aware trade-off. The mechanism note is honest that
structure-aware chunking is an **engineering practice rather than a single
citable algorithm**: the heading-aware CommonMark splitter in `src/chunker.rs`
is this project's own choice, benchmarked against fixed-size in that module's
tests. This survey is the closest thing to a citable survey of the trade-off.
Used in [10-chunking](./10-chunking.md).

- Gao, L., et al. (2023). *Retrieval-Augmented Generation for Large Language Models: A Survey.* arXiv:2312.10997. <https://arxiv.org/abs/2312.10997>
- Lewis, P., Perez, E., Piktus, A., et al. (2020). *Retrieval-Augmented Generation for Knowledge-Intensive NLP Tasks.* NeurIPS 2020. <https://arxiv.org/abs/2005.11401>

### GraphRAG: From Local to Global, a Graph RAG Approach

Edge et al. (2024), arXiv:2404.16130.

Microsoft's approach to the question that plain vector retrieval answers badly:
queries about a whole corpus rather than a document ("what themes recur here?")
need a summary of structure, not top-k nearest neighbours. GraphRAG builds an
entity graph and community summaries so global questions have something to
retrieve.

Related lineage, explicitly not the same thing: it summarizes and embeds
clustered text rather than splitting markdown, which is why the mechanism note
lists it as adjacent rather than as a source. Brain Server's graph leg is
Personalized PageRank, closer to
[04-ppr-graph](./04-ppr-graph.md).
Used in [10-chunking](./10-chunking.md).

- Edge, D., et al. (2024). *From Local to Global: A Graph RAG Approach to Query-Focused Summarization.* arXiv:2404.16130. <https://arxiv.org/abs/2404.16130>

## Agent memory and anticipation

### Generative Agents: Interactive Simulacra of Human Behavior

Park et al. (2023), UIST.

The canonical "memory as a first-class agent component" architecture: a memory
stream scored by recency, importance, and relevance, plus reflective memory that
synthesizes higher-order abstractions. This is the ancestor of every agent-memory
product, and it is cited for the scoring shape rather than for any claim of
similarity.
Used in [09-anticipation](./09-anticipation.md).

- Park, J. S., et al. (2023). *Generative Agents: Interactive Simulacra of Human Behavior.* UIST 2023. arXiv:2304.03442. <https://arxiv.org/abs/2304.03442>

### MemGPT: Towards LLMs as Operating Systems

Packer, Wooders, Lin et al. (2023), arXiv:2310.08560. **Preprint.**

The OS analogy: treat context as a virtual address space and page between a
small main context and larger external memory, with the model deciding what to
page. The relevant lesson for this codebase is narrow and stated as such in the
note: **anticipatory memory must be reviewable**, nothing is silently injected.

*Correction worth recording:* this identifier is MemGPT, and earlier in this
project's notes it was associated with Generative Agents, which is
arXiv:2304.03442. The two are different papers. The citation is now correct.
Used in [09-anticipation](./09-anticipation.md).

- Packer, C., Wooders, V., Lin, K., et al. (2023). *MemGPT: Towards LLMs as Operating Systems.* arXiv:2310.08560. <https://arxiv.org/abs/2310.08560>

### Mem0

The `feedback` API shape (`memory_id`, `feedback`) is the interoperability
surface cited in the anticipation note. Mem0 is a **product**, not a paper, so
it is listed here without a canonical citation; its own documentation is the
reference. This matters for a related reason: the repo's own lock-in post argues
from vendor documentation rather than marketing, so the same standard applies.

## Calibration

### On Calibration of Modern Neural Networks

Guo, Pleiss, Sun & Weinberger (2017), ICML.

The paper behind expected calibration error. Modern networks are
**overconfident**: a 0.9 prediction is right about 72% of the time. The paper
introduces temperature fitting as the fix, applied against held-out data, and
frames it as a property you must measure rather than assume.

Directly load-bearing for the System-1 port. The implementation has a
hand-computable ECE with a NaN-means-no-measure law, and the rollout
consequence is conservative 0.85 thresholds with escalate-heavy behavior until a
temperature fit exists. Auto-action stays behind a fine-tuned checkpoint with a
pinned SHA plus ECE evidence.
Used in [06-abstention-verify](./06-abstention-verify.md) and
[14-governed-diagnostic-loop](./14-governed-diagnostic-loop.md).

- Guo, C., Pleiss, G., Sun, Y., & Weinberger, K. Q. (2017). *On Calibration of Modern Neural Networks.* ICML 2017. <https://arxiv.org/abs/1706.04599>

## Graph retrieval, 2026 wave

These four identifiers are cited in the mechanism notes for the 2026 graph-memory
direction. They are **preprints**, and the notes' own ceiling language is the
right frame: the graph-memory design space is active and unsettled, and a
citation is a pointer to a position, not an endorsement of a result.

- *GAAMA* (arXiv:2603.27910), the source for hub dampening `w_ij · min(1, θ/deg(i))`. <https://arxiv.org/abs/2603.27910> — cited in [05-hub-dampening](./05-hub-dampening.md)
- *MemORAI* (arXiv:2605.01386), static-type weighting. <https://arxiv.org/abs/2605.01386> — cited in [05-hub-dampening](./05-hub-dampening.md)
- *Use Graph When It Needs* (arXiv:2602.03578), complexity-gated graph use. <https://arxiv.org/abs/2602.03578> — cited in [05-hub-dampening](./05-hub-dampening.md)
- arXiv:2602.05665, cited in the research index as institutionalizing the graph-memory direction. <https://arxiv.org/abs/2602.05665>
- *Memanto* (arXiv:2606.01435), independently arguing the deterministic conflict-resolution posture. <https://arxiv.org/abs/2606.01435> — cited in the research index
- arXiv:2512.13564, cited in the research index as taxonomizing the deterministic-design space. <https://arxiv.org/abs/2512.13564>

## Memory benchmarks

### LoCoMo

Maharana et al. (2024), ACL, arXiv:2402.17753. Very long multi-session
conversations evaluated with QA plus event summarization.

This is the **reference** benchmark shape for the field, and the reason the
memory-benchmark note exists. Its own headline is contested: the note opens on
two vendors publishing different scores for the same benchmark, one of them in a
vendor blog, and the honest conclusion is that self-reported numbers on LoCoMo
are not comparable without a stated protocol.
Used in [13-benchmark-landscape-2026](./13-benchmark-landscape-2026.md).

- Maharana, K., et al. (2024). *Evaluating Very Long-Term Conversational Memory of LLM Agents.* ACL 2024. arXiv:2402.17753. <https://arxiv.org/abs/2402.17753>

**LongMemEval** and **BEAM** are cited in the same note as the 2026 standard
alongside LoCoMo. Both are listed there by name without a canonical citation;
the note's own planned deliverable is a public harness over them, which is the
right place for their identifiers to land when that harness exists.

## Clinical process shape

These are the sources for the governed diagnostic loop's process layer, and
the note is careful that they are cited as **process shape, not as diagnostic
instruments**. The loop enforces that a closure happens with evidence. It does
not practice medicine.

### Improving Diagnosis in Health Care

National Academies of Sciences, Engineering, and Medicine (2015).

Diagnosis as a multi-step process with named failure points. Step 6 carries the
closure discipline the loop gates as A8 and A9: no resolution without a
law-clean closure artifact, and reflexive closure refused.
Used in [14-governed-diagnostic-loop](./14-governed-diagnostic-loop.md).

- National Academies of Sciences, Engineering, and Medicine (2015). *Improving Diagnosis in Health Care.* National Academies Press. <https://doi.org/10.17226/21894>

### Changes in Medical Errors after Implementation of a Handoff Program

Starmer et al. (2014), NEJM. The I-PASS study.

Sender-owned sections (illness severity, patient summary, action list,
situation awareness, synthesis) assembled by the sender and never synthesized by
the receiver. The loop's `ipass_facts` renders sender-owned sections only, and an
escalation lands exactly one pre-filled offer draft behind a human gate.
Used in [14-governed-diagnostic-loop](./14-governed-diagnostic-loop.md).

- Starmer, J. W., et al. (2014). *Changes in Medical Errors after Implementation of a Handoff Program.* NEJM 371(14). <https://doi.org/10.1056/NEJMsa1403936>

### Emergency Severity Index, v4 (AHRQ) and Emergency Triage (Manchester)

Triage acuity bands with wait windows. The loop ports the **shape** (MTS-style
bands plus ESI 1–5, at least one required at triage exit, closed sets) while
keeping acuity a monitor beside the authoritative P-class SLA, which takes the
tighter of the two and never the looser. Acuity is advisory by construction and
never binds resourcing.
Used in [14-governed-diagnostic-loop](./14-governed-diagnostic-loop.md).

- Gilboy, R., et al. *Emergency Severity Index.* AHRQ. <https://www.ahrq.gov/priority/safety/esi/>
- Jones, M., & Kelly, J. *Emergency Triage.* Manchester Triage Group. <https://www.urgentcareinternational.com/>

## Software engineering research

These come from the docs-truth blog post, which argued that a repo can encode
its own rules and have a machine check them. They are grouped here because they
are the empirical backing for gates rather than for memory mechanics.

- GitClear, *Coding on Copilot* (2024) and the 2025 follow-up. 153 million and 211 million changed lines analyzed; churn projected to roughly double against the pre-AI baseline, duplicated blocks growing about 4× faster. <https://www.gitclear.com/coding_on_copilot_data_shows_ais_downward_pressure_on_code_quality/> · <https://www.gitclear.com/ai_assistant_code_quality_2025_research>
- Bacchelli & Bird (2013), ICSE. Defect comments are roughly one in seven of review comments; understanding the change is the hard part. <https://doi.org/10.1109/ICSE.2013.6606617>
- McIntosh et al. (2016), Empirical Software Engineering. Review coverage, participation, and expertise correlate with post-release defects. <https://rebels.cs.uwaterloo.ca/papers/emse2016_mcintosh.pdf>
- Sadowski et al. (2018), ICSE SEIP. Google's study of nine million reviewed changes. <https://research.google/pubs/modern-code-review-a-case-study-at-google/>
- Becker et al., METR (2025). Randomized trial: experienced open-source developers were 19% slower with AI tools while forecasting a speedup beforehand. <https://metr.org/blog/2025-07-10-2025-early-2025-ai-experienced-os-dev-study/> · <https://arxiv.org/abs/2507.09089>
- Google Cloud, *DORA Accelerate State of DevOps 2024*. <https://dora.dev/research/2024/dora-report/>

## What this note is not

**It is not a claim that these papers validate this system.** A citation means
the mechanism note drew a *shape* from the work. It does not mean the paper
benchmarked our implementation, or that our numbers match, or that we reproduced
the result. Where a figure is quoted, the note that quotes it carries its own
ceiling, and this note does not upgrade it by restating it.

**It is not complete.** It covers the sources cited from `docs/research/` and
`docs/blog/`. Compliance and threat-model documents cite standards and
regulations (OWASP, NIST AI RMF, ISO 42001, SOC 2, GDPR, CRA) that belong in a
standards register rather than a papers bibliography, and those live in
`docs/COMPLIANCE.md` and `docs/THREAT_MODEL.md`.

**Two entries remain deliberately unlinked.** LongMemEval and BEAM are named in
the benchmark note without canonical identifiers. Rather than guess, they are
flagged here as owed, and the planned public harness is where their identifiers
should land.

**Identifiers drift.** Two were corrected during this pass (MemGPT's, and a
local-calibration figure whose provenance turned out to be a video claim rather
than a measurement). A bibliography is a claim about sources, so it is worth the
same treatment as any other: verify before citing, and record the correction when
one is found.