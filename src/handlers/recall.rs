//! `POST /recall` — deterministic, end-to-end recall.
//!
//! Per `API_CONTRACT.md` §2, the server does embed → centroid auto-route →
//! hybrid (vec0 + FTS5, RRF) search → optional cross-domain fallback → cap.
//! One call per turn from the OpenClaw plugin's `before_prompt_build` hook.
//!
//! Implementation status:
//!   - Request/response serde ✅ (wire is locked)
//!   - Validation ✅ (bounds, regex, types)
//!   - Heavy logic: v0.9.0 Phase 1 (sqlite-vec) + Phase 2 (hybrid + RRF),
//!     and v1.0.0 Phase 3 (per-domain DBs + centroid routing).

use axum::{extract::State, Json};
use serde::de::Error as _;
use serde::Deserialize;
use std::sync::Arc;
use tokio::{
    task,
    time::{timeout, Duration as StdDuration},
};

/// Deserialize `lex` from either a bare string (legacy: treated as one term)
/// or a full `LexSpec` object (v0.9.5 structured form). Keeps the OpenClaw
/// plugin's `{"lex":"foo"}` working while enabling phrases/exclusions/code.
fn lex_from_string_or_struct<'de, D>(
    deserializer: D,
) -> Result<crate::search::query::LexSpec, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawLex {
        Str(String),
        Spec(crate::search::query::LexSpec),
    }
    match RawLex::deserialize(deserializer)? {
        RawLex::Str(s) => Ok(crate::search::query::LexSpec {
            terms: if s.trim().is_empty() {
                Vec::new()
            } else {
                vec![s]
            },
            ..Default::default()
        }),
        RawLex::Spec(spec) => Ok(spec),
    }
}

use crate::AppState;

use super::{
    normalize_domain, HitSource, RecallHit, RecallResponse, DEFAULT_RECALL_LIMIT, MAX_LIMIT,
    MAX_QUERY, MIN_LIMIT,
};
use crate::contains_suspicious_pattern;
use crate::handlers::HandlerError;

#[derive(Debug, Deserialize)]
pub struct RecallRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    pub domain: Option<String>,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub provenance: bool,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
    /// Structured lexical controls (phrases, exclusions, exact code paths).
    /// Accepts either a bare string (treated as a single term) OR a full
    /// `LexSpec` object, so legacy callers sending `"lex":"foo"` still work.
    #[serde(default, deserialize_with = "lex_from_string_or_struct")]
    pub lex: crate::search::query::LexSpec,
    #[serde(default)]
    pub vec: Option<String>,
    #[serde(default)]
    pub hyde: Option<String>,
    #[serde(default)]
    pub intent: Option<String>,
    /// Multi-source OR scope (v0.9.5 M1). Empty = unrestricted.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Retrieval profile hint (passthrough in M1).
    #[serde(default)]
    pub profile: Option<String>,
    /// v0.9.7 Guard: when true, include quarantined (`flagged`) chunks in the
    /// results. Operator review path only — the default agent path stays clean.
    #[serde(default)]
    pub include_flagged: bool,
    /// v0.9.8 "Evidence": point-in-time recall. RFC3339 instant; returns the
    /// revision current at that time (historical mode). When set, hits are
    /// tagged `lifecycle: "historical"`.
    #[serde(default)]
    pub as_of: Option<String>,
    /// v0.9.8 "Evidence": include structured `Evidence` (time + lifecycle +
    /// links) on every hit even without `provenance`.
    #[serde(default)]
    pub evidence: bool,
    /// v1.4.0 "Calibrate" M1: bi-temporal valid-time point-in-time filter.
    /// RFC3339 or `YYYY-MM-DD` instant; only chunks whose valid-interval
    /// (valid_from, valid_to) contains this instant are returned. Distinct
    /// from `as_of` (transaction-time / revision recall). Graphiti semantics.
    #[serde(default)]
    pub at: Option<String>,
    /// v1.4.0 "Calibrate" M2: submodular evidence packing token budget. When
    /// set, results are re-ranked by budgeted monotone submodular maximization
    /// (relevance + coverage + representativeness + diversity) and truncated
    /// to fit the budget. Replaces fixed `k` truncation for the packed path.
    #[serde(default)]
    pub max_context_tokens: Option<usize>,
    /// v1.4.0 "Calibrate" M2: optional gold-answer substring for the
    /// `answer_in_context` diagnostic (did the gold survive packing?). Reported
    /// in telemetry when `provenance=true`.
    #[serde(default)]
    pub gold_answer: Option<String>,
    /// v1.11.0 "Associate": enable the graph-PPR retriever as a third RRF leg.
    /// Opt-in; default `false` keeps the two-retriever (vector + FTS) path
    /// unchanged. Deterministic, zero-token, no embeddings.
    #[serde(default)]
    pub graph: bool,
}

