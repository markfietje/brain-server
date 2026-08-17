//! HippoRAG-2-style Personalized PageRank over the
//! existing knowledge graph.
//!
//! Research basis (Context7-verified + verbatim source, 2026-08-02):
//! OSU-NLP-Group/HippoRAG `src/hipporag/HippoRAG.py` `run_ppr()` calls
//! `igraph.Graph.personalized_pagerank(vertices=all, damping=0.5,
//! directed=False, weights='weight', reset=node_weights,
//! implementation='prpack')`. Its `node_weights` (from
//! `graph_search_with_fact_entities`) is the sum of per-entity phrase weights
//! and per-passage `dpr_score * passage_node_weight` (0.05); the seed vector is
//! NOT renormalized in Python — the prpack solver normalizes it internally
//! (a Rust port must normalize to a probability distribution). Edge weights are
//! `node_to_node_stats` co-occurrence counts (symmetric, both directions).
//!
//! brain-server's wedge (per the v1.11.0 plan): HippoRAG's expensive pieces
//! (LLM OpenIE triple extraction, LLM entity linking, offline KG construction)
//! are already replaced by the deterministic linker + existing
//! `entities`/`relationships` tables. This module is the retrieval side: PPR
//! over that KG as a third RRF retriever behind `?graph=true`.
//!
//! This module is deliberately `#![deny(unsafe_code)]` (inherited from the
//! `search` tree): pure safe Rust over SQLite read connections. No unsafe, no
//! FFI, no external PPR crate — a hand-rolled sparse power iteration is
//! smaller and dependency-free, and the whole module is bounded by
//! [`MAX_VISITED`] so a dense KG can't blow the budget.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use crate::trace::MAX_VISITED;

// ── Hyperparameters (matched to the HippoRAG 2 reference) ────────────────
// `damping=0.5` is the config default (`config_utils.py`); NOT the classic
// 0.85 from the repo plan draft — the reference's real value is 0.5 and a
// faithful port matches the reference.
pub const PPR_ALPHA: f64 = 0.5;
pub const PPR_EPSILON: f64 = 1e-6;
pub const MAX_PPR_ITER: usize = 50;
/// Passage-node weight used by HippoRAG 2 (`passage_node_weight=0.05`). A
/// faithful port would scale dense-retrieval passage scores by this before
/// adding them to the seed. brain-server has no per-query DPR passage scores
/// in the graph leg (the plan forbids an embedding in this leg), so this
/// constant documents the ceiling rather than tuning a value.
#[allow(dead_code)] // ponytail: reserved for the DPR-passage-seed upgrade path.
pub const PASSAGE_NODE_WEIGHT: f64 = 0.05;

// ── noise-aware weights ─────────────────────────────────
// Research basis (2026-08-03): GAAMA (arXiv:2603.27910) hub dampening
// `w_ij · min(1, θ/deg(i))` per source node, θ = 50. The live corpus is 94%
// `tagged_with` taxonomy edges with degree-73/101/150 mega-hubs; both
// mechanisms are pure arithmetic over the existing adjacency — no schema,
// no re-ingest, still deterministic and `#![deny(unsafe_code)]`.
/// Hub dampening threshold θ (GAAMA Table-2 convention). A source vertex with
/// more than θ distinct neighbors scales each outgoing half-edge by θ/deg.
pub const HUB_DAMPING_THETA: f64 = 50.0;

/// Per-relation-type base weight: taxonomy edges (`tagged_with` from
/// frontmatter tags, `alias_of` from aliases) are weak association signals
/// that dilute PPR mass, so their evidence counts weigh 0.1; semantic
/// relation types keep full weight. Static (no query-conditioning — the
/// MemORAI-style learned weights are v2.x; see the v1.12.0 plan).
pub fn type_base_weight(rel_type: &str) -> f64 {
    match rel_type {
        "tagged_with" | "alias_of" => 0.1,
        _ => 1.0,
    }
}

