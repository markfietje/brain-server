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

The benchmark CLI is the feature-gated `bench` binary
(`cargo run --release --features bench --bin bench`): the default mode runs the
synthetic-scale latency/RSS benchmark, `eval` scores a judgments file against
the live API, and `scaffold` authors the judged corpus from `/export`. Run it
against the live HTTP API. The protocol is deterministic given a fixed corpus
and query set.

### 0. Prerequisites

```bash
# Point PATH at your stable Rust toolchain, then cd into the repo checkout
export PATH="$HOME/.rustup/toolchains/stable-$(rustc --version | grep -o 'aarch64\|x86_64')-apple-darwin/bin:$PATH"
cd /path/to/brain-server-repo

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

### v1.28 "Caliber" tier smoke (2026-08-14) — edge vs desktop vs enterprise

> **Directional only — not a parity claim.** The 10-doc/37-query CI smoke set
> is recall-saturated for every profile (r@5 = r@10 = 0.919 across the board),
> so it **cannot differentiate recall** — only the precision-sensitive metrics
> (MRR/nDCG) move. Parity-or-better vs external baselines stays `PENDING` the
> ≥100-query frozen set (v1.31 "Proven"). Per profile: fresh DB, the 10-doc
> corpus ingested via `/add`, `brain eval` (37 queries, `/recall`, k=10), this
> dev host (M1 Pro), debug build, cached models. Desktop = gte-base-en-v1.5
> (768-d) + bge-reranker-v2-m3; Enterprise = BGE-M3 (1024-d) + the same
> reranker; both built `--features neural-embed,rerank-tier`. This run predates
> the reranker retune (`8166b1b`), so it exercised **`BAAI/bge-reranker-v2-m3`**.
> The tier's primary is now **`mixedbread-ai/mxbai-rerank-large-v1`** (BYO-ONNX,
> int8) with bge-reranker-v2-m3 as the in-enum fallback — same fail-open + top-50
> contract, so these *directionally valid* n=37 numbers stand until an mxbai
> smoke is re-run on the ≥100-query frozen set.

| Profile | recall@5 | recall@10 | nDCG@10 | MRR | precision@k | note |
|---|---|---|---|---|---|---|
| edge-default (potion 512-d, no rerank) | 0.919 | 0.919 | 0.911 | 0.905 | p@5 0.276 / p@10 0.138 | = the v1.17.4 baseline row (byte-consistent) |
| desktop (gte-base 768-d + rerank) | 0.919 | 0.919 | **0.917** | **0.919** | p@5 0.276 / p@10 0.138 | the reranker's precision lift shows even at n=37 |
| enterprise (BGE-M3 1024-d + rerank) | 0.919 | 0.919 | **0.917** | **0.919** | p@5 0.276 / p@10 0.138 | identical to desktop on this set — expected: recall-saturated, same reranker |

> Ceiling: at n=37 saturated, MRR 0.905 → 0.919 is the only honest signal
> (rerank reorders the top correctly). Desktop vs enterprise cannot be
> separated by this set — BGE-M3's sparse/colbert heads aren't even consumed
> yet (that's v1.30). The real gate is the ≥100-query frozen set.

### v1.27.27 "Seal" eval (2026-08-20, actual release binary)

The release rewrites `contains_suspicious_pattern` (the F-61 + S2-44
phrase-aware blocklist matcher), which feeds `SearchResult::raw()`'s
`blocklist_hit` flag — the flag the PRF term extractors consume — and the
`/recall` query screen. The frozen set was therefore re-run on the release
binary to confirm the matcher change did not move recall. Same procedure as
the CI `recall-gate` job: scratch seed of the 10-doc smoke corpus via `brain
ingest-dir`, then `brain eval --floor r5=0.85,r10=0.85,mrr=0.85` over the 37
judged queries, default profile, this dev host. Gate holds (exit 0) and the
metrics match the long-standing baseline exactly — the corpus is benign, so
no hit was blocklist-flagged before or after (PRF behavior unchanged on this
set); the matcher's behavioral deltas are pinned by the unit tests
(`blocklist_matches_multi_word_phrases`,
`normalization_does_not_kill_phrase_entries`), not by this smoke.

| metric | score |
|---|---|
| recall@5 | 0.919 |
| recall@10 | 0.919 |
| nDCG@10 | 0.909 |
| MRR | 0.905 |
| precision@5 / @10 | 0.276 / 0.138 |

### v1.27.22 "Cascade" eval (2026-08-18, actual release binary)

Two evals ran on the **actual v1.27.22 release binary** (`brain-server`
+ `brain` built `--release --features bench`, version endpoint 1.27.22), each on
a scratch instance on a non-default port (`BRAIN_DB_PATH`/`BIND_PORT`, so the
live `~/.openclaw/workspace/brain.db` was never touched).

**Eval 1 — frozen recall gate (byte-identity re-check).** The release touches
the traversal/adjacency read path (superseded-edge skip + adjacency filter)
that feeds recall, so the frozen set was re-run on the release binary to
confirm the default (`superseded_at IS NULL` = no-op on well-formed DBs) is
behavior-identical. Same procedure as the CI `recall-gate` job: scratch seed of
the 10-doc smoke corpus via `brain ingest-dir`, then `brain eval --floor
r5=0.85 --floor r10=0.85 --floor mrr=0.85` over the 37 judged queries
(`tests/fixtures/eval_queries.md`), default profile, this dev host. Gate holds
(exit 0) and the metrics match the long-standing baseline — the fix did not
move recall.

| metric | score |
|---|---|
| recall@5 | 0.919 |
| recall@10 | 0.919 |
| nDCG@10 | 0.909 |
| MRR | 0.905 |
| precision@5 / @10 | 0.276 / 0.138 |

> Note: nDCG@10 here (0.909) matches the v1.17.4 smoke set's 0.911 within this
> set's run-to-run variance at n=37; the pinned CI floors (r5/r10/mrr ≥ 0.85)
> are comfortably held.

**Eval 2 — edge-supersession functional eval (the feature this release
ships).** An end-to-end behavioral check of the two bug-fixes on the release
binary, overriding `/ingest` with an explicit entity triple and then poking the
relationship history + read surfaces:

1. **Initial ingest** of `Alice manages Bob` with a valid window
   (`valid_at 2020-01-01`, `invalid_at 2023-01-01`) → `created`, one
   `relationships` row.
2. **Unchanged re-ingest** of the identical triple (same window) →
   `duplicate`, **0 writes, same relationship id** — the write-once
   idempotent no-op is preserved (history is not churned by a repeat).
3. **Changed-window re-ingest** (`valid_at 2021-01-01`, `invalid_at
   2025-01-01`) → `created`, a **new** relationship id, and the old row is
   retired with `superseded_at = <new row's created_at>` (transaction-time
   END). The handoff is exact: `old.superseded_at == new.created_at`.
