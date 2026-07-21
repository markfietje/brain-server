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
    // `strict` only affects cross-domain fallback, which doesn't exist yet.
    let _ = req.strict;

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
                    if req.strict {
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

    let search_future = task::spawn_blocking(move || {
        let mut all: Vec<(crate::SearchResult, String)> = Vec::new();
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
                all.extend(rs.into_iter().map(|r| (r, domain.clone())));
            }
        }
        // Merge across domains by score (descending), cap to k.
        all.sort_by(|a, b| {
            b.0.score
                .partial_cmp(&a.0.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all.truncate(k);
        (all, tel)
    });

    let (tagged, tel) = match timeout(StdDuration::from_secs(8), search_future).await {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            return Err(HandlerError::unavailable(format!("recall failed: {e}")));
        }
        Err(_) => return Err(HandlerError::unavailable("recall timed out")),
    };

    // ---- render (RecallHit carries its source domain for federation) ----
    let primary_domain = forced_domain
        .clone()
        .or(routed)
        .unwrap_or_else(|| "global".to_string());
    let hits = results_to_hits(tagged, req.provenance);

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
        Some(SearchSource::Both) => HitSource::Both,
        None => HitSource::Vector,
    }
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
}
