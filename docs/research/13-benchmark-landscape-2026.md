# 13 · The Memory-Benchmark Landscape (2026): LoCoMo, LongMemEval, BEAM, and contested scores

*The problem.* Agent-memory systems in 2026 market themselves with benchmark
numbers, but the numbers do not agree: the same system can score 92.5 on
LoCoMo in a vendor blog and 67.1 in a third-party comparison. Meanwhile the
field standardized on three benchmarks, **LoCoMo** (very long multi-session
conversations; QA + event summarization), **LongMemEval** (long-horizon
memory abilities), and **BEAM**, and a widely-cited Letta experiment showed a
**plain filesystem baseline** reaching competitive accuracy, which puts the
burden of proof on every specialized memory architecture: *what exactly does
your complexity buy?*

*The reference.* LoCoMo (Snap Research, ACL 2024,
[arXiv:2402.17753](https://arxiv.org/abs/2402.17753)) for the
multi-session evaluation shape; LongMemEval and BEAM for the 2026 standard
triad; the 2026 landscape writeups (Mem0's state-of-memory roundup; Letta's
filesystem-baseline study; third-party comparison tables) for the
score-controversy finding. The 2026 survey wave ([arXiv:2512.13564](https://arxiv.org/abs/2512.13564),
2603.07670, 2605.06716, 2602.06052) gives the taxonomy the per-category scores map onto.

**An honest gap.** LongMemEval and BEAM are named here as the 2026 standard triad
without canonical identifiers. Rather than guess at a citation, both are flagged as
**owed** in the [cited-work bibliography](./18-cited-work.md), and the planned
public harness below is where their identifiers should land.

*The deterministic way brain-server implements it.* The repo does **not**
self-report on these benchmarks yet, and that is the honest position until
the harness ships. What exists today:

- an **eval ship-gate**: a scale floor pinned in code — the frozen set must
  hold ≥100 judged queries (`tests/eval.rs`
  `test_eval_frozen_set_meets_scale_floor`; the 37-query starter was the
  wiring fixture, the 10-doc `DOCS` set the manual harness — neither is the
  evidence), with recorded floors (25-doc corpus, 106 queries, r@5 0.976 /
  mrr 0.956 — `docs/BENCHMARKS.md`). Retrieval regressions fail the build,
  which is stronger than a published number nobody can re-run;
- a **deterministic pipeline** (no LLM in the retrieval path, pinned
  embedding model, no API drift), which makes every future benchmark run
  *reproducible by construction*, the property the contested scores lack;
- per-category shape already present in the surfaces the benchmarks measure:
  single-hop (`/get`), multi-hop (graph traversal), temporal
  (bi-temporal `?at=` recall), open-domain (hybrid recall).

*The planned deliverable.* A public harness for LoCoMo + LongMemEval (BEAM
optional) behind the same eval gate: pinned seeds, pinned model, the corpus
hash committed, per-category results published alongside the harness that
reproduces them. Self-reported numbers without the harness are against the
house rules.

**Addendum 2026-10-12 — the numbers moved again.** MindMemOS
([arXiv:2608.12428](https://arxiv.org/abs/2608.12428), Huawei) self-reports
**LOCOMO 94.03** (its schema-guided mode; 87.60 schema-free) against
baselines it **cites from the EverOS paper under a matched configuration** —
EverOS 93.05, Zep 85.22, MemOS 80.76, memU 66.67, Mem0 64.20 — and
**PersonaMem 70.63** with locally re-run baselines. Two provenance caveats
keep it in the contested column: the LOCOMO baselines are second-hand
(cited, not re-run), and the system's recall path is LLM-planned, so its
scores measure a different cost class entirely (~25M tokens per
evolved-skill agent run, the paper's own table) than a deterministic
pipeline. Recorded as the current self-reported frontier, not as comparable
numbers — the harness remains the only comparison this repo will stand
behind. The paper also surfaces **MemoryAgentBench FactConsolidation**
(consolidation measurably improving fact scores) as a fixture candidate for
the planned harness, alongside the LoCoMo/LongMemEval pair above.

*The ceiling.* The shipped smoke-set floors are a regression gate, not a
quality claim on production-sized corpora. Benchmark scores are comparable
only through the harness, once it lands, and third-party runs may still
disagree, which is the point of publishing the method.