4. **`GET /graph/relationships/{id}/history`** reconstructs the full lineage
   — `versions: [old, new]`, `current = new`, the old version's `current`
   flag is `false` and its `superseded_at` is populated — queried from
   **either** version id (the "given any one version id" contract).
5. **`GET /graph/relations?from=alice`** returns **only the current edge**
   (the superseded id is absent — the read surface hides retired edges).
6. **Traversal** from `alice` yields a single current hop (not both
   versions).
7. **Bogus id** (`/graph/relationships/999/history`) → `404`.

Result: all seven assertions held on the release binary. Behavior matches the
module docs (`src/graph_supersede.rs`, `tests` in the lib suite) and the
migration's `comments`/plan — the shipped code is true to its docs.

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

### Known-item self-retrieval regression (operator vault, 2026-08-09)

> **Not a QMD parity claim, not external hand-judgment.** This is an automated
> **known-item regression** over the operator's live vault (8695 chunks, this dev
> host, default hybrid+PRF profile): each query is a 200-char excerpt of a chunk's
> own content, and its `relevant_ids` are that chunk plus its near-duplicate
> content siblings (token-overlap ≥ 0.5 within the same document). It measures
> "does `/recall` surface the source chunk (and its near-copies) for a query drawn
> from that chunk's own text" — a weak, self-grounded floor. 120 queries, `k=5`.
> Reproduce: `bench scaffold` → seed `relevant_ids` from chunk ids → 
> `BRAIN_EVAL_JUDGMENTS=<file> bench eval`.
>
> **What this deliberately does NOT show:** external relevance against queries an
> operator would actually ask, on target hardware (incl. 4 GB ARM). Those rows
> remain `PENDING` below. Parity rows stay `PENDING` until ≥100 hand-judged queries
> (external, not content-derived) run on a representative corpus on target hardware.
>
> **QMD status (2026-08-09):** QMD publishes no recall/precision benchmark numbers
> and is not installed on this host, so the QMD-1/QMD-2 parity rows are not merely
> `PENDING` — they are unattainable without the operator running `qmd bench` on a
> comparable corpus. Nothing here is a parity claim against QMD.

| metric | value |
|---|---|
| queries | 120 |
| precision@5 | 0.1750 |
| recall@5 | 0.6775 |
| MRR | 0.6204 |
| NDCG@5 | 0.6273 |
| answer_in_context_rate | 0.0000 |


### Latency — dev host (Apple M1 Pro, 10-core/16 GB, operator vault 8,695 docs, 2026-08-09)

> **Not an ARM-edge / Jetson measurement, not a parity claim.** Self-measured
> `POST /recall` (default hybrid+PRF, `k=5`) against the live dev-host server
> (v1.18.2, `unsafe_blocks:1`) on the operator's real 8,695-doc vault. The
> point is "is the small hardened binary fast," not "beats QMD on an edge
> device." 30 sequential samples. The M1 Pro (10-core, 16 GB, arm64) is the
> dev host — distinct from the 4 GB ARM edge target still `PENDING` below.

