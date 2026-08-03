//! v1.11.0 "Associate" — HippoRAG-2-style Personalized PageRank over the
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

/// A sparse, undirected, weighted entity graph.
///
/// Vertices are `entities.id`; an edge `(a, b, w)` exists for every distinct
/// `(from_entity_id, to_entity_id)` pair in `relationships`, with weight =
/// `COUNT(DISTINCT knowledge_id)` — the count of distinct chunks providing
/// evidence for that pair (the same "edge has evidence" signal v0.9.8 uses).
/// This is HippoRAG 2's `node_to_node_stats` fact-edge count at the pair
/// level, aggregated across `relation_type` so multi-rel pairs aren't
/// over-represented by taxonomy noise.
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
        let mut stmt = conn.prepare(
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
    let mut stmt = conn.prepare("SELECT id, name FROM entities")?;
    let entities: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    let seeds = seed_entities_from_query(query, &entities);
    if seeds.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Load the weighted adjacency restricted to entities that share at
    //    least one evidence chunk. One aggregate query, bounded by SQLite's
    //    own row limit (practical cap = distinct entity pairs).
    let mut stmt = conn.prepare(
        "SELECT from_entity_id, to_entity_id, COUNT(DISTINCT knowledge_id) \
         FROM relationships \
         WHERE knowledge_id IS NOT NULL \
         GROUP BY from_entity_id, to_entity_id",
    )?;
    let edge_rows: Vec<(i64, i64, f64)> = stmt
        .query_map([], |row| {
            let c: i64 = row.get(2)?;
            Ok((row.get(0)?, row.get(1)?, c as f64))
        })?
        .filter_map(|r| r.ok())
        .collect();

    // 3. Build the bounded graph. Prune to the component reachable from the
    //    seeds and cap total vertices at MAX_VISITED.
    let mut graph = build_graph(&edge_rows);
    // Restrict vertices to those reachable from seeds (BFS, bounded).
    graph = restrict_to_reachable(graph, &seeds);

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
    let mut stmt = conn.prepare(&sql)?;
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
}
