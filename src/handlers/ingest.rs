//! `POST /ingest` — structured store (title + content + optional graph data).
//!
//! Per `API_CONTRACT.md` §3. The server trusts the caller's graph data after
//! validation; entities/relations are upserted idempotently into the
//! resolved domain's knowledge graph. The TOML annotation engine is retired;
//! this is the primary KG write path.
//!
//! Implementation status:
//!   - Request/response serde ✅
//!   - Validation ✅ (bounds, regex, type, dedup-hash boundary)
//!   - Heavy logic: embed + store + KG upsert is wired to the existing
//!     `add_chunk` and KG code paths. The JSON-vector storage was swapped
//!     for sqlite-vec; the domain-router/centroid piece came later.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::AppState;

use super::{
    IngestResponse, MAX_CONTENT, MAX_ENTITIES, MAX_RELATIONS, MAX_TITLE, normalize_domain,
    normalize_name, normalize_rel_type,
};
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

#[derive(Debug, Deserialize)]
pub struct EntityInput {
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RelationInput {
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// optional explicit valid-at (ISO-8601). Overrides
    /// the deterministic extractor when the caller knows the fact's interval.
    #[serde(default)]
    pub valid_at: Option<String>,
    /// optional explicit invalid-at (ISO-8601).
    #[serde(default)]
    pub invalid_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub title: String,
    pub content: String,
    pub domain: Option<String>,
    #[serde(default)]
    pub entities: Vec<EntityInput>,
    #[serde(default)]
    pub relations: Vec<RelationInput>,
    /// UMP-lowered overlays persisted onto the knowledge row
    /// (node_kind/assertion_kind/confidence/access_scope/expires_at/observed_at/
    /// valid_from/valid_to/ump_meta). Absent for legacy callers → column
    /// defaults/NULL — byte-identical to the pre-existing behavior. `ump_meta` is
    /// `Some` exactly when this request came from a UMP record (drives the
    /// computed `ump_id` write).
    #[serde(default)]
    pub memory_kind: Option<String>,
    #[serde(default)]
    pub assertion_kind: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// friendly retention — days from now (only honored
    /// when `expires_at` is absent; an explicit absolute always wins).
    /// Bounded 1..=36500 (the `POST /retention` bound).
    #[serde(default)]
    pub ttl_days: Option<i64>,
    #[serde(default)]
    pub access_scope: Option<String>,
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub valid_from: Option<String>,
    #[serde(default)]
    pub valid_to: Option<String>,
    #[serde(default)]
    pub ump_meta: Option<String>,
    /// an optional provenance source hint. Scraped
    /// data (`scrape`/`scraped`/`crawler`) without a documented [`Self::lawful_basis`]
    /// is quarantined, not stored (the NPC 2026-01 scraping advisory posture).
    #[serde(default)]
    pub source: Option<String>,
    /// the documented lawful basis for scraped data.
    /// Absent/blank on a scrape ingest → the record is quarantined (fail-closed).
    #[serde(default)]
    pub lawful_basis: Option<String>,
    /// the purpose-limitation label (TEXT) stored
    /// on the record (Art 5(1)(b) purpose evidence, alongside `lawful_basis`).
    #[serde(default)]
    pub purpose: Option<String>,
}

/// `?format=ump` accepts a UMP envelope instead of the
/// plain `IngestRequest` body (see `ingest`).
#[derive(Debug, Deserialize)]
pub struct IngestQuery {
    #[serde(default)]
    pub format: Option<String>,
}

/// `POST /ingest` with `?format=ump`: the body is a UMP
/// envelope (`{"ump":"1.0","records":[…]}`) lowered into the same structured-
/// ingest path. The one-record ceiling is lifted: any number of
/// records, each processed through the identical single-record core with
/// per-record status — one failure never aborts the batch. A single record
/// (the legacy single-record shape) keeps its plain `IngestResponse` reply;
/// a multi-record batch returns `{"ump":"1.0","results":[…],…}`. There is also
/// `?format=ump-md`: the raw §6.3 markdown projection (records joined by
/// `\n---\n`) — the file binding's import path.
pub async fn ingest(
    State(_state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<IngestQuery>,
    body: String,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let batch = q.format.as_deref() == Some("ump");
    let md = q.format.as_deref() == Some("ump-md");
    let lowered: Vec<Result<(IngestRequest, crate::handlers::ump::UmpMeta), HandlerError>> =
        if batch {
            let value: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| HandlerError::bad_request("invalid_body", e.to_string()))?;
            let records = value
                .get("records")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty())
                .ok_or_else(|| {
                    HandlerError::bad_request(
                        "ump_envelope",
                        "UMP body needs {\"ump\":\"1.0\",\"records\":[…]}",
                    )
                })?;
            records
                .iter()
                .map(|rec| lower_ump(rec).map_err(|e| HandlerError::bad_request("ump_invalid", e)))
                .collect()
        } else if md {
            // Split on the shared two-line record separator; every chunk
            // starts with its own `---\n` opener (the separator consumed the
            // join line only, never a projection's frontmatter closer).
            let chunks: Vec<String> = body
                .split(crate::handlers::ump::MD_RECORD_SEP)
                .filter(|c| !c.trim().is_empty())
                .map(str::to_string)
                .collect();
            if chunks.is_empty() {
                return Err(HandlerError::bad_request(
                    "ump_envelope",
                    "UMP markdown body needs at least one record",
                ));
            }
            chunks
                .iter()
                .map(|chunk| {
                    let rec = crate::handlers::ump::record_from_markdown(chunk)
                        .map_err(|e| HandlerError::bad_request("ump_invalid", e))?;
                    lower_ump(&rec).map_err(|e| HandlerError::bad_request("ump_invalid", e))
                })
                .collect()
        } else if q.format.is_some() {
            return Err(HandlerError::bad_request(
                "unknown_format",
                "format must be 'ump' or 'ump-md'",
            ));
        } else {
            let value: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| HandlerError::bad_request("invalid_body", e.to_string()))?;
            vec![
                serde_json::from_value(value)
                    .map(|r: IngestRequest| (r, crate::handlers::ump::UmpMeta::default()))
                    .map_err(|e| HandlerError::bad_request("invalid_body", e.to_string())),
            ]
        };

    if lowered.len() == 1 {
        // was `.next().unwrap()` — trivially safe after
        // the len==1 guard, but express it without a panic fallback (the lint
        // wall denies unwrap/expect in production).
        let mut lowered = lowered;
        let (req, _) = lowered.pop().ok_or_else(|| {
            HandlerError::internal("single-element batch vanished before dispatch".to_string())
        })??;
        // Seatbelt (Seatbelt): review posture proposes instead of inserting.
        if crate::config::write_posture() == "review" {
            return Err(propose_structured(&_state, &principal.0, req).await);
        }
        let r = ingest_one(&_state, &principal.0, req).await?;
        return Ok(Json(serde_json::to_value(r).unwrap_or_default()));
    }

    // Multi-record UMP batch: per-record status, one failure never aborts.
    let mut results = Vec::with_capacity(lowered.len());
    for lowered_req in lowered {
        if let Err(e) = &lowered_req {
            results.push(err_envelope(e));
            continue;
        }
        // The error arm was handled above; this binding always succeeds.
        let Ok((req, _)) = lowered_req else {
            continue;
        };
        let outcome = if crate::config::write_posture() == "review" {
            Err(propose_structured(&_state, &principal.0, req).await)
        } else {
            ingest_one(&_state, &principal.0, req).await
        };
        results.push(
            outcome
                .map(|r| serde_json::to_value(r).unwrap_or_default())
                .unwrap_or_else(err_envelope),
        );
    }
    Ok(Json(
        json!({ "ump": "1.0", "count": results.len(), "results": results }),
    ))
}