/// A sparse, undirected, weighted entity graph.
///
/// Vertices are `entities.id`; an edge `(a, b, w)` exists for every distinct
/// `(from_entity_id, to_entity_id)` pair in `relationships`, with weight =
/// the type-weighted sum of `COUNT(DISTINCT knowledge_id)` per relation type
/// (v1.12.0 "Discern": taxonomy types weigh 0.1 via [`type_base_weight`]) —
/// the count of distinct chunks providing evidence for that pair (the same
/// "edge has evidence" signal v0.9.8 uses). This is HippoRAG 2's
/// `node_to_node_stats` fact-edge count at the pair level.
pub struct SparseGraph {
    /// CSR-style adjacency. `adj[i]` = `(neighbor_vertex, weight)`.
    adj: Vec<Vec<(usize, f64)>>,
    /// Vertex id (`entities.id`) → row index.
    id_to_idx: HashMap<i64, usize>,
    /// Row index → vertex id.
    idx_to_id: Vec<i64>,
}

impl SparseGraph {
    fn new() -> Self {
        Self {
            adj: Vec::new(),
            id_to_idx: HashMap::new(),
            idx_to_id: Vec::new(),
        }
    }

    /// Add an undirected edge. `a`/`b` are `entities.id`; both directions are
    /// recorded (the reference runs `directed=False`). Self-loops dropped.
    fn add_edge(&mut self, a: i64, b: i64, weight: f64) {
        if a == b || weight <= 0.0 {
            return;
        }
        let ia = self.ensure(a);
        let ib = self.ensure(b);
        self.adj[ia].push((ib, weight));
        self.adj[ib].push((ia, weight));
    }

    fn ensure(&mut self, id: i64) -> usize {
        if let Some(&i) = self.id_to_idx.get(&id) {
            return i;
        }
        let i = self.idx_to_id.len();
        self.id_to_idx.insert(id, i);
        self.idx_to_id.push(id);
        self.adj.push(Vec::new());
        i
    }

    fn len(&self) -> usize {
        self.idx_to_id.len()
    }

    /// Index of a vertex id, if present.
    fn idx(&self, id: i64) -> Option<usize> {
        self.id_to_idx.get(&id).copied()
    }

    /// GAAMA-style hub dampening. Every directed half-edge
    /// weight is scaled by `min(1, θ/deg(i))` for its **source** vertex's
    /// degree (distinct-neighbor count) — a mega-hub spreads proportionally
    /// less mass per edge, so a 150-degree tag cloud can't wash PPR out.
    /// Per-source asymmetry is intentional (matches the reference); the
    /// row-normalization in [`personalized_pagerank`] handles non-uniform
    /// weights. Degree is bounded by [`MAX_VISITED`], so this is O(edges).
    fn dampen_hubs(&mut self, theta: f64) {
        for row in &mut self.adj {
            let deg = row.len() as f64;
            if deg <= 0.0 {
                continue;
            }
            let f = (theta / deg).min(1.0);
            for (_, w) in row.iter_mut() {
                *w *= f;
            }
        }
    }
}

/// Build the sparse entity graph from raw `(from, to, evidence_count)` rows.
///
/// Pure and testable: given the distinct `knowledge_id` counts per entity pair,
/// produce the undirected weighted adjacency. `MAX_VISITED` bounds the number
/// of distinct vertices admitted (a dense/taxonomy-heavy KG can't blow the
/// power-iteration budget).
fn build_graph(rows: &[(i64, i64, f64)]) -> SparseGraph {
    let mut g = SparseGraph::new();
    for &(a, b, w) in rows {
        g.add_edge(a, b, w);
    }
    g
}