fn default_limit() -> u32 {
    DEFAULT_RECALL_LIMIT
}

/// `POST /recall` — deterministic end-to-end recall.
///
/// v0.9.0 scope: single-DB treated as the `global` domain. Reuses the proven
/// `perform_search` path (sqlite-vec int8 KNN with a brute-force fallback,
/// model2vec local embeddings). Per-domain centroid routing + cross-domain
/// fallback + hybrid RRF fusion land in v1.0.0 / v0.9.1; the `domain`/
/// `strict`/`provenance` fields are accepted and honored to the extent the
/// single-DB model allows, so the contract stays stable as those phases ship.
///
/// Deterministic by construction: no LLM in the loop, embeddings are a local
/// library call (zero embedding-API cost). One search per turn.
pub async fn recall(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RecallRequest>,
) -> Result<Json<RecallResponse>, HandlerError> {
    // ---- validation (fail fast; never embed on bad input) ----
    // Delegates to the pure, unit-tested validator so the contract bounds and
    // the handler can never drift apart.
    let query = validate_recall(&req)?;
    // `domain` is validated for shape in validate_recall; re-normalize here for
    // the response label. Pre-v1.0.0 the single DB answers every domain as-is.
    let forced_domain = match &req.domain {
        Some(d) => Some(normalize_domain(d)?),
        None => None,
    };
    // `strict` controls cross-domain fan-out: when true, stay in the resolved
    // domain even on miss/low-confidence; when false (default), federate. Used
    // below in the no-confident-route branch.
    let strict = req.strict;

    // Prompt-injection guard (same defense the legacy /search applies).
    if contains_suspicious_pattern(&query) {
        return Err(HandlerError::bad_request(
            "query_rejected",
            "query matches a blocked prompt-injection pattern",
        ));
    }

    // ---- embed + search (deterministic core) ----
    // Resolve the (domain, pool) targets to search. In shim mode (or with an
    // explicit domain) this is a single target. In multi-db mode with no forced
    // domain, centroid routing picks one domain (strict isolation) or, failing
    // a confident route and non-strict mode, federates across all known domains.
    let model = Arc::clone(&state.model);
    let multi_db = state.registry.is_multi_db();
    let mut targets: Vec<(String, crate::Pool)> = Vec::new();
    let mut routed: Option<String> = None;

    match &forced_domain {
        Some(d) => {
            let p = state.registry.pool_for(d).map_err(|e| {
                HandlerError::bad_request("domain_invalid", format!("cannot resolve domain: {e}"))
            })?;
            targets.push((d.clone(), p));
        }
        None if !multi_db => {
            let p = state.registry.pool_for("global").map_err(|e| {
                HandlerError::bad_request("domain_invalid", format!("cannot resolve domain: {e}"))
            })?;
            targets.push(("global".to_string(), p));
        }
        None => {
            // Centroid routing: encode the query once, compare to each domain
            // centroid, route if a domain clears the confidence threshold.
            let qvec = {
                let m = Arc::clone(&model);
                m.encode(std::slice::from_ref(&query))
                    .into_iter()
                    .next()
                    .unwrap_or_default()
            };
            let centroids = crate::domain_router::read_centroids(&state.pool).unwrap_or_default();
            routed = crate::domain_router::route(&qvec, &centroids);
            match &routed {
                Some(d) => {
                    let p = state.registry.pool_for(d).map_err(|e| {
                        HandlerError::bad_request(
                            "domain_invalid",
                            format!("cannot resolve domain: {e}"),
                        )
                    })?;
                    targets.push((d.clone(), p));
                }
                None => {
                    if strict {
                        // Strict: no cross-domain fallback; search the home domain only.
                        let p = state.registry.pool_for("global").map_err(|e| {
                            HandlerError::bad_request(
                                "domain_invalid",
                                format!("cannot resolve domain: {e}"),
                            )
                        })?;
                        targets.push(("global".to_string(), p));
                    } else {
                        // Federate across all known domains (labelled fallback).
                        for d in state.registry.known_domains() {
                            if let Ok(p) = state.registry.pool_for(&d) {
                                targets.push((d, p));
                            }
                        }
                        if targets.is_empty() {
                            let p = state.registry.pool_for("global").map_err(|e| {
                                HandlerError::bad_request(
                                    "domain_invalid",
                                    format!("cannot resolve domain: {e}"),
                                )
                            })?;
                            targets.push(("global".to_string(), p));
                        }
                    }
                }
            }
        }
    }

    let k = req.limit as usize;
    // Lower the request into the shared v0.9.5 structured QueryDoc so recall
    // and search use one lexical compiler + validation path. The bare `query`
    // is the embedding/lexical fallback; structured `lex`/`vec`/`hyde` override.
    let doc = crate::QueryDoc {
        q: Some(query.clone()),
        sources: req
            .sources
            .iter()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .collect(),
        source: req.source.filter(|s| !s.is_empty()),
        since: req.since.filter(|s| !s.is_empty()),
        domain: forced_domain.clone(),
        vec: req.vec.filter(|s| !s.trim().is_empty()),
        hyde: req.hyde.filter(|s| !s.trim().is_empty()),
        intent: req.intent.filter(|s| !s.trim().is_empty()),
        profile: req.profile.filter(|s| !s.trim().is_empty()),
        lex: req.lex.clone(),
        include_flagged: req.include_flagged,
        as_of: req.as_of.filter(|s| !s.trim().is_empty()),
        evidence: req.evidence,
        at: req
            .at
            .as_deref()
            .map(|s| crate::search::normalize_since(s.trim()))
            .transpose()
            .map_err(|e| HandlerError::bad_request("at_invalid", e.to_string()))?,
        graph: req.graph,
        ..Default::default()
    };
    let (_qtext, base_filters) = match doc.into_filters() {
        Ok(pair) => pair,
        Err(e) => {
            return Err(crate::handlers::HandlerError::bad_request(
                "query_invalid",
                e.to_string(),
            ))
        }
    };
    let snippet_q = base_filters
        .lex
        .clone()
        .or_else(|| base_filters.embedding_query.clone())
        .unwrap_or_else(|| query.clone());
    // v1.4.0 "Calibrate" M2: capture the query for the post-search packing pass
    // before `query` is moved into the spawn_blocking closure below.
    let packing_query = query.clone();
    let max_context_tokens = req.max_context_tokens;
    let gold_answer = req.gold_answer.clone();

    let search_future = task::spawn_blocking(move || {
        // Per-domain result lists (already ranked within each domain by the
        // underlying hybrid search). Collected separately so cross-domain
        // merge can treat each domain as one RRF retriever — raw scores are
        // NOT comparable across domains (different IDF tables, different embed
        // norms after quantization); RRF on ranks is the correct merge.
        let mut per_domain: Vec<(String, Vec<crate::SearchResult>)> = Vec::new();
        let mut tel = crate::search::SearchTelemetry::default();
        for (domain, pool) in &targets {
            let mut f = base_filters.clone();
            // In multi-db the pool IS the domain, so drop the in-DB domain filter
            // to avoid double-restricting; in shim mode keep it to narrow the
            // single shared pool.
            if multi_db {
                f.domain = None;
            } else {
                f.domain = Some(domain.clone());
            }
            if let Ok((mut rs, t)) =
                crate::perform_search_with_prf(pool, &model, query.clone(), k, &f)
            {
                tel = t;
                for r in &mut rs {
                    r.with_snippet(&snippet_q);
                }
                // M2.1: enrich with span + source link + highlights per-domain.
                if let Ok(conn) = pool.get() {
                    let _ = crate::search::SearchResult::enrich_evidence(
                        &conn,
                        &mut rs,
                        &snippet_q,
                        f.as_of.is_some(),
                    );
                }
                // v0.9.7 Guard: strip snippet/evidence for flagged hits (after
                // enrichment) unless the caller opted into flagged rows.
                for r in &mut rs {
                    crate::suppress_flagged_evidence(r, f.include_flagged);
                }
                per_domain.push((domain.clone(), rs));
            }
        }
        let all = rrf_merge_domains(per_domain, k);
        (all, tel)
    });

    let (mut tagged, mut tel) = match timeout(StdDuration::from_secs(8), search_future).await {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            return Err(HandlerError::unavailable(format!("recall failed: {e}")));
        }
        Err(_) => return Err(HandlerError::unavailable("recall timed out")),
    };

    // v1.4.0 "Calibrate" M2: submodular evidence packing. When a token budget
    // is supplied, re-rank the merged results by budgeted monotone submodular
    // maximization (relevance + coverage + representativeness + diversity) and
    // truncate to the budget. Replaces fixed-k truncation for the packed path.
    if let Some(budget) = max_context_tokens {
        let cands: Vec<crate::SearchResult> = tagged.iter().map(|(r, _)| r.clone()).collect();
        let domains: Vec<String> = tagged.iter().map(|(_, d)| d.clone()).collect();
        let gold = gold_answer.as_deref();
        let weights = crate::search::packing::weights_from_env();
        let packed = crate::search::packing::pack(cands, &packing_query, budget, weights, gold);
        tel.packed_tokens = Some(packed.packed_tokens);
        tel.packing_candidates = Some(packed.candidates);
        tel.answer_in_context = packed.answer_in_context;
        tagged = packed.results.into_iter().zip(domains).collect();
    }

    // ---- render (RecallHit carries its source domain for federation) ----
    let primary_domain = forced_domain
        .clone()
        .or(routed)
        .unwrap_or_else(|| "global".to_string());

    // v1.5.0 "Epistemic" – calibrated abstention. The existing
    // `HeuristicEstimator` already classified this query via overlap + gap +
    // lexical density into a `Recommendation`. When it says `ClarifyQuery`, the
    // retrieval is too weak to support a claim — ship the empty-hits
    // `low_confidence` envelope instead of top-1 garbage. NOT a magic score
    // cutoff; abstention is driven by the calibrated multi-signal
    // `Recommendation`, which is what the evidence-gated roadmap v1.5 requires
    // ("no fixed universal confidence threshold until held-out benefit is
    // demonstrated").
    let decision = abstention_decision(tel.recommendation);
    let hits = if matches!(decision, crate::handlers::RecallDecision::LowConfidence) {
        Vec::new()
    } else {
        results_to_hits(tagged, req.provenance)
    };

    let domains_searched = if req.provenance {
        let mut seen: Vec<String> = hits.iter().filter_map(|h| h.domain.clone()).collect();
        seen.sort();
        seen.dedup();
        Some(seen)
    } else {
        None
    };

    Ok(Json(RecallResponse {
        hits,
        decision,
        domain: Some(primary_domain),
        domains_searched,
        telemetry: req.provenance.then_some(tel),
    }))
}

