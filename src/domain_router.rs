//! Centroid routing for per-domain recall.
//!
//! Each domain's mean embedding vector (centroid) is stored in the global DB.
//! At query time the query vector is compared (cosine) to every domain
//! centroid; the best-scoring domain above a confidence threshold is searched
//! exclusively (strict isolation). With no confident domain and non-strict
//! mode, recall federates across all known domains and labels each hit with its
//! source domain. ponytail: centroids are simple arithmetic means of raw f32
//! embeddings (not learned); upgrade path is a per-domain probe-set / SVM if a
//! corpus needs sharper separation.
//!
//! The probe-set upgrade lives here now: `route_multi` scores a query against
//! per-domain PROTOTYPE vectors (k-means cluster means of the domain's chunk
//! embeddings, rebuilt by the sweep) using the mean of each domain's top-k
//! prototype similarities. A topic that is a sliver of a large domain — a
//! green-tea cluster inside a gut-microbiome corpus — scores against its own
//! prototype instead of being averaged away by the domain mean, which is the
//! measured failure single-centroid routing has with large heterogeneous
//! domains. Centroids remain the per-write-refresh signal and the fallback
//! when a domain carries no prototypes yet.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::Pool;
use crate::search::cosine_sim;

/// Minimum cosine similarity for a query to be confidently routed to a domain
/// (prototype-cluster score for `route_multi`, centroid score for `route`).
/// Below this, unscoped recall falls back to `global`-only. CALIBRATED, not
/// inherited: measured over the live corpus (2026-10-09, potion-retrieval-32M
/// space, 8,792 chunks / 11 domains), unrelated-domain prototype scores sit at
/// ≤ 0.10 while true-topic clusters score ≥ 0.27 — 0.25 sits 0.15 above the
/// measured noise ceiling and just under the measured topic floor. The
/// previous 0.30 rejected a true topic whose best prototype (0.306) was
/// diluted below the bar by blind top-k averaging.
pub const DOMAIN_CONFIDENCE_THRESHOLD: f32 = 0.25;

/// Prototype clusters per domain (k-means, deterministic). A domain with fewer
/// vectors than this keeps every vector as its own prototype. 48 clusters over
/// the largest live domain bounds the sweep at well under a second while
/// giving a heterogeneous corpus one prototype per topic — coarser grids
/// absorb minority topics (a 124-chunk green-tea cluster inside 7,700
/// gut-microbiome chunks) into broader means and the topic loses its own
/// routing signature.
pub const PROTO_K: usize = 48;

/// Width of the supporting cluster around the best prototype, in cosine
/// units. Prototypes scoring within this margin of the best are averaged
/// into the domain's score; anything farther is a different topic and does
/// not dilute it. See [`cluster_mean_sim`].
const PROTO_CLUSTER_MARGIN: f32 = 0.08;

/// k-means iteration cap. Convergence almost always fires first; the cap makes
/// worst-case sweep cost bounded and the result deterministic regardless.
const KMEANS_ITERS: usize = 12;

/// Arithmetic mean of a set of equal-length f32 vectors. Returns an empty vec
/// if the input is empty. All vectors are assumed to share the model's dim.
pub fn mean_vector(vectors: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = vectors.first() else {
        return Vec::new();
    };
    let dim = first.len();
    let mut acc = vec![0.0f32; dim];
    for v in vectors {
        for (a, x) in acc.iter_mut().zip(v.iter()) {
            *a += *x;
        }
    }
    let n = vectors.len() as f32;
    acc.iter_mut().for_each(|a| *a /= n);
    acc
}

/// The domain's routing score: the mean of the prototype similarities within
/// [`PROTO_CLUSTER_MARGIN`] of the best prototype — the query's SUPPORTING
/// CLUSTER. Two measured failures shaped this over its alternatives:
/// blind top-k means dilute a precise topic with unrelated prototypes (the
/// EGCG measurement: best 0.306 averaged with two 0.25s to 0.273, under a
/// 0.30 bar — the domain held 124 matching chunks and still did not route);
/// raw max has no support requirement at all. The margin cluster keeps both
/// properties: a topic confirmed by several close prototypes averages them,
/// and a topic that lives in exactly one prototype keeps its own score —
/// a lone strong prototype means content LIKE the query genuinely exists in
/// that domain; the threshold, the global rescue leg, and per-hit domain
/// labels own the noise case. Pure + deterministic.
fn cluster_mean_sim(query: &[f32], protos: &[&Vec<f32>]) -> f32 {
    let mut sims: Vec<f32> = protos
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| cosine_sim(query, p))
        .collect();
    if sims.is_empty() {
        return f32::NEG_INFINITY;
    }
    sims.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let cut = sims[0] - PROTO_CLUSTER_MARGIN;
    let cluster = sims.iter().take_while(|&&s| s >= cut);
    let n = cluster.clone().count().max(1) as f32;
    cluster.sum::<f32>() / n
}