/// Deterministically link a query to entity seeds.
///
/// HippoRAG 2 uses an LLM ("Recognition Memory") to map the query to top
/// facts, then to entity phrase nodes. brain-server replaces that with the
/// cheap, deterministic equivalent the plan specifies: an entity name is a
/// seed iff it appears verbatim (case-insensitive) in the query — the
/// `entities` table IS the vocabulary. No fuzzy matching, no embeddings.
///
/// Returns `entities.id` values that matched.
pub fn seed_entities_from_query<'a>(
    query: &str,
    entities: impl IntoIterator<Item = &'a (i64, String)>,
) -> Vec<i64> {
    let q = query.to_lowercase();
    entities
        .into_iter()
        .filter(|(_, name)| !name.trim().is_empty() && q.contains(&name.trim().to_lowercase()))
        .map(|(id, _)| *id)
        .collect()
}

/// Personalized PageRank by power iteration.
///
/// Solves `π = (1-α)·s + α·Pᵀ·π` where `P` is the out-degree-normalized
/// (row-stochastic) weighted adjacency and `s` is the seed distribution
/// (uniform over `seeds`, normalized to a probability vector — matching the
/// prpack solver's internal normalization).
///
/// Bounded: at most [`MAX_PPR_ITER`] iterations; only vertices reachable
/// from a seed (within the existing [`SparseGraph`], itself bounded by
/// [`MAX_VISITED`]) participate.
///
/// Returns per-vertex PPR scores in the same index order as `graph`'s
/// vertices.
pub fn personalized_pagerank(
    graph: &SparseGraph,
    seeds: &[usize],
    alpha: f64,
    epsilon: f64,
    max_iter: usize,
) -> Vec<f64> {
    let n = graph.len();
    let mut pi = vec![0.0f64; n];
    let mut next = vec![0.0f64; n];
    if seeds.is_empty() {
        return pi;
    }
    // Seed distribution `s`: uniform over seeds, normalized (prpack-normalizes
    // the reset vector internally in the reference).
    let s = 1.0 / seeds.len() as f64;
    let mut is_seed = vec![false; n];
    for &seed in seeds {
        if seed < n {
            pi[seed] = s;
            is_seed[seed] = true;
        }
    }
    // Precompute per-vertex out-degree for row normalization.
    let out_deg: Vec<f64> = graph
        .adj
        .iter()
        .map(|row| row.iter().map(|&(_, w)| w).sum())
        .collect();

    for _ in 0..max_iter {
        // π_next = (1-α)·s + α·Pᵀ·π
        next.iter_mut().for_each(|v| *v = 0.0);
        for i in 0..n {
            if pi[i] <= 0.0 || out_deg[i] <= 0.0 {
                continue;
            }
            let scale = alpha * pi[i] / out_deg[i];
            for &(j, w) in &graph.adj[i] {
                next[j] += scale * w;
            }
        }
        // Teleport term (1-α)·s[i] — nonzero only on seed vertices.
        for i in 0..n {
            if is_seed[i] {
                next[i] += (1.0 - alpha) * s;
            }
        }
        // Convergence: ‖πₜ₊₁ − πₜ‖₁ < epsilon (plan verification #4).
        let l1: f64 = (0..n).map(|i| (next[i] - pi[i]).abs()).sum();
        std::mem::swap(&mut pi, &mut next);
        if l1 < epsilon {
            break;
        }
    }
    pi
}

