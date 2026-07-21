//! Pure ranking-metric functions and unit tests.
//!
//! These functions are dependency-free (std only) and operate on abstract rank
//! lists so they can be exercised without the embedding model or a live server.
//! `results` are document ids in rank order (rank 0 = top), `relevant` are the
//! indices of judged-relevant documents, and `k` is the cutoff depth.
//!
//! Metric definitions follow the standard information-retrieval conventions:
//!   - Järvelin & Kekäläinen (2002), "Cumulated Gain-Based Evaluation of IR
//!     Techniques", ACM TOIS 20(4). https://dl.acm.org/doi/10.1145/582415.582418
//!   - Wikipedia, "Discounted cumulative gain".
//!     https://en.wikipedia.org/wiki/Discounted_cumulative_gain
//!
//! Relevance is binary (graded value 1 for relevant, 0 otherwise). A relevant
//! document contributes gain at most ONCE — at its best (highest) rank; later
//! duplicate occurrences of the same relevant id count as non-relevant. This
//! keeps nDCG in [0, 1] and recall/precision in [0, 1] even when a rank list
//! contains duplicate ids.

#![cfg(test)]

/// recall@k: fraction of judged-relevant docs present in the top-k results.
///
/// = |distinct relevant ∩ top-k| / |relevant|. Duplicate result ids do not
/// inflate the count. Empty relevant set returns 1.0 (perfect, by convention).
fn recall_at_k(results: &[i64], relevant: &[usize], k: usize) -> f32 {
    if relevant.is_empty() {
        return 1.0;
    }
    let rel: std::collections::HashSet<i64> = relevant.iter().map(|&r| r as i64).collect();
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let found = results
        .iter()
        .take(k)
        .filter(|r| rel.contains(r))
        .filter(|r| seen.insert(**r))
        .count();
    found as f32 / relevant.len() as f32
}

/// precision@k: fraction of the top-k results that are judged relevant.
///
/// = |distinct relevant ∩ top-k| / k.
fn precision_at_k(results: &[i64], relevant: &[usize], k: usize) -> f32 {
    if k == 0 {
        return 0.0;
    }
    let rel: std::collections::HashSet<i64> = relevant.iter().map(|&r| r as i64).collect();
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let hits = results
        .iter()
        .take(k)
        .filter(|r| rel.contains(r))
        .filter(|r| seen.insert(**r))
        .count();
    hits as f32 / k as f32
}

/// ndcg@k: Normalized Discounted Cumulative Gain at depth k (binary relevance).
///
/// DCG@k = Σ_{i=1..k} rel_i / log2(i+1), where rel_i = 1 the FIRST time a
/// relevant id appears at position i (1-indexed), else 0. Each distinct
/// relevant id contributes gain at most once, at its best rank.
/// IDCG@k = Σ_{i=1..min(k, |relevant|)} 1 / log2(i+1)  (ideal = all relevant first).
/// nDCG@k = DCG@k / IDCG@k, in [0, 1]. Empty relevant set returns 1.0; k==0 -> 0.0.
fn ndcg_at_k(results: &[i64], relevant: &[usize], k: usize) -> f32 {
    if k == 0 {
        return 0.0;
    }
    if relevant.is_empty() {
        return 1.0;
    }
    let rel: std::collections::HashSet<i64> = relevant.iter().map(|&r| r as i64).collect();
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let dcg: f32 = results
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, r)| {
            if rel.contains(r) && seen.insert(*r) {
                1.0 / (i as f32 + 2.0).log2()
            } else {
                0.0
            }
        })
        .sum();
    let ideal_depth = k.min(relevant.len());
    let idcg: f32 = (0..ideal_depth)
        .map(|i| 1.0 / (i as f32 + 2.0).log2())
        .sum();
    dcg / idcg
}

/// mrr: Mean Reciprocal Rank for a single query.
///
/// Reciprocal rank = 1 / (rank of first relevant result, 1-indexed).
/// Returns 0.0 if no relevant result appears. (MRR over many queries is the
/// mean of these per-query values.)
fn mrr(results: &[i64], relevant: &[usize]) -> f32 {
    let rel: std::collections::HashSet<i64> = relevant.iter().map(|&r| r as i64).collect();
    for (i, r) in results.iter().enumerate() {
        if rel.contains(r) {
            return 1.0 / (i as f32 + 1.0);
        }
    }
    0.0
}

#[test]
fn test_recall_at_k_metric() {
    // 1 of 2 relevant in top-5 -> 0.5
    let r = recall_at_k(&[0, 3, 5, 1, 7, 2, 9], &[0, 8], 5);
    assert!((r - 0.5).abs() < 1e-6, "expected 0.5, got {r}");

    // both relevant in top-k -> 1.0
    assert!((recall_at_k(&[1, 7, 0], &[1, 7], 5) - 1.0).abs() < 1e-6);

    // none relevant -> 0.0
    assert!((recall_at_k(&[2, 4, 6], &[0, 8], 5) - 0.0).abs() < 1e-6);

    // empty relevant -> perfect by convention
    assert!((recall_at_k(&[1, 2], &[], 5) - 1.0).abs() < 1e-6);

    // k smaller than first relevant hit
    assert!((recall_at_k(&[2, 0, 8], &[0, 8], 1) - 0.0).abs() < 1e-6);
}