/// Route a query vector to the single best-matching domain by PROTOTYPE
/// similarity, or `None` below [`DOMAIN_CONFIDENCE_THRESHOLD`]. Pure +
/// deterministic (ties broken alphabetically). `prototypes` is
/// `(domain, vector)` and may hold many vectors per domain. Domains whose
/// prototype list is empty (or all-empty vectors) cannot win — the caller
/// falls back to centroid routing for them.
pub fn route_multi(query: &[f32], prototypes: &[(String, Vec<f32>)]) -> Option<String> {
    // prototypes arrive ordered by domain (the reader sorts); score each
    // contiguous same-domain run in one pass.
    let mut best: Option<(f32, &str)> = None;
    let mut run_start = 0usize;
    while run_start < prototypes.len() {
        let domain = prototypes[run_start].0.as_str();
        let mut run_end = run_start + 1;
        while run_end < prototypes.len() && prototypes[run_end].0 == prototypes[run_start].0 {
            run_end += 1;
        }
        let protos: Vec<&Vec<f32>> = prototypes[run_start..run_end]
            .iter()
            .map(|(_, v)| v)
            .collect();
        run_start = run_end;

        let score = cluster_mean_sim(query, &protos);
        if score == f32::NEG_INFINITY {
            continue;
        }
        match best {
            None => best = Some((score, domain)),
            Some((bs, _)) if score > bs => best = Some((score, domain)),
            Some((bs, bd)) if (score - bs).abs() < f32::EPSILON && domain < bd => {
                best = Some((score, domain))
            }
            _ => {}
        }
    }
    best.and_then(|(score, domain)| {
        (score >= DOMAIN_CONFIDENCE_THRESHOLD).then(|| domain.to_string())
    })
}

/// Route a query vector to the single best-matching domain, or `None` if no
/// domain centroid clears [`DOMAIN_CONFIDENCE_THRESHOLD`]. Pure + deterministic
/// (ties broken alphabetically). `centroids` is `(domain, vector)`.
pub fn route(query: &[f32], centroids: &[(String, Vec<f32>)]) -> Option<String> {
    let mut best: Option<(f32, &str)> = None;
    for (domain, c) in centroids {
        if c.is_empty() {
            continue;
        }
        let score = cosine_sim(query, c);
        match best {
            None => best = Some((score, domain.as_str())),
            Some((bs, _)) if score > bs => best = Some((score, domain.as_str())),
            Some((bs, bd)) if (score - bs).abs() < f32::EPSILON && domain.as_str() < bd => {
                best = Some((score, domain.as_str()))
            }
            _ => {}
        }
    }
    best.and_then(|(score, domain)| {
        (score >= DOMAIN_CONFIDENCE_THRESHOLD).then(|| domain.to_string())
    })
}

/// Resolve the target domain for an ingest. A caller-forced domain always
/// wins; otherwise auto-route the chunk embedding against the stored
/// centroids, falling back to `global` when no centroid clears the confidence
/// threshold. Pure + deterministic — the same `route()` recall uses.
pub fn route_domain_label(
    forced: &Option<String>,
    embedding: &[f32],
    centroids: &[(String, Vec<f32>)],
) -> String {
    match forced {
        Some(d) => d.clone(),
        None => route(embedding, centroids).unwrap_or_else(|| "global".to_string()),
    }
}

/// The authorised splitter for over-ceiling choice schemas: chunks of at
/// most 20 options, in order, coarse chunk first. `<= 20` options ride a
/// single schema; a larger set MUST build the two-step tree the decide
/// lane consumes (coarse choice over the first chunk, fine choice within
/// the winner) — `workflow::decide::sequence::validate_schema` enforces
/// the ceiling, this helper is the only sanctioned way to satisfy it.
/// Pure + total: the input order is preserved exactly. Truthful allow:
/// the caller is the decide lane's advisor wiring, which lands with the
/// lane (Phase 1+); until then the tests hold the contract.
#[allow(dead_code)]
pub(crate) fn split_for_decide(options: &[String]) -> Vec<Vec<String>> {
    const CEILING: usize = 20;
    options
        .chunks(CEILING.max(1))
        .map(<[String]>::to_vec)
        .collect()
}