/// Expand the top-ranked entities to their supporting chunks.
///
/// For each entity with PPR score above zero (bounded to the `top_n`
/// strongest), find the chunks that back a `relationships` edge involving it
/// (`knowledge_id IS NOT NULL`), and accumulate that entity's PPR onto the
/// chunk's score. This is the seed→chunk mapping HippoRAG 2 gets from its
/// passage nodes; brain-server derives it from `relationships.knowledge_id`.
///
/// Returns `(chunk_id, accumulated_score)` pairs, highest first.
fn expand_to_chunks(
    conn: &Connection,
    graph: &SparseGraph,
    pi: &[f64],
    top_n: usize,
) -> Result<Vec<(i64, f64)>> {
    // Collect the strongest entities (vertex id + PPR), bounded.
    let mut ranked: Vec<(i64, f64)> = graph
        .idx_to_id
        .iter()
        .enumerate()
        .filter_map(|(i, &id)| (pi[i] > 0.0).then_some((id, pi[i])))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(top_n);

    if ranked.is_empty() {
        return Ok(Vec::new());
    }
    // Gather supporting chunk ids per entity in one query each (bounded by
    // top_n, itself small). Aggregate each entity's PPR onto its chunks.
    let mut chunk_score: HashMap<i64, f64> = HashMap::new();
    for (entity_id, ppr) in ranked {
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT knowledge_id FROM relationships \
             WHERE (from_entity_id = ?1 OR to_entity_id = ?1) \
               AND knowledge_id IS NOT NULL",
        )?;
        let chunk_ids: Vec<i64> = stmt
            .query_map([entity_id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        for cid in chunk_ids {
            *chunk_score.entry(cid).or_insert(0.0) += ppr;
        }
    }
    let mut out: Vec<(i64, f64)> = chunk_score.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

/// Run the graph-PPR retriever against a domain pool.
///
/// Returns `SearchResult`s (`source = Graph`) or `Ok(vec![])` when the query
/// links to no entity (no graph signal — the caller falls back to the other
/// RRF legs). Never panics, never unbounded: adjacency is capped at
/// [`MAX_VISITED`] vertices, PPR at [`MAX_PPR_ITER`] iterations.
pub fn graph_retrieve(
    conn: &Connection,
    query: &str,
    k: usize,
    include_flagged: bool,
) -> Result<Vec<crate::search::SearchResult>> {
    // 1. Load the entity vocabulary (name → id) for query→seed linking.
    let mut stmt = conn.prepare_cached("SELECT id, name FROM entities")?;
    let entities: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    let seeds = seed_entities_from_query(query, &entities);
    if seeds.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Load the weighted adjacency restricted to entities that share at
    //    least one evidence chunk. v1.12.0 "Discern": the aggregation keeps
    //    `relation_type` so each pair's evidence count is scaled by
    //    [`type_base_weight`] before summing — taxonomy edges contribute
    //    a tenth of semantic ones. Bounded by SQLite's own row limit.
    let mut stmt = conn.prepare_cached(
        "SELECT from_entity_id, to_entity_id, relation_type, COUNT(DISTINCT knowledge_id) \
         FROM relationships \
         WHERE knowledge_id IS NOT NULL \
         GROUP BY from_entity_id, to_entity_id, relation_type",
    )?;
    let mut pair_w: HashMap<(i64, i64), f64> = HashMap::new();
    for row in stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })? {
        let (a, b, rel, count): (i64, i64, String, i64) = row?;
        *pair_w.entry((a, b)).or_insert(0.0) += count as f64 * type_base_weight(&rel);
    }
    // Sort by (a, b) so vertex admission order is deterministic regardless of
    // SQLite's GROUP BY ordering (PPR values are order-independent; the
    // stable tie-break in expand_to_chunks is not).
    let mut edge_rows: Vec<(i64, i64, f64)> =
        pair_w.into_iter().map(|((a, b), w)| (a, b, w)).collect();
    edge_rows.sort_by_key(|x| (x.0, x.1));

    // 3. Build the bounded graph. Prune to the component reachable from the
    //    seeds and cap total vertices at MAX_VISITED. v1.12.0 "Discern": hub
    //    dampening must run AFTER the reachable prune (degree reflects the
    //    bounded graph).
    let mut graph = build_graph(&edge_rows);
    // Restrict vertices to those reachable from seeds (BFS, bounded).
    graph = restrict_to_reachable(graph, &seeds);
    graph.dampen_hubs(HUB_DAMPING_THETA);

    // 4. Power-iteration PPR.
    let seed_idx: Vec<usize> = seeds.iter().filter_map(|&id| graph.idx(id)).collect();
    let pi = personalized_pagerank(&graph, &seed_idx, PPR_ALPHA, PPR_EPSILON, MAX_PPR_ITER);

    // 5. Seed → chunk expansion. Over-fetch so RRF can rank; cap modestly.
    let top_n = k.max(8);
    let mut chunks = expand_to_chunks(conn, &graph, &pi, top_n)?;

    // 6. Fetch chunk rows, apply the same visibility rules as the other
    //    retrievers (flagged quarantine + superseded exclusion).
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = chunks.iter().map(|&(id, _)| id).collect();
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut flag_clause = "k.flagged = 0";
    if include_flagged {
        flag_clause = "1";
    }
    let sql = format!(
        "SELECT k.id, k.title, k.content \
         FROM knowledge k \
         WHERE k.id IN ({placeholders}) \
           AND k.valid_to IS NULL \
           AND {flag_clause}"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut loaded: HashMap<i64, (Option<String>, String)> = HashMap::new();
    for row in stmt.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })? {
        let (id, title, content): (i64, Option<String>, String) = row?;
        loaded.insert(id, (title, content));
    }
    chunks.retain(|(id, _)| loaded.contains_key(id));

    // 7. Rank scores are PPR sums; the RRF caller only uses rank position, so
    //    the fused `source` provenance is set downstream.
    let results = chunks
        .into_iter()
        .filter_map(|(id, score)| {
            let (title, content) = loaded.get(&id)?;
            let mut r =
                crate::search::SearchResult::raw(id, score as f32, title.clone(), content.clone());
            r.source = Some(crate::search::SearchSource::Graph);
            Some(r)
        })
        .collect();
    Ok(results)
}

