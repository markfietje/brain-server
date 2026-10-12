# 20 · The Agent-Memory Research Sweep (October 2026): what validates, what threatens, what is refused

*The problem.* Agent-memory research now publishes faster than a shipped
system can read it, and each paper arrives wearing two possible costumes: a
confirmation of a design decision, or a scope-creep proposal wearing a
benchmark number. The failure mode for a small project is neither ignoring
the literature nor rebuilding the product around every preprint — it is
having no standing discipline for telling the two costumes apart. This file
is that discipline's first monthly snapshot: every relevant paper from the
October 2026 sweep, triaged against shipped behavior, with the refusal
reasons written down so they do not get re-litigated.

*The method.* Eight standing lanes (agent memory, LLM agents + memory, MCP,
memory poisoning, machine unlearning, right-to-erasure, injection + memory,
AI Act) are swept against the arXiv API daily, newest first, deduplicated
across lanes. Each relevant paper receives one of four verdicts:
**VALIDATES** (independently supports a shipped design decision),
**THREAT-INPUT** (names an attack or failure mode the threat model should
carry), **EVAL-FIXTURE** (a dataset or benchmark usable by the planned
public harness), or **REFUSED** (a component deliberately not integrated,
with the reason). Sources dated 2026-09-26 through 2026-10-09; status labels
are as of 2026-10-12. Exactly one item below is peer-reviewed; everything
else is a preprint and labeled as such.

## What validated

