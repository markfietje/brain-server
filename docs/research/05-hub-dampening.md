# Noise-Aware Graph + Hub Dampening (Discern)

**File:** `src/search/graph_ppr.rs` (`type_base_weight`, `dampen_hubs`)

## The problem

The live knowledge graph was ~94% taxonomy noise: `tagged_with` edges
(note → tag noun) dwarfed the ~134 semantic edges, and degree-73/101/150
mega-hubs let PPR mass wash out across tag clouds. Unweighted PPR on such a
graph returns noise. And a query that looked "too vague" to answer (abstention)
never got a graph chance at all.

## The references

- **GAAMA** (arXiv:2603.27910) — **hub dampening** `w_ij · min(1, θ/deg(i))`
  tames mega-hubs; **edge-type weights** separate taxonomy from semantics.
- **MemORAI** (arXiv:2605.01386) — static-type weighting.
- **"Use Graph When It Needs"** (arXiv:2602.03578) — complexity-gated
  activation: engage the graph leg precisely when the estimator says it helps.

## The implementation (v1.12.0 "Discern")

1. **Edge-type weights:** `type_base_weight` — `tagged_with`/`alias_of` → 0.1,
   all other relation types → 1.0. The pair-aggregation SQL groups by
   `relation_type`, scales each group by its type weight, then sums per pair.
2. **Hub dampening:** `SparseGraph::dampen_hubs(θ)` with `HUB_DAMPING_THETA =
   50` — GAAMA's per-source `min(1, θ/deg(i))`, applied to the
   reachable-bounded graph before PPR. Per-source asymmetry is intentional
   (matches the reference). Determinism hardened by sorting edge rows.
3. **Complexity-gated rescue:** `should_attempt_graph_rescue` fires a bounded
   graph-augmented pass **only** when the estimator says `ClarifyQuery`, the
   graph leg isn't already on, and `BRAIN_GRAPH_RESCUE_ENABLED` (default true).
   `abstention_decision` returns `low_confidence` only when `ClarifyQuery` AND
   the final hit list is empty — a successful rescue returns its hits with
   `decision: "ok"`, strictly additive, no behavior regression when the kill
   switch is off.

## Measured ceiling

- θ=50 and the 0.1 type weight are **corpus-calibrated constants, not learned**
  (deterministic + auditable by design).
- The rescue fires only on the would-be-abstention path; a query with no KG
  structure (no entity match → no seeds) still abstains.
- Type weights are static (no query conditioning); concept nodes (GAAMA),
  query-conditioned weights (MemORAI), and noun-phrase seeding remain future
  options. The tag cloud is structural — re-created on every re-ingest.

*Pinned by a regression test that temporarily reverting to the v1.11 arithmetic
fails — the mechanism is proven, not asserted.*
