//! `POST /ingest` — structured store (title + content + optional graph data).
//!
//! Per `API_CONTRACT.md` §3. The server trusts the caller's graph data after
//! validation; entities/relations are upserted idempotently into the
//! resolved domain's knowledge graph. The TOML annotation engine is retired
//! in v0.9.0; this is the primary KG write path.
//!
//! Implementation status:
//!   - Request/response serde ✅
//!   - Validation ✅ (bounds, regex, type, dedup-hash boundary)
//!   - Heavy logic: embed + insert + KG upsert is wired to the existing
//!     `add_chunk` and KG code paths. v0.9.0 swaps the JSON-vector storage
//!     for sqlite-vec; v1.0.0 adds the domain-router/centroid piece.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::AppState;
use xxhash_rust::xxh3::xxh3_64;
use zerocopy::IntoBytes;

use super::{
    normalize_domain, normalize_name, normalize_rel_type, IngestResponse, MAX_CONTENT,
    MAX_ENTITIES, MAX_RELATIONS, MAX_TITLE,
};
use crate::handlers::auth::OptPrincipal;
use crate::handlers::HandlerError;

/// A normalized relation ready for insert: (from, to, kind, optional explicit
/// valid_at, optional explicit invalid_at). The temporal pair is caller override;
/// when None the ingest path runs the deterministic temporal extractor.
type NormalizedRelation = (String, String, String, Option<String>, Option<String>);

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
    /// v1.4.0 "Calibrate" M1: optional explicit valid-at (ISO-8601). Overrides
    /// the deterministic extractor when the caller knows the fact's interval.
    #[serde(default)]
    pub valid_at: Option<String>,
    /// v1.4.0 "Calibrate" M1: optional explicit invalid-at (ISO-8601).
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
    /// v1.17.3 "UMP" M2: UMP-lowered overlays persisted onto the knowledge row
    /// (node_kind/assertion_kind/confidence/access_scope/expires_at/observed_at/
    /// valid_from/valid_to/ump_meta). Absent for legacy callers → column
    /// defaults/NULL — byte-identical behavior to v1.17.2. `ump_meta` is
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
}

/// v1.17.1 "Govern" M4: `?format=ump` accepts a UMP envelope instead of the
/// plain `IngestRequest` body (see `ingest`).
#[derive(Debug, Deserialize)]
pub struct IngestQuery {
    #[serde(default)]
    pub format: Option<String>,
}

/// `POST /ingest` — v1.17.1 "Govern" M4 adds `?format=ump`: the body is a UMP
/// envelope (`{"ump":"1.0","records":[…]}`) lowered into the same structured-
/// ingest path. v1.17.3 "UMP" M2 lifts the one-record ceiling: any number of
/// records, each processed through the identical single-record core with
/// per-record status — one failure never aborts the batch. A single record
/// (the v1.17.1 shape) keeps its plain `IngestResponse` reply; a multi-record
/// batch returns `{"ump":"1.0","results":[…],…}`. v1.17.3 M4 adds
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
            vec![serde_json::from_value(value)
                .map(|r: IngestRequest| (r, crate::handlers::ump::UmpMeta::default()))
                .map_err(|e| HandlerError::bad_request("invalid_body", e.to_string()))]
        };

    if lowered.len() == 1 {
        let (req, _) = lowered.into_iter().next().unwrap()?;
        let r = ingest_one(&_state, &principal.0, req).await?;
        return Ok(Json(serde_json::to_value(r).unwrap_or_default()));
    }

    // Multi-record UMP batch: per-record status, one failure never aborts.
    let mut results = Vec::with_capacity(lowered.len());
    for lowered_req in lowered {
        match lowered_req {
            Ok((req, _)) => match ingest_one(&_state, &principal.0, req).await {
                Ok(r) => results.push(serde_json::to_value(r).unwrap_or_default()),
                Err(e) => results.push(json!({
                    "status": "error",
                    "error": format!("{e:?}"),
                })),
            },
            Err(e) => results.push(json!({
                "status": "error",
                "error": format!("{e:?}"),
            })),
        }
    }
    Ok(Json(
        json!({ "ump": "1.0", "count": results.len(), "results": results }),
    ))
}

/// v1.17.3 M2: lower one UMP record into the ingest shape + the persisted UMP
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
        access_scope: row["access_scope"].as_str().map(|s| s.to_string()),
        observed_at: row["observed_at"].as_str().map(|s| s.to_string()),
        valid_from: row["valid_from"].as_str().map(|s| s.to_string()),
        valid_to: row["valid_to"].as_str().map(|s| s.to_string()),
        ump_meta: Some(serde_json::to_string(&meta).unwrap_or_default()),
    };
    Ok((req, meta))
}