- **Typed memory with per-type retrieval.**
  [MemoType](https://arxiv.org/abs/2610.11573) — *"Memory Type Varies"*
  (NeurIPS 2026; [code](https://github.com/wenyiwy99/MemoType)). Finding:
  unified retrieval across memory types is suboptimal; episodic /
  personal-semantic / general-semantic partitions with tailored strategies
  win. The taxonomy is a description of brain-server's existing columns —
  `owner`/`access_scope`/`domain` for the personal/general split,
  `observed_at`/`valid_from` for episodic anchoring, separate procedure and
  decision stores. **Refused:** the fine-tuned router (Qwen3-1.7B + LoRA)
  classifying every write — a learned model between hostile input and
  permanent memory is an injection surface, plus token cost the zero-token
  core does not pay. **Eval fixture:** their TriMEM labeled dataset for a
  future deterministic-classifier evaluation.
- **State-trajectory retention.**
  [QGMem](https://arxiv.org/abs/2610.11920) (preprint) consolidates events
  into traces that *"retain state trajectories and current states"* — the
  bi-temporal supersession design (keep the superseded fact under
  `valid_to`; the pair is queryable in both directions) argued for on
  retrieval-quality grounds. Its conflict-aware reasoning and hybrid
  retrieval correspond to contradiction flags with supersession links and
  RRF over BM25 + quantized vectors. **Refused:** the learned graph-token
  encoding inside the recall path.
- **Admission vs presentation.**
  [What to Admit and How to Present](https://arxiv.org/abs/2610.11188)
  (preprint) splits memory governance into admission (what enters working
  context) and presentation (how it is expressed) — the recall-side names
  for two gates brain-server already operates: the human review queue and
  calibrated abstention on the admission side, the untrusted-banner
  treatment and superseded-fact flags on the presentation side. Vocabulary
  adopted; machinery refused (their compiled admission controller is
  model-driven).
- **On-device memory.** [HGP](https://arxiv.org/abs/2610.10071) (preprint)
  argues personalized memory belongs on-device with hybrid graph storage —
  the local-first lane, independently.
- **Pre-generation failure taxonomy.**
  [Stale, Misattributed, or Late](https://arxiv.org/abs/2610.10265)
  (preprint) measures memory failures *before* generation: obsolete values,
  wrong-person attribution, missed deadlines — the exact failures provenance
  labels, bi-temporal filters, and abstention exist to prevent.
- **Derivation-gated governance.**
  [Lineage-Aware Memory Governance](https://arxiv.org/abs/2610.07258)
  (preprint) gates enterprise memory by derivation lineage and conflicting
  KPI logic — adjacent to the evidence-link graph (`derived_from`) and
  record read gates. Watch item, not an integration.

## What threatens

- **Memory as a delayed-action payload.**
  [Self-propagating misalignment](https://arxiv.org/abs/2610.04083)
  (preprint): with no adversary at all, a misaligned agent writes a goal it
  cannot yet act on, for a future agent to execute. The title's claim —
  auditing or disabling memory is not enough — is accepted; the shipped
  counter-design is the write gate (nothing enters permanent memory without
  a human), provenance on every row, and the tamper-evident audit chain.
  Threat-model candidate.
- **Harness-based persistent poisoning.**
  [PMPA](https://arxiv.org/abs/2609.13889) (preprint) attacks the exact
  architecture class (harness + persistent memory) this ecosystem deploys.
- **Detection frameworks.**
  [MemSentry](https://arxiv.org/abs/2609.08747) (preprint) detects persistent
  poisoning; a comparison point for the quarantine posture.
- **Defense economics.**
  [The Price of Safety](https://arxiv.org/abs/2609.22818) (preprint)
  measures utility loss and token overhead of poisoning defenses. Every
  surveyed defense pays tokens on the hot path; the deterministic gate and
  review queue pay zero — a cost axis worth publishing on.
- **Repair by decay.**
  [Forgetting is Not Always Bad](https://arxiv.org/abs/2610.13267)
  (preprint) repairs poisoned memory through reactivation-triggered decay.
  Useful contrast: heuristic forgetting is a probability; certified erasure
  is a receipt.

## The erasure lane

- [The Tokens Remember](https://arxiv.org/abs/2609.29045) (preprint):
  deletion and unlearning claims break on physical residue in model
  artifacts — the field-wide version of the residue problem the sealed-snapshot
  and shred-verify posture already treats, including the stated-ceiling
  habit of naming what erasure cannot control.
- [Modeling Deletion Requests](https://arxiv.org/abs/2610.04310) (preprint):
  how requesters behave when exercising erasure (adaptivity, collectivity) —
  DSAR UX input.
- [TRIAGE](https://arxiv.org/abs/2609.32103) (preprint): unlearning
  evaluation needs mechanistic insight beyond behavioral benchmarks — the
  model-world still lacks what the store-side provides: deletion with
  verifiable bytes-gone evidence.

## Governance and ecosystem

- [Auditable Claims about AI Agents](https://arxiv.org/abs/2610.07459)
  (preprint): under the EU AI Act's record-keeping duties, a claim must name
  its policy, scope, and the records that settle it — the register-and-evidence
  discipline, argued for publicly.
- [Runtime Compliance Verification (C-Trace)](https://arxiv.org/abs/2606.19242)
  (preprint): runtime GDPR conformance tracing for agents — the nearest
  peer framing of the gate layer.
- [The MCP ecosystem landscape](https://arxiv.org/abs/2610.05319) (preprint)
  and [COPEX](https://arxiv.org/abs/2610.04378) (preprint): the server-side
  market structure this project ships into, and adversarial-context
  robustness across MCP layers.

## What was integrated: nothing, on purpose

Every integration proposal in the sweep fails the same standing law: no
learned component in the write or recall path, no LLM inside the memory
core, no new dependency for an unproven retrieval gain — the zero-token,
deterministic boundary stated across these notes. What the sweep produced
instead is evidence and fixtures: validation citations for the typed-memory,
state-trajectory, admission/presentation, and erasure designs; a
threat-model candidate; and three eval fixtures (TriMEM, the QGMem benchmark
suite, a LongMemEval subset) owed to the public harness planned in
[the benchmark landscape](./13-benchmark-landscape-2026.md).

*The ceiling.* One of the fourteen papers above is peer-reviewed; the rest
are preprints assessed as of October 2026 and could change or withdraw.
Verdicts are relevance triage against shipped behavior, not replication of
results. This file is the October snapshot of an operational daily sweep; a
fuller working register with per-paper dispositions is maintained outside
the repository. No affiliation with any listed work.
