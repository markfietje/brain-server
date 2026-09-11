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

*The reference.* LoCoMo (Snap Research, ACL 2024, arXiv:2402.17753) for the
multi-session evaluation shape; LongMemEval and BEAM for the 2026 standard
triad; the 2026 landscape writeups (Mem0's state-of-memory roundup; Letta's
filesystem-baseline study; third-party comparison tables) for the
score-controversy finding. The 2026 survey wave (arXiv 2512.13564, 2603.07670,
2605.06716, 2602.06052) gives the taxonomy the per-category scores map onto.

*The deterministic way brain-server implements it.* The repo does **not**
self-report on these benchmarks yet, and that is the honest position until
the harness ships. What exists today:

- an **eval ship-gate**: a frozen judged corpus (37 queries / 10 docs) with
  CI-pinned floors (r@5/r@10/MRR ≥ 0.85), retrieval regressions fail the
  build, which is stronger than a published number nobody can re-run;
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

*The ceiling.* The shipped smoke-set floors are a regression gate, not a
quality claim on production-sized corpora. Benchmark scores are comparable
only through the harness, once it lands, and third-party runs may still
disagree, which is the point of publishing the method.