// ---------------------------------------------------------------------------
// Pure helpers (testable without AppState / StaticModel)
// ---------------------------------------------------------------------------

/// Map search results into the recall response shape, tagging every hit with
/// its source domain (per-hit, for federated recall) and provenance source.
/// Kept pure so it can be unit-tested without a model or database.
fn abstention_decision(
    recommendation: Option<crate::Recommendation>,
) -> crate::handlers::RecallDecision {
    // ponytail: the only signal that triggers abstention is the calibrated
    // `ClarifyQuery` recommendation, which already encodes overlap + gap +
    // lexical-density gates. A single-score threshold here would duplicate the
    // estimator's job and drift from it; v1.6+ may add a learned threshold once
    // the judged-query baseline (Carry-forward) is recorded.
    match recommendation {
        Some(crate::Recommendation::ClarifyQuery) => crate::handlers::RecallDecision::LowConfidence,
        _ => crate::handlers::RecallDecision::Ok,
    }
}

/// Map search results into the recall response shape, tagging every hit with
/// its source domain (per-hit, for federated recall) and provenance source.
/// Kept pure so it can be unit-tested without a model or database.
fn results_to_hits(
    results: Vec<(crate::SearchResult, String)>,
    include_provenance: bool,
) -> Vec<RecallHit> {
    results
        .into_iter()
        .map(|(r, domain)| RecallHit {
            id: r.id,
            title: r.title,
            content: r.content,
            score: r.score,
            domain: Some(domain),
            source: Some(map_source(r.source)),
            provenance: if include_provenance {
                Some(r.provenance)
            } else {
                None
            },
            evidence: r.evidence.clone(),
            snippet: r.snippet,
            untrusted: true,
            conflict: r.evidence.as_ref().map(|e| {
                e.links
                    .iter()
                    .any(|l| l.kind == "contradicts" || l.kind == "supersedes")
            }),
        })
        .collect()
}

