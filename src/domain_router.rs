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
use rusqlite::params;

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
    let mut stmt = dconn.prepare(
        "SELECT e.vector FROM embeddings e
         JOIN knowledge k ON k.id = e.knowledge_id
         WHERE k.domain = ?1",
    )?;
    let mut vectors: Vec<Vec<f32>> = Vec::new();
    let rows = stmt.query_map(params![domain], |row| {
        let json: String = row.get(0)?;
        Ok(json)
    })?;
    for json in rows.flatten() {
        if let Ok(v) = serde_json::from_str::<Vec<f32>>(&json) {
            if !v.is_empty() {
                vectors.push(v);
            }
        }
    }
    drop(stmt);
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
}