#[test]
fn test_precision_at_k_metric() {
    // 1 of 5 top results relevant -> 0.2
    let p = precision_at_k(&[0, 3, 5, 1, 7], &[0, 8], 5);
    assert!((p - 0.2).abs() < 1e-6, "expected 0.2, got {p}");

    // all top-k relevant -> 1.0
    assert!((precision_at_k(&[0, 8], &[0, 8], 2) - 1.0).abs() < 1e-6);

    // none relevant -> 0.0
    assert!((precision_at_k(&[2, 4], &[0, 8], 2) - 0.0).abs() < 1e-6);

    // empty relevant set -> hits=0 -> 0.0 (k>0)
    assert!((precision_at_k(&[0, 8], &[], 5) - 0.0).abs() < 1e-6);

    // k == 0 -> 0.0
    assert!((precision_at_k(&[0], &[0], 0) - 0.0).abs() < 1e-6);
}

#[test]
fn test_ndcg_at_k_metric() {
    // Both relevant in top-2, ideal order -> 1.0
    let r = ndcg_at_k(&[0, 8, 2, 4, 6], &[0, 8], 5);
    assert!((r - 1.0).abs() < 1e-6, "ideal placement -> 1.0, got {r}");

    // One relevant pushed to rank 4: hand-computed.
    // DCG = 1/log2(3) + 1/log2(5) = 0.630930 + 0.430677 = 1.061606
    // IDCG = 1/log2(2) + 1/log2(3) = 1.630930
    // nDCG = 0.650918
    let r = ndcg_at_k(&[3, 0, 2, 8, 1], &[0, 8], 4);
    assert!((r - 0.650918).abs() < 1e-4, "expected ~0.650918, got {r}");

    // Only one relevant, at rank 2: DCG = 1/log2(3) = 0.630930, IDCG = 1.0 -> 0.630930
    let r = ndcg_at_k(&[3, 0, 2], &[0], 3);
    assert!((r - 0.630930).abs() < 1e-4, "expected ~0.630930, got {r}");

    // Duplicate relevant ids count only once (at best rank): rel=[0],
    // results=[0,0,0], k=3 -> DCG = 1.0 (only the first), IDCG = 1.0 -> 1.0
    let r = ndcg_at_k(&[0, 0, 0], &[0], 3);
    assert!(
        (r - 1.0).abs() < 1e-4,
        "duplicate relevant must not inflate nDCG, got {r}"
    );

    // empty relevant -> 1.0; k==0 -> 0.0
    assert!((ndcg_at_k(&[1, 2], &[], 5) - 1.0).abs() < 1e-6);
    assert!((ndcg_at_k(&[1, 2], &[0], 0) - 0.0).abs() < 1e-6);
}

#[test]
fn test_mrr_metric() {
    // first relevant at rank 3 (1-indexed) -> 1/3
    let r = mrr(&[3, 1, 8, 0], &[0, 8]);
    assert!((r - 1.0 / 3.0).abs() < 1e-6, "expected 1/3, got {r}");

    // first relevant at rank 1 -> 1.0
    assert!((mrr(&[0, 8, 2], &[0, 8]) - 1.0).abs() < 1e-6);

    // no relevant -> 0.0
    assert!((mrr(&[2, 4, 6], &[0, 8]) - 0.0).abs() < 1e-6);
}

// ── v0.9.8 "Evidence" metrics ───────────────────────────────────────────────
//
// These measure *correctness across change* — independent of conventional
// recall@k. Each operates on abstract inputs (lists of (chunk_id, is_current)
// tuples and a relevance set of current-chunk ids) so it can be unit-tested
// without a model or live server, mirroring the metric style above.

/// Stale-result rate: fraction of top-k hits that are NOT current (i.e.
/// superseded/tombstoned chunks that leaked into current-mode recall).
/// Should be 0.0 by construction. `current` maps chunk id -> true if current.
fn stale_result_rate(
    results: &[(i64, bool)],
    current: &std::collections::HashSet<i64>,
    k: usize,
) -> f32 {
    if k == 0 {
        return 0.0;
    }
    let top: Vec<&(i64, bool)> = results.iter().take(k).collect();
    if top.is_empty() {
        return 0.0;
    }
    let stale = top.iter().filter(|(id, _)| !current.contains(id)).count();
    stale as f32 / top.len() as f32
}

