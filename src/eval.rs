//! Retrieval-quality metrics.
//!
//! Pure metric functions for the regression bench harness: precision@k, recall@k,
//! MRR, NDCG, plus the paper's `answer_in_context` diagnostic. No I/O, no
//! deps — the smallest checks that fail if a metric is computed wrong.
//!
//! The harness loads a judgments file (`BRAIN_EVAL_JUDGMENTS`, JSON: a list of
//! `{query, relevant_ids: [int], gold_answer?: string}`) and runs each query
//! through `/recall`, then computes the metrics over the ranked results. The
//! 100-query hand-judged corpus against the live DB is an operator step — these
//! functions are the reproducible engine any corpus plugs into.

#![deny(unsafe_code)]

/// Precision@k: fraction of the top-k retrieved ids that are relevant.
/// Empty relevant set ⇒ 0 (no judgment means we can't credit a hit).
pub fn precision_at_k(retrieved: &[i64], relevant: &[i64], k: usize) -> f32 {
    if relevant.is_empty() || k == 0 {
        return 0.0;
    }
    let topk = retrieved.iter().take(k);
    let hits = topk.filter(|r| relevant.contains(r)).count();
    hits as f32 / k as f32
}

/// Recall@k: fraction of relevant ids appearing in the top-k retrieved.
pub fn recall_at_k(retrieved: &[i64], relevant: &[i64], k: usize) -> f32 {
    if relevant.is_empty() {
        return 0.0;
    }
    let topk: Vec<&i64> = retrieved.iter().take(k).collect();
    let hits = relevant.iter().filter(|r| topk.contains(r)).count();
    hits as f32 / relevant.len() as f32
}

/// Mean Reciprocal Rank: 1/rank of the first relevant result (0 if none in list).
pub fn mrr(retrieved: &[i64], relevant: &[i64]) -> f32 {
    if relevant.is_empty() {
        return 0.0;
    }
    for (i, r) in retrieved.iter().enumerate() {
        if relevant.contains(r) {
            return 1.0 / (i as f32 + 1.0);
        }
    }
    0.0
}

/// Normalized Discounted Cumulative Gain. Binary relevance (relevant=1 else 0),
/// ideal DCG = sort by relevance desc (all relevant first). NDCG ∈ [0,1].
pub fn ndcg(retrieved: &[i64], relevant: &[i64], k: usize) -> f32 {
    if relevant.is_empty() || k == 0 {
        return 0.0;
    }
    let dcg: f32 = retrieved
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, r)| {
            let rel = if relevant.contains(r) { 1.0 } else { 0.0 };
            rel / (i as f32 + 2.0).log2()
        })
        .sum();
    // Ideal: all relevant items ranked first.
    let ideal_hits = relevant.len().min(k);
    let idcg: f32 = (0..ideal_hits).map(|i| 1.0 / (i as f32 + 2.0).log2()).sum();
    if idcg == 0.0 { 0.0 } else { dcg / idcg }
}

/// Aggregate metrics over a set of queries.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EvalReport {
    pub queries: usize,
    pub precision_at_5: f32,
    pub recall_at_5: f32,
    pub mrr: f32,
    pub ndcg_at_5: f32,
    /// Fraction of queries whose gold answer survived into the packed context.
    pub answer_in_context_rate: f32,
}

/// One query's judgment + (optional) gold answer.
#[derive(Debug, Clone, Deserialize)]
pub struct Judgment {
    pub query: String,
    pub relevant_ids: Vec<i64>,
    #[serde(default)]
    pub gold_answer: Option<String>,
}

use serde::{Deserialize, Serialize};

/// Compute the aggregate report from per-query (retrieved_ids, gold_survived)
/// pairs. `k` defaults to 5 for P/R/NDCG (the recall default limit).
pub fn evaluate(judged: &[(Judgment, Vec<i64>, Option<bool>)], k: usize) -> EvalReport {
    if judged.is_empty() {
        return EvalReport::default();
    }
    let n = judged.len() as f32;
    let mut p = 0.0;
    let mut r = 0.0;
    let mut m = 0.0;
    let mut nd = 0.0;
    let mut aic_sum = 0.0;
    let mut aic_n = 0.0;
    for (j, retrieved, aic) in judged {
        p += precision_at_k(retrieved, &j.relevant_ids, k);
        r += recall_at_k(retrieved, &j.relevant_ids, k);
        m += mrr(retrieved, &j.relevant_ids);
        nd += ndcg(retrieved, &j.relevant_ids, k);
        if let Some(b) = aic {
            aic_sum += if *b { 1.0 } else { 0.0 };
            aic_n += 1.0;
        }
    }
    EvalReport {
        queries: judged.len(),
        precision_at_5: p / n,
        recall_at_5: r / n,
        mrr: m / n,
        ndcg_at_5: nd / n,
        answer_in_context_rate: if aic_n > 0.0 { aic_sum / aic_n } else { 0.0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_at_k_hand_computed() {
        // retrieved [1,2,3,4,5], relevant {2,4}, k=5 → 2/5
        let p = precision_at_k(&[1, 2, 3, 4, 5], &[2, 4], 5);
        assert!((p - 0.4).abs() < 1e-6);
    }

    #[test]
    fn recall_at_k_hand_computed() {
        // retrieved [1,2,3], relevant {2,4,6}, k=3 → 1/3
        let r = recall_at_k(&[1, 2, 3], &[2, 4, 6], 3);
        assert!((r - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn mrr_first_position() {
        assert!((mrr(&[1, 2], &[1]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mrr_third_position() {
        assert!((mrr(&[4, 5, 1], &[1]) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn mrr_no_relevant() {
        assert_eq!(mrr(&[1, 2], &[99]), 0.0);
    }

    #[test]
    fn ndcg_perfect_ranking_is_one() {
        // Both relevant items ranked first → NDCG = 1.0
        let n = ndcg(&[1, 2, 3], &[1, 2], 5);
        assert!((n - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ndcg_worst_ranking_is_zero() {
        // No relevant in top-k → NDCG = 0
        let n = ndcg(&[7, 8, 9], &[1, 2], 3);
        assert!((n - 0.0).abs() < 1e-6);
    }

    #[test]
    fn empty_relevant_returns_zero() {
        assert_eq!(precision_at_k(&[1], &[], 1), 0.0);
        assert_eq!(recall_at_k(&[1], &[], 1), 0.0);
        assert_eq!(mrr(&[1], &[]), 0.0);
        assert_eq!(ndcg(&[1], &[], 1), 0.0);
    }

    #[test]
    fn evaluate_aggregates_across_queries() {
        let judged = vec![
            (
                Judgment {
                    query: "q1".into(),
                    relevant_ids: vec![1, 2],
                    gold_answer: Some("answer".into()),
                },
                vec![1, 3],
                Some(true),
            ),
            (
                Judgment {
                    query: "q2".into(),
                    relevant_ids: vec![5],
                    gold_answer: None,
                },
                vec![5, 6],
                None,
            ),
        ];
        let rep = evaluate(&judged, 5);
        assert_eq!(rep.queries, 2);
        assert!((rep.answer_in_context_rate - 1.0).abs() < 1e-6); // 1/1 judged
    }
}