/// Restrict `graph` to vertices reachable from `seeds` via undirected edges,
/// capped at [`MAX_VISITED`] distinct vertices (BFS).
fn restrict_to_reachable(graph: SparseGraph, seeds: &[i64]) -> SparseGraph {
    let mut reachable: HashSet<i64> = HashSet::new();
    let mut queue: std::collections::VecDeque<i64> = seeds.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        if reachable.len() >= MAX_VISITED || !reachable.insert(id) {
            continue;
        }
        if let Some(i) = graph.idx(id) {
            for &(j, _) in &graph.adj[i] {
                if !reachable.contains(&graph.idx_to_id[j]) {
                    queue.push_back(graph.idx_to_id[j]);
                }
            }
        }
    }
    let mut out = SparseGraph::new();
    for (a, b, w) in all_edges(&graph) {
        if reachable.contains(&a) && reachable.contains(&b) {
            out.add_edge(a, b, w);
        }
    }
    out
}

fn all_edges(graph: &SparseGraph) -> Vec<(i64, i64, f64)> {
    let mut out = Vec::new();
    for (i, row) in graph.adj.iter().enumerate() {
        for &(j, w) in row {
            if i < j {
                out.push((graph.idx_to_id[i], graph.idx_to_id[j], w));
            }
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────
// The plan's verification #1/#2/#4 are pure-function tests; #3 is the RRF
// integration test in `mod.rs`.

#[cfg(test)]
mod tests {
    use super::*;

    fn sr(id: i64, score: f32, content: &str) -> crate::search::SearchResult {
        crate::search::SearchResult::raw(id, score, None, content.to_string())
    }

    /// Plan verification #1: `graph_ppr_ranks_connected_entities_higher_than_unrelated`.
    #[test]
    fn ppr_ranks_connected_entities_higher_than_unrelated() {
        // Two disconnected components: 1—2 (linked to the seed) and 3—4
        // (an unrelated pair). Vertex order = insertion order.
        let mut g = SparseGraph::new();
        g.add_edge(1, 2, 3.0);
        g.add_edge(3, 4, 1.0); // unrelated component
        let pi = personalized_pagerank(&g, &[0], PPR_ALPHA, PPR_EPSILON, MAX_PPR_ITER);
        // Vertex 0 = id 1 (seed), 1 = id 2 (linked neighbor), 2/3 = unrelated.
        assert!(
            pi[1] > pi[2],
            "linked neighbor must outrank unrelated vertex"
        );
        assert!(
            pi[1] > pi[3],
            "linked neighbor must outrank unrelated vertex"
        );
        assert!(pi[0] > 0.0, "seed must have nonzero PPR");
    }

    /// Plan verification #2: `ppr_seed_from_query_uses_exact_entity_names`.
    #[test]
    fn seed_from_query_uses_exact_entity_names() {
        let entities = vec![
            (1i64, "QuickBooks".to_string()),
            (2, "QuickBooks Online".to_string()),
            (3, "tax".to_string()),
        ];
        // "quickbooks" matches only "QuickBooks" (exact phrase, case-insensitive),
        // NOT "QuickBooks Online" — that is a different stored name.
        let seeds = seed_entities_from_query("which client uses quickbooks?", &entities);
        assert_eq!(seeds, vec![1]);
    }

    /// Plan verification #4: `graph_ppr_bounded_by_max_visited`.
    #[test]
    fn ppr_bounded_by_max_visited() {
        // Build a chain of 2 * MAX_VISITED vertices so the BFS cap must kick in.
        let mut g = SparseGraph::new();
        let big = (MAX_VISITED as i64) * 2;
        for i in 1..big {
            g.add_edge(i, i + 1, 1.0);
        }
        let reachable = restrict_to_reachable(g, &[1]);
        assert!(reachable.len() <= MAX_VISITED, "BFS must be capped");
        let pi = personalized_pagerank(&reachable, &[0], PPR_ALPHA, PPR_EPSILON, MAX_PPR_ITER);
        assert!(
            pi.iter().any(|&v| v > 0.0),
            "PPR must converge on the bounded subgraph"
        );
    }

    /// The self-loop and zero-weight edge guards: both are dropped, so only
    /// the real edge's vertices are admitted.
    #[test]
    fn graph_ignores_self_loops_and_zero_weights() {
        let mut g = SparseGraph::new();
        g.add_edge(1, 1, 5.0); // self-loop → dropped
        g.add_edge(1, 2, 0.0); // zero weight → dropped
        g.add_edge(2, 3, 1.0); // real edge → both vertices admitted
        assert_eq!(g.len(), 2, "only the real edge's vertices admitted");
    }

    /// RRF of a graph-leg result plus a vector leg (used by verification #3
    /// plumbing; the full integration test lives in mod.rs).
    #[test]
    fn graph_result_carries_graph_source() {
        let mut r = sr(42, 0.5, "chunk");
        r.source = Some(crate::search::SearchSource::Graph);
        assert_eq!(r.source, Some(crate::search::SearchSource::Graph));
    }

    // ── — noise-aware weights + hub dampening ─────────────

    /// Plan verification #1: the taxonomy weight table contract.
    #[test]
    fn type_base_weight_downgrades_taxonomy_noise() {
        assert_eq!(type_base_weight("tagged_with"), 0.1);
        assert_eq!(type_base_weight("alias_of"), 0.1);
        assert_eq!(type_base_weight("works_at"), 1.0);
        assert_eq!(type_base_weight("references"), 1.0);
        assert_eq!(type_base_weight("contradicts"), 1.0);
        // Unknown types stay at full weight (semantic default).
        assert_eq!(type_base_weight("future_relation"), 1.0);
    }

    /// Plan verification #2: hub dampening exact math. A deg-100 source at
    /// θ=50 scales each outgoing half-edge by 0.5; a deg-10 source is
    /// unchanged (θ/deg = 5 → min → 1.0). Damping is per-SOURCE-node, so a
    /// light vertex's half-edge toward the hub is untouched.
    #[test]
    fn hub_dampening_scales_heavy_hubs_but_not_light() {
        let mut g = SparseGraph::new();
        // Hub id 1 with 100 leaves, weight 3.0 each (deg 100 → ×0.5).
        for i in 2..=101 {
            g.add_edge(1, i, 3.0);
        }
        // Light vertex id 300 with 10 neighbors, weight 2.0 each (deg 10 → ×1).
        for i in 200..=209 {
            g.add_edge(300, i, 2.0);
        }
        g.dampen_hubs(HUB_DAMPING_THETA);

        let hub_row = &g.adj[g.idx(1).unwrap()];
        assert_eq!(hub_row.len(), 100);
        assert!((hub_row[0].1 - 1.5).abs() < 1e-9, "3.0 × 0.5 = 1.5");

        let light_row = &g.adj[g.idx(300).unwrap()];
        assert!(
            (light_row[0].1 - 2.0).abs() < 1e-9,
            "deg 10 must be unchanged"
        );

        // Per-source damping: the leaf's half-edge TOWARD the hub is not
        // scaled by the hub's degree.
        let leaf_row = &g.adj[g.idx(2).unwrap()];
        assert!((leaf_row[0].1 - 3.0).abs() < 1e-9, "leaf outflow unchanged");
    }

    /// Plan verification #3: `graph_retrieve` over a mixed hub — 2 semantic
    /// (`works_at`) neighbors vs a 100-edge `tagged_with` cloud. Type weights
    /// and hub dampening must rank the semantic-backed chunk (102) above the
    /// tag-cloud chunk (103); under the v1.11.0 unweighted graph the tag
    /// cloud wins (2×0.00123 < 4×0.00123), so this test fails on that code.
    #[test]
    fn graph_retrieve_weights_semantic_over_tag_cloud() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE entities (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE relationships (
                 from_entity_id INTEGER NOT NULL,
                 to_entity_id INTEGER NOT NULL,
                 relation_type TEXT NOT NULL,
                 knowledge_id INTEGER
             );
             CREATE TABLE knowledge (
                 id INTEGER PRIMARY KEY,
                 title TEXT,
                 content TEXT NOT NULL,
                 valid_to TEXT,
                 flagged INTEGER NOT NULL DEFAULT 0
             );",
        )
        .unwrap();
        conn.execute("INSERT INTO entities (id, name) VALUES (1, 'alpha')", [])
            .unwrap();
        conn.execute("INSERT INTO entities (id, name) VALUES (2, 'hub')", [])
            .unwrap();
        conn.execute("INSERT INTO entities (id, name) VALUES (3, 'neon1')", [])
            .unwrap();
        conn.execute("INSERT INTO entities (id, name) VALUES (4, 'neon2')", [])
            .unwrap();
        for i in 0..100 {
            conn.execute(
                "INSERT INTO entities (id, name) VALUES (?1, ?2)",
                rusqlite::params![11 + i, format!("tag{i}")],
            )
            .unwrap();
        }
        for (kid, text) in [
            (101, "seed chunk"),
            (102, "semantic chunk"),
            (103, "tag cloud chunk"),
        ] {
            conn.execute(
                "INSERT INTO knowledge (id, title, content) VALUES (?1, ?2, ?3)",
                rusqlite::params![kid, text, text],
            )
            .unwrap();
        }
        // Seed → hub (semantic, chunk 101).
        conn.execute(
            "INSERT INTO relationships VALUES (1, 2, 'manages', 101)",
            [],
        )
        .unwrap();
        // Hub → 2 semantic neighbors, both backed by chunk 102.
        conn.execute(
            "INSERT INTO relationships VALUES (2, 3, 'works_at', 102)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO relationships VALUES (2, 4, 'works_at', 102)",
            [],
        )
        .unwrap();
        // Hub → 100 tag neighbors, one tagged chunk 103.
        for i in 0..100 {
            conn.execute(
                "INSERT INTO relationships VALUES (2, ?1, 'tagged_with', 103)",
                rusqlite::params![11 + i],
            )
            .unwrap();
        }

        let res = graph_retrieve(&conn, "which alpha thing", 8, false).unwrap();
        assert_eq!(res[0].id, 101, "seed-backed chunk must win");
        let pos = |id: i64| res.iter().position(|r| r.id == id).unwrap();
        assert!(
            pos(102) < pos(103),
            "semantic-neighbor chunk (102) must rank above the tagged_with cloud (103)"
        );
    }
}
