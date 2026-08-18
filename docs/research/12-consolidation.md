# Deterministic Consolidation: Duplicates, Conflicts & Stale Sources (the reviewable sweep)

**File:** `src/consolidate.rs` (`find_near_duplicates`, `find_subject_conflicts`,
`find_stale_sources`) · surfaced by `POST /consolidate/propose` + `brain consolidate`

## The problem

A growing store accretes duplicates, near-duplicates, contradictory beliefs
about the same subject, and chunks whose source file was deleted. Left alone,
these silently degrade recall (a false answer you once believed survives
because nothing ever flagged it as superseded or duplicated). The challenge:
**detect exactly these** over a live corpus deterministically, without an LLM
in the hot path and without ever mutating content — the operator stays the only
writer.

## The reference

- **Record linkage / duplicate detection** — the classic Fellegi–Sunter +
  blocking idea: group *blocks* by a cheap key (here the subject key formed
  from `title`/`heading_path`) and compare only within a block, so pairwise
  cost is bounded by block size, not corpus size.
- **Near-duplicates via embedding cosine** — the `web near-duplicate`
  clustering line (e.g. shingles-as-vectors / vector cosine thresholds as a
  near-dup signal). brain-server uses KNN to bound it: each chunk's *nearest
  neighbor* (`k=2 = self + nearest`), not all pairs, via the existing vec0
  index.
- **Conflicts as typed evidence links** (`supersedes`/`contradicts`) — the
  "atomic supersession, faithful resolution" design: a correction *links*, it
  never anonymizes the old belief (bi-temporal retention).

## The implementation (v1.8.0 "Reviewable proposals"; v1.20.18 grouping fix)

1. **Exact duplicates** — separate content-hash pass: two chunks with the same
   content are flagged regardless of title (dedup is not a near-dup threshold).
2. **Near-duplicates** (`find_near_duplicates`, v1.8.0, hardened v1.20.18) —
   for each *current* chunk (`valid_to IS NULL`), run the existing vec0 KNN
   (`k=2`: self + nearest), dequantize via `decode_embedding`, and propose a
   pair when cosine > `threshold` (parameter default 0.95 — very high, only
   propose when confident). Bounded **O(n×k)** via KNN, **not O(n²)** pairwise;
   re-quantization via `vec_quantize_int8` matches the `/recall` value, so the
   int8 quantization error is the same bounded error recall already lives with
   (and which the 0.95 threshold tolerates). `max_pairs` caps the output —
   the proposal endpoint is a review queue, not a dump truck.
3. **Subject conflicts** (`find_subject_conflicts`, v1.8.0) — group *current*
   rows by subject key (`COALESCE(title, heading_path)`), exclude rows superseded
   (an incoming `supersedes` link) or from a deleted/tombstoned source, and
   flag pairs that share a subject but differ in content. Each pair carries
   `age_gap_secs` + `authority_delta` so the operator can see which is
   newer/more authoritative. v1.20.18 regrouped the scan by subject key to
   collapse the O(n²) to O(Σ m² per subject) — ~linear on mostly-unique
   subjects — and sorted the output for determinism.
4. **Stale sources** — `find_stale_sources`: chunks whose `source` file was
   deleted from the vault (the v1.8 `stale sources` proposal). Pure detection;
   `POST /sources/reconcile` separately sweeps orphans.
5. **Nothing is mutated** — all pure detection returning proposals; a human
   applies them via `/consolidate/apply` (typed links) or `brain undo-resolve`,
   and every apply is audit-recorded. The write-once invariant:
   consolidation *detects + links*, it never deletes.

## Measured ceiling

- **Subject key = `title`/`heading_path` only, no NER** (documented): two
  chunks about "the API key" under different titles are *not* flagged. The
  upgrade path feeds the `entities` table into the subject key.
- The 0.95 near-dup threshold is a **conservative parameter default, not
  calibrated** — it trades a few missed near-dups for essentially zero false
  positives.
- Runs **on-demand** (`brain consolidate` / `/consolidate/propose`), never in
  the recall hot path; the conflict scan is still quadratic *within* a single
  heavily-duplicated subject (inherent to the pairwise rule).
- It is **visibility, not action**: proposals surface decisions; a human still
  makes them. No cron, no autonomous edit.

*Pinned by the unit suite (`find_subject_conflicts_*`,
`find_near_duplicates_*`, exact-dup, stale-source cases) — the detection
arithmetic is proven, not asserted.*

---

*Part of the deterministic-retrieval explainer series. The near-dup + conflict
detection is the store's self-consistency layer (Duplicates / Conflicts /
Stale in the `consolidate` vocabulary), complementing the bi-temporal lineage in
[`01-bi-temporal.md`](./01-bi-temporal.md) and the trace edges in
[`03-trace-edges.md`](./03-trace-edges.md).*