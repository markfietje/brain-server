//! The production context retriever: the decision harness's
//! [`ContextRetriever`] seam wired to the SHIPPED hybrid-search internals.
//! This module is a NEW CALLER of `crate::search` — the search module is
//! never edited by this wiring, and the recall hot path is untouched (no
//! existing caller changes shape or cost).
//!
//! The laws this wiring keeps:
//! - TEXT NEVER CROSSES THE SEAM. The search results' content exists only
//!   inside `retrieve`, is hashed immediately, and is dropped — the
//!   reference-only [`ContextHit`]s are all that escapes.
//! - TAINT IS TRUTH. `flagged`/`untrusted` carry through untouched, and
//!   every hit is recorded at the least-trusting tier: the search
//!   contract itself marks ALL retrieved content untrusted evidence
//!   (OWASP LLM01), and no per-row governance law exists that would
//!   honestly justify a higher tier for retrieved rows. A decision run
//!   over retrieved evidence therefore escalates under the policy stage
//!   as honest data instead of trusting a guess.
//! - PER-LEG PROVENANCE IS THE RECORD. The search pipeline's per-retriever
//!   ranks and fused score map one-to-one onto the hit's leg fields, so
//!   the trace shows exactly which legs contributed what.

use std::sync::Arc;

use crate::Pool;
use crate::embed::Embedder;
use crate::search::{SearchFilters, perform_search};

use super::config::RetrievalParams;
use super::pipeline::{ContextHit, ContextRetriever, RetrievalRefusal, SerdeTier};

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// The live retriever: pool + embedder, the exact state the shipped
/// `perform_search` entry point needs — nothing more. Sync by the seam's
/// contract; the route handlers call it inside `spawn_blocking`.
pub(crate) struct SearchRetriever {
    pub(crate) pool: Pool,
    pub(crate) model: Arc<dyn Embedder>,
}

impl ContextRetriever for SearchRetriever {
    fn retrieve(
        &self,
        query: &str,
        params: &RetrievalParams,
    ) -> Result<Vec<ContextHit>, RetrievalRefusal> {
        let k = params.limit as usize;
        let filters = SearchFilters {
            source_leg: params.leg.leg_filter(),
            include_flagged: false,
            ..SearchFilters::default()
        };
        let results = perform_search(&self.pool, &*self.model, query.to_string(), k, &filters)
            .map_err(|_| RetrievalRefusal::Unavailable)?;
        let mut hits: Vec<ContextHit> = results.into_iter().map(to_context_hit).collect();
        hits.truncate(k);
        Ok(hits)
    }

    fn algorithm(&self) -> &str {
        "search.perform_search/v1"
    }
}

/// The reference-only mapping: one digested, tainted, per-leg-annotated
/// hit — the row's text is consumed here and never leaves.
pub(crate) fn to_context_hit(r: crate::search::SearchResult) -> ContextHit {
    ContextHit {
        evidence_id: r.id.to_string(),
        content_digest: sha256_hex(r.content.as_bytes()),
        tier: SerdeTier::Untrusted,
        vector_rank: r.provenance.vector_rank,
        fts_rank: r.provenance.fts_rank,
        graph_rank: r.provenance.graph_rank,
        fused_score: r.provenance.fused_score,
        flagged: r.flagged,
        untrusted: r.untrusted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Provenance;

    /// The mapping law: per-leg provenance and taint carry through, the
    /// tier pins at the least-trusting value, the digest is the content's
    /// sha256, and the text itself never survives the mapping.
    #[test]
    fn search_retriever_maps_provenance_and_taint() {
        let prov = Provenance {
            vector_rank: Some(0),
            fts_rank: Some(3),
            graph_rank: None,
            fused_score: Some(0.0175),
            ..Default::default()
        };
        let hit = to_context_hit(crate::search::SearchResult {
            id: 4041,
            score: 0.9,
            title: Some("t".into()),
            content: "sensitive body text that must never cross the seam".into(),
            provenance: prov,
            flagged: true,
            untrusted: true,
            ..Default::default()
        });
        assert_eq!(hit.evidence_id, "4041");
        assert_eq!(
            hit.content_digest,
            sha256_hex("sensitive body text that must never cross the seam".as_bytes())
        );
        assert_eq!(hit.tier, SerdeTier::Untrusted);
        assert_eq!(hit.vector_rank, Some(0));
        assert_eq!(hit.fts_rank, Some(3));
        assert_eq!(hit.graph_rank, None);
        assert_eq!(hit.fused_score, Some(0.0175));
        assert!(hit.flagged);
        assert!(hit.untrusted);
        let json = serde_json::to_string(&hit).unwrap();
        assert!(
            !json.contains("sensitive"),
            "text is unrepresentable: {json}"
        );
        assert!(
            !json.contains("body text"),
            "text is unrepresentable: {json}"
        );
    }
}
