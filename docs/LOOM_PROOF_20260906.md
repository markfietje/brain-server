# Loom Live-Proof Session Log (2026-09-06)

> v1.28.60 "Loom" — the milestone's live-proof record, per the execution
> prompt: an ingest burst with `BRAIN_LOOM=0` then `=1` (wall-clock delta +
> RSS delta), the byte-equality check, and the `/health/db` echo in all four
> states. All against COPY instances — the live deployment was untouched.

## Environment

- Apple M1 Pro (10 cores), 16 GB, macOS (Darwin 25.6.0), arm64.
- Copy instances: ports 18765–18767, `BRAIN_DB_PATH=<scratch>/brain.db`,
  each a fresh `cp` of the live `~/.openclaw/workspace/brain.db`
  (~48 MiB, 8 790 docs) so every burst started from an identical state.
  Tokenless loopback (no `AUTH_TOKEN_FILE` on the scratch servers).
- Binary: release build `--features bench,loom` (`target/release/brain-server`,
  rayon 1.12.0 linked — verified via `strings`), plus the default-feature
  build (`/tmp/brain-server-noloom`) for the `off:no-feature` echo.
- Load: UMP batch `POST /ingest?format=ump` — the site-1 fan-out path.
  Burst A: 500 records × ~450 B. Burst B: 80 records × ~4.5 KB
  (366 KiB body — under the shared 1 MiB body cap).
- Byte-equality: `sha256` over `SELECT rowid, hex(vectors) FROM
  vec_knowledge_vector_chunks00 ORDER BY rowid` (the sqlite3 CLI cannot load
  the vec0 module; the shadow tables are the same bytes).

## The `/health/db` echo — all four states

| Binary | Target | `BRAIN_LOOM` | Echo |
|---|---|---|---|
| `bench,loom` | desktop | `1` | `active (4 threads)` |
| `bench,loom` | desktop | `0` | `off:env` |
| `bench,loom` | jetson | `1` | `off:jetson` |
| default (no loom) | desktop | `1` | `off:no-feature` |

Fail-closed boot refusal, live: `BRAIN_LOOM=yolo` → the process exits before
serving with `error: fatal loom config: BRAIN_LOOM='yolo' is invalid; must be
0 or 1`. Jetson never looms even when the operator asks; a no-feature binary
never looms either. `cap_from(10) = 4` — the pool carried exactly 4 threads.

## Determinism — the load-bearing result

The full vector index is byte-identical between the loom and serial postures
after every burst (identical starting copies, identical payloads):

```
after burst A (9 291 vectors): ea8bb05299c3e1bf… == ea8bb05299c3e1bf…
after burst B (9 371 vectors): 8c47ce74ff83bad241bb… == 8c47ce74ff83bad241bb…
```

Both runs created exactly 500 / 80 rows with identical id ranges
(12055..12554, then the big notes) — the ordered collect preserved chunk
sequence exactly as `loom_preserves_fused_ranks` and
`loom_batch_order_invariant` pin at the unit level. Eval floors, run after
each fan-out commit in BOTH postures, landed identical to three decimals:
`r@5=0.976 r@10=0.991 mrr=0.956` (floors 0.85) — 25-doc corpus, 106 queries.

## Wall-clock + RSS (paste-the-numbers cell)

| Burst | Posture | Wall | RSS during burst | Created |
|---|---|---|---|---|
| A: 500 × 450 B | LOOM=1 | 1.60 s | +5.4 MiB (186.0→191.4 MB) | 500/500 |
| A: 500 × 450 B | LOOM=0 | 1.06 s | +10.5 MiB (326.9→337.4 MB) | 500/500 |
| B: 80 × 4.5 KB | LOOM=1 | 0.48 s | +4.9 MiB | 80/80 |
| B: 80 × 4.5 KB | LOOM=0 | 0.49 s | +2.5 MiB | 80/80 |

**The honest reading: the static potion tier is not CPU-bound enough for the
fan-out to pay at these sizes** — per-item `encode_one` on the potion model
is µs-scale, and the pre-pass (content collection + ordered fan-out + one
extra collect) costs about what the parallelism saves. Burst A's 0.54 s gap
is confounded by run order (the loom instance ran first against a cold OS
page cache over a fresh 48 MiB DB copy; the serial instance ran second,
warm) — burst B, same order, came out even. What the tier is FOR is the
CPU-bound enterprise neural profile (bge-m3, ~ms-per-item encode), which
this session did not measure (no HuggingFace download in scope).

RSS: both postures stayed far under the envelope's 512 MiB `max_rss_mib`;
burst-time deltas are single-digit MiB either way. The boot-RSS baseline
asymmetry between the two instances (187 vs 327 MB) is dev-box state, not a
loom signal, and is reported for completeness only.

## Envelope re-measured (jetson untouched)

The copy instance's `/health/db` capacity echo during the session:
`docs 8790→9371 / max_docs 10000`, `db_mib 49 / max_db_mib 512`,
`rss_mib 190 / max_rss_mib 512`, `status: ok` — the Headroom envelope fields
are untouched by Loom (no new envelope knobs; the durability echo is
byte-identical to v1.28.59's).

## Ceilings (honest)

- Static-profile throughput is neutral-to-slightly-negative for the fan-out;
  the value case is the neural tier, UNMEASURED here.
- Run order was not randomized (loom first both pairs); the burst-A gap is
  therefore not attributed to loom.
- One site exercised live (batch ingest); site 2 (the near-dup scan's
  preprocessing fan-out) is covered by the unit pins + byte-identity of the
  scan inputs, not by a dedicated live run — the scan's KNN loop is
  connection-bound and stays serial by design.
- Jetson hardware unmeasured (no ARM runner — the standing CI gap); the
  jetson row above is the RESOLVER's verdict on this desktop box.
