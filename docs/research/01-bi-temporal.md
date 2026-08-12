# Bi-temporal Knowledge Graph (validity-aware facts)

**File:** `src/temporal.rs` (extraction) · `src/search/mod.rs` (filters)

## The problem

Memory stores usually overwrite a fact when a newer one arrives. That silently
destroys *history* — the one thing an audit-driven agent memory must keep.
When was this fact true? When did it stop being true? A store that answers
those two questions is bi-temporal: it tracks both **valid time** (when the
fact holds in the world) and, via the audit chain, **when the store learned
it**.

## The reference

**Graphiti** (Zep) models an `EntityEdge` with `valid_at`/`invalid_at`
(valid-time) + `expired_at` (wall-clock invalidation) + `reference_time`
(source provenance). The canonical pattern is: on a contradiction, **expire the
old fact, never delete it** (`resolve_edge_contradictions`).

## The implementation

brain-server stores `knowledge.valid_from` / `valid_to` (added v0.9.8, wired
bi-temporal v1.4.0):

- `src/temporal.rs::extract_interval(text, now)` — a **deterministic** marker
  extractor ("from 2011 to 2017", "since 2020", "currently" → `valid_at = now`).
  English, bounded marker set, no LLM.
- The bi-temporal filter used by every retrieval leg is exactly the Graphiti
  shape: `valid_at <= ? AND (invalid_at IS NULL OR invalid_at > ?)`.
- `/recall` and `/graph/traverse` accept `?at=<time>`; `?since=` is normalized
  alongside. Superseding a chunk sets `valid_to = now` (v1.6
  `resolve_supersession`) — the old fact becomes invisible to *default* recall
  but still retrievable with `?at=<past>`.

## Measured ceiling

- Extraction is English-only + deterministic; no relative dates, no inferred
  durations, no LLM extractor (a v2.x option). A fact with no marker simply has
  an open interval.
- Resolving one conflict expires one chunk per call; multi-way conflicts need
  multiple calls.
- The KG (`entities`/`relationships`) has its own `?at=` filter; chunk-level
  supersession is separate from graph-edge temporality.

*See the audit-replay playbook in `COMPLIANCE.md` §3.6 — bi-temporal validity is
what lets you answer "what did the agent believe at time T?"*