/// lower one UMP record into the ingest shape + the persisted UMP
/// overlay. `memory_kind`/`assertion_kind`/`confidence`/`access_scope`/
/// `expires_at`/times map onto the knowledge columns so a re-export loses
/// nothing; [`crate::handlers::ump::UmpMeta`] carries the fields with no brain
/// column (the raw kind, the owner DID, visibility, the origin record id).
pub fn lower_ump(record: &Value) -> Result<(IngestRequest, crate::handlers::ump::UmpMeta), String> {
    let row = crate::handlers::ump::from_ump(record)?;
    let entities: Vec<EntityInput> = row["entities"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|e| EntityInput {
            name: e["name"].as_str().unwrap_or_default().to_string(),
            kind: e["type"].as_str().map(|s| s.to_string()),
        })
        .collect();
    let relations: Vec<RelationInput> = row["relations"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|r| RelationInput {
            from: r["from"].as_str().unwrap_or_default().to_string(),
            to: r["to"].as_str().unwrap_or_default().to_string(),
            kind: r["type"].as_str().unwrap_or("relates_to").to_string(),
            valid_at: None,
            invalid_at: None,
        })
        .collect();
    let meta = crate::handlers::ump::UmpMeta {
        kind: record["kind"].as_str().map(|s| s.to_string()),
        owner: row["owner"].as_str().map(|s| s.to_string()),
        visibility: row["access_scope"].as_str().map(|s| s.to_string()),
        origin: record["id"].as_str().map(|s| s.to_string()),
        provenance: record.get("provenance").cloned().filter(|v| !v.is_null()),
        consent: record.get("consent").cloned().filter(|v| !v.is_null()),
    };
    let req = IngestRequest {
        title: row["title"].as_str().unwrap_or("untitled").to_string(),
        content: row["content"].as_str().unwrap_or_default().to_string(),
        domain: None,
        entities,
        relations,
        memory_kind: row["memory_kind"].as_str().map(|s| s.to_string()),
        assertion_kind: row["assertion_kind"].as_str().map(|s| s.to_string()),
        confidence: row["confidence"].as_f64(),
        expires_at: row["expires_at"].as_i64(),
        ttl_days: None, // UMP records carry absolute expiry (lifecycle.decay)
        access_scope: row["access_scope"].as_str().map(|s| s.to_string()),
        observed_at: row["observed_at"].as_str().map(|s| s.to_string()),
        valid_from: row["valid_from"].as_str().map(|s| s.to_string()),
        valid_to: row["valid_to"].as_str().map(|s| s.to_string()),
        ump_meta: Some(serde_json::to_string(&meta).unwrap_or_default()),
        source: None,       // UMP provenance carries no scrape source
        lawful_basis: None, // a UMP record declares lawful basis separately
        purpose: row["purpose"].as_str().map(|s| s.to_string()),
    };
    Ok((req, meta))
}