/// Current-evidence recall: fraction of judged-current relevant docs present
/// in the top-k, where a hit only counts if it is itself current. `relevant`
/// are chunk ids that are the *current* truth; a stale revision of the same
/// subject does not satisfy it.
fn current_evidence_recall(
    results: &[(i64, bool)],
    relevant: &[i64],
    current: &std::collections::HashSet<i64>,
    k: usize,
) -> f32 {
    if relevant.is_empty() {
        return 1.0;
    }
    let rel: std::collections::HashSet<i64> = relevant.iter().copied().collect();
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let found = results
        .iter()
        .take(k)
        .filter(|(id, is_current)| rel.contains(id) && *is_current && current.contains(id))
        .filter(|(id, _)| seen.insert(*id))
        .count();
    found as f32 / relevant.len() as f32
}

/// Citation correctness: fraction of hits whose cited revision id dereferences
/// to a row that exists and is current. `deref_current` maps revision_id ->
/// true if the revision row exists and is `active`. A hit with no revision link
/// (legacy unlinked chunk) is treated as correct (nothing to dereference).
fn citation_correctness(
    results: &[(i64, Option<i64>)],
    deref_current: &dyn Fn(Option<i64>) -> bool,
) -> f32 {
    if results.is_empty() {
        return 1.0;
    }
    let ok = results
        .iter()
        .filter(|(_, rev)| deref_current(*rev))
        .count();
    ok as f32 / results.len() as f32
}

/// Consolidation false-positive rate: fraction of pairs the detector flagged as
/// conflicts that are actually identical content (same content_hash). Should be
/// 0.0 — a duplicate is not a conflict. `flagged_same_content` counts how many
/// of `flagged` pairs share content.
fn consolidation_false_positive_rate(flagged: usize, flagged_same_content: usize) -> f32 {
    if flagged == 0 {
        return 0.0;
    }
    flagged_same_content as f32 / flagged as f32
}

#[test]
fn test_stale_result_rate() {
    // All current -> 0.0
    let r = stale_result_rate(
        &[(1, true), (2, true), (3, true)],
        &[1, 2, 3].into_iter().collect(),
        5,
    );
    assert!((r - 0.0).abs() < 1e-6);

    // One superseded in top-3 -> 1/3
    let cur: std::collections::HashSet<i64> = [1, 2].into_iter().collect();
    let r = stale_result_rate(&[(1, true), (9, false), (2, true)], &cur, 3);
    assert!((r - 1.0 / 3.0).abs() < 1e-6);

    // k truncates before the stale hit -> 0.0
    let r = stale_result_rate(&[(1, true), (2, true), (9, false)], &cur, 2);
    assert!((r - 0.0).abs() < 1e-6);
}

#[test]
fn test_current_evidence_recall() {
    let cur: std::collections::HashSet<i64> = [1, 2, 3].into_iter().collect();
    // relevant = [1, 4]; only current chunk 1 present in top-k -> 0.5
    let r = current_evidence_recall(&[(1, true), (9, false)], &[1, 4], &cur, 5);
    assert!((r - 0.5).abs() < 1e-6);

    // both relevant current present -> 1.0
    let cur2: std::collections::HashSet<i64> = [1, 2, 3, 4].into_iter().collect();
    assert!(
        (current_evidence_recall(&[(1, true), (4, true)], &[1, 4], &cur2, 5) - 1.0).abs() < 1e-6
    );

    // stale revision of relevant subject does not satisfy -> 0.0 for that one
    let r = current_evidence_recall(&[(4, false)], &[4], &cur, 5);
    assert!((r - 0.0).abs() < 1e-6);

    // empty relevant -> 1.0
    assert!((current_evidence_recall(&[], &[], &cur, 5) - 1.0).abs() < 1e-6);
}

#[test]
fn test_citation_correctness() {
    // All dereference to current -> 1.0
    let ok = |rev: Option<i64>| rev.is_some();
    assert!((citation_correctness(&[(1, Some(7)), (2, Some(8))], &ok) - 1.0).abs() < 1e-6);

    // One dangling revision id -> 0.5
    let partially = |rev: Option<i64>| rev == Some(7);
    let r = citation_correctness(&[(1, Some(7)), (2, Some(99))], &partially);
    assert!((r - 0.5).abs() < 1e-6);

    // legacy unlinked (None) counts as correct (deref fn treats None as valid)
    let r = citation_correctness(&[(1, None)], &|_| true);
    assert!((r - 1.0).abs() < 1e-6);
}

#[test]
fn test_consolidation_false_positive_rate() {
    // No flags -> 0.0 (by convention, not NaN)
    assert!((consolidation_false_positive_rate(0, 0) - 0.0).abs() < 1e-6);
    // 2 flagged, 1 same-content -> 0.5
    assert!((consolidation_false_positive_rate(2, 1) - 0.5).abs() < 1e-6);
    // 3 flagged, 0 same-content -> 0.0 (clean detector)
    assert!((consolidation_false_positive_rate(3, 0) - 0.0).abs() < 1e-6);
}
