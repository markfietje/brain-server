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

use axum::{extract::State, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::AppState;
use xxhash_rust::xxh3::xxh3_64;
use zerocopy::IntoBytes;

use super::{
    normalize_domain, normalize_name, normalize_rel_type, IngestResponse, MAX_CONTENT,
    MAX_ENTITIES, MAX_RELATIONS, MAX_TITLE,
};
use crate::handlers::HandlerError;

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
}

/// `POST /ingest`
pub async fn ingest(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<IngestRequest>,
) -> Result<Json<IngestResponse>, HandlerError> {
    // v0.9.9: refuse new writes when over the capacity envelope (HTTP 507).
    // Read routes do not call this guard; an over-capacity brain still answers.
    super::guard_capacity(&_state)?;

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
    let mut relations: Vec<(String, String, String)> = Vec::with_capacity(req.relations.len());
    for r in &req.relations {
        let from = normalize_name(&r.from)?;
        let to = normalize_name(&r.to)?;
        let kind = normalize_rel_type(&r.kind)?;
        relations.push((from, to, kind));
    }

    // ---- heavy logic ----
    // embed via model2vec, dedup via xxh3-64 content_hash, route to the
    // resolved domain (forced, else "global"), and insert knowledge + vec0 +
    // legacy embedding + entities + relations in a single SQLite transaction.
    let domain_label = forced_domain
        .clone()
        .unwrap_or_else(|| "global".to_string());
    let model = Arc::clone(&_state.model);
    // Resolve the domain's pool via the registry (shim mode → global pool).
    let pool = _state.registry.pool_for(&domain_label).map_err(|e| {
        HandlerError::bad_request("domain_invalid", format!("cannot resolve domain: {e}"))
    })?;
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

        tx.execute(
            "INSERT INTO knowledge (title, content, source, content_hash, domain)
             VALUES (?1, ?2, 'structured', ?3, ?4)",
            rusqlite::params![&title_for_store, &content, &content_hash, &domain_label],
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
        for (from, to, kind) in &relations_norm {
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
            tx.execute(
                "INSERT OR IGNORE INTO relationships (from_entity_id, to_entity_id, relation_type, knowledge_id)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![from_id, to_id, kind, id],
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
        let dpool = _state.registry.pool_for(d).ok();
        if let Some(dp) = dpool {
            let _ = crate::domain_router::recompute_centroid(&dp, d, &_state.pool);
        }
    }

    Ok(Json(result))
}
