# Centroid Domain Auto-Routing (carving the store)

**File:** `src/domain_router.rs` (`mean_vector`, `route`, `route_domain_label`)
· `src/config.rs` (`DOMAIN_CONFIDENCE_THRESHOLD`, `BRAIN_DOMAIN_MIN_COUNT`)

## The problem

A single embedding store mixes unrelated corpora (engineering notes, HR policy,
a client's GDPR posture). Retrieval is cheapest and cleanest when a query is
answered **within one domain** (strict isolation — no cross-"noise") and only
falls back to federating across domains when no single domain is confident. The
question: how to decide, at query time and at ingest time, *which* domain a
chunk or query belongs to — **deterministically**, with no learned router and
no data egress.

## The reference

- **Nearest-centroid classification** — represent each class by its
  arithmetic-mean prototype vector and assign a query to the nearest prototype
  by a similarity measure. The mean-vector class prototype is the Rocchio
  relevance-feedback idea (Rocchio, 1971, "Relevance Feedback in Information
  Retrieval"), and the same mean-of-class prototype reappears as the *support
  set prototype* in prototypical networks (Snell *et al.*, "Prototypical
  Networks for Few-shot Learning", 2017). It is the cheap, fully reproducible
  baseline every vector-RAG router cites.
- The **confidence threshold + fallback** pattern (route when a *margin* of
  confidence exists, else federate) mirrors one-vs-rest margin decisions; the
  deterministic tie-break is brain-server's own (alphabetical) for reproducible
  output.

## The implementation (v1.0.0 "Domains"; query/ingest routing wired v1.13.0)

1. **Centroid is an arithmetic mean of raw f32 vectors** (`mean_vector`): each
   domain's mean embedding, stored once in the global DB as `domain_centroids`
   (a raw le-bytes blob). Compute sources the *live* `vec_knowledge` int8 index
   (`read_domain_vectors`, dequantized via `decode_embedding`), not the legacy
   frozen `embeddings` table — the v1.13.0 fix that stopped centroids silently
   zeroing on live DBs.
2. **Query routing** (`route`): cosine(query, centroid) for every domain;
   keep the single best above `DOMAIN_CONFIDENCE_THRESHOLD` (default 0.30),
   ties broken alphabetically for determinism. Below the threshold → `None` →
   non-strict recall **federates** across domains and labels each hit with its
   source domain. Pure + deterministic, unit-tested.
3. **Ingest routing** (`route_domain_label`): a caller-forced domain always
   wins; otherwise the chunk's own embedding routes the same way, falling back
   to `global` when no centroid clears the threshold. Back-compat: a fresh DB
   with no centroids behaves exactly as before (everything lands in `global`).
4. **Centroid lifecycle** (`recompute_centroid` / `recompute_all_centroids`):
   an idempotent post-migration sweep rebuilds every domain's centroid from the
   corrected M1 source; a domain below `BRAIN_DOMAIN_MIN_COUNT` (default 1, a
   no-op) drops its centroid so `route()` stops sending traffic to an empty
   bucket. Superseded chunks (`valid_to IS NULL`) are excluded so a centroid
   isn't pulled toward outdated content.

## Measured ceiling

- The centroid is a **plain arithmetic mean, not learned** — the documented
  (and unit-tested) upgrade path is a per-domain probe-set or SVM if a corpus
  needs sharper separation. Routing confidence is one cosine threshold, not a
  calibrated probability.
- Strict routing **hard-isolates**: a confident route searches that domain
  exclusively and cannot see a better answer in another domain. Both directions
  of the isolation tradeoff are deliberate — the threshold + federation
  fallback is the escape valve.
- `DOMAIN_MIN_COUNT = 1` means a single-vector domain keeps a centroid that is
  *exactly* that vector (nothing suppressed) unless the operator raises the
  floor.
- This is the routing *decision*; the per-route authorization that scopes a
  scoped reader to their granted domain(s) is the separate read-seam in
  `auth.rs`/`gate.rs` (v1.27.x), not this module.

*Pinned by the unit tests (`route_picks_best_above_threshold`,
`route_returns_none_below_threshold`, `route_domain_label_is_deterministic`) —
the routing arithmetic is proven, not asserted.*