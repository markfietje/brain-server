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

use axum::{
    extract::{Query, State},
    Json,
};
use rand::Rng;
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
    #[serde(alias = "explain")]
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
    /// OR filter over ingest kind (`memory` | `markdown` | `structured` |
    /// `manual` | `vault`) — applied to the `source` column, NOT source URIs.
    /// Empty = unrestricted. Document/source-URI scoping is a future param.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Retrieval profile hint (passthrough in M1).
    #[serde(default)]
    pub profile: Option<String>,
    /// when true, include quarantined (`flagged`) chunks in the
    /// results. Operator review path only — the default agent path stays clean.
    #[serde(default)]
    pub include_flagged: bool,
    /// point-in-time recall. RFC3339 instant; returns the
    /// revision current at that time (historical mode). When set, hits are
    /// tagged `lifecycle: "historical"`.
    #[serde(default)]
    pub as_of: Option<String>,
    /// include structured `Evidence` (time + lifecycle +
    /// links) on every hit even without `provenance`.
    #[serde(default)]
    pub evidence: bool,
    /// bi-temporal valid-time point-in-time filter.
    /// RFC3339 or `YYYY-MM-DD` instant; only chunks whose valid-interval
    /// (valid_from, valid_to) contains this instant are returned. Distinct
    /// from `as_of` (transaction-time / revision recall). Graphiti semantics.
    #[serde(default)]
    pub at: Option<String>,
    /// submodular evidence packing token budget. When
    /// set, results are re-ranked by budgeted monotone submodular maximization
    /// (relevance + coverage + representativeness + diversity) and truncated
    /// to fit the budget. Replaces fixed `k` truncation for the packed path.
    #[serde(default)]
    pub max_context_tokens: Option<usize>,
    /// optional gold-answer substring for the
    /// `answer_in_context` diagnostic (did the gold survive packing?). Reported
    /// in telemetry when `provenance=true`.
    #[serde(default)]
    pub gold_answer: Option<String>,
    /// enable the graph-PPR retriever as a third RRF leg.
    /// Opt-in; default `false` keeps the two-retriever (vector + FTS) path
    /// unchanged. Deterministic, zero-token, no embeddings.
    #[serde(default)]
    pub graph: bool,
    /// include decayed chunks (`expires_at` in the past) in
    /// the results, tagged `decayed: true`. Default false — decayed facts are
    /// excluded from current recall (the operator review path opts in).
    #[serde(default)]
    pub include_decayed: bool,
    /// `memory_kind` filter (fact|procedure|step|decision|
    /// episodic). Restricts retrieval to that `knowledge.node_kind`.
    #[serde(default)]
    pub memory_kind: Option<String>,
    /// minimum relevance tier (high|medium|low). Drops
    /// lower-tier hits after fusion — the "stop poisoning the context window"
    /// filter, deterministic and zero-token.
    #[serde(default)]
    pub min_relevance: Option<String>,
    /// when true AND read-event audit is enabled
    /// (JWT mode default), the response includes a `trace_id` (the audit row
    /// id) that `/recall/{trace_id}/trace` can replay. Pure opt-in: without
    /// it the read event (if recorded) is still hash-only and unreplayable.
    #[serde(default)]
    pub trace: bool,
}

fn default_limit() -> u32 {
    DEFAULT_RECALL_LIMIT
}

/// the shared recall core's return — the tagged (result, domain)
/// pairs plus the decision/telemetry/trace envelope. Each binding (`/recall`,
/// `/ump/recall`) renders its own hit shape from `tagged`.
pub(crate) struct RecallOutcome {
    pub tagged: Vec<(crate::SearchResult, String)>,
    pub tel: crate::search::SearchTelemetry,
    pub decision: crate::handlers::RecallDecision,
    pub trace_id: Option<i64>,
    pub domains_searched: Vec<String>,
    pub primary_domain: String,
}

/// optional query-string `?source=` on `POST /recall`. The JSON body
/// `source` is primary; this fills the gap when the body omits it and is always
/// validated (422 on unknown), so a query-string value is never silently
/// ignored. Parity with `GET /search?source=`.
#[derive(Debug, Default, Deserialize)]
pub struct RecallSourceQuery {
    #[serde(default)]
    source: Option<String>,
}