/// The single-record ingest core shared by the plain, single-UMP, and
/// batch-UMP paths (`ingest` dispatches; this is the one place that writes).
pub(crate) async fn ingest_one(
    state: &Arc<AppState>,
    principal: &Option<crate::auth::Principal>,
    req: IngestRequest,
) -> Result<IngestResponse, HandlerError> {
    // v0.9.9: refuse new writes when over the capacity envelope (HTTP 507).
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

    // Forced domain (auto-route when omitted; v1.0.0)
    let forced_domain: Option<String> = match &req.domain {
        Some(d) => Some(normalize_domain(d)?),
        None => None,
    };
    // v1.2.0 M3 AuthZ: write gate at handler entry, scoped to the actual
    // target domain (forced, else "global"). Back-compat — `None` principal
    // (no JWT) is superuser.
    let gate_domain = forced_domain.as_deref().unwrap_or("global");
    super::authorize(principal, crate::auth::Action::Write, "", gate_domain)?;

    // Normalize + validate every entity/relation. Collect normalized forms
    // so downstream code can trust them.
    let mut entities: Vec<(String, Option<String>)> = Vec::with_capacity(req.entities.len());
    for e in &req.entities {
        let name = normalize_name(&e.name)?;
        if let Some(t) = &e.kind {
            if t.len() > 64 {
                return Err(HandlerError::bad_request(
                    "entity_invalid",
                    "entity type exceeds 64 characters",
                ));
            }
        }
        entities.push((name, e.kind.clone()));
    }
    // (from, to, kind, explicit valid_at, explicit invalid_at). The
    // temporal pair is optional caller override; None ⇒ run the extractor.
    let mut relations: Vec<NormalizedRelation> = Vec::with_capacity(req.relations.len());
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
    if entities.is_empty() && relations.is_empty() {
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

    let embedding = tokio::task::spawn_blocking(move || {
        model
            .encode(std::slice::from_ref(&content_for_embed))
            .into_iter()
            .next()
    })
    .await
    .map_err(|e| HandlerError::internal(format!("embedding task failed: {e}")))?
    .ok_or_else(|| HandlerError::internal("embedding produced no vector"))?;

    // v1.13.0 M2: auto-route when the caller omits a domain. The chunk
    // embedding is already computed above — reuse it against the stored
    // centroids. No confident centroid → fall back to `global` (the designed
    // safety net). Deterministic + zero extra embedding work.
    let domain_label = {
        let centroids = crate::domain_router::read_centroids(&state.pool).unwrap_or_default();
        crate::domain_router::route_domain_label(&forced_domain, &embedding, &centroids)
    };
    // Resolve the domain's pool via the registry (shim mode → global pool).
    let pool = state.registry.pool_for(&domain_label).map_err(|e| {
        HandlerError::bad_request("domain_invalid", format!("cannot resolve domain: {e}"))
    })?;
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

        // Baseline counts so we can report what THIS ingest actually added
        // (relations may auto-create entities that weren't in the input array).
        let entities_before: i64 = tx
            .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
            .unwrap_or(0);
        let relations_before: i64 = tx
            .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
            .unwrap_or(0);

        let content_hash = format!("{:016x}", xxh3_64(content.as_bytes()));

        // Idempotent dedup: if this exact content already exists, report duplicate.
        let existing: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE content_hash = ?1",
                rusqlite::params![&content_hash],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if existing > 0 {
            let id: i64 = tx
                .query_row(
                    "SELECT id FROM knowledge WHERE content_hash = ?1 LIMIT 1",
                    rusqlite::params![&content_hash],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            return Ok(IngestResponse {
                id,
                status: "duplicate",
                domain: Some(domain_label),
                entities_added: Some(0),
                relations_added: Some(0),
            });
        }

        // v1.17.3 "UMP": persist the UMP overlay onto the row. The
        // content-addressed `ump_id` is COMPUTED here (domain \0 content —
        // §6.2 ids are derived, never trusted from a record), so re-imports
        // of the same content land on the same id and the unique index holds.
        let ump_id = req.ump_meta.as_ref().map(|_| {
            brain_server::ump_integrity::content_id(&brain_server::ump_integrity::record_hash(
                format!("{domain_label}\0{content}").as_bytes(),
            ))
        });
        tx.execute(
            "INSERT INTO knowledge (title, content, source, content_hash, domain, pii, owner, \
                node_kind, assertion_kind, confidence, access_scope, expires_at, valid_from, \
                valid_to, observed_at, ump_id, ump_meta) \
             VALUES (?1, ?2, 'structured', ?3, ?4, ?5, ?6, COALESCE(?7, 'fact'), \
                COALESCE(?8, 'stated'), COALESCE(?9, 1.0), COALESCE(?10, 'private'), \
                ?11, ?12, ?13, ?14, ?15, ?16)",
            rusqlite::params![
                &title_for_store,
                &content,
                &content_hash,
                &domain_label,
                !crate::gate::scan_pii(&content).is_empty(),
                &owner,
                req.memory_kind.as_deref(),                req.assertion_kind.as_deref(),
                req.confidence,
                req.access_scope.as_deref(),
                req.expires_at,
                req.valid_from.as_deref(),
                req.valid_to.as_deref(),
                req.observed_at.as_deref(),
                ump_id.as_deref(),
                req.ump_meta.as_deref(),
            ],
        )
        .map_err(|e| HandlerError::internal(format!("insert knowledge failed: {e}")))?;
        let id: i64 = tx.last_insert_rowid();

        // vec0 (int8 + binary quantized). v0.9.0 DoD: vec0 is the sole vector
        // store; no raw f32 JSON is written to the legacy `embeddings` column.
        let _ = tx.execute(
            "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
             VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'structured', datetime('now'))",
            rusqlite::params![id, embedding.as_bytes()],
        );

        // Entities (idempotent upsert, with optional type).
        for (name, kind) in &entities_norm {
            tx.execute(
                "INSERT OR IGNORE INTO entities (name, entity_type) VALUES (?1, ?2)",
                rusqlite::params![name, kind],
            )
            .map_err(|e| HandlerError::internal(format!("insert entity failed: {e}")))?;
        }
        // Relations (idempotent upsert, anchored to this knowledge row).
        // ponytail: relations may reference entities that weren't explicitly
        // declared in `entities` — auto-create them on miss so the canonical
        // plan example (`vitamin d3 helps inflammation`) works even when only
        // `vitamin d3` is declared. Idempotent: INSERT OR IGNORE on existing
        // rows is a no-op; the SELECT then finds the row.
        //
        // v1.4.0 "Calibrate" M1: populate bi-temporal valid_at/invalid_at.
        // Caller-supplied explicit values win; otherwise run the deterministic
        // temporal extractor over the ingested content (best-effort, no LLM).
        // The extractor is pure; we run it once per relation keyed on content.
        let content_interval = crate::temporal::extract_interval_now(&content);
        for (from, to, kind, explicit_va, explicit_via) in &relations_norm {
            tx.execute(
                "INSERT OR IGNORE INTO entities (name, entity_type) VALUES (?1, NULL)",
                rusqlite::params![from],
            )
            .map_err(|e| HandlerError::internal(format!("auto-create from-entity failed: {e}")))?;
            tx.execute(
                "INSERT OR IGNORE INTO entities (name, entity_type) VALUES (?1, NULL)",
                rusqlite::params![to],
            )
            .map_err(|e| HandlerError::internal(format!("auto-create to-entity failed: {e}")))?;
            let from_id: i64 = tx
                .query_row(
                    "SELECT id FROM entities WHERE name = ?1",
                    rusqlite::params![from],
                    |r| r.get(0),
                )
                .map_err(|e| HandlerError::internal(format!("resolve from-entity failed: {e}")))?;
            let to_id: i64 = tx
                .query_row(
                    "SELECT id FROM entities WHERE name = ?1",
                    rusqlite::params![to],
                    |r| r.get(0),
                )
                .map_err(|e| HandlerError::internal(format!("resolve to-entity failed: {e}")))?;
            // Resolve the valid-time interval: explicit caller value, else the
            // extractor's result. `None` ⇒ leave the column NULL (always valid).
            let va: Option<&str> = explicit_va
                .as_deref()
                .or(content_interval.valid_at.as_deref());
            let via: Option<&str> = explicit_via
                .as_deref()
                .or(content_interval.invalid_at.as_deref());
            // INSERT OR IGNORE so re-ingesting the same (from,to,kind) is a
            // no-op; the temporal columns are set on first insert. Updating
            // them on a later ingest would require a separate UPDATE path,
            // intentionally not wired (re-ingest = idempotent no-op by design).
            tx.execute(
                "INSERT OR IGNORE INTO relationships \
                    (from_entity_id, to_entity_id, relation_type, knowledge_id, valid_at, invalid_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![from_id, to_id, kind, id, va, via],
            )
            .map_err(|e| HandlerError::internal(format!("insert relation failed: {e}")))?;
        }

        let entities_after: i64 = tx
            .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
            .unwrap_or(entities_before);
        let relations_after: i64 = tx
            .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
            .unwrap_or(relations_before);
        let entities_added = (entities_after - entities_before).max(0) as u32;
        let relations_added = (relations_after - relations_before).max(0) as u32;

        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;

        Ok(IngestResponse {
            id,
            status: "created",
            domain: Some(domain_label),
            entities_added: Some(entities_added),
            relations_added: Some(relations_added),
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
