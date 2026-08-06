//! Centroid routing for per-domain recall (P2).
//!
//! Each domain's mean embedding vector (centroid) is stored in the global DB.
//! At query time the query vector is compared (cosine) to every domain
//! centroid; the best-scoring domain above a confidence threshold is searched
//! exclusively (strict isolation). With no confident domain and non-strict
//! mode, recall federates across all known domains and labels each hit with its
//! source domain. ponytail: centroids are simple arithmetic means of raw f32
//! embeddings (not learned); upgrade path is a per-domain probe-set / SVM if a
//! corpus needs sharper separation.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::search::cosine_sim;
use crate::Pool;

/// Minimum cosine similarity between a query and a domain centroid for the
/// query to be confidently routed to that domain. Below this, non-strict recall
/// federates across domains.
pub const DOMAIN_CONFIDENCE_THRESHOLD: f32 = 0.30;

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
/// threshold. v1.13.0 M2. Pure + deterministic — the same `route()` recall uses.
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
    let centroid = mean_vector(&vectors);
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

/// v1.13.0 M4: one-shot recompute of every known domain's centroid from the
/// corrected M1 source. Domain set = `DISTINCT knowledge.domain` ∪ existing
/// `domain_centroids` rows (so a domain that emptied out also gets its stale
/// centroid cleaned). In shim mode all domains share the global pool. Returns
/// `(domain, vector_count)` per domain — the post-migration catch-up sweep
/// that makes M2's auto-route meaningful (until real centroids exist, `route()`
/// only ever sees `global`).
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
        out.push((d, count));
    }
    Ok(out)
}

/// Read a domain's current (non-superseded) chunk vectors from the live vec0
/// index. v1.13.0 fix: was reading the frozen legacy `embeddings` JSON table
/// (2 rows since v0.9.0), which silently zeroed every centroid. Now reads
/// `vec_knowledge`, matching `find_near_duplicates` (consolidate.rs:260), and
/// dequantizes via `decode_embedding`. `valid_to IS NULL` excludes superseded
/// chunks (the loser of a contradiction resolution) so a centroid isn't pulled
/// toward outdated content. Kept Connection-taking so tests use `test_db()`.
pub(crate) fn read_domain_vectors(conn: &Connection, domain: &str) -> Result<Vec<Vec<f32>>> {
    let mut stmt = conn.prepare(
        "SELECT v.embedding_int8
         FROM vec_knowledge v
         JOIN knowledge k ON k.id = v.knowledge_id
         WHERE k.domain = ?1 AND k.valid_to IS NULL",
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
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
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

    // ── v1.13.0 M2: ingest auto-routing (pure decision) ──────────────────

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
}