/// `POST /recall` — deterministic end-to-end recall.
///
/// single-DB treated as the `global` domain. Reuses the proven
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
    principal: crate::handlers::auth::OptPrincipal,
    source_query: Query<RecallSourceQuery>,
    Json(req): Json<RecallRequest>,
) -> Result<Json<RecallResponse>, HandlerError> {
    // the deterministic pipeline lives in `run_recall` so the
    // HTTP and UMP bindings share one core; only the renderer differs.
    let provenance = req.provenance;
    let include_decayed = req.include_decayed;
    let outcome = run_recall(&state, &principal.0, req, source_query.0).await?;
    let hits = results_to_hits(outcome.tagged, provenance, include_decayed, &principal.0);
    Ok(Json(RecallResponse {
        hits,
        decision: outcome.decision,
        domain: Some(outcome.primary_domain),
        domains_searched: outcome.domains_searched,
        telemetry: provenance.then_some(outcome.tel),
        trace_id: outcome.trace_id,
    }))
}

/// the deterministic recall core shared by `/recall` and
/// `/ump/recall`. Runs embed → centroid routing → hybrid search → cross-domain
/// merge → packing → tier filter → calibrated abstention, and records the
/// optional read-event audit trace. Returns the tagged (result, domain) pairs
/// so each binding renders its own hit shape.
/// the recall core emits a `recall` span under
/// `--features otel` carrying decision/count/domain/principal labels + a short
/// `query_hash` — never the query body (PII rule; see `otel.rs`).
#[cfg_attr(
    feature = "otel",
    tracing::instrument(
        name = "recall",
        skip_all,
        fields(
            decision = tracing::field::Empty,
            graph_rescued = tracing::field::Empty,
            hits = tracing::field::Empty,
            domain = tracing::field::Empty,
            principal = tracing::field::Empty,
            query_hash = tracing::field::Empty
        )
    )
)]
pub(crate) async fn run_recall(
    state: &Arc<AppState>,
    principal: &Option<crate::auth::Principal>,
    req: RecallRequest,
    source_query: RecallSourceQuery,
) -> Result<RecallOutcome, HandlerError> {
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
    // AuthZ read gate, scoped to the requested domain.
    // `None` (no JWT) = superuser.
    super::authorize(
        principal,
        crate::auth::Action::Read,
        "",
        forced_domain.as_deref().unwrap_or("global"),
    )?;
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
            let p = state
                .registry
                .pool_for(d)
                .map_err(super::map_domain_error)?;
            targets.push((d.clone(), p));
        }
        None if !multi_db => {
            // automatic retrieval routing in shim mode. The
            // old code pushed the `global` pool and skipped centroid routing
            // entirely — so after v1.13.0 moved rows into a non-global label,
            // they became unreachable by default recall (a `k.domain='global'`-
            // scoped search). Routing here makes the matched domain reachable
            // AND stops a bulk domain from dominating un-routed queries.
            if !crate::config::brain_recall_routing_enabled() {
                // Kill switch: legacy shim behavior — search global only.
                let p = state.registry.pool_for("global").map_err(|e| {
                    HandlerError::bad_request(
                        "domain_invalid",
                        format!("cannot resolve domain: {e}"),
                    )
                })?;
                targets.push(("global".to_string(), p));
            } else {
                // Centroid routing: encode once, compare to each centroid.
                let qvec = {
                    let m = Arc::clone(&model);
                    m.encode_one(&query)
                };
                let centroids =
                    crate::domain_router::read_centroids(&state.pool).unwrap_or_default();
                routed = crate::domain_router::route(&qvec, &centroids);
                for d in shim_routing_targets(routed.as_deref()) {
                    let p = state
                        .registry
                        .pool_for(&d)
                        .map_err(super::map_domain_error)?;
                    targets.push((d, p));
                }
            }
        }
        None => {
            // Centroid routing: encode the query once, compare to each domain
            // centroid, route if a domain clears the confidence threshold.
            let qvec = {
                let m = Arc::clone(&model);
                m.encode_one(&query)
            };
            let centroids = crate::domain_router::read_centroids(&state.pool).unwrap_or_default();
            routed = crate::domain_router::route(&qvec, &centroids);
            match &routed {
                Some(d) => {
                    let p = state
                        .registry
                        .pool_for(d)
                        .map_err(super::map_domain_error)?;
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

    // drop any target the principal may not
    // read BEFORE a search runs against it. The domain-level authorize above
    // covers the forced/explicit domain only; the federation + centroid
    // branches collect every known domain, and a tenant-scoped principal must
    // not query the pools of foreign domains (the domain label is the trust
    // boundary). `None` principal (loopback/opaque) = superuser, unchanged.
    targets.retain(|(d, _)| super::can_read_domain(principal, d));
    if targets.is_empty() {
        // Retain can empty targets for a scoped principal (e.g. forced global
        // is filtered because the principal only holds other domains) — that
        // is a legitimate "nothing to search" outcome, not an error.
        tracing::debug!(
            principal = ?principal.as_ref().map(|p| p.sub.as_str()),
            "recall: all targets filtered by domain read-gate"
        );
    }

    let k = req.limit as usize;
    // parse `source` from the body AND the
    // query string. Ingest-kind values stay SQL equality; retrieval-leg values
    // become a post-fusion filter; "both" is unrestricted. Body `source` wins
    // when both are present; the query string fills in when the body omits it.
    // Unknown values in either are rejected with 422 before any DB/embed work —
    // a query-string `?source=` is never silently ignored (GET /search parity).
    let (source_kind, source_leg) = crate::search::query::resolve_source_filter(
        req.source.as_deref(),
        source_query.source.as_deref(),
    )
    .map_err(|e| HandlerError::unprocessable("invalid_source", e.to_string()))?;
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
        source: source_kind,
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
    let (_qtext, mut base_filters) = match doc.into_filters() {
        Ok(pair) => pair,
        Err(e) => {
            return Err(crate::handlers::HandlerError::bad_request(
                "query_invalid",
                e.to_string(),
            ))
        }
    };
    base_filters.source_leg = source_leg;
    // decay + memory_kind + min_relevance + access scope.
    base_filters.include_decayed = req.include_decayed;
    base_filters.now_unix = chrono::Utc::now().timestamp();
    base_filters.memory_kind = req.memory_kind.as_deref().map(str::to_string);
    if let Some(t) = &req.min_relevance {
        if !matches!(t.as_str(), "high" | "medium" | "low") {
            return Err(HandlerError::bad_request(
                "invalid_min_relevance",
                "min_relevance must be high|medium|low",
            ));
        }
        base_filters.min_relevance = Some(t.clone());
    }
    base_filters.access_scopes =
        crate::handlers::gate::scope_filter(principal).map(std::sync::Arc::new);
    // a JWT principal with a `roles` claim is scoped by its
    // role bundles (narrowed access_scopes + an owner predicate for
    // self/reports). No roles → the v1.14 scope path above applies unchanged.
    // The gate (resolved once, below) also feeds the v1.27.7 "Qa" scope-violation
    // detection so it does not re-resolve it for every recall.
    let mut role_restricted = false;
    if let Some(gate) = crate::handlers::gate::role_retrieval_gate(principal, &state.pool) {
        role_restricted = gate.owner_in.is_some();
        crate::handlers::gate::apply_role_gate(&mut base_filters, &gate);
    }
    // when per-kind retention is enabled, carry the policy
    // into the retriever so chunks whose kind-default expiry has elapsed are
    // excluded from default recall exactly like an explicit `expires_at`.
    if crate::config::brain_retention_enabled() {
        base_filters.retention_days =
            std::sync::Arc::new(crate::config::retention_kind_days().into_iter().collect());
    }
    // a bound profile's retention block REPLACES the
    // server-wide policy for that domain (explicit nulls remove a kind's
    // decay; an empty block = no kind decay at all). One read, before the
    // search closure; an exhausted pool degrades to the server-wide policy
    // (transient, availability-first). The map also carries the audit_level
    // for the read-event decision below.
    let bound_profiles = match state.pool.get() {
        Ok(conn) => brain_server::profile::domain_profiles(&conn).unwrap_or_default(),
        Err(_) => std::collections::HashMap::new(),
    };
    let profile_retention: std::collections::HashMap<String, Vec<(String, i64)>> = bound_profiles
        .iter()
        .filter_map(|(d, p)| {
            p.retention_map()
                .map(|m| (d.clone(), m.into_iter().collect::<Vec<_>>()))
        })
        .collect();
    // capture the access-scope decision for the read-event
    // trace before the filters are moved into the search closure.
    let applied_scopes = base_filters.access_scopes.clone();
    let trace_now = base_filters.now_unix;
    let snippet_q = base_filters
        .lex
        .clone()
        .or_else(|| base_filters.embedding_query.clone())
        .unwrap_or_else(|| query.clone());
    // capture the query for the post-search packing pass
    // before `query` is moved into the spawn_blocking closure below.
    let packing_query = query.clone();
    // capture the query text for the read-event trace
    // (same reason — the closure consumes `query`).
    let trace_query = query.clone();
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
            // the bound profile's retention map is THE
            // policy for this domain (replaces the server-wide map; an empty
            // map = no kind decay — the smb-simple posture).
            if let Some(days) = profile_retention.get(domain) {
                f.retention_days = std::sync::Arc::new(days.clone());
            }
            if let Ok((mut rs, t)) =
                crate::perform_search_with_prf(pool, &*model, query.clone(), k, &f)
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
                // strip snippet/evidence for flagged hits (after
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

    // submodular evidence packing. When a token budget
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

    // post-fusion relevance-tier filter (min_relevance drops
    // low-tier hits — the "poison the context window" ask, zero-token) and
    // decay tagging (include_decayed returns decayed chunks tagged decayed).
    let min_tier = req.min_relevance.clone();
    let include_decayed = req.include_decayed;
    tagged.retain(|(r, _)| match min_tier.as_deref() {
        Some("high") => crate::gate::relevance_tier(r.score) == "high",
        Some("medium") => crate::gate::relevance_tier(r.score) != "low",
        _ => true,
    });

    // ---- render (RecallHit carries its source domain for federation) ----
    let primary_domain = forced_domain
        .clone()
        .or(routed)
        .unwrap_or_else(|| "global".to_string());

    // Calibrated abstention. The
    // existing `HeuristicEstimator` classified this query via overlap + gap +
    // lexical density into a `Recommendation`. When it says `ClarifyQuery`,
    // the retrieval is too weak to support a claim — but v1.12.0 gave the
    // graph leg a chance first (the rescue pass inside `perform_search_with_prf`
    // already ran and fused its hits in). Abstention is now scoped to the
    // FINAL outcome: `ClarifyQuery` + zero hits → the empty `low_confidence`
    // envelope (v1.5.0 contract, unchanged); `ClarifyQuery` + rescued hits →
    // `ok`. NOT a magic score cutoff; driven by the calibrated multi-signal
    // `Recommendation`, which is what the evidence-gated roadmap requires
    // ("no fixed universal confidence threshold until held-out benefit is
    // demonstrated").
    let decision = abstention_decision(tel.recommendation, tagged.is_empty());

    // domains of the returned hits, always present
    // (empty array when no hits). Only telemetry stays provenance-gated.
    let mut domains_searched: Vec<String> = tagged.iter().map(|(_, d)| d.clone()).collect();
    domains_searched.sort();
    domains_searched.dedup();

    // a role-restricted agent that searched beyond the global
    // perimeter domain crossed a client border — a security event on the
    // established Auth/Denied channel. Best-effort (never fails the recall).
    if crate::qa::scope_violation(role_restricted, &domains_searched) {
        if let Ok(conn) = state.pool.get() {
            crate::audit::record(
                &conn,
                crate::audit::AuditKind::Auth,
                "api",
                "scope_violation",
                crate::audit::AuditStatus::Denied,
                &format!(
                    "agent={} domains={domains_searched:?}",
                    principal_label(principal)
                ),
            );
        }
    }

    // emit a read event into the hash-chained audit
    // (opt-in: on in JWT mode, off in loopback, overridable via
    // BRAIN_AUDIT_READ_EVENTS + BRAIN_AUDIT_READ_SAMPLE_RATE). The trace id is
    // surfaced in the response only when `?trace=true`. Best-effort — a failure
    // here must never fail the recall the caller asked for.
    // when the env is unset, the primary domain's bound
    // profile decides (verbose on / minimal off / standard = JWT posture);
    // `/search`, `/get`, `/multi-get` keep the global env posture (ceiling —
    // they are not the decision-path read).
    let audit_on = brain_server::profile::audit_read_events_for(
        crate::config::audit_read_events_explicit(),
        bound_profiles.get(&primary_domain),
        principal.is_some(),
    );
    let trace_id = if audit_on {
        let rate = crate::config::audit_read_sample_rate();
        let sampled = rate >= 1.0 || rand::thread_rng().gen_range(0.0..1.0) < rate;
        if !sampled {
            None
        } else {
            let trace_detail = if req.trace {
                Some(
                    serde_json::json!({
                        // the trace records a bounded hash of the
                        // query (SHA-256, v1.20.25), never the raw text — a recall query
                        // can itself be personal data of the subject (the DSAR
                        // residue-sweep in observe.rs relies on this too).
                        "query_hash": crate::audit::hash(&trace_query),
                        "decision": format!("{:?}", decision),
                        "domains_searched": domains_searched,
                        "scope": applied_scopes.as_deref(),
                        "actor": principal_label(principal),
                        "hits": tagged.iter().map(|(r, _)| serde_json::json!({
                            "id": r.id,
                            "score": r.score,
                            "assertion_kind": r.assertion_kind,
                            "source": map_source(r.source),
                            "relevance": crate::gate::relevance_tier(r.score),
                            "decayed": include_decayed.then(|| crate::gate::is_decayed(r.expires_at, trace_now)),
                        })).collect::<Vec<_>>(),
                    })
                    .to_string(),
                )
            } else {
                None
            };
            let pool = state.pool.clone();
            let actor = principal_label(principal);
            let tenant = principal_tenant(principal);
            let event_query = trace_query.clone();
            task::spawn_blocking(move || {
                if let Ok(conn) = pool.get() {
                    let id = crate::audit::record_read_event(
                        &conn,
                        crate::audit::AuditKind::Recall,
                        &actor,
                        &event_query,
                        trace_detail.as_deref(),
                        &tenant,
                    );
                    if let Some(days) = crate::config::audit_read_retention_days() {
                        // No-op on failure — prunes are fail-safe (retention
                        // lingers; it never false-deletes); the warning
                        // is logged inside the helper.
                        crate::audit::prune_audit_retention(&conn, days);
                    }
                    // piggyback the DSAR ledger retention on the
                    // same read-event prune cadence (no dedicated timer).
                    crate::handlers::observe::purge_stale_dsar_ledger(
                        &conn,
                        crate::config::dsar_ledger_retention_days(),
                    );
                    return id;
                }
                None
            })
            .await
            .unwrap_or(None)
        }
    } else {
        None
    };

    #[cfg(feature = "otel")]
    {
        let span = tracing::Span::current();
        span.record("decision", format!("{:?}", decision).to_lowercase());
        span.record("graph_rescued", tel.graph_rescued);
        span.record("hits", tagged.len() as i64);
        span.record("domain", primary_domain.clone());
        span.record("principal", principal_label(principal));
        span.record("query_hash", crate::otel::query_hash(&trace_query));
    }

    Ok(RecallOutcome {
        tagged,
        tel,
        decision,
        trace_id,
        domains_searched,
        primary_domain,
    })
}

// ---------------------------------------------------------------------------
// Pure helpers (testable without AppState / StaticModel)
// ---------------------------------------------------------------------------

/// audit actor label for a recall read event — the JWT
/// principal's `sub`, or `loopback` in opaque/no-auth mode.
pub(crate) fn principal_label(principal: &Option<crate::auth::Principal>) -> String {
    principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".to_string())
}

/// audit tenant for a recall read event — the JWT
/// principal's tenant, or the default tenant in opaque/no-auth mode.
pub(crate) fn principal_tenant(principal: &Option<crate::auth::Principal>) -> String {
    principal
        .as_ref()
        .map(|p| p.tenant.clone())
        .unwrap_or_else(|| crate::audit::DEFAULT_TENANT.to_string())
}

/// resolve the shim-mode target domains given the
/// centroid-route result. Pure + deterministic.
///
/// - `Some(d)`, `d != "global"`: `[d, global]` — the routed domain is primary,
///   and a `global` rescue leg keeps the real working-memory corpus reachable
///   (in shim mode both labels share one physical pool, so this is a label
///   scope, not a second search pool).
/// - `Some("global")` or `None` (below the confidence threshold): `[global]` —
///   the real working-memory corpus. Deliberately NOT federating into the bulk
///   domain; that federation is what let a 90%-of-rows domain swamp un-routed
///   working-memory queries.
fn shim_routing_targets(route: Option<&str>) -> Vec<String> {
    match route {
        Some(d) if d != "global" => vec![d.to_string(), "global".to_string()],
        _ => vec!["global".to_string()],
    }
}

/// Map the final outcome to the recall decision: abstain (`low_confidence`)
/// only when the calibrated estimator said `ClarifyQuery` AND the retrieval
/// still produced zero hits (v1.12.0 "Discern": a graph-rescue pass may have
/// turned a ClarifyQuery into real hits — those return `ok`). Pure.
fn abstention_decision(
    recommendation: Option<crate::Recommendation>,
    hits_empty: bool,
) -> crate::handlers::RecallDecision {
    // ponytail: the only signal that triggers abstention is the calibrated
    // `ClarifyQuery` recommendation, which already encodes overlap + gap +
    // lexical-density gates. A single-score threshold here would duplicate the
    // estimator's job and drift from it; v1.6+ may add a learned threshold once
    // the judged-query baseline (Carry-forward) is recorded.
    match recommendation {
        Some(crate::Recommendation::ClarifyQuery) if hits_empty => {
            crate::handlers::RecallDecision::LowConfidence
        }
        _ => crate::handlers::RecallDecision::Ok,
    }
}

/// Map search results into the recall response shape, tagging every hit with
/// its source domain (per-hit, for federated recall) and provenance source.
/// Kept pure so it can be unit-tested without a model or database.
fn results_to_hits(
    results: Vec<(crate::SearchResult, String)>,
    include_provenance: bool,
    include_decayed: bool,
    principal: &Option<crate::auth::Principal>,
) -> Vec<RecallHit> {
    let now_unix = chrono::Utc::now().timestamp();
    results
        .into_iter()
        .map(|(mut r, domain)| {
            // every text field the chunk emits goes through the read
            // seam — PII redaction + invisible-Unicode strip — not just `content`.
            let pii = r.pii;
            let evidence = r.evidence.map(|mut e| {
                e.text = crate::gate::sanitize_read_cow(&e.text, pii, principal).into_owned();
                e.heading_path = crate::gate::sanitize_read_opt(e.heading_path, pii, principal);
                e
            });
            let conflict = evidence.as_ref().map(|e| {
                e.links
                    .iter()
                    .any(|l| l.kind == "contradicts" || l.kind == "supersedes")
            });
            RecallHit {
                id: r.id,
                title: crate::gate::sanitize_read_opt(r.title, pii, principal),
                content: crate::gate::sanitize_read_cow(&r.content, pii, principal).into_owned(),
                score: r.score,
                domain: Some(domain),
                source: Some(map_source(r.source)),
                provenance: if include_provenance {
                    Some(r.provenance)
                } else {
                    None
                },
                evidence,
                snippet: crate::gate::sanitize_read_opt(r.snippet, pii, principal),
                untrusted: true,
                conflict,
                confidence: r.confidence,
                // assertion_kind is free text at POST /ingest (no
                // vocabulary check), ingest_kind is free text via loopback
                // /add, memory_kind rides the UMP raw_kind round-trip — all
                // stored text, all through the same read seam.
                assertion_kind: crate::gate::sanitize_read_opt(r.assertion_kind, pii, principal),
                relevance: Some(crate::gate::relevance_tier(r.score)),
                decayed: include_decayed.then(|| crate::gate::is_decayed(r.expires_at, now_unix)),
                ingest_kind: crate::gate::sanitize_read_opt(r.ingest_kind, pii, principal),
                memory_kind: crate::gate::sanitize_read_opt(r.memory_kind, pii, principal),
                // provenance labels are stored text
                // (lawful_basis is free-form; region is regex-validated but the
                // read seam is the uniform control) — run through sanitize.
                lawful_basis: crate::gate::sanitize_read_opt(r.lawful_basis.take(), pii, principal),
                region: crate::gate::sanitize_read_opt(r.region.take(), pii, principal),
            }
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
            include_decayed: false,
            memory_kind: None,
            min_relevance: None,
            trace: false,
        }
    }

    #[test]
    fn shim_routing_targets_routed_domain_plus_global_rescue() {
        assert_eq!(
            shim_routing_targets(Some("gutmindsynergy")),
            vec!["gutmindsynergy", "global"]
        );
    }

    #[test]
    fn shim_routing_targets_routed_to_global_scopes_to_global() {
        assert_eq!(shim_routing_targets(Some("global")), vec!["global"]);
    }

    #[test]
    fn shim_routing_targets_unrouted_scopes_to_global_not_bulk_domain() {
        // Below the confidence threshold (None): the real working-memory
        // corpus only — never federate into a bulk domain. This is the
        // blog-domination regression guard.
        assert_eq!(shim_routing_targets(None), vec!["global"]);
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
            include_decayed: false,
            memory_kind: None,
            min_relevance: None,
            trace: false,
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
            false,
            &None,
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, 1);
        assert_eq!(hits[0].domain.as_deref(), Some("health"));
        assert_eq!(hits[0].source, Some(HitSource::Vector));
        assert!((hits[0].score - 0.9).abs() < f32::EPSILON);
        assert!(hits[1].title.is_none());
    }

    #[test]
    fn results_to_hits_forwards_provenance_labels() {
        let results = vec![SearchResult {
            id: 7,
            score: 0.6,
            title: None,
            content: "c".into(),
            source: None,
            provenance: Default::default(),
            flagged: false,
            untrusted: true,
            snippet: None,
            evidence: None,
            ingest_kind: Some("connector-github".into()),
            memory_kind: Some("procedure".into()),
            lawful_basis: Some("consent".into()),
            region: Some("eu-west-1".into()),
            ..Default::default()
        }];
        let hits = results_to_hits(
            results
                .into_iter()
                .map(|r| (r, "global".to_string()))
                .collect(),
            false,
            false,
            &None,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ingest_kind.as_deref(), Some("connector-github"));
        assert_eq!(hits[0].memory_kind.as_deref(), Some("procedure"));
        assert_eq!(hits[0].lawful_basis.as_deref(), Some("consent"));
        assert_eq!(hits[0].region.as_deref(), Some("eu-west-1"));
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
            false,
            &None,
        );
        let ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
        assert_eq!(ids, vec![10, 11, 12]);
        // scores descend
        assert!(hits[0].score > hits[1].score && hits[1].score > hits[2].score);
    }

    #[test]
    fn results_to_hits_empty() {
        assert!(results_to_hits(
            Vec::<(crate::SearchResult, String)>::new(),
            false,
            false,
            &None
        )
        .is_empty());
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
        let hits = results_to_hits(
            vec![mk(1, "work"), mk(2, "health"), mk(3, "work")],
            false,
            false,
            &None,
        );
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
        let hits = results_to_hits(vec![(r, "health".into())], true, false, &None);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].provenance.is_some());
        assert_eq!(hits[0].provenance.as_ref().unwrap().vector_rank, Some(0));
    }

    // ---- v1.5.0 "Epistemic" — calibrated abstention ----

    #[test]
    fn abstention_returns_low_confidence_only_on_clarify_query() {
        // The only signal that should trigger abstention is the calibrated
        // ClarifyQuery recommendation WITH an empty final hit list.
        // Return/RunPrf/RunReranker/IncreaseTopK all map to Ok — they
        // produced (or could produce) usable hits.
        use crate::Recommendation::*;
        // a graph rescue may have produced hits, so
        // ClarifyQuery + non-empty hits → Ok (never abstain on real results).
        assert_eq!(
            abstention_decision(Some(ClarifyQuery), false),
            crate::handlers::RecallDecision::Ok,
            "rescued hits must be returned, not abstained"
        );
        // ClarifyQuery + zero hits → the v1.5.0 low_confidence envelope.
        assert_eq!(
            abstention_decision(Some(ClarifyQuery), true),
            crate::handlers::RecallDecision::LowConfidence
        );
        for r in [Return, RunPrf, RunReranker, IncreaseTopK] {
            assert_eq!(
                abstention_decision(Some(r), true),
                crate::handlers::RecallDecision::Ok,
                "{r:?} should not abstain"
            );
        }
        // Missing recommendation (pre-quality-estimator path) must NOT
        // abstain — that would silently break legacy callers.
        assert_eq!(
            abstention_decision(None, true),
            crate::handlers::RecallDecision::Ok
        );
    }

    /// the stored recall trace records `query_hash` (SHA-256, v1.20.25),
    /// never the raw query text — a recall query typed by a user is itself
    /// personal data of that subject, and must not linger in the replay
    /// artifact (the DSAR residue sweep relies on this invariant).
    #[test]
    fn stored_trace_hashes_query_never_stores_raw_text() {
        crate::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_server::migration::run_migration(&mut conn, 1).unwrap();
        let secret_query = "alice@example.com's medical history";
        let trace_detail = serde_json::json!({
            "query_hash": crate::audit::hash(secret_query),
            "decision": "Ok",
            "graph_rescued": false,
            "hits": [],
        })
        .to_string();
        let id = crate::audit::record_read_event(
            &conn,
            crate::audit::AuditKind::Recall,
            "alice@example.com",
            secret_query,
            Some(&trace_detail),
            "api",
        )
        .expect("trace row");
        let replayed = crate::audit::read_trace(&conn, id).unwrap();
        assert!(
            !replayed.contains(secret_query),
            "raw query text must never be stored in the trace"
        );
        let v: serde_json::Value = serde_json::from_str(&replayed).unwrap();
        assert_eq!(v["query_hash"], crate::audit::hash(secret_query));
        // The raw query lives only on the tamper-evident audit row's target
        // (which is what record_read_event stores), never the replay artifact.
    }

    /// the recall read seam (results_to_hits) must strip invisible
    /// Unicode (bidi / zero-width) AND PII-redact EVERY text field a hit emits —
    /// title, content, snippet, evidence.text, evidence.heading_path — not just
    /// `content`. This closes the gap where title/snippet/evidence rode raw past
    /// redaction and the HTTP surface emitted raw invisible bytes.
    #[test]
    fn results_to_hits_strips_invisible_and_redacts_all_fields() {
        use crate::auth::{Action, Principal, Scope};
        let non_admin = Some(Principal {
            sub: "reader".into(),
            tenant: "team-a".into(),
            scopes: vec![Scope {
                action: Action::Read,
                team: "team-a".into(),
                domain: "global".into(),
            }],
            jti: "t".into(),
            roles: vec![],
            manages: vec![],
        });
        let mut r = crate::SearchResult::raw(
            1,
            0.9,
            Some("ti\u{200B}tle".into()),
            "contact alice@example.com\n\u{202A}hidden\u{202C}".into(),
        );
        r.pii = true;
        r.snippet = Some("alice@example.com snippet".into());
        r.evidence = Some(crate::search::Evidence {
            text: "alice@example.com evidence".into(),
            line_start: None,
            line_end: None,
            heading_path: Some("sec\u{200B}tion".into()),
            source_uri: None,
            revision_id: None,
            highlights: vec![],
            valid_from: None,
            valid_to: None,
            observed_at: None,
            authority: None,
            lifecycle: None,
            links: vec![],
            untrusted: true,
        });

        let hits = results_to_hits(vec![(r, "global".into())], false, false, &non_admin);
        let h = &hits[0];
        // PII redacted in content/snippet/evidence.
        assert!(
            !h.content.contains("alice@example.com"),
            "content leaked PII"
        );
        assert!(h.content.contains("[redacted:email]"));
        assert!(
            !h.content.contains('\u{202A}') && !h.content.contains('\u{202C}'),
            "bidi stripped from content"
        );
        assert!(
            !h.snippet
                .as_deref()
                .unwrap_or("")
                .contains("alice@example.com"),
            "snippet leaked PII"
        );
        let ev = h.evidence.as_ref().unwrap();
        assert!(
            !ev.text.contains("alice@example.com"),
            "evidence.text leaked PII"
        );
        // Invisible chars stripped from title + heading.
        assert!(
            !h.title.as_deref().unwrap_or("").contains('\u{200B}'),
            "zero-width stripped from title"
        );
        assert!(
            !ev.heading_path
                .as_deref()
                .unwrap_or("")
                .contains('\u{200B}'),
            "zero-width stripped from heading"
        );
        // Loopback (None) keeps full text but still strips invisible bytes.
        let hits_loop = results_to_hits(
            vec![(
                crate::SearchResult::raw(
                    2,
                    0.8,
                    Some("t\u{200B}it".into()),
                    "x\u{202A} y\u{202C}".into(),
                ),
                "global".into(),
            )],
            false,
            false,
            &None,
        );
        assert!(!hits_loop[0].content.contains('\u{202A}'));
        assert!(!hits_loop[0].title.as_deref().unwrap().contains('\u{200B}'));
    }
}