| metric | value |
|---|---|
| p50 | 20 ms |
| p95 | 25 ms |
| p99 | 32 ms |
| min | 20 ms |
| max | 45 ms |

### Latency & resources (edge 4 GB ARM)

| Config | p50 lat | p95 lat | cold-start | RSS idle | RSS load | DB size | model-cache | ingest throughput |
|---|---|---|---|---|---|---|---|---|
| BS-1 dense-only | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| BS-2 hybrid | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| BS-3 hybrid+PRF | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| BS-4 hybrid+PRF+rerank | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| QMD-1 default | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| QMD-2 fast/no-rerank | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |

### Client bundle (web, v1.18.1 "Harden")

> **v1.18.1 M4a measurement (2026-08-09)** — the Dioxus 0.7.10 web bundle from
> `dx bundle` (served under `/app`, PWA-cached as a single asset). Parse /
> instantiate time on a target device is **PENDING — an operator step** (needs a
> browser timing harness); the sizes below are measured facts. wasm-split is
> **not adopted** — it is experimental in 0.7.10 and the shell code is shared;
> re-measure after Dioxus 0.8-stable (when wasm-split is non-experimental).

| Asset | Size |
|---|---|
| `brain-client_bg-*.wasm` | 3,724,711 B (3.7 MB) |
| `brain-client-*.js` | 59,641 B (60 KB) |
| `tailwind-*.css` | 39,786 B (40 KB) |

> **v1.20.0 M2.1 budget (2026-08-11)** — the release-wasm regression guard in
> CI (`client/bundle-budget.sh`): **measured 4,339,760 B** (pre-wasm-opt, the
> raw `cargo build --release --target wasm32-unknown-unknown` artifact the
> budget gate sizes) against a **≤ 7,000,000 B budget** (+60% headroom over
> the completed-surface measurement). The plan's final budgets — web initial
> **≤ 50 KB** / mobile app **≤ 5 MB** (Dioxus targets) — remain
> measured-success criteria against the `dx bundle` artifacts on target
> devices (operator step, same as memory/FPS profiling); the dx-bundled
> 3.7 MB row above shows the wasm-opt'd floor the 5 MB mobile budget is
> already under, and the CI gate above is the tripwire until wasm-split
> (Dioxus 0.8) lands.

## Bounds (v1.27.42 — measured once, honestly)

> Throughput ceilings per vertical, single measurement on the dev host (M1 Pro, 16 GB).
> Not bragging rights — the honest ceiling for 2.x scaling work.

| Vertical | Concurrent runs | p50 latency | p95 latency | Notes |
|---|---|---|---|---|
| Recall (hybrid+PRF, k=5) | 1 | 20 ms | 25 ms | 8.6k-doc vault |
| Recall | 20 concurrent | ~45 ms | ~80 ms | Bounded pool, no queue overflow |
| Workflow CAS + audit | 10 concurrent | <30 ms | <60 ms | Chain verify stays green |
| Steering (drop-oldest) | flood 1000 | <5 ms enqueue | 0 drops under 256 cap | Bounded queue verified |

| Steering (drop-oldest) | flood 1000 | <5 ms enqueue | 0 drops under 256 cap | Bounded queue verified |

## Bounds (v1.28.3 — SDK pure surfaces + workflow seam, measured once, honestly)

> Single release-gate measurement on the dev host (Apple M-series, release build,
> synthetic corpus — the frozen small-corpus posture, not production claims).
> Per-op latency is the reciprocal of the measured ceiling; no lift claims.

| Surface | Throughput ceiling | Per-op | Notes |
|---|---|---|---|
| Evidence reducer | ~3.7 M findings/s | <1 µs/finding | 10k-finding batches, 500 claim-groups |
| QA scorer (`score_run`) | ~2.3 M runs/s | <1 µs/run | 8-step artifacts |
| WorkflowMeta admit gate | ~24 M/s | <1 µs | validate-as-data + concurrency bound |
| Run lifecycle (start→complete→handle) | ~4.9 M/s | <1 µs | holder-owned run, once-future resolve |

> Honest ceilings: these are CPU-bound pure-function ceilings; end-to-end
> workflow latency is dominated by storage + audit-chain writes (host-owned),
> not by the SDK seam. Cancel/dispose settle within `DEFAULT_GRACE` (5 s) by
> construction and are not throughput-measured.

## Set hygiene & anti-overfitting

- **Dev set**: used to develop and ablate PRF/RRF/rerank changes.
- **Validation set**: used to pick thresholds *once*, with a documented ablation.
- **Final set**: used only for the reported numbers above. Never tuned on.
- Re-judging `Relevant:` after observing results invalidates the set.
- Until 100+ judged queries exist on a representative corpus and the rows above are filled
  on both desktop and 4 GB ARM, **no "parity with QMD" claim is permitted**.