/// The single-record ingest core shared by the plain, single-UMP, and
/// batch-UMP paths (`ingest` dispatches; this is the one place that writes).
/// Seatbelt (Seatbelt): route a structured record through the proposal
/// pipeline instead of inserting. Always returns `HandlerError::accepted`
/// carrying the 202 proposal envelope (the caller's `Err` arm renders it).
async fn propose_structured(
    state: &Arc<AppState>,
    principal: &Option<crate::auth::Principal>,
    req: IngestRequest,
) -> HandlerError {
    let p = super::gate::create_proposal(
        state.clone(),
        principal.clone(),
        super::gate::ProposalRequest {
            content: req.content,
            kind: req.memory_kind.unwrap_or_else(|| "fact".to_string()),
            source: Some("structured".to_string()),
            authority: None,
            observed_at: None,
            domain: req.domain,
            source_prompt: None,
        },
    )
    .await;
    p.map_or_else(
        |e| e,
        |p| HandlerError::accepted(p.id, json!({ "proposal_id": p.id, "status": "pending" })),
    )
}

/// the uniform per-record error envelope (lowering + ingest failures share it).
fn err_envelope(e: impl std::fmt::Debug) -> serde_json::Value {
    json!({ "status": "error", "error": format!("{e:?}") })
}

pub(crate) async fn ingest_one(
    state: &Arc<AppState>,
    principal: &Option<crate::auth::Principal>,
    mut req: IngestRequest,
) -> Result<IngestResponse, HandlerError> {
    // refuse new writes when over the capacity envelope (HTTP 507).
    // Read routes do not call this guard; an over-capacity brain still answers.
    super::guard_capacity(state)?;

    // ---- validation ----
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(HandlerError::bad_request(
            "title_invalid",
            "title must not be empty",
        ));
    }
    if title.len() > MAX_TITLE {
        return Err(HandlerError::bad_request(
            "title_invalid",
            format!("title exceeds {MAX_TITLE} characters"),
        ));
    }

    let content = req.content; // do not trim: content is full prose
    if content.is_empty() {
        return Err(HandlerError::bad_request(
            "content_empty",
            "content must not be empty",
        ));
    }
    if content.len() > MAX_CONTENT {
        return Err(HandlerError::bad_request_with(
            "content_too_large",
            format!("content exceeds {MAX_CONTENT} characters"),
            serde_json::json!({ "max": MAX_CONTENT }),
        ));
    }

    // screen this shared write core exactly like its
    // siblings (`/add`, `/ingest/memory`, `/ingest/markdown`). The screen
    // stage lives in the ingest core (the fence is of the FUNCTION):
    // `Reject` keeps the HTTP-400; `Quarantine` (default) ingests then flags
    //     after the store so a flagged plant is excluded from retrieval and its KG
    // edges are skipped.
    let quarantine_flagged = crate::service::ingest::screen_structured(
        &content,
        &title,
        req.source.as_deref(),
        req.lawful_basis.as_deref(),
    )
    .map_err(map_ingest_error)?;

    // trust labels are not client-asserted.
    // A DIRECT client assert (HTTP `/ingest`, `/proposals` — `ump_meta` is
    // None) must be a real kind, the same strict round-trip the proposal path
    // now enforces — an unknown value must not silently store as `fact`. The
    // lowered UMP path (`ump_meta` present) deliberately DOES carry UMP kinds
    // with no brain-column equivalent (`working`/`identity`/`semantic`, and
    // `procedural` for a stored step) — those are preserved in `ump_meta.kind`
    // and must not be rejected (that would break UMP revise/re-import, the
    // §3.5/§6 round-trip seam). `confidence` is a probability on BOTH paths.
    if let Some(kind) = req.memory_kind.as_deref()
        && req.ump_meta.is_none()
        && !crate::procedural::MemoryKind::is_strict_valid(kind)
    {
        return Err(HandlerError::bad_request_with(
            "invalid_memory_kind",
            "memory_kind must be one of: fact, procedure, step, decision, episodic, entitlement",
            serde_json::json!({
                "allowed": ["fact", "procedure", "step", "decision", "episodic"]
            }),
        ));
    }
    if let Some(c) = req.confidence
        && !(0.0..=1.0).contains(&c)
    {
        return Err(HandlerError::bad_request_with(
            "invalid_confidence",
            "confidence must be within 0.0..=1.0",
            serde_json::json!({ "min": 0.0, "max": 1.0 }),
        ));
    }

    if req.entities.len() > MAX_ENTITIES {
        return Err(HandlerError::bad_request_with(
            "too_many_entities",
            format!("entities exceed {MAX_ENTITIES}"),
            serde_json::json!({ "max": MAX_ENTITIES }),
        ));
    }
    if req.relations.len() > MAX_RELATIONS {
        return Err(HandlerError::bad_request_with(
            "too_many_relations",
            format!("relations exceed {MAX_RELATIONS}"),
            serde_json::json!({ "max": MAX_RELATIONS }),
        ));
    }

    // `ttl_days` is the friendly retention field (days
    // from now). It only applies when `expires_at` is absent — an explicit
    // absolute expiry always wins (the row-wins invariant). The clock is
    // injected by the handler (the core is pinned with a fixed one).
    req.expires_at = crate::service::ingest::ttl_days_to_expires(
        req.expires_at,
        req.ttl_days,
        chrono::Utc::now().timestamp(),
    )
    .map_err(map_ingest_error)?;

    // Forced domain (auto-route when omitted)
    let forced_domain: Option<String> = req.domain.as_deref().map(normalize_domain).transpose()?;
    // write gate at handler entry, scoped to the actual
    // target domain (forced, else "global"). Back-compat — `None` principal
    // (no JWT) is superuser.
    let gate_domain = forced_domain.as_deref().unwrap_or("global");
    super::authorize(principal, crate::auth::Action::Write, "", gate_domain)?;

    // Normalize + validate every entity/relation. Collect normalized forms
    // so downstream code can trust them.
    let mut entities: Vec<(String, Option<String>)> = Vec::with_capacity(req.entities.len());
    for e in &req.entities {
        let name = normalize_name(&e.name)?;
        if let Some(t) = &e.kind
            && t.len() > 64
        {
            return Err(HandlerError::bad_request(
                "entity_invalid",
                "entity type exceeds 64 characters",
            ));
        }
        entities.push((name, e.kind.clone()));
    }
    // (from, to, kind, explicit valid_at, explicit invalid_at). The
    // temporal pair is optional caller override; None ⇒ run the extractor.
    let mut relations: Vec<crate::service::ingest::NormalizedRelation> =
        Vec::with_capacity(req.relations.len());
    for r in &req.relations {
        let from = normalize_name(&r.from)?;
        let to = normalize_name(&r.to)?;
        let kind = normalize_rel_type(&r.kind)?;
        // Normalize explicit temporal overrides (validate, don't trust raw).
        let va = r
            .valid_at
            .as_deref()
            .map(|s| crate::search::normalize_since(s.trim()))
            .transpose()
            .map_err(|e| HandlerError::bad_request("temporal_invalid", e.to_string()))?;
        let via = r
            .invalid_at
            .as_deref()
            .map(|s| crate::search::normalize_since(s.trim()))
            .transpose()
            .map_err(|e| HandlerError::bad_request("temporal_invalid", e.to_string()))?;
        relations.push((from, to, kind, va, via));
    }

    // Auto-extract entities + relationships via the deterministic linker when
    // the caller supplies neither (OpenClaw plugin sends title + content only).
    // Skipped for a quarantined plant — a flagged chunk gets no graph edges.
    if !quarantine_flagged && entities.is_empty() && relations.is_empty() {
        let code_ranges = crate::linker::find_code_ranges(&content);
        let table_ranges = crate::linker::find_table_ranges(&content);
        let list_bold_ranges = crate::linker::find_list_item_bold_ranges(&content);
        let mut excluded: Vec<(usize, usize)> = code_ranges;
        excluded.extend(table_ranges);
        excluded.extend(list_bold_ranges);

        let mut vocab = crate::linker::extract_vocabulary(&content, &excluded);
        vocab.finalize();
        let entity_names: Vec<String> = vocab.entities.clone();
        entities = entity_names.iter().map(|n| (n.clone(), None)).collect();
        let entity_set: std::collections::HashSet<String> = entity_names.into_iter().collect();
        let matcher: crate::linker::EntityMatcher = vocab.into();

        let extra_patterns = matcher.discover_verb_patterns(&content, 3, &excluded);
        let extra_refs: Vec<(&str, &str)> = extra_patterns
            .iter()
            .map(|(v, _)| (v.as_str(), v.as_str()))
            .collect();

        let edges = matcher.find_relationships(&content, &excluded, &extra_refs);

        let heading_edges =
            crate::linker::extract_heading_relationships(&content, &entity_set, &excluded);

        for edge in edges.into_iter().chain(heading_edges) {
            relations.push((edge.from, edge.to, edge.relation, None, None));
        }
    }

    // ---- heavy logic ----
    // embed via model2vec, dedup via xxh3-64 content_hash, route to the
    // resolved domain (forced, else auto-routed via centroids), and insert
    // knowledge + vec0 + legacy embedding + entities + relations in a single
    // SQLite transaction.
    let model = Arc::clone(&state.model);
    let entities_norm = entities;
    let relations_norm = relations;
    let content_for_embed = content.clone();
    let title_for_store = title.clone();

    let embedding = tokio::task::spawn_blocking(move || model.encode_one(&content_for_embed))
        .await
        .map_err(|e| HandlerError::internal(format!("embedding task failed: {e}")))?;
    if embedding.is_empty() {
        return Err(HandlerError::internal("embedding produced no vector"));
    }

    // auto-route when the caller omits a domain. The chunk
    // embedding is already computed above — reuse it against the stored
    // centroids. No confident centroid → fall back to `global` (the designed
    // safety net). Deterministic + zero extra embedding work.
    let domain_label = {
        let centroids = crate::domain_router::read_centroids(&state.pool).unwrap_or_default();
        crate::domain_router::route_domain_label(&forced_domain, &embedding, &centroids)
    };
    // Re-authorize on the ACTUAL target. The gate above ran on
    // forced-or-global; auto-routing can then resolve any domain with a
    // centroid, which previously let a `write:<t>/global`-only principal
    // contaminate another tenant's domain (and poison its centroid). Loud 403
    // on a foreign target — the caller must force a domain it holds.
    if domain_label != gate_domain {
        super::authorize(principal, crate::auth::Action::Write, "", &domain_label)?;
    }
    // Resolve the domain's pool via the registry (shim mode → global pool).
    // registered-only in multi-db — an unregistered label
    // 404s (`domain_unknown`); creation happens only in `POST /domains`.
    let pool = state
        .registry
        .pool_for(&domain_label)
        .map_err(super::map_domain_error)?;

    // apply the bound domain profile's ingest
    // defaults (the pure core is `apply_profile_ingest` — unit-tested there).
    // A domain with no bound profile skips straight through (byte-identical
    // legacy behavior). An unreadable bound profile fails CLOSED — a
    // strict-posture domain must not silently ingest raw PII.
    let profile = {
        let conn = state
            .pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        brain_server::profile::profile_for_domain(&conn, &domain_label)
            .map_err(HandlerError::internal)?
    };
    let (title_for_store, content, access_scope) = crate::service::ingest::apply_profile_ingest(
        profile.as_ref(),
        &title_for_store,
        &content,
        req.access_scope.clone(),
        req.memory_kind.as_deref(),
    )
    .map_err(map_ingest_error)?;
    req.access_scope = access_scope;
    // whether this domain's bound profile is a
    // strict-posture one — the flag that marks a record with no documented
    // lawful_basis (purpose-limitation + data-minimization evidence).
    let strict_domain = profile
        .as_ref()
        .is_some_and(brain_server::profile::Profile::pii_strict);
    // Resolve the owner label before the closure (the reference can't cross
    // the spawn_blocking boundary).
    let owner = super::gate::principal_to_owner(principal);

    let result = tokio::task::spawn_blocking(move || -> Result<IngestResponse, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("transaction failed: {e}")))?;

        // The store stage — every statement of the ingest runs inside THIS
        // tx (the audit-per-write law travels with the logic). A dropped or
        // rolled-back tx takes the whole store with it.
        let store = crate::service::ingest::StoreRecord {
            domain: &domain_label,
            title: &title_for_store,
            content: &content,
            owner: owner.as_deref(),
            strict_domain,
            quarantine_flagged,
            embedding: &embedding,
            memory_kind: req.memory_kind.as_deref(),
            assertion_kind: req.assertion_kind.as_deref(),
            confidence: req.confidence,
            access_scope: req.access_scope.as_deref(),
            expires_at: req.expires_at,
            valid_from: req.valid_from.as_deref(),
            valid_to: req.valid_to.as_deref(),
            observed_at: req.observed_at.as_deref(),
            ump_meta: req.ump_meta.as_deref(),
            lawful_basis: req.lawful_basis.as_deref(),
            purpose: req.purpose.as_deref(),
            entities: &entities_norm,
            relations: &relations_norm,
        };
        let outcome =
            crate::service::ingest::store_record(&tx, &store).map_err(map_ingest_error)?;

        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;

        Ok(match outcome {
            crate::service::ingest::StoreOutcome::Duplicate { id } => IngestResponse {
                id,
                status: "duplicate",
                domain: Some(domain_label),
                entities_added: Some(0),
                relations_added: Some(0),
                compliance: None,
            },
            crate::service::ingest::StoreOutcome::Created {
                id,
                entities_added,
                relations_added,
                lawful_basis_missing,
            } => IngestResponse {
                id,
                status: "created",
                domain: Some(domain_label),
                entities_added: Some(entities_added),
                relations_added: Some(relations_added),
                // a strict-posture domain storing a record with no
                // lawful_basis is flagged (the purpose-limitation evidence).
                // Only present when flagged (additive; absent otherwise).
                compliance: lawful_basis_missing
                    .then_some(serde_json::json!({ "lawful_basis_missing": true })),
            },
        })
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    // Recompute this domain's centroid so future queries can route to it.
    // Best-effort: a centroid failure must not fail an otherwise-successful ingest.
    if let Some(d) = &result.domain {
        let dpool = state.registry.pool_for(d).ok();
        if let Some(dp) = dpool {
            let _ = crate::domain_router::recompute_centroid(&dp, d, &state.pool);
        }
    }

    Ok(result)
}