/// Map the internal SearchSource to the contract's HitSource. Defaults to
/// Vector when the search result wasn't tagged (e.g. legacy callers).
fn map_source(src: Option<crate::SearchSource>) -> HitSource {
    use crate::SearchSource;
    match src {
        Some(SearchSource::Vector) => HitSource::Vector,
        Some(SearchSource::Fts) => HitSource::Fts,
        Some(SearchSource::Graph) => HitSource::Graph,
        Some(SearchSource::Both) => HitSource::Both,
        None => HitSource::Vector,
    }
}

/// Merge per-domain ranked result lists via Reciprocal Rank Fusion (per the
/// v1.0 plan M3: "Merge across domains with the same RRF from v0.9.1").
///
/// Each domain's list is treated as one retriever; a chunk that appears in
/// multiple domains accumulates RRF contributions from each. RRF is rank-based,
/// so it correctly merges results whose raw scores are not comparable across
/// domains (different IDF, different embed norms after quantization).
///
/// Dedup key is `(id, domain)` — the same content legitimately stored in two
/// domains stays distinct (two memories), but a chunk can only appear once per
/// domain (the in-domain search already deduplicated).
///
/// `k` is the final cap. The RRF contribution uses the same `RRF_K = 60`
/// constant as the in-domain hybrid fusion (`search::RRF_K`).
pub(super) fn rrf_merge_domains(
    per_domain: Vec<(String, Vec<crate::SearchResult>)>,
    k: usize,
) -> Vec<(crate::SearchResult, String)> {
    use std::collections::HashMap;
    let rrf_k = crate::search::RRF_K as f32;

    // First pass: collect fused scores per (domain, id).
    let mut fused: HashMap<(String, i64), (f32, &crate::SearchResult)> = HashMap::new();
    for (domain, rs) in &per_domain {
        for (rank, r) in rs.iter().enumerate() {
            let key = (domain.clone(), r.id);
            let contribution = 1.0 / (rrf_k + rank as f32);
            fused
                .entry(key)
                .and_modify(|(score, _)| *score += contribution)
                .or_insert((contribution, r));
        }
    }

    // Sort by fused score descending; truncate to k.
    let mut entries: Vec<((String, i64), f32, &crate::SearchResult)> = fused
        .into_iter()
        .map(|((d, id), (score, r))| ((d, id), score, r))
        .collect();
    entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    entries.truncate(k);

    // Clone the (cheaply cloneable) SearchResult for the caller. ponytail:
    // cloning here keeps the helper pure + testable without lifetime gymnastics.
    entries
        .into_iter()
        .map(|((d, _), _, r)| (r.clone(), d))
        .collect()
}

