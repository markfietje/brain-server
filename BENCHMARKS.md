# Brain Server Benchmarks — "Better than QMD" measurement plan

> Status: **scaffold / pre-results.** No benchmark has been executed yet. All result
> rows are `PENDING`. This document defines the *reproducible* protocol; numbers land in
> the tables only after a run on the target hardware (including the 4 GB ARM edge).
>
> Companion files: `tests/metrics.rs` (pure metric functions + unit tests) and
> `tests/fixtures/eval_queries.md` (frozen judged query set).

## Purpose

 "Better than QMD" is a **measured** claim, not a list of features. This document fixes
the workload, hardware, metrics, and commands so a third party can reproduce every number
Brain Server publishes.

## "Better than QMD" measurement rules

1. **Same everything.** Use the **same corpus, chunking, judged queries, and hardware**
   for Brain Server and QMD. No cherry-picked subsets.
2. **Quality must match/exceed on:** `recall@5`, `recall@10`, `nDCG@10`, `MRR`, and
   answer-grounding/citation accuracy.
3. **Edge win is mandatory on 4 GB ARM.** The default profile must show a documented win
   in RSS, cold start, model-disk footprint, p95 latency, and power. "No API cost" alone
   is not a win — QMD also runs locally.
4. **Explainability.** Every returned result must be explainable: source URI/path, source
   revision, chunk span, retrieval paths/ranks, rerank contribution, domain.
5. **Optional heavy retrieval only.** Heavyweight learned retrieval is a *quality profile*,
   never a hidden dependency of the default build.
6. **No unqualified marketing claims** ("zero model download", "HNSW", "production-ready",
   "100× cheaper", "best on the market") unless a reproducible measurement proves each one.
7. **Set hygiene.** Keep dev / validation / final query sets separate. Do **not** tune
   PRF/RRF/rerank thresholds on the final set.

## Metrics & formulas

All ranking metrics are implemented in `tests/metrics.rs` (`recall_at_k`,
`precision_at_k`, `ndcg_at_k`, `mrr`) and unit-tested with hand-computed values.

- **recall@k** = |relevant ∩ top-k| / |relevant|.
- **precision@k** = |relevant ∩ top-k| / k.
- **nDCG@k** (Normalized Discounted Cumulative Gain):
  - DCG@k = Σ_{i=1..k} rel_i / log₂(i+1), with binary graded relevance rel_i ∈ {0,1}.
  - IDCG@k = Σ_{i=1..min(k, |relevant|)} 1 / log₂(i+1)  (ideal = all relevant first).
  - nDCG@k = DCG@k / IDCG@k.
  - Sources: Järvelin & Kekäläinen (2002), *Cumulated Gain-Based Evaluation of IR
    Techniques*, ACM TOIS 20(4), https://dl.acm.org/doi/10.1145/582415.582418 ; and
    Wikipedia, "Discounted cumulative gain",
    https://en.wikipedia.org/wiki/Discounted_cumulative_gain .
  - Note (per TODO fixture spec): if a relevant id appears multiple times in the result
    list, each occurrence is graded at its own position; IDCG is over the distinct relevant
    set, so duplicate relevant hits can inflate DCG above IDCG.
- **MRR** (Mean Reciprocal Rank): per query, reciprocal of the 1-indexed rank of the first
  relevant result (0.0 if none); MRR is the mean across queries.
  - Source: standard IR definition; see Wikipedia "Discounted cumulative gain" and the
    MRR explainer at https://www.evidentlyai.com/ranking-metrics/mean-reciprocal-rank-mrr .

### Resource / latency metrics

- **p50 / p95 latency** of `/search` (and `/recall` once it exists) over the frozen query set.
- **Cold-start time**: process start → first successful query served.
- **RSS**: resident memory of the server process at idle and under query load.
- **DB size**: on-disk size of the SQLite database (incl. sqlite-vec index) after ingest.
- **Model-cache size**: on-disk footprint of the embedding model (and reranker, when
  `--features rerank`) — the "complete installed footprint", not just RSS.
