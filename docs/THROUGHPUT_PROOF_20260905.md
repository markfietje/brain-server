# Throughput Live-Proof Session Log (2026-09-05)

> v1.28.58 "Throughput" — the milestone's live proof record, per the
> execution prompt: bench measured runs (3×, desktop), the same-seed
> determinism pair, the three `/metrics` captures around a parallel burst,
> and the CRA drill baseline. All against a COPY instance — the live
> deployment was untouched.

## Environment

- Copy instance: `BIND_PORT=18765`, fresh scratch DB, opaque-token auth
  (`AUTH_TOKEN_FILE`, 0600). Release build (`cargo build --release
  --features bench --bin brain-server --bin bench`).
- Corpus: 1 000 synthetic docs (`BENCH_SCALES=1000`), ingest ≈ 1 050–2 500
  docs/s on this desktop box.
- Rate-limit arithmetic observed: the per-IP limiter is 10 000 req/min; a
  default-scales run (1k+5k+10k cumulative ingest) trips it — every
  measurement run here stayed ≈ 2 700 requests, far under the budget. The
  CI `bench-concurrency` job (~1 800 requests) has ample margin.

## Concurrent bench — three measured runs (desktop, BENCH_CLIENTS=8)

`BENCH_CLIENTS=8 BENCH_SEARCHES=200 BENCH_SCALES=1000`

| Run | ops ok | failures | p50 (ms) | p95 (ms) | p99 (ms) | max (ms) |
|---|---|---|---|---|---|---|
| 1 | 1600 | 0 | 20.67 | 22.86 | 24.52 | 50.92 |
| 2 | 1600 | 0 | 20.39 | 22.28 | 23.33 | 26.23 |
| 3 | 1600 | 0 | 20.89 | 23.07 | 24.13 | 30.89 |

Per-client skew across all runs: 8×200 ops, evenly — p50 spread between
clients < 1 ms; the deterministic mix means divergence would be server-side
queuing, and none was observed.

**Ceiling derived:** desktop `search_p95_ms_ceiling` = 60 ms (worst run
23.07 + ~2.5× margin). Jetson stays 150 ms, unmeasured pending a device
run (no ARM runner — the known repo CI gap).

## Same-seed determinism pair (BENCH_SEED=42, twice)

| Run | ops ok | failures | p50 (ms) | p95 (ms) | p99 (ms) | max (ms) |
|---|---|---|---|---|---|---|
| d1 | 1600 | 0 | 20.81 | 22.98 | 24.17 | 27.86 |
| d2 | 1600 | 0 | 21.44 | 23.59 | 24.71 | 36.08 |

Structural diff (non-latency columns) between the two merged reports:
**identical** — same total ops ok, same failures, same per-client counts
(8 × 200). Latency values are timing physics and vary within noise
(p95 spread ≈ 2.7%); the seeded mix makes every breach reproducible.

## `/metrics` captures — before / during / after a parallel burst

Burst: `BENCH_CLIENTS=8 BENCH_SEARCHES=800 BENCH_SCALES=10` (6 400
searches, 0 failures). A `/health/db` scrape preceded capture 1 to
populate the WAL snapshot (the PASSIVE-checkpoint PRAGMA lives only
there).

**Capture 1 — BEFORE:**

```
brain_pool_in_use{domain="global"} 0
brain_pool_idle{domain="global"} 20
brain_pool_timeouts_total 0
brain_busy_errors_total 0
brain_wal_pages_pending{domain="global"} 0
```

**Capture 2 — DURING (8-client search phase in flight):**

```
brain_pool_in_use{domain="global"} 5
brain_pool_idle{domain="global"} 15
brain_pool_timeouts_total 0
brain_busy_errors_total 0
```

**Capture 3 — AFTER:**

```
brain_pool_in_use{domain="global"} 0
brain_pool_idle{domain="global"} 20
brain_pool_timeouts_total 0
brain_busy_errors_total 0
```

The pool-saturation gauge moves 0 → 5 → 0 with the burst; the counters
stay at 0 because nothing waited 30 s for a slot and no write BEGIN
burned through busy_timeout — the honest no-contention reading, not a
wired-off gauge. `brain_wal_pages_pending` appears only after a
`/health/db` scrape, per the cold-path design.

## CRA drill baseline (tabletop, 2026-09-05T04:40:18Z)

`scripts/cra-report-drill.sh` — fabricated actively-exploited-vulnerability
notice against the current release; filled 24 h template + timing report
in `dist/cra-drill/` (and `/tmp/cra-drill-final/` for this record):

| Step | Elapsed since awareness |
|---|---|
| Classified trigger | 0 s |
| Artifacts assembled (SBOM + version matrix + audit posture) | 0 s |
| 24 h template drafted | 0 s |
| "Sent" (tabletop receipts) | 0 s |
| **Total drill elapsed** | **0 s of the 86 400 s budget** |
| 72 h notification due | 2026-09-08T04:40:18Z |
| Final report due | 2026-10-05T04:40:18Z |

The timings are machine-fast because the tabletop is deterministic shell
work — the rehearsal value is the artifact walk (SBOM located, version
matrix consulted, template filled, channels named), not the stopwatch.
Baseline archived per the DSAR-drill precedent.