/// The frozen handler boundary map for the ingest core's typed errors:
/// every variant renders the exact status + body its pre-move `HandlerError`
/// produced, so the wire vocabulary is unchanged (404-vs-403-vs-409 semantics
/// are pinned per route; the service never names an HTTP status).
fn map_ingest_error(e: crate::service::ingest::IngestError) -> HandlerError {
    use crate::service::ingest::IngestError as E;
    match e {
        E::InputRejected => {
            HandlerError::bad_request("input_rejected", "input contains suspicious patterns")
        }
        E::TtlDaysInvalid(_) => HandlerError::bad_request(
            "ttl_days_invalid",
            "ttl_days must be an integer in [1, 36500]",
        ),
        E::KindNotAllowed { effective, profile } => HandlerError::bad_request(
            "kind_not_allowed",
            format!("memory_kind '{effective}' is not in profile '{profile}''s allowed kinds"),
        ),
        E::ProfileChanged => HandlerError::conflict_with(
            "profile_changed",
            "the domain's bound profile changed during ingest — retry",
            serde_json::json!([]),
        ),
        E::QuarantineFlag(e) => HandlerError::internal(format!("quarantine flag failed: {e}")),
        // Database messages carry their statement context verbatim
        // (e.g. "… knowledge failed: …", "resolve tx timestamp failed: …").
        E::Database(msg) => HandlerError::internal(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M2: a UMP record lowers into the ingest shape with every overlay
    /// field mapped onto the knowledge columns + the persisted meta string.
    #[test]
    fn lower_ump_maps_fields_and_meta() {
        let rec = serde_json::json!({
            "ump": "1.0",
            "id": "urn:ump:brain:global:7",
            "kind": "working",
            "scope": {"owner": "did:key:zAlive", "visibility": "shared"},
            "lifecycle": {"decay": 1_800_000_000},
            "time": {
                "observed": "2026-08-09T00:00:00Z",
                "valid_from": "2026-01-01T00:00:00Z"
            },
            "body": {
                "text": "Dave works at Acme.",
                "structured": {
                    "title": "dave note",
                    "memory_kind": "fact",
                    "assertion_kind": "claim",
                    "confidence": 0.8,
                    "expires_at": 1_800_000_000,
                    "entities": [{"name": "Dave", "type": "person"}],
                    "relations": [{"from": "Dave", "to": "Acme", "type": "works_at"}]
                }
            }
        });
        let (req, meta) = lower_ump(&rec).expect("lower");
        assert_eq!(req.title, "dave note");
        assert_eq!(req.content, "Dave works at Acme.");
        assert_eq!(req.memory_kind.as_deref(), Some("fact"));
        assert_eq!(req.assertion_kind.as_deref(), Some("claim"));
        assert_eq!(req.confidence, Some(0.8));
        assert_eq!(req.expires_at, Some(1_800_000_000));
        assert_eq!(req.access_scope.as_deref(), Some("shared"));
        assert_eq!(req.observed_at.as_deref(), Some("2026-08-09 00:00:00"));
        assert_eq!(req.valid_from.as_deref(), Some("2026-01-01 00:00:00"));
        assert_eq!(req.valid_to, None);
        assert_eq!(req.entities.len(), 1);
        assert_eq!(req.relations.len(), 1);
        assert_eq!(req.relations[0].kind, "works_at");
        assert_eq!(meta.kind.as_deref(), Some("working"));
        assert_eq!(meta.owner.as_deref(), Some("did:key:zAlive"));
        assert_eq!(meta.visibility.as_deref(), Some("shared"));
        assert_eq!(meta.origin.as_deref(), Some("urn:ump:brain:global:7"));
        let persisted: Value = serde_json::from_str(req.ump_meta.as_deref().unwrap()).unwrap();
        assert_eq!(persisted["kind"], "working");
        assert_eq!(persisted["owner"], "did:key:zAlive");
    }

    /// M2: lowering drops the `ump`/`id` top-level (they are envelope + origin
    /// metadata, not knowledge columns) and rejects a malformed record.
    #[test]
    fn lower_ump_rejects_malformed_records() {
        assert!(lower_ump(&serde_json::json!({"ump": "1.0"})).is_err());
        assert!(lower_ump(&serde_json::json!({"ump": "1.0", "body": {"text": 42}})).is_err());
    }
}