- **Ingestion throughput**: docs (or chunks) ingested per second over the fixture corpus.

## Machine specification (template — fill with PLACEHOLDERS)

| Field | Desktop (PLACEHOLDER) | 4 GB ARM edge (PLACEHOLDER) |
|---|---|---|
| CPU | `<model, cores, freq>` | `ARM Cortex-A57 / 4 GB RAM (Jetson Nano-class)` |
| RAM | `<GB>` | `4 GB` |
| OS | `<distro + kernel>` | `<distro + kernel>` |
| Arch | `<x86_64 / aarch64>` | `aarch64` |
| Rust / toolchain | `<rustc version>` | `<rustc version>` |
| Model cache state | `<model id + size on disk>` | `<model id + size on disk>` |
| Date measured | `PENDING` | `PENDING` |

> Replace every PLACEHOLDER and `PENDING` with real values at run time. Record the exact
> commit hash and `Cargo.lock` so the run is reproducible.

## Configurations under test

Four Brain Server profiles plus the two QMD reference profiles:

> **Note (v0.9.5, `3fcac72`):** **BS-4 is suspended.** The rerank tier was
> deleted entirely (Cargo feature flag + `src/search/rerank.rs`), so
> `cargo build --features rerank` errors and BS-4 cannot be built without
> reverting `3fcac72` on a CUDA-GPU host. The BS-4 rows below stay as the
> historical record of what the profile measured when rerank shipped; treat
> them as `N/A` until rerank is restored. BS-1/BS-2/BS-3 are unaffected.

| Config ID | System | Profile | Notes |
|---|---|---|---|
| BS-1 | Brain Server | dense-only | vector retrieval only (no FTS/PRF/rerank) |
| BS-2 | Brain Server | hybrid | dense + FTS, RRF fusion |
| BS-3 | Brain Server | hybrid + PRF | BS-2 plus pseudo-relevance feedback |
| BS-4 | Brain Server | hybrid + PRF + rerank | **Suspended in v0.9.5** — requires reverting `3fcac72` to build |
| QMD-1 | QMD | default | QMD default profile (expansion + rerank) |
| QMD-2 | QMD | fast / no-rerank | QMD fast profile (rerank disabled) |

> BS-1/BS-2/BS-3 build with the default feature set. BS-4 previously built with
> `cargo build --release --features rerank`; that flag was removed in `3fcac72`.
> PRF/RRF constants must come from the committed config, **not** tuned per run.

## Reproducible command protocol

There is no benchmark CLI yet (planned `cargo run --bin bench`). Until
then, run against the live HTTP API. The protocol is deterministic given a fixed corpus
and query set.

### 0. Prerequisites

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cd /Users/mark/Sites/brain-server

# Unit-test the metric functions themselves (fast, no model download):
cargo test --test metrics
```

### 1. Build the server (default)

```bash
# Default features (dense / hybrid / PRF; no reranker — rerank tier deleted in 3fcac72)
RUSTFLAGS="-C target-cpu=native -C opt-level=3 -C codegen-units=1" \
  cargo build --release

