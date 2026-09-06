# Headroom Live-Proof Session Log (2026-09-05)

> v1.28.59 "Headroom" — the milestone's live-proof record, per the execution
> prompt: the `brain_wal_pages_pending` trajectory during an ingest burst
> before vs after tuning, the durability echo, and the lock-wait gauges'
> first live readings. All against a COPY instance — the live deployment was
> untouched.

## Environment

- Apple M1 Pro (10 cores), 16 GB, macOS (Darwin 25.6.0), arm64.
- Copy instance: `BIND_PORT=18765`, fresh scratch DB per run
  (`BRAIN_DB_PATH=/tmp/headroom-proof/brain.db`), opaque-token auth
  (`AUTH_TOKEN_FILE`, 0600). Release build (`cargo build --release
  --features bench --bin brain-server --bin brain --bin bench`).
- Harness: `/tmp/headroom-proof/proof.sh` — `/health` probe, `/metrics`
  grep, `/health/db` durability + concurrency echo, WAL scrape.
- Corpus/load per cell: `BENCH_SCALES=2000 BENCH_SEARCHES=200
  BENCH_CLIENTS=8` — 2 000 docs ingested, then the 8×200 concurrent search.

## Boot-time durability echo (the new `/health/db` keys)

Defaults (BEFORE cell) — the behavior-neutral posture the envelope pin
`envelope_defaults_equal_current_behavior` demands:

```json
{
 "durability": {
  "capacity_target": "jetson",
  "synchronous": "full",
  "wal_autocheckpoint_pages": 1000
 }
}
```

Tuned (AFTER cell — `BRAIN_WAL_AUTOCHECKPOINT=256 BRAIN_SYNCHRONOUS=normal`):

```json
{
 "durability": {
  "capacity_target": "jetson",
  "synchronous": "normal",
  "wal_autocheckpoint_pages": 256
 }
}
```

The env override path works end-to-end: fail-closed parse at boot →
per-connection init beside `busy_timeout` → static echo. `PRAGMA
synchronous` is per-connection, so the init closure (not the one-shot
migration) is what makes the policy real on every pooled connection.

## WAL trajectory — 2 000-doc bench cells (the BENCHMARKS.md table)

30 × `/health/db` scrapes at 150 ms while the bench runs:

```
BEFORE (full/1000):  0 ×30        (no pages pending at any scrape)
AFTER  (normal/256): 0 ×30        (no pages pending at any scrape)
```

Concurrent bench merged rows (identical corpus/load):

```
BEFORE:  1600 ok | 0 fail | p50 21.28 | p95 24.52 | p99 93.33 | max 130.42
AFTER:   1600 ok | 0 fail | p50 21.20 | p95 24.19 | p99 90.00 | max 117.44
```

Ingest rate: 1182 docs/s (BEFORE) vs 1155 docs/s (AFTER).

## WAL trajectory — 6 000-doc ingest burst (the one mechanistic delta)

Single-client ingest burst, 40 × scrapes at 250 ms (burst completes in
seconds, so most samples land post-drain):

```
BEFORE (full/1000):  {'global': 0} ×19, {'global': 34} ×1   ← transient peak
AFTER  (normal/256): {'global': 0} ×21                      ← flat
```

The 1 000-page threshold lets a 34-page WAL accumulate transiently mid-burst
before the autocheckpoint (or the scrape's PASSIVE row) drains it; the
256-page ceiling keeps it at zero. That is the checkpoint-lag knob doing
exactly what it says — available to operators, defaulted OFF (defaults equal
today's behavior).

## Lock-wait gauges — first live readings

```
BEFORE:  brain_lock_wait_micros_p50 0      brain_lock_wait_micros_p95 10
AFTER:   brain_lock_wait_micros_p50 10     brain_lock_wait_micros_p95 10
(6000-doc burst, tuned instance earlier in the session: p50 10 / p95 50)
```

µs-scale bucket edges on every reading: the request-path locks carry no
meaningful contention at desktop load. Counters stayed at 0 the whole session
(`brain_pool_timeouts_total`, `brain_busy_errors_total`) — the honest
no-contention reading, not a wired-off gauge (the fast-path/no-record pin
proves the gauges record when contention exists; the live numbers show it
doesn't, at this load).

## Ceilings (honest)

- Single-site desktop run; Jetson envelope unmeasured (no ARM runner — the
  standing repo CI gap). `capacity_target` echoed `jetson` (the conservative
  default) on this desktop box.
- The 6 000-doc transient is ONE sample, not a distribution.
- RSS varies with corpus size and dev-box state; not a durability signal and
  not reported as one.
- The `/health/db` scrape itself runs the PASSIVE checkpoint — each sample is
  also a drain event. The trajectory is "pending at scrape time", the same
  semantics .58 pinned.
