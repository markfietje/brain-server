//! Retrieval Quality Estimation (Query Performance Prediction)
//!
//! Provides a retrieval-agnostic assessment of search result quality
//! to drive adaptive retrieval decisions (PRF, clarify, etc.).

use crate::config::QualityConfig;
use crate::search::{SearchResult, SearchTelemetry};
use serde::Serialize;

/// Quality signals extracted from the hybrid retrieval result set.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Confidence {
    /// Overall confidence score (0.0–1.0).
    pub score: f32,
    /// Fraction of top-k results appearing in both vector and lexical ranks.
    pub overlap: f32,
    /// Normalized score gap between top-1 and top-2 results.
    pub gap: f32,
    /// Reciprocal rank of the top result (1 / rank).
    pub reciprocal_rank: f32,
    /// Lexical density proxy (term overlap / query length).
    pub lexical_density: f32,
}

/// Actionable recommendation based on confidence assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Recommendation {
    /// Results are strong enough; return immediately.
    Return,
    /// Confidence is moderate; run pseudo-relevance feedback expansion.
    RunPrf,
    /// High confidence; retrieval is strong, no further refinement needed.
    RunReranker,
    /// Low confidence but some signal; increase candidate window.
    IncreaseTopK,
    /// Very low confidence; ask caller to clarify or reformulate.
    ClarifyQuery,
}

/// Complete quality assessment with versioning for forward compatibility.
#[derive(Debug, Clone, Serialize)]
pub struct RetrievalAssessment {
    /// Assessment schema version (increment on heuristic changes).
    pub version: u16,
    /// Detailed confidence signals.
    pub confidence: Confidence,
    /// Recommended next action.
    pub recommendation: Recommendation,
}

impl RetrievalAssessment {
    /// Current assessment schema version.
    pub const VERSION: u16 = 1;
}

/// Trait for pluggable retrieval quality estimators.
///
/// Implementations must be deterministic given identical inputs
/// (query, results, telemetry, config) to enable regression testing.
pub trait RetrievalQualityEstimator: Send + Sync {
    /// Assess retrieval quality and return an actionable recommendation.
    fn assess(
        &self,
        query: &str,
        results: &[SearchResult],
        telemetry: &SearchTelemetry,
    ) -> RetrievalAssessment;
}

/// Heuristic estimator using weighted linear combination of quality signals.
///
/// Weights and thresholds are loaded from [`QualityConfig`].
#[derive(Debug, Clone)]
pub struct HeuristicEstimator {
    overlap_w: f32,
    gap_w: f32,
    rr_w: f32,
    lex_w: f32,
    agreement_min: usize,
    gap_threshold: f32,
    confidence_threshold: f32,
    rerank_threshold: f32,
}

impl HeuristicEstimator {
    /// Build estimator from [`QualityConfig`].
    pub fn new(cfg: QualityConfig) -> Self {
        Self {
            overlap_w: cfg.overlap_weight,
            gap_w: cfg.gap_weight,
            rr_w: cfg.rr_weight,
            lex_w: cfg.lex_weight,
            agreement_min: cfg.agreement_min,
            gap_threshold: cfg.gap_threshold,
            confidence_threshold: cfg.confidence_threshold,
            rerank_threshold: cfg.rerank_threshold,
        }
    }

    /// Compute quality signals from hybrid retrieval results.
    fn compute_signals(
        &self,
        query: &str,
        results: &[SearchResult],
        telemetry: &SearchTelemetry,
    ) -> Confidence {
        let _ = telemetry; // available for future signal enrichment
        let k = results.len().max(1);

        // Overlap: fraction of top-k results that have both vector and FTS ranks.
        let both = results
            .iter()
            .filter(|r| r.provenance.vector_rank.is_some() && r.provenance.fts_rank.is_some())
            .count();
        let overlap = both as f32 / k as f32;

        // Gap: normalized score difference between top-1 and top-2.
        let gap = if results.len() >= 2 {
            let top = results[0].score.max(1e-6);
            ((results[0].score - results[1].score) / top).clamp(0.0, 1.0)
        } else {
            1.0 // single result -> maximal gap
        };

        // Reciprocal rank: 1 / (1 + min_rank) where min_rank is best position in either list.
        let reciprocal_rank = results
            .iter()
            .map(|r| {
                let vr = r.provenance.vector_rank.unwrap_or(usize::MAX);
                let fr = r.provenance.fts_rank.unwrap_or(usize::MAX);
                vr.min(fr)
            })
            .min()
            .map(|r| 1.0 / (1.0 + r as f32))
            .unwrap_or(0.0);

        // Lexical density: query term coverage in top result snippet/content.
        let lexical_density = if let Some(top) = results.first() {
            let content = top
                .snippet
                .as_deref()
                .unwrap_or(&top.content)
                .to_lowercase();
            let query_lower = query.to_lowercase();
            let query_terms: Vec<&str> = query_lower.split_whitespace().collect();
            if query_terms.is_empty() {
                0.0
            } else {
                let matches = query_terms.iter().filter(|t| content.contains(*t)).count();
                (matches as f32 / query_terms.len() as f32).min(1.0)
            }
        } else {
            0.0
        };

        // Weighted combination.
        let score = (self.overlap_w * overlap
            + self.gap_w * gap
            + self.rr_w * reciprocal_rank
            + self.lex_w * lexical_density)
            .clamp(0.0, 1.0);

        Confidence {
            score,
            overlap,
            gap,
            reciprocal_rank,
            lexical_density,
        }
    }