# Rerank profile (BS-4) is SUSPENDED in v0.9.5. To re-enable on a CUDA-GPU
# host, revert commit 3fcac72, then:
#   RUSTFLAGS="-C target-cpu=native -C opt-level=3 -C codegen-units=1" \
#     cargo build --release --features rerank
```

### 2. Start the server + record cold-start

```bash
# In one terminal; note the start timestamp for cold-start measurement.
./target/release/brain-server &
SERVER_PID=$!
# Poll until ready, record (now - start) as cold-start time:
curl -fsS http://localhost:8765/health
```

### 3. Ingest the fixture corpus

The frozen query/doc fixture lives in `tests/eval.rs` (`DOCS`) and
`tests/fixtures/eval_queries.md`. For a real benchmark, ingest the **versioned,
representative corpus** (≥ 100 queries' worth of docs), not just the 10-doc smoke set.

```bash
# Example ingest (loop over corpus markdown files):
for f in corpus/*.md; do
  curl -X POST http://localhost:8765/ingest/markdown \
    -H 'Content-Type: application/json' \
    -d "{\"title\":\"$(basename "$f" .md)\",\"content\":\"$(cat "$f")\"}"
done
# Record ingestion duration + DB size (sqlite .db file) for throughput/size metrics.
```

> For the smoke/CI fixture, ingest the 10 `DOCS` strings via `/ingest/markdown`.

### 4. Query the frozen set + collect ranks

```bash
# For each judged query in tests/fixtures/eval_queries.md, capture the ranked id list.
# Map returned chunk ids back to DOCS indices, then feed results + Relevant into the
# metrics in tests/metrics.rs (or a thin harness that replicates them).
curl 'http://localhost:8765/search?q=<QUERY>&k=10'
# When available: curl 'http://localhost:8765/recall?q=<QUERY>&k=10'
```

A small offline scorer (mirroring `tests/metrics.rs`) reduces the captured ranks + the
`Relevant:` judgments to `recall@5/10`, `ndcg@10`, `mrr`, `precision@k` per query, then
averages across the set. Keep dev / validation / final sets separate; only the **final**
set is reported.

### 5. Record resource metrics

```bash
# RSS at idle and under load:
ps -o rss= -p $SERVER_PID
# p50/p95 latency: timestamp each /search call across the frozen set.
# DB size:
du -h brain.db  # or the path from BRAIN_DB_PATH
# Model-cache size: du -sh <model cache dir>
```

### 6. Tear down

```bash
kill $SERVER_PID
```

> Reproducibility gate: a release may not claim parity unless this command
> sequence is repeatable by a third party on the same corpus/queries/hardware. Commit the
> raw captured ranks, the `Relevant:` judgments, machine spec, model versions, and the
> computed tables alongside this file.

## Results — PENDING

All rows below are **PENDING — run on target hardware (incl. 4 GB ARM); not yet executed.**

> - **Latency & RSS**: `cargo run --release --features bench --bin bench` against a running server (`brain`). Run on target hardware and paste the output here.
> - **Recall quality**: `cargo test --release -- --ignored --nocapture eval_recall_harness` (loads the model2vec weights; directional signal on the 10-doc smoke set). Expand to ≥100 judged queries before drawing release-blocking conclusions.

### v0.9.9 "Qualify" — measured capacity envelope (production target, 2026-07-25)

**Run:** `BENCH_ENVELOPE=desktop BENCH_SCALES=1000,5000 BENCH_SEARCHES=100 bench`
**Target hardware:** mini PC — AMD Ryzen 7 2700U (8 threads, x86_64), 30 GB RAM, Ubuntu kernel 7.0
**Commit:** `8a36b6a` (v0.9.9) · **Rust:** 1.93.1 · **Server:** v0.9.9, default features, systemd unit
**Envelope checked:** `desktop` (50k docs / 2 GiB DB / 512 MB RSS; p95 ≤ 200 ms)
  — RSS ceiling raised 320 → 512 MiB in v1.16.x (soft signal: Warning only,
  never blocks writes)

| scale | process RSS (MB) | ingest docs/s | p50 /search (ms) | p95 /search (ms) | p99 /search (ms) | envelope |
|---|---|---|---|---|---|---|
| 1 000  | 166 |  321 | 16.03 | 17.98 | 19.20 | OK |
| 5 000  | 172 |  175 | 32.36 | 50.88 | 56.08 | OK |

**Reading the numbers:**

- **RSS is flat at ~166–172 MB** across +5 000 docs (6 MB total growth).
  model2vec's `StaticModel` (~120 MB) is the fixed cost; the int8 + binary
  vec0 indexes + mmap'd SQLite keep the variable cost near zero. The 512 MB
  ceiling has ~340 MB of headroom at this scale on a 30 GB host.
- **p95 /search stays under 51 ms** at 5 000 docs — 4× under the 200 ms UX
  ceiling for the OpenClaw plugin's turn loop. Latency grows with corpus size
  (vec0 KNN + FTS5 are both indexed); the Ryzen 2700U is slower per-core than
  the dev M1 Pro but still well inside the envelope.
- **Ingest throughput drops from 321 → 175 docs/s** as the index grows —
  expected, since each insert updates both the FTS5 shadow table and the vec0
  int8+binary indexes. The mini PC's older x86 cores are noticeably slower than
  the M1 Pro proxy (1772 → 321 docs/s at 1k), but ingest remains comfortably
  above interactive rate.
- **The envelope gate passed at both scales** (`bench` exit 0).

**Honest ceiling — 10k scale not measured:** the bench fires `/add` as fast as
it can; at 10k docs in <60s it trips the server's hardcoded loopback rate
limit (10 000 req/60s, `src/main.rs:RateLimiter`). The 1k+5k run stays under
the limit (6k requests). To measure 10k+ on this host, either raise the
loopback rate limit, exempt loopback in `rate_limit_middleware`, or add a
small inter-request delay in `bench`. Tracked as a follow-up; the 5k numbers
already demonstrate 10× headroom under the docs ceiling (50 000).

#### M1 Pro dev-host proxy (superseded by the mini PC run above)

Captured on an Apple M1 Pro (16 GB) as a cross-check before the mini PC was
reachable. Faster per-core but a different machine; kept for the delta.

| scale | process RSS (MB) | ingest docs/s | p50 /search (ms) | p95 /search (ms) | envelope |
|---|---|---|---|---|---|
| 1 000  | 183 | 1 772 | 17.38 | 17.86 | OK |
| 5 000  | 184 |   923 | 25.22 | 25.72 | OK |

### Quality (frozen final query set)

> **v1.17.4 smoke run (2026-08-09)** — the 10-doc CI smoke corpus
> (`tests/fixtures/eval_queries.md`, 37 judged queries) on the default
> profile, scratch instance, this dev host. **Not a parity claim** — per the
> protocol, parity rows stay `PENDING` until ≥100 judged queries run on a
> representative corpus on target hardware (incl. 4 GB ARM). Numbers here
> only pin the `brain eval` gate (`brain eval --floor r5=0.85,r10=0.85,mrr=0.85`
> exits 0; `BENCH_RECALL_FLOOR` env drives the CI job).

| Config | recall@5 | recall@10 | nDCG@10 | MRR | precision@k |
|---|---|---|---|---|---|
| BS-3 hybrid+PRF (smoke set) | 0.919 | 0.919 | 0.911 | 0.905 | p@5 0.276 / p@10 0.138 |
| BS-1 dense-only | PENDING | PENDING | PENDING | PENDING | PENDING |
| BS-2 hybrid | PENDING | PENDING | PENDING | PENDING | PENDING |
| BS-3 hybrid+PRF | PENDING | PENDING | PENDING | PENDING | PENDING |
| BS-4 hybrid+PRF+rerank | PENDING | PENDING | PENDING | PENDING | PENDING |
| QMD-1 default | PENDING | PENDING | PENDING | PENDING | PENDING |
| QMD-2 fast/no-rerank | PENDING | PENDING | PENDING | PENDING | PENDING |

### Latency & resources (edge 4 GB ARM)

| Config | p50 lat | p95 lat | cold-start | RSS idle | RSS load | DB size | model-cache | ingest throughput |
|---|---|---|---|---|---|---|---|---|
| BS-1 dense-only | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| BS-2 hybrid | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| BS-3 hybrid+PRF | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| BS-4 hybrid+PRF+rerank | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| QMD-1 default | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| QMD-2 fast/no-rerank | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |

## Set hygiene & anti-overfitting

- **Dev set**: used to develop and ablate PRF/RRF/rerank changes.
- **Validation set**: used to pick thresholds *once*, with a documented ablation.
- **Final set**: used only for the reported numbers above. Never tuned on.
- Re-judging `Relevant:` after observing results invalidates the set.
- Until 100+ judged queries exist on a representative corpus and the rows above are filled
  on both desktop and 4 GB ARM, **no "parity with QMD" claim is permitted**.