/// Read every stored `(domain, centroid)` from the global DB's centroid table.
pub fn read_centroids(global_pool: &Pool) -> Result<Vec<(String, Vec<f32>)>> {
    let conn = global_pool
        .get()
        .context("centroid read: DB connection failed")?;
    let mut stmt = conn.prepare("SELECT domain, centroid FROM domain_centroids ORDER BY domain")?;
    let rows: Vec<(String, Vec<f32>)> = stmt
        .query_map([], |row| {
            let domain: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            let v = blob_to_f32(&blob);
            Ok((domain, v))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Ensure the prototype table exists. Created lazily and additively
/// (`IF NOT EXISTS`) rather than through the migration ladder: prototypes are
/// routing hints rebuilt wholesale by the sweep — not source-of-truth data —
/// so the table earns no schema-version bump and a fresh or old DB converges
/// on first sweep.
fn ensure_prototype_table(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS domain_route_prototypes (
            domain TEXT NOT NULL,
            idx INTEGER NOT NULL,
            proto BLOB NOT NULL,
            PRIMARY KEY (domain, idx)
        )",
        [],
    )?;
    Ok(())
}

/// Read every stored `(domain, prototype)` ordered by domain (the order
/// `route_multi`'s run-scanning relies on).
pub fn read_prototypes(global_pool: &Pool) -> Result<Vec<(String, Vec<f32>)>> {
    let conn = global_pool
        .get()
        .context("prototype read: DB connection failed")?;
    ensure_prototype_table(&conn)?;
    let mut stmt =
        conn.prepare("SELECT domain, proto FROM domain_route_prototypes ORDER BY domain, idx")?;
    let rows: Vec<(String, Vec<f32>)> = stmt
        .query_map([], |row| {
            let domain: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((domain, blob_to_f32(&blob)))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Deterministic k-means over the domain's chunk vectors; the cluster means
/// become the domain's routing prototypes. Initial centers are evenly strided
/// over the (id-ordered) vector list — no RNG — and the assignment loop is
/// capped at [`KMEANS_ITERS`], so the same vectors always produce the same
/// prototypes. Empty clusters are dropped; a domain with fewer vectors than
/// [`PROTO_K`] keeps every vector (k = count).
fn kmeans_prototypes(vectors: &[Vec<f32>], k: usize) -> Vec<Vec<f32>> {
    if vectors.is_empty() || k == 0 {
        return Vec::new();
    }
    let k = k.min(vectors.len());
    let mut centers: Vec<Vec<f32>> = (0..k)
        .map(|i| vectors[i * vectors.len() / k].clone())
        .collect();
    let mut assignments = vec![0usize; vectors.len()];
    for _ in 0..KMEANS_ITERS {
        // assignment: first-max on ties keeps the pass deterministic
        let mut changed = false;
        for (vi, v) in vectors.iter().enumerate() {
            let mut best = 0usize;
            let mut best_score = f32::NEG_INFINITY;
            for (ci, c) in centers.iter().enumerate() {
                let s = cosine_sim(v, c);
                if s > best_score {
                    best_score = s;
                    best = ci;
                }
            }
            if assignments[vi] != best {
                assignments[vi] = best;
                changed = true;
            }
        }
        // update: mean of assigned members; an emptied center keeps its old
        // position (it will absorb members next pass)
        let mut sums = vec![vec![0.0f32; centers.first().map_or(0, Vec::len)]; centers.len()];
        let mut counts = vec![0usize; centers.len()];
        for (vi, v) in vectors.iter().enumerate() {
            let c = assignments[vi];
            counts[c] += 1;
            for (a, x) in sums[c].iter_mut().zip(v.iter()) {
                *a += *x;
            }
        }
        for (ci, center) in centers.iter_mut().enumerate() {
            if counts[ci] > 0 {
                *center = sums[ci].iter().map(|a| a / counts[ci] as f32).collect();
            }
        }
        if !changed {
            break;
        }
    }
    centers
}

/// Rebuild one domain's routing prototypes from its live chunk vectors and
/// upsert them into the global DB (replacing the domain's previous set).
/// Returns the prototype count (0 clears any stale set, mirroring the
/// centroid law for emptied domains).
pub fn rebuild_prototypes(domain_pool: &Pool, domain: &str, global_pool: &Pool) -> Result<usize> {
    let dconn = domain_pool
        .get()
        .context("prototype compute: domain DB connection failed")?;
    let vectors = read_domain_vectors(&dconn, domain)?;
    drop(dconn);

    let protos = kmeans_prototypes(&vectors, PROTO_K);
    let gconn = global_pool
        .get()
        .context("prototype compute: global DB connection failed")?;
    ensure_prototype_table(&gconn)?;
    gconn.execute(
        "DELETE FROM domain_route_prototypes WHERE domain = ?1",
        params![domain],
    )?;
    for (idx, proto) in protos.iter().enumerate() {
        gconn.execute(
            "INSERT INTO domain_route_prototypes (domain, idx, proto) VALUES (?1, ?2, ?3)",
            params![domain, idx as i64, f32_to_blob(proto)],
        )?;
    }
    Ok(protos.len())
}

/// Recompute and store a single domain's centroid. Vectors are read from
/// `domain_pool` (the domain's own DB) joined to `knowledge` on `domain`; the
/// centroid is upserted into the global DB's `domain_centroids` table. Returns
/// the number of vectors averaged.
pub fn recompute_centroid(domain_pool: &Pool, domain: &str, global_pool: &Pool) -> Result<usize> {
    let dconn = domain_pool
        .get()
        .context("centroid compute: domain DB connection failed")?;
    let vectors = read_domain_vectors(&dconn, domain)?;
    drop(dconn);

    let count = vectors.len();
    // a domain below DOMAIN_MIN_COUNT (default 1) keeps no centroid
    // so `route()` stops sending traffic to a near-empty bucket. Default 1 is a
    // no-op — nothing is suppressed unless the operator raises the floor.
    let min_count = crate::config::brain_domain_min_count() as usize;
    let centroid = if count >= min_count {
        mean_vector(&vectors)
    } else {
        Vec::new()
    };
    let gconn = global_pool
        .get()
        .context("centroid compute: global DB connection failed")?;
    if centroid.is_empty() {
        // No vectors: remove any stale centroid so the domain stops routing.
        gconn.execute(
            "DELETE FROM domain_centroids WHERE domain = ?1",
            params![domain],
        )?;
    } else {
        let blob = f32_to_blob(&centroid);
        gconn.execute(
            "INSERT INTO domain_centroids (domain, centroid, count, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(domain) DO UPDATE SET
               centroid = excluded.centroid, count = excluded.count, updated_at = excluded.updated_at",
            params![domain, blob, count as i64],
        )?;
    }
    Ok(count)
}

/// one-shot recompute of every known domain's centroid AND routing prototypes
/// from the corrected vector source. Domain set = `DISTINCT knowledge.domain` ∪
/// existing `domain_centroids` rows (so a domain that emptied out also gets its
/// stale centroid cleaned). In shim mode all domains share the global pool.
/// Returns `(domain, vector_count)` per domain — the post-migration catch-up
/// sweep that makes auto-route meaningful (until real centroids exist, `route()`
/// only ever sees `global`). Prototypes ride the same sweep: per-write
/// `recompute_centroid` refreshes only the centroid (k-means over a large
/// domain is sweep-cost, not write-cost), so a domain's prototypes may lag its
/// newest chunks until the next sweep — a routing-hint staleness the fallback
/// path absorbs (no prototypes → centroid routing, unchanged).
pub fn recompute_all_centroids(global_pool: &Pool) -> Result<Vec<(String, usize)>> {
    let conn = global_pool
        .get()
        .context("centroid sweep: DB connection failed")?;
    let mut domains: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for sql in [
        "SELECT DISTINCT domain FROM knowledge",
        "SELECT DISTINCT domain FROM domain_centroids",
    ] {
        let mut stmt = conn.prepare(sql)?;
        for d in stmt.query_map([], |r| r.get::<_, String>(0))?.flatten() {
            domains.insert(d);
        }
    }
    drop(conn);
    let mut out = Vec::new();
    for d in domains {
        let count = recompute_centroid(global_pool, &d, global_pool)?;
        rebuild_prototypes(global_pool, &d, global_pool)?;
        out.push((d, count));
    }
    Ok(out)
}

/// Read a domain's current (non-superseded) chunk vectors from the live vec0
/// index. Previously read the frozen legacy `embeddings` JSON table
/// (2 rows since the vec0 cutover), which silently zeroed every centroid. Now reads
/// `vec_knowledge`, matching `find_near_duplicates` (consolidate.rs:260), and
/// dequantizes via `decode_embedding`. `valid_to IS NULL` excludes superseded
/// chunks (the loser of a contradiction resolution) so a centroid isn't pulled
/// toward outdated content. `ORDER BY k.id` pins the read order — the k-means
/// init strides this list, so an unpinned order would make prototypes
/// nondeterministic. Kept Connection-taking so tests use `test_db()`.
pub fn read_domain_vectors(conn: &Connection, domain: &str) -> Result<Vec<Vec<f32>>> {
    let mut stmt = conn.prepare(
        "SELECT v.embedding_int8
         FROM vec_knowledge v
         JOIN knowledge k ON k.id = v.knowledge_id
         WHERE k.domain = ?1 AND k.valid_to IS NULL
         ORDER BY k.id",
    )?;
    let rows = stmt.query_map(params![domain], |row| row.get::<_, Vec<u8>>(0))?;
    let mut vectors: Vec<Vec<f32>> = Vec::new();
    for blob in rows.flatten() {
        let v = crate::consolidate::decode_embedding(&blob);
        if !v.is_empty() {
            vectors.push(v);
        }
    }
    Ok(vectors)
}

fn f32_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn blob_to_f32(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_of_identical_vectors_is_that_vector() {
        let v = vec![0.2, 0.4, 0.6];
        let m = mean_vector(&[v.clone(), v.clone(), v.clone()]);
        assert_eq!(m.len(), 3);
        for (a, b) in m.iter().zip(v.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn mean_averages_elementwise() {
        let m = mean_vector(&[vec![1.0, 2.0], vec![3.0, 4.0]]);
        assert!((m[0] - 2.0).abs() < 1e-6);
        assert!((m[1] - 3.0).abs() < 1e-6);
    }

    #[test]
    fn mean_of_empty_is_empty() {
        assert!(mean_vector(&[]).is_empty());
    }

    /// split_for_decide_chunks_of_20 — the 77-label banking fixture: four
    /// chunks (20/20/20/17), every chunk within the ceiling, order
    /// preserved, and the union exactly the input.
    #[test]
    fn split_for_decide_chunks_of_20() {
        // The 77-label banking fixture (the port spec's own example).
        let families = [
            ("account", 14),
            ("card", 12),
            ("loan", 11),
            ("fee", 9),
            ("fraud", 8),
            ("statement", 8),
            ("payment", 8),
            ("branch", 7),
        ];
        assert_eq!(families.iter().map(|(_, n)| n).sum::<usize>(), 77);
        let labels: Vec<String> = families
            .iter()
            .flat_map(|(f, n)| (0..*n).map(move |i| format!("{f}_{i}")))
            .collect();
        let chunks = split_for_decide(&labels);
        assert_eq!(chunks.len(), 4, "77 labels split into 4 chunks");
        assert!(
            chunks.iter().all(|c| c.len() <= 20),
            "every chunk within the ceiling"
        );
        assert_eq!(
            chunks.concat(),
            labels,
            "the split is order-preserving and lossless"
        );
        assert_eq!(
            chunks.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![20, 20, 20, 17]
        );
    }

    #[test]
    fn split_for_decide_single_schema_at_or_under_the_ceiling() {
        let small: Vec<String> = (0..20).map(|i| format!("opt{i}")).collect();
        let chunks = split_for_decide(&small);
        assert_eq!(chunks.len(), 1, "20 options ride one schema");
        assert_eq!(chunks[0], small);
        let tiny = vec!["a".to_string(), "b".to_string()];
        assert_eq!(split_for_decide(&tiny), vec![tiny.clone()]);
    }

    #[test]
    fn split_for_decide_total_on_empty() {
        let empty: Vec<String> = Vec::new();
        let chunks = split_for_decide(&empty);
        assert!(chunks.is_empty(), "no options, no chunks");
    }

    #[test]
    fn route_picks_best_above_threshold() {
        // query aligns with "rust" centroid, orthogonal to "cooking".
        let q = vec![1.0, 0.0];
        let centroids = vec![
            ("cooking".to_string(), vec![0.0, 1.0]),
            ("rust".to_string(), vec![0.99, 0.01]),
        ];
        let picked = route(&q, &centroids);
        assert_eq!(picked.as_deref(), Some("rust"));
    }

    #[test]
    fn route_returns_none_below_threshold() {
        // Nothing close to the query → no confident domain.
        let q = vec![1.0, 0.0];
        let centroids = vec![
            ("a".to_string(), vec![0.0, 1.0]),
            ("b".to_string(), vec![-0.1, 0.9]),
        ];
        assert!(route(&q, &centroids).is_none());
    }

    #[test]
    fn route_ignores_empty_centroids() {
        let q = vec![1.0, 0.0];
        let centroids = vec![
            ("empty".to_string(), vec![]),
            ("real".to_string(), vec![1.0, 0.0]),
        ];
        assert_eq!(route(&q, &centroids).as_deref(), Some("real"));
    }

    #[test]
    fn f32_blob_roundtrips() {
        let v = vec![0.1, -0.2, 3.5, 0.0];
        let got = blob_to_f32(&f32_to_blob(&v));
        assert_eq!(got.len(), v.len());
        for (a, b) in got.iter().zip(v.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    // ── prototype routing (the probe-set upgrade) ─────────────

    /// In 8 dims: `bulk` is a large heterogeneous domain whose MEAN is pulled
    /// orthogonal to the query by its many off-topic vectors, but which carries
    /// a tight on-topic cluster (3 aligned prototypes). `tiny` is a focused
    /// domain whose single centroid IS the query direction. The measured
    /// failure: single-centroid routing sends this query to `tiny` (or
    /// nowhere), never `bulk`, even though `bulk` holds the matching topic.
    /// Top-k prototype means let the on-topic cluster carry `bulk`.
    #[test]
    fn route_multi_finds_a_topic_the_domain_mean_averages_away() {
        let on_topic = vec![1.0, 0.9, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0];
        let off_topic = vec![0.0, 0.0, 0.0, 1.0, 1.0, 0.9, 0.8, 0.7];
        let query = vec![1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];

        // the single-centroid world: bulk's mean is dominated by off-topic
        // mass and loses to tiny's exact match.
        let bulk_mean = mean_vector(&[on_topic.clone(), off_topic.clone(), off_topic.clone()]);
        let centroids = vec![
            ("bulk".to_string(), bulk_mean),
            ("tiny".to_string(), query.clone()),
        ];
        assert_eq!(
            route(&query, &centroids).as_deref(),
            Some("tiny"),
            "precondition: centroid routing picks the focused domain"
        );

        // the prototype world: bulk's on-topic cluster is a real prototype,
        // and its top-3 mean (one on-topic + two damped) still beats a domain
        // whose prototypes are all orthogonal — bulk wins with tiny below
        // threshold.
        let prototypes = vec![
            ("bulk".to_string(), on_topic.clone()),
            ("bulk".to_string(), off_topic.clone()),
            ("bulk".to_string(), off_topic.clone()),
            ("bulk".to_string(), off_topic.clone()),
            ("tiny".to_string(), off_topic.clone()),
        ];
        assert_eq!(
            route_multi(&query, &prototypes).as_deref(),
            Some("bulk"),
            "the on-topic cluster routes the query into the large domain"
        );
    }

    /// The routing score is the SUPPORTING CLUSTER's mean, and the pin
    /// discriminates it from BOTH degenerate aggregations (each arm fails
    /// exactly one):
    ///  - arm A fails blind top-k: a lone exact prototype followed by far
    ///    ones scores 1.0 (its own cluster), not the diluted top-3 mean
    ///    (0.367) — dilution is the measured EGCG failure;
    ///  - arm B fails raw max: a best prototype supported by two close ones
    ///    scores at the cluster mean (0.967), below its lone best (1.0).
    ///
    /// A lone strong prototype with no support keeps its own score — that is
    /// deliberate: content LIKE the query genuinely exists in that domain;
    /// the threshold, the global rescue leg, and per-hit domain labels own
    /// the noise case.
    #[test]
    fn route_multi_scores_the_supporting_cluster() {
        let query = vec![1.0, 0.0];

        // arm A — no dilution: the outlier stays out of the cluster.
        let diluted_if_topk: Vec<(String, Vec<f32>)> = vec![
            ("d".to_string(), vec![1.0, 0.0]),
            ("d".to_string(), vec![0.0, 1.0]),
            ("d".to_string(), vec![0.0, 1.0]),
            ("d".to_string(), vec![0.0, 1.0]),
        ];
        let a = {
            let protos: Vec<&Vec<f32>> = diluted_if_topk.iter().map(|(_, v)| v).collect();
            cluster_mean_sim(&query, &protos)
        };
        assert!(
            (a - 1.0).abs() < 1e-4,
            "arm A: the supporting cluster is {a}, blind top-3 would give 0.367"
        );

        // arm B — support damps: close prototypes average with the best.
        let supported: Vec<(String, Vec<f32>)> = vec![
            ("d".to_string(), vec![1.0, 0.0]),
            ("d".to_string(), vec![0.96, 0.28]),
            ("d".to_string(), vec![0.94, 0.34]),
        ];
        let b = {
            let protos: Vec<&Vec<f32>> = supported.iter().map(|(_, v)| v).collect();
            cluster_mean_sim(&query, &protos)
        };
        assert!(
            (b - 0.967).abs() < 0.01,
            "arm B: the cluster mean averages the supporters: {b}"
        );
        assert!(b < a, "supported clusters score below a lone exact match");
    }

    #[test]
    fn route_multi_returns_none_when_nothing_clears_the_threshold() {
        let q = vec![1.0, 0.0];
        let prototypes = vec![
            ("a".to_string(), vec![0.0, 1.0]),
            ("b".to_string(), vec![-0.1, 0.9]),
        ];
        assert!(route_multi(&q, &prototypes).is_none());
    }

    #[test]
    fn route_multi_ties_break_alphabetically_and_are_deterministic() {
        let q = vec![1.0, 0.0];
        let protos = |order: bool| {
            let a = ("alpha".to_string(), vec![1.0, 0.0]);
            let b = ("beta".to_string(), vec![1.0, 0.0]);
            if order { vec![b, a] } else { vec![a, b] }
        };
        let first = route_multi(&q, &protos(false));
        let second = route_multi(&q, &protos(true));
        assert_eq!(first.as_deref(), Some("alpha"));
        assert_eq!(
            first, second,
            "same scores → same winner either input order"
        );
    }

    #[test]
    fn route_multi_skips_empty_prototype_vectors() {
        let q = vec![1.0, 0.0];
        let prototypes = vec![
            ("empty".to_string(), Vec::new()),
            ("empty".to_string(), Vec::new()),
            ("real".to_string(), vec![1.0, 0.0]),
        ];
        assert_eq!(route_multi(&q, &prototypes).as_deref(), Some("real"));
    }

    #[test]
    fn route_multi_on_empty_input_is_none() {
        assert!(route_multi(&[1.0, 0.0], &[]).is_none());
    }

    /// The sweep builds prototypes from the real vec0 index, deterministically:
    /// two sweeps over identical content produce byte-identical prototype rows,
    /// and a below-centroid query still routes into the domain that holds its
    /// tight cluster. Driven through the real migration + vec0 + sweep path.
    #[test]
    fn prototype_sweep_is_deterministic_and_routes_the_cluster() {
        crate::register_sqlite_vec::register_sqlite_vec();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proto.db");
        let insert = |conn: &rusqlite::Connection,
                      id: i64,
                      content: &str,
                      domain: &str,
                      v: &[f32]| {
            conn.execute(
                "INSERT INTO knowledge(id, content, content_hash, domain) VALUES (?1, ?2, ?2, ?3)",
                rusqlite::params![id, content, domain],
            )
            .unwrap();
            let blob: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
            conn.execute(
                "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
                 VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'test', datetime('now'))",
                rusqlite::params![id, blob],
            )
            .unwrap();
        };
        // a tight 6-vector cluster in the first band (dims 0..64) and a
        // second topic in the next band (dims 64..128) — 512-dim like the
        // real vec0 table.
        let topic_vec = |jitter: f32| -> Vec<f32> {
            (0..512)
                .map(|i| {
                    if i < 64 {
                        1.0 + jitter
                    } else if i < 128 {
                        0.1
                    } else {
                        0.0
                    }
                })
                .collect()
        };
        let filler_vec = |jitter: f32| -> Vec<f32> {
            (0..512)
                .map(|i| {
                    if (64..128).contains(&i) {
                        1.0 + jitter
                    } else if i < 64 {
                        0.1
                    } else {
                        0.0
                    }
                })
                .collect()
        };
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            crate::migration::run_migration(&mut conn, crate::config::DB_MMAP_SIZE_MIB)
                .expect("migration");
            for i in 0..6 {
                insert(
                    &conn,
                    i + 1,
                    &format!("topic chunk {i}"),
                    "site",
                    &topic_vec(i as f32 * 0.01),
                );
            }
            for i in 0..6 {
                insert(
                    &conn,
                    100 + i,
                    &format!("filler chunk {i}"),
                    "site",
                    &filler_vec(i as f32 * 0.01),
                );
            }
        }
        let pool: crate::Pool = r2d2::Pool::builder()
            .build(crate::pool::SqliteConnectionManager::file(&path))
            .expect("pool build");
        let first = crate::domain_router::recompute_all_centroids(&pool).unwrap();
        let protos_a = crate::domain_router::read_prototypes(&pool).unwrap();
        let _ = crate::domain_router::recompute_all_centroids(&pool);
        let protos_b = crate::domain_router::read_prototypes(&pool).unwrap();
        assert_eq!(
            protos_a, protos_b,
            "identical content → identical prototypes"
        );
        assert!(
            !protos_a.is_empty(),
            "the sweep stored at least one prototype ({first:?})"
        );

        // A query near the first band's cluster: the domain's MEAN sits between
        // the two topic bands, but a prototype sits on this cluster.
        let site_protos = protos_a
            .iter()
            .filter(|(d, _)| d == "site")
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>();
        assert!(!site_protos.is_empty());
        let query = topic_vec(0.0);
        let routed = crate::domain_router::route_multi(&query, &protos_a);
        assert_eq!(
            routed.as_deref(),
            Some("site"),
            "the cluster's prototype routes the query into its domain"
        );
    }

    // ── ingest auto-routing (pure decision) ──────────────────

    #[test]
    fn route_domain_label_forced_wins_over_centroids() {
        // An explicit domain always beats routing, even if a centroid matches.
        let forced = Some("visa".to_string());
        let embedding = vec![1.0, 0.0];
        let centroids = vec![("visa".to_string(), vec![1.0, 0.0])];
        assert_eq!(route_domain_label(&forced, &embedding, &centroids), "visa");
    }

    #[test]
    fn route_domain_label_auto_routes_when_omitted() {
        // No forced domain + a centroid clearing the threshold → routed domain.
        let forced = None;
        let embedding = vec![1.0, 0.0];
        let centroids = vec![
            ("cooking".to_string(), vec![0.0, 1.0]),
            ("rust".to_string(), vec![0.99, 0.01]),
        ];
        assert_eq!(
            route_domain_label(&forced, &embedding, &centroids),
            "rust",
            "omitted domain auto-routes to the best-matching centroid"
        );
    }

    #[test]
    fn route_domain_label_defaults_to_global_without_centroids() {
        // Empty centroids → global (back-compat: a fresh DB behaves as before).
        let forced = None;
        assert_eq!(route_domain_label(&forced, &[1.0, 0.0], &[]), "global");
    }

    #[test]
    fn route_domain_label_is_deterministic() {
        let forced = None;
        let embedding = vec![0.5, -0.3];
        let centroids = vec![
            ("a".to_string(), vec![1.0, 0.0]),
            ("b".to_string(), vec![-1.0, 0.0]),
        ];
        let first = route_domain_label(&forced, &embedding, &centroids);
        let second = route_domain_label(&forced, &embedding, &centroids);
        assert_eq!(first, second, "same content + same centroids → same domain");
    }

    /// the one-shot sweep recomputes every known domain's centroid
    /// from vec_knowledge and cleans a stale centroid for an emptied domain.
    /// Driven through the real vec0 + `recompute_all_centroids` path (a Pool is
    /// required, so this spins up a real pool on a temp file like the M1 tests).
    /// (v1.28.49: repointed here from the domains handler's test home — the
    /// pin's subject was always THIS module's sweep.)
    #[test]
    fn recompute_sweep_recomputes_all_and_cleans_stale() {
        crate::register_sqlite_vec::register_sqlite_vec();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sweep.db");
        // Schema first on a raw connection (the pool reads the same file).
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            crate::migration::run_migration(&mut conn, crate::config::DB_MMAP_SIZE_MIB)
                .expect("migration");
            conn.execute(
                "INSERT INTO knowledge(id, content, content_hash, domain) VALUES
                    (1, 'a', 'a', 'visa')",
                [],
            )
            .unwrap();
            let v: Vec<f32> = (0..512).map(|i| (i as f32 * 0.01).sin()).collect();
            let blob: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
            conn.execute(
                "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
                 VALUES (1, vec_quantize_int8(?1, 'unit'), vec_quantize_binary(?1), 'test', datetime('now'))",
                rusqlite::params![blob],
            )
            .unwrap();
            // A stale centroid for a domain with zero rows must be cleaned.
            conn.execute(
                "INSERT INTO domain_centroids (domain, centroid, count) VALUES ('dead', X'ABCD', 3)",
                [],
            )
            .unwrap();
        }
        let pool: crate::Pool = r2d2::Pool::builder()
            .build(crate::pool::SqliteConnectionManager::file(&path))
            .expect("pool build");
        let out = crate::domain_router::recompute_all_centroids(&pool).unwrap();
        let rows: std::collections::BTreeMap<String, usize> = out.into_iter().collect();
        assert_eq!(rows.get("visa"), Some(&1), "visa recomputed from vec0");
        assert_eq!(
            rows.get("dead"),
            Some(&0),
            "emptied domain processed with count 0 (centroid cleaned)"
        );
        let dead_rows: i64 = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM domain_centroids WHERE domain='dead'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dead_rows, 0, "stale centroid deleted");
    }
}