    /// Map confidence signals to a recommendation.
    fn decide(&self, confidence: &Confidence) -> Recommendation {
        // Hard gates first (data-dependent, not score-dependent).
        if confidence.overlap < self.agreement_min as f32 / 10.0 {
            // agreement_min is absolute count
            return Recommendation::IncreaseTopK;
        }
        if confidence.gap < self.gap_threshold {
            return Recommendation::RunPrf;
        }

        // Score-based policy.
        if confidence.score >= self.rerank_threshold {
            Recommendation::RunReranker
        } else if confidence.score >= self.confidence_threshold {
            Recommendation::RunPrf
        } else if confidence.score >= 0.35 {
            Recommendation::IncreaseTopK
        } else {
            Recommendation::ClarifyQuery
        }
    }
}

impl RetrievalQualityEstimator for HeuristicEstimator {
    fn assess(
        &self,
        query: &str,
        results: &[SearchResult],
        telemetry: &SearchTelemetry,
    ) -> RetrievalAssessment {
        let _ = telemetry; // available for future signal enrichment
        let confidence = self.compute_signals(query, results, telemetry);
        let recommendation = self.decide(&confidence);

        RetrievalAssessment {
            version: RetrievalAssessment::VERSION,
            confidence,
            recommendation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{Provenance, SearchResult, SearchSource, SearchTelemetry};

    fn sr(id: i64, score: f32, vr: Option<usize>, fr: Option<usize>) -> SearchResult {
        SearchResult {
            id,
            score,
            title: None,
            content: format!("doc {id} content"),
            source: Some(SearchSource::Both),
            provenance: Provenance {
                vector_rank: vr,
                fts_rank: fr,
                fused_score: Some(score),
                rerank_score: None,
                rerank_truncated: false,
                prf_expanded: false,
                top_retrieval_mode: None,
                quality_assessment: None,
                prf_decision: None,
                retrieval_strategy: None,
            },
            flagged: false,
            untrusted: true,
            snippet: None,
            evidence: None,
            ..Default::default()
        }
    }

    #[test]
    fn deterministic_assessment() {
        let cfg = QualityConfig::default();
        let est = HeuristicEstimator::new(cfg);
        let results = vec![
            sr(1, 0.95, Some(0), Some(0)),
            sr(2, 0.85, Some(1), Some(1)),
            sr(3, 0.70, Some(2), Some(2)),
        ];
        let tel = SearchTelemetry::default();
        let a1 = est.assess("test query", &results, &tel);
        let a2 = est.assess("test query", &results, &tel);
        assert_eq!(a1.confidence.score, a2.confidence.score);
        assert_eq!(a1.recommendation, a2.recommendation);
    }

    #[test]
    fn high_overlap_high_gap_returns_prf() {
        let cfg = QualityConfig {
            overlap_weight: 0.4,
            gap_weight: 0.3,
            rr_weight: 0.2,
            lex_weight: 0.1,
            agreement_min: 2,
            gap_threshold: 0.023,
            confidence_threshold: 0.6,
            rerank_threshold: 0.85,
        };
        let est = HeuristicEstimator::new(cfg);
        // Use query terms that appear in content to get non-zero lexical_density
        let results = vec![
            sr(1, 0.95, Some(0), Some(0)),
            sr(2, 0.60, Some(1), Some(1)), // large gap
        ];
        let tel = SearchTelemetry::default();
        let assessment = est.assess("doc content terms", &results, &tel);
        // score ≈ 0.4*1 + 0.3*0.368 + 0.2*1 + 0.1*0.67 = 0.4 + 0.11 + 0.2 + 0.067 = 0.777
        // Below rerank_threshold (0.85), so RunPrf
        assert_eq!(assessment.recommendation, Recommendation::RunPrf);
    }

    #[test]
    fn perfect_signals_returns_reranker() {
        let cfg = QualityConfig {
            overlap_weight: 0.4,
            gap_weight: 0.3,
            rr_weight: 0.2,
            lex_weight: 0.1,
            agreement_min: 2,
            gap_threshold: 0.023,
            confidence_threshold: 0.6,
            rerank_threshold: 0.85,
        };
        let est = HeuristicEstimator::new(cfg);
        // Perfect signals: overlap=1, gap=1 (top2=0), rr=1, lex=1 (all terms match)
        let results = vec![
            sr(1, 1.0, Some(0), Some(0)),
            sr(2, 0.0, Some(1), Some(1)), // gap = 1.0
        ];
        let tel = SearchTelemetry::default();
        let assessment = est.assess("doc content", &results, &tel);
        // score = 0.4*1 + 0.3*1 + 0.2*1 + 0.1*1 = 1.0 >= 0.85
        assert_eq!(assessment.recommendation, Recommendation::RunReranker);
    }

    #[test]
    fn low_overlap_returns_increase_topk() {
        let cfg = QualityConfig::default();
        let est = HeuristicEstimator::new(cfg);
        let results = vec![
            sr(1, 0.9, Some(0), None), // vector only
            sr(2, 0.8, None, Some(0)), // fts only
        ];
        let tel = SearchTelemetry::default();
        let assessment = est.assess("query", &results, &tel);
        assert_eq!(assessment.recommendation, Recommendation::IncreaseTopK);
    }

    #[test]
    fn small_gap_triggers_prf() {
        let cfg = QualityConfig {
            overlap_weight: 0.4,
            gap_weight: 0.3,
            rr_weight: 0.2,
            lex_weight: 0.1,
            agreement_min: 1,
            gap_threshold: 0.023,
            confidence_threshold: 0.6,
            rerank_threshold: 0.85,
        };
        let est = HeuristicEstimator::new(cfg);
        let results = vec![
            sr(1, 0.90, Some(0), Some(0)),
            sr(2, 0.89, Some(1), Some(1)), // tiny gap
        ];
        let tel = SearchTelemetry::default();
        let assessment = est.assess("query", &results, &tel);
        assert_eq!(assessment.recommendation, Recommendation::RunPrf);
    }
}