/// Validate a parsed recall request against the contract bounds. Returns the
/// trimmed query on success, or the first contract violation. Pure so the
/// validation matrix can be tested without spinning up the server.
pub(super) fn validate_recall(req: &RecallRequest) -> Result<String, HandlerError> {
    let query = req.query.trim().to_string();
    if query.is_empty() {
        return Err(HandlerError::bad_request(
            "query_empty",
            "query must not be empty",
        ));
    }
    if query.len() > MAX_QUERY {
        return Err(HandlerError::bad_request(
            "query_too_long",
            format!("query exceeds {MAX_QUERY} characters"),
        ));
    }
    if !(MIN_LIMIT..=MAX_LIMIT).contains(&req.limit) {
        return Err(HandlerError::bad_request_with(
            "limit_out_of_range",
            format!("limit must be {MIN_LIMIT}..={MAX_LIMIT}"),
            serde_json::json!({ "min": MIN_LIMIT, "max": MAX_LIMIT }),
        ));
    }
    // domain shape is validated; the per-domain registry check is v1.0.0.
    if let Some(d) = &req.domain {
        normalize_domain(d)?;
    }
    Ok(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SearchResult;

    fn req(query: &str) -> RecallRequest {
        RecallRequest {
            query: query.to_string(),
            limit: DEFAULT_RECALL_LIMIT,
            domain: None,
            strict: false,
            provenance: false,
            source: None,
            since: None,
            lex: Default::default(),
            vec: None,
            hyde: None,
            intent: None,
            sources: Vec::new(),
            profile: None,
            include_flagged: false,
            as_of: None,
            evidence: false,
            at: None,
            max_context_tokens: None,
            gold_answer: None,
            graph: false,
        }
    }

    #[test]
    fn rejects_empty_query() {
        let err = validate_recall(&req("   ")).unwrap_err();
        assert_eq!(err.inner.code, "query_empty");
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn rejects_oversized_query() {
        let r = RecallRequest {
            query: "a".repeat(MAX_QUERY + 1),
            limit: DEFAULT_RECALL_LIMIT,
            domain: None,
            strict: false,
            provenance: false,
            source: None,
            since: None,
            lex: Default::default(),
            vec: None,
            hyde: None,
            intent: None,
            sources: Vec::new(),
            profile: None,
            include_flagged: false,
            as_of: None,
            evidence: false,
            at: None,
            max_context_tokens: None,
            gold_answer: None,
            graph: false,
        };
        let err = validate_recall(&r).unwrap_err();
        assert_eq!(err.inner.code, "query_too_long");
    }

    #[test]
    fn rejects_limit_out_of_range() {
        let mut r = req("a real query");
        r.limit = 0;
        assert_eq!(
            validate_recall(&r).unwrap_err().inner.code,
            "limit_out_of_range"
        );
        r.limit = MAX_LIMIT + 1;
        assert_eq!(
            validate_recall(&r).unwrap_err().inner.code,
            "limit_out_of_range"
        );
    }

    #[test]
    fn rejects_invalid_domain_shape() {
        let mut r = req("a real query");
        // uppercase + invalid chars fail the domain regex
        r.domain = Some("Bad Domain!".to_string());
        assert_eq!(
            validate_recall(&r).unwrap_err().inner.code,
            "domain_invalid"
        );
    }

    #[test]
    fn accepts_valid_request_and_trims_query() {
        let mut r = req("  hello world  ");
        r.limit = 10;
        r.domain = Some("health".to_string());
        assert_eq!(validate_recall(&r).unwrap(), "hello world");
    }

    #[test]
    fn results_to_hits_tags_domain_and_vector_source() {
        let results = vec![
            SearchResult {
                id: 1,
                score: 0.9,
                title: Some("t1".into()),
                content: "c1".into(),
                source: None,
                provenance: Default::default(),
                flagged: false,
                untrusted: true,
                snippet: None,
                evidence: None,
                ..Default::default()
            },
            SearchResult {
                id: 2,
                score: 0.5,
                title: None,
                content: "c2".into(),
                source: None,
                provenance: Default::default(),
                flagged: false,
                untrusted: true,
                snippet: None,
                evidence: None,
                ..Default::default()
            },
        ];
        let hits = results_to_hits(
            results
                .into_iter()
                .map(|r| (r, "health".to_string()))
                .collect(),
            false,
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, 1);
        assert_eq!(hits[0].domain.as_deref(), Some("health"));
        assert_eq!(hits[0].source, Some(HitSource::Vector));
        assert!((hits[0].score - 0.9).abs() < f32::EPSILON);
        assert!(hits[1].title.is_none());
    }

    #[test]
    fn results_to_hits_preserves_order_and_count() {
        // perform_search returns descending by score; mapping must not reorder.
        let results = vec![
            SearchResult {
                id: 10,
                score: 0.99,
                title: None,
                content: "a".into(),
                source: None,
                provenance: Default::default(),
                flagged: false,
                untrusted: true,
                snippet: None,
                evidence: None,
                ..Default::default()
            },
            SearchResult {
                id: 11,
                score: 0.80,
                title: None,
                content: "b".into(),
                source: None,
                provenance: Default::default(),
                flagged: false,
                untrusted: true,
                snippet: None,
                evidence: None,
                ..Default::default()
            },
            SearchResult {
                id: 12,
                score: 0.10,
                title: None,
                content: "c".into(),
                source: None,
                provenance: Default::default(),
                flagged: false,
                untrusted: true,
                snippet: None,
                evidence: None,
                ..Default::default()
            },
        ];
        let hits = results_to_hits(
            results
                .into_iter()
                .map(|r| (r, "global".to_string()))
                .collect(),
            false,
        );
        let ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
        assert_eq!(ids, vec![10, 11, 12]);
        // scores descend
        assert!(hits[0].score > hits[1].score && hits[1].score > hits[2].score);
    }

    #[test]
    fn results_to_hits_empty() {
        assert!(results_to_hits(Vec::<(crate::SearchResult, String)>::new(), false).is_empty());
    }

    /// Cross-domain RRF: results are ranked by their RRF contribution
    /// `1/(k+rank)` per-domain. Raw scores are NOT comparable across domains
    /// (different IDF tables, different embed norms); rank IS comparable.
    /// Each (domain, id) pair is a distinct hit tagged with its source domain.
    #[test]
    fn rrf_merge_ranks_by_per_domain_rank_not_raw_score() {
        let mk = |id: i64, score: f32, content: &str| crate::SearchResult {
            id,
            score,
            content: content.into(),
            untrusted: true,
            ..Default::default()
        };
        // Domain A: chunk 1 rank 0 (raw score 0.10), chunk 2 rank 1 (raw 0.95).
        // Domain B: chunk 3 rank 0 (raw 0.99), chunk 4 rank 1 (raw 0.50).
        //
        // Under the OLD raw-score merge, chunk 3 (0.99) would win. Under RRF,
        // the two rank-0 hits (chunk 1 in A, chunk 3 in B) tie for first because
        // each contributes exactly `1/60`. The high raw 0.99 score must NOT
        // outweigh the low raw 0.10 score — both are rank 0 in their domain.
        let per_domain = vec![
            ("a".to_string(), vec![mk(1, 0.10, "a1"), mk(2, 0.95, "a2")]),
            ("b".to_string(), vec![mk(3, 0.99, "b3"), mk(4, 0.50, "b4")]),
        ];
        let merged = rrf_merge_domains(per_domain, 10);
        // Top two must be the two rank-0 hits (ids 1 and 3), in either order.
        let top_ids: std::collections::HashSet<i64> =
            merged.iter().take(2).map(|(r, _)| r.id).collect();
        assert_eq!(
            top_ids,
            [1, 3].into_iter().collect(),
            "rank-0 hits should win regardless of raw score"
        );
        // Every hit is tagged with its source domain.
        let tags: Vec<&str> = merged.iter().map(|(_, d)| d.as_str()).collect();
        assert!(tags.contains(&"a") && tags.contains(&"b"));
        // Cap to k.
        let capped = rrf_merge_domains(
            vec![
                ("a".to_string(), vec![mk(1, 0.1, "a1"), mk(2, 0.2, "a2")]),
                ("b".to_string(), vec![mk(3, 0.3, "b3"), mk(4, 0.4, "b4")]),
            ],
            2,
        );
        assert_eq!(capped.len(), 2, "k truncation must apply");
    }

    /// Same chunk id in the SAME domain twice (shouldn't happen, but the dedup
    /// key is (domain, id) so we must keep them distinct across domains).
    #[test]
    fn rrf_merge_keeps_same_id_in_different_domains() {
        let mk = |id: i64, content: &str| crate::SearchResult {
            id,
            score: 0.5,
            content: content.into(),
            untrusted: true,
            ..Default::default()
        };
        let per_domain = vec![
            ("a".to_string(), vec![mk(7, "a-copy")]),
            ("b".to_string(), vec![mk(7, "b-copy")]),
        ];
        let merged = rrf_merge_domains(per_domain, 10);
        assert_eq!(
            merged.len(),
            2,
            "same id in different domains stays distinct"
        );
    }

    #[test]
    fn results_to_hits_tags_per_hit_domain() {
        // Federation: each hit keeps its own source domain.
        let mk = |id, dom: &str| {
            (
                SearchResult {
                    id,
                    score: 0.5,
                    title: None,
                    content: format!("c{id}"),
                    source: None,
                    provenance: Default::default(),
                    flagged: false,
                    untrusted: true,
                    snippet: None,
                    evidence: None,
                    ..Default::default()
                },
                dom.to_string(),
            )
        };
        let hits = results_to_hits(vec![mk(1, "work"), mk(2, "health"), mk(3, "work")], false);
        let domains: Vec<&str> = hits.iter().map(|h| h.domain.as_deref().unwrap()).collect();
        assert_eq!(domains, vec!["work", "health", "work"]);
    }

    #[test]
    fn results_to_hits_includes_provenance_when_requested() {
        let r = SearchResult {
            id: 1,
            score: 0.9,
            title: None,
            content: "c".into(),
            source: None,
            provenance: crate::search::Provenance {
                vector_rank: Some(0),
                ..Default::default()
            },
            flagged: false,
            untrusted: true,
            snippet: None,
            evidence: None,
            ..Default::default()
        };
        let hits = results_to_hits(vec![(r, "health".into())], true);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].provenance.is_some());
        assert_eq!(hits[0].provenance.as_ref().unwrap().vector_rank, Some(0));
    }

    // ---- v1.5.0 "Epistemic" — calibrated abstention ----

    #[test]
    fn abstention_returns_low_confidence_only_on_clarify_query() {
        // The only signal that should trigger abstention is the calibrated
        // ClarifyQuery recommendation. Return/RunPrf/RunReranker/IncreaseTopK
        // all map to Ok — they produced (or could produce) usable hits.
        use crate::Recommendation::*;
        assert_eq!(
            abstention_decision(Some(ClarifyQuery)),
            crate::handlers::RecallDecision::LowConfidence
        );
        for r in [Return, RunPrf, RunReranker, IncreaseTopK] {
            assert_eq!(
                abstention_decision(Some(r)),
                crate::handlers::RecallDecision::Ok,
                "{r:?} should not abstain"
            );
        }
        // Missing recommendation (pre-quality-estimator path) must NOT
        // abstain — that would silently break legacy callers.
        assert_eq!(
            abstention_decision(None),
            crate::handlers::RecallDecision::Ok
        );
    }
}
