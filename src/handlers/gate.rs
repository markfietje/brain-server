//! v1.14.0 "Gate" — HTTP handlers for write-back gating (M1), decay + GDPR
//! lifecycle (M2). The pure logic lives in `src/gate.rs`; this module does the
//! HTTP + transaction wiring, reusing the existing ingest / consolidate /
//! sources machinery instead of re-implementing it.
//!
//! Routes:
//!   POST /ingest/proposal     — queue a scored candidate; no `knowledge` row.
//!   GET  /proposals?status=   — the human review queue.
//!   POST /proposals/{id}/approve[?supersedes=<id>] — promote (one tx).
//!   POST /proposals/{id}/reject — reject (audited, never deleted).
//!   GET  /decayed             — operator review of decayed chunks.
//!   GET  /export[?include_pii_map=true] — portable JSON export (GDPR).
//!   POST /purge               — hard, explicit, audited deletion (GDPR).
//!
//! Human-in-the-loop: nothing here auto-promotes, auto-decays-away, or
//! auto-deletes. The human decides. Zero tokens, no LLM, no background worker.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::handlers::auth::OptPrincipal;
use crate::handlers::{HandlerError, MAX_QUERY};
use crate::AppState;

/// Max proposals returned per review page. Bounded so a runaway queue can't
/// unbounded a response.
const MAX_PROPOSALS: usize = 200;
/// Max ids accepted by a single `/purge` call. Explicit-only deletion must be
/// deliberate; a huge batch is a footgun.
const MAX_PURGE_IDS: usize = 1000;

/// `POST /ingest/proposal`
#[derive(Debug, Deserialize)]
pub struct ProposalRequest {
    pub content: String,
    /// memory_kind vocabulary (fact/procedure/step/decision/episodic).
    #[serde(default = "default_fact")]
    pub kind: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub authority: Option<f32>,
    #[serde(default)]
    pub observed_at: Option<i64>,
    #[serde(default)]
    pub domain: Option<String>,
}

fn default_fact() -> String {
    "fact".to_string()
}

#[derive(Debug, Serialize)]
pub struct ProposalResponse {
    pub id: i64,
    pub status: &'static str,
    pub novelty: f32,
    pub conflict_with: Option<i64>,
    pub salience: f32,
}

/// `POST /ingest/proposal` — queue a scored candidate. No `knowledge` row is
/// created; `/recall` cannot see it until a human approves.
pub async fn ingest_proposal(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<ProposalRequest>,
) -> Result<Json<ProposalResponse>, HandlerError> {
    let domain = req
        .domain
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| "global".to_string());
    super::authorize(&principal.0, crate::auth::Action::Write, "", &domain)?;
    let content = req.content.trim().to_string();
    if content.is_empty() {
        return Err(HandlerError::bad_request(
            "empty_content",
            "content is required",
        ));
    }
    if content.chars().count() > MAX_QUERY {
        return Err(HandlerError::bad_request(
            "content_too_long",
            format!("content exceeds {MAX_QUERY} chars"),
        ));
    }
    if !crate::procedural::MemoryKind::from_str(&req.kind).is_valid_for_gate() {
        return Err(HandlerError::bad_request(
            "invalid_kind",
            format!("unknown memory_kind '{}'", req.kind),
        ));
    }

    let pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    let model = Arc::clone(&state.model);
    let content_for_task = content.clone();

    let resp = tokio::task::spawn_blocking(move || -> Result<ProposalResponse, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        // Deterministic scoring (M1): novelty via vec0 KNN, conflict via the
        // consolidate machinery, salience via the length/entity heuristic.
        let embedding = match model
            .encode(std::slice::from_ref(&content_for_task))
            .into_iter()
            .next()
        {
            Some(e) => e,
            None => {
                return Err(HandlerError::internal("embedding generation failed"));
            }
        };
        let novelty = crate::gate::novelty(&conn, &embedding).unwrap_or(1.0); // first memory / no index → max novelty
        let conflict_with = find_conflict(&conn, &content_for_task);
        let entity_count = crate::linker::extract_vocabulary(&content_for_task, &[])
            .entities
            .len();
        let salience = crate::gate::salience(&content_for_task, entity_count);
        let now = chrono::Utc::now().timestamp();

        let id: i64 = conn
            .query_row(
                "INSERT INTO proposals(kind, content, source, authority, observed_at,
                                   novelty, conflict_with, salience, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             RETURNING id",
                rusqlite::params![
                    req.kind,
                    content_for_task,
                    req.source,
                    req.authority,
                    req.observed_at,
                    novelty,
                    conflict_with,
                    salience,
                    now
                ],
                |r| r.get(0),
            )
            .map_err(|e| HandlerError::internal(format!("proposal insert failed: {e}")))?;

        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Ingest,
            "api",
            &format!("proposal:{id}"),
            crate::audit::AuditStatus::Ok,
            "proposal_pending",
        );

        Ok(ProposalResponse {
            id,
            status: "pending",
            novelty,
            conflict_with,
            salience,
        })
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(resp))
}

/// Find a live chunk whose subject conflicts with the candidate content. Reuses
/// [`crate::consolidate::find_subject_conflicts`]'s signal: a candidate that
/// contradicts an existing current claim is flagged in the review queue so the
/// human sees the conflict, not a silent overwrite.
fn find_conflict(conn: &rusqlite::Connection, content: &str) -> Option<i64> {
    // Cheap exact-subject pre-check before the O(n²) pairwise scan: only run
    // the full conflict scan when the candidate's subject appears somewhere.
    let subject = content
        .lines()
        .next()
        .unwrap_or(content)
        .chars()
        .take(120)
        .collect::<String>();
    let mut stmt = conn
        .prepare("SELECT id FROM knowledge WHERE (title IS NOT NULL AND title = ?1) OR (heading_path IS NOT NULL AND heading_path = ?1) AND valid_to IS NULL LIMIT 1")
        .ok()?;
    let matched: Option<i64> = stmt
        .query_row(rusqlite::params![subject], |r| r.get(0))
        .ok();
    drop(stmt);
    // Full pairwise conflict scan only when we have a subject-anchored hit.
    if matched.is_some() {
        if let Ok(pairs) = crate::consolidate::find_subject_conflicts(conn) {
            return pairs.into_iter().map(|p| p.from_chunk).next();
        }
    }
    None
}

#[derive(Debug, Deserialize)]
pub struct ProposalListQuery {
    #[serde(default = "default_pending")]
    pub status: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn default_pending() -> String {
    "pending".to_string()
}

#[derive(Debug, Serialize)]
pub struct ProposalView {
    pub id: i64,
    pub kind: String,
    pub content: String,
    pub source: Option<String>,
    pub authority: Option<f32>,
    pub novelty: f32,
    pub conflict_with: Option<i64>,
    pub salience: f32,
    pub created_at: i64,
}

/// `GET /proposals?status=pending&limit=` — the human review queue. Each item
/// carries its score components so the decision is evidence-based.
pub async fn list_proposals(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<ProposalListQuery>,
) -> Result<Json<Vec<ProposalView>>, HandlerError> {
    let domain = "global";
    super::authorize(&principal.0, crate::auth::Action::Read, "", domain)?;
    let status = q.status.trim().to_string();
    if !matches!(status.as_str(), "pending" | "approved" | "rejected") {
        return Err(HandlerError::bad_request(
            "invalid_status",
            "status must be pending|approved|rejected",
        ));
    }
    let limit = q.limit.unwrap_or(50).min(MAX_PROPOSALS);
    let pool = super::resolve_domain_pool(&state.registry, Some(domain))?;

    let rows = tokio::task::spawn_blocking(move || -> Result<Vec<ProposalView>, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, content, source, authority, novelty, conflict_with,
                        salience, created_at
                 FROM proposals WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![status, limit as i64], |r| {
                Ok(ProposalView {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    content: r.get(2)?,
                    source: r.get(3)?,
                    authority: r.get(4)?,
                    novelty: r.get(5)?,
                    conflict_with: r.get(6)?,
                    salience: r.get(7)?,
                    created_at: r.get(8)?,
                })
            })
            .map_err(|e| HandlerError::internal(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>();
        Ok(rows)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(rows))
}

/// `POST /proposals/{id}/approve` — promote a candidate into long-term memory.
/// One transaction: creates the chunk (memory_kind, authority, observed_at),
/// marks the proposal approved + decided_at. With `?supersedes=<id>`, calls
/// [`crate::consolidate::resolve_supersession`] in the SAME transaction so
/// approving a conflicting fact atomically supersedes the old one.
pub async fn approve_proposal(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Query(q): Query<ApproveQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let domain = "global";
    super::authorize(&principal.0, crate::auth::Action::Write, "", domain)?;
    let pool = super::resolve_domain_pool(&state.registry, Some(domain))?;
    let model = Arc::clone(&state.model);

    tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;

        // Load the pending proposal.
        #[derive(Default)]
        struct ProposalRow {
            kind: String,
            content: String,
            source: Option<String>,
            authority: Option<f32>,
            observed_at: Option<i64>,
        }
        let p: Option<ProposalRow> = tx
            .query_row(
                "SELECT kind, content, source, authority, observed_at
                 FROM proposals WHERE id = ?1 AND status = 'pending'",
                rusqlite::params![id],
                |r| {
                    Ok(ProposalRow {
                        kind: r.get(0)?,
                        content: r.get(1)?,
                        source: r.get(2)?,
                        authority: r.get(3)?,
                        observed_at: r.get(4)?,
                    })
                },
            )
            .ok();
        let Some(p) = p else {
            return Err(HandlerError::not_found(format!(
                "no pending proposal with id {id}"
            )));
        };
        let (kind, content, source, authority, observed_at) =
            (p.kind, p.content, p.source, p.authority, p.observed_at);

        // Embed + insert the chunk through the same knowledge + vec0 path.
        let embedding = model
            .encode(std::slice::from_ref(&content))
            .into_iter()
            .next()
            .ok_or_else(|| HandlerError::internal("embedding generation failed"))?;
        let content_hash = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(content.as_bytes()));
        let source_kind = source.clone().unwrap_or_else(|| "manual".to_string());
        let assertion = "stated"; // promoted proposals are declarative by default
        let confidence = crate::gate::confidence(
            Some(source_kind.as_str()),
            false,
            assertion,
        );
        let now_utc = chrono::Utc::now().to_rfc3339();

        tx.execute(
            "INSERT INTO knowledge(content, title, source, content_hash, authority,
                                   observed_at, node_kind, assertion_kind, confidence, owner)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                content,
                None::<String>,
                source_kind,
                content_hash,
                authority,
                observed_at.map(|o| o.to_string()),
                kind,
                assertion,
                confidence,
                principal_to_owner(&principal.0)
            ],
        )
        .map_err(|e| HandlerError::internal(format!("insert failed: {e}")))?;
        let chunk_id = tx.last_insert_rowid();

        // v1.13.6: strip reasoning traces at the ingest door (same as /add).
        tx.execute(
            "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
             VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), ?3, datetime('now'))",
            rusqlite::params![chunk_id, embedding.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>(), source_kind],
        )
        .map_err(|e| HandlerError::internal(format!("vec0 insert failed: {e}")))?;

        // Optional supersession in the same tx.
        if let Some(supersedes) = q.supersedes {
            if supersedes == chunk_id {
                return Err(HandlerError::bad_request(
                    "self_supersede",
                    "cannot supersede the chunk being created",
                ));
            }
            crate::consolidate::resolve_supersession(&tx, chunk_id, supersedes, &now_utc)
                .map_err(|e| HandlerError::internal(format!("supersession failed: {e}")))?;
        }

        tx.execute(
            "UPDATE proposals SET status = 'approved', decided_at = ?1 WHERE id = ?2",
            rusqlite::params![chrono::Utc::now().timestamp(), id],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;

        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;

        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("proposal:{id}"),
            crate::audit::AuditStatus::Ok,
            "proposal_approved",
        );

        Ok(serde_json::json!({
            "proposal_id": id,
            "chunk_id": chunk_id,
            "status": "approved",
            "superseded": q.supersedes,
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?
    .map(Json)
}

#[derive(Debug, Default, Deserialize)]
pub struct ApproveQuery {
    #[serde(default)]
    pub supersedes: Option<i64>,
}

/// The owner string recorded on a chunk at ingest: the principal's subject when
/// a JWT principal exists, else NULL (loopback/opaque = unowned, the documented
/// legacy default).
fn principal_to_owner(p: &Option<crate::auth::Principal>) -> Option<String> {
    p.as_ref().map(|pr| pr.sub.clone())
}

/// v1.14.0 "Gate" M4: record-level access-scope filter for retrieval. In JWT
/// mode a principal may only see chunks whose `access_scope` is in their
/// allowed set (deny-by-default). The set is derived from the principal's
/// existing scopes: an `admin` scope sees everything; otherwise the principal
/// sees `private` (own) + `domain` + `team` scopes they hold. `None` (loopback/
/// opaque = no JWT) trusts localhost and sees everything (SECURITY.md posture).
pub fn scope_filter(p: &Option<crate::auth::Principal>) -> Option<Vec<String>> {
    match p {
        None => None, // loopback/opaque: trusts localhost
        Some(pr) => {
            if pr
                .scopes
                .iter()
                .any(|s| s.action == crate::auth::Action::Admin)
            {
                None // admin: unrestricted (standing trusted-reader group)
            } else {
                Some(vec![
                    "private".to_string(),
                    "domain".to_string(),
                    "team".to_string(),
                ])
            }
        }
    }
}

/// `POST /proposals/{id}/reject` — mark rejected + decided_at. Kept in the
/// audit trail (append-only, hash-only via `/audit`); never silently dropped,
/// never deleted.
pub async fn reject_proposal(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;

    let updated = tokio::task::spawn_blocking(move || -> Result<usize, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let n = conn
            .execute(
                "UPDATE proposals SET status = 'rejected', decided_at = ?1
                 WHERE id = ?2 AND status = 'pending'",
                rusqlite::params![chrono::Utc::now().timestamp(), id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        if n > 0 {
            crate::audit::record(
                &conn,
                crate::audit::AuditKind::Reconcile,
                "api",
                &format!("proposal:{id}"),
                crate::audit::AuditStatus::Ok,
                "proposal_rejected",
            );
        }
        Ok(n)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    if updated == 0 {
        return Err(HandlerError::not_found(format!(
            "no pending proposal with id {id}"
        )));
    }
    Ok(Json(
        serde_json::json!({ "proposal_id": id, "status": "rejected" }),
    ))
}

// ── M2: decay + GDPR lifecycle ─────────────────────────────────────────────

/// `GET /decayed` — list decayed chunks (id, content_hash, expires_at) for
/// operator review. `brain sweep --list` wraps it. Nothing is ever deleted
/// autonomously.
pub async fn list_decayed(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<Vec<serde_json::Value>>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;
    let now = chrono::Utc::now().timestamp();

    let rows =
        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, content_hash, expires_at FROM knowledge
                 WHERE expires_at IS NOT NULL AND expires_at < ?1
                 ORDER BY expires_at",
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map(rusqlite::params![now], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "content_hash": r.get::<_, Option<String>>(1)?,
                        "expires_at": r.get::<_, i64>(2)?,
                    }))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            Ok(rows)
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(rows))
}

/// `POST /purge` — audited, hard, explicit-only deletion. Two bodies:
/// `{"ids": [...]}` (specific chunks) or `{"owner": "<principal>"}` (every
/// record owned by that principal). One transaction removes knowledge +
/// vec_knowledge + graph + proposals refs; a `tombstones` row + `/audit` event
/// keep the chain verifiable. No escape hatch: purged = gone from recall,
/// search, graph, AND historical `?at=` recall.
#[derive(Debug, Deserialize)]
pub struct PurgeRequest {
    #[serde(default)]
    pub ids: Vec<i64>,
    #[serde(default)]
    pub owner: Option<String>,
}

pub async fn purge(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<PurgeRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    // Purging is Admin (irreversible). Loopback/opaque = superuser (back-compat).
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    if req.ids.is_empty() && req.owner.is_none() {
        return Err(HandlerError::bad_request(
            "no_target",
            "purge requires ids or owner",
        ));
    }
    if req.ids.len() > MAX_PURGE_IDS {
        return Err(HandlerError::bad_request(
            "too_many_ids",
            format!("purge accepts at most {MAX_PURGE_IDS} ids"),
        ));
    }
    if !req.ids.is_empty() && req.owner.is_some() {
        return Err(HandlerError::bad_request(
            "ambiguous_target",
            "purge accepts ids OR owner, not both",
        ));
    }
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;

    let count = tokio::task::spawn_blocking(move || -> Result<i64, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let now = chrono::Utc::now().timestamp();

        // Resolve target ids: explicit list, or owner-anchored (M4).
        let ids: Vec<i64> = if let Some(owner) = &req.owner {
            let mut stmt = tx
                .prepare("SELECT id FROM knowledge WHERE owner = ?1")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let mut collected = Vec::new();
            {
                let rows = stmt
                    .query_map(rusqlite::params![owner], |r| r.get::<_, i64>(0))
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                for v in rows.flatten() {
                    collected.push(v);
                }
            }
            collected
        } else {
            req.ids.clone()
        };
        if ids.is_empty() {
            return Err(HandlerError::not_found("no matching chunks to purge"));
        }

        for id in &ids {
            // Capture the content_hash for the tombstone before deletion.
            let content_hash: Option<String> = tx
                .query_row(
                    "SELECT content_hash FROM knowledge WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .ok()
                .flatten();
            // Graph nodes/edges + supersession pointers cascade via FKs or are
            // swept explicitly; vec0 rows are deleted by knowledge_id.
            let _ = tx.execute(
                "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
                rusqlite::params![id],
            );
            let _ = tx.execute(
                "DELETE FROM relationships WHERE knowledge_id = ?1 OR from_entity_id IN (SELECT id FROM entities WHERE knowledge_id = ?1) OR to_entity_id IN (SELECT id FROM entities WHERE knowledge_id = ?1)",
                rusqlite::params![id],
            );
            let _ = tx.execute(
                "DELETE FROM evidence_links WHERE from_chunk = ?1 OR to_chunk = ?1",
                rusqlite::params![id],
            );
            let _ = tx.execute(
                "DELETE FROM proposals WHERE conflict_with = ?1",
                rusqlite::params![id],
            );
            let n = tx
                .execute("DELETE FROM knowledge WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            if n > 0 {
                tx.execute(
                    "INSERT INTO tombstones(knowledge_id, content_hash, purged_at) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, content_hash.unwrap_or_else(|| "unknown".into()), now],
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            }
        }
        let purged = ids.len() as i64;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("purge:{purged}"),
            crate::audit::AuditStatus::Ok,
            "purge",
        );
        Ok(purged)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(serde_json::json!({ "purged": count })))
}

/// `GET /export` — portable, machine-readable JSON export (data portability,
/// the GDPR "give me my data"). Live `knowledge` rows + graph + proposals
/// ledger. `pii_map` values excluded by default; only included with
/// `?include_pii_map=true` AND a `pii:read` principal.
#[derive(Debug, Default, Deserialize)]
pub struct ExportQuery {
    #[serde(default)]
    pub include_pii_map: bool,
}

pub async fn export(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<ExportQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, Some("global"))?;

    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let mut knowledge = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, content, node_kind, authority, assertion_kind, confidence,
                            access_scope, owner, observed_at, valid_from, valid_to,
                            content_hash
                     FROM knowledge ORDER BY id",
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "content": r.get::<_, String>(1)?,
                        "memory_kind": r.get::<_, String>(2)?,
                        "authority": r.get::<_, Option<f32>>(3)?,
                        "assertion_kind": r.get::<_, String>(4)?,
                        "confidence": r.get::<_, f32>(5)?,
                        "access_scope": r.get::<_, String>(6)?,
                        "owner": r.get::<_, Option<String>>(7)?,
                        "observed_at": r.get::<_, Option<String>>(8)?,
                        "valid_from": r.get::<_, Option<String>>(9)?,
                        "valid_to": r.get::<_, Option<String>>(10)?,
                        "content_hash": r.get::<_, Option<String>>(11)?,
                    }))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                knowledge.push(v);
            }
        }
        let mut proposals = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, kind, content, novelty, conflict_with, salience, status,
                            created_at, decided_at
                     FROM proposals ORDER BY id",
                )
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "kind": r.get::<_, String>(1)?,
                        "content": r.get::<_, String>(2)?,
                        "novelty": r.get::<_, f32>(3)?,
                        "conflict_with": r.get::<_, Option<i64>>(4)?,
                        "salience": r.get::<_, f32>(5)?,
                        "status": r.get::<_, String>(6)?,
                        "created_at": r.get::<_, i64>(7)?,
                        "decided_at": r.get::<_, Option<i64>>(8)?,
                    }))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                proposals.push(v);
            }
        }
        let mut entities = Vec::new();
        {
            let mut stmt = conn
                .prepare("SELECT id, name, entity_type FROM entities ORDER BY id")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "name": r.get::<_, String>(1)?,
                        "entity_type": r.get::<_, Option<String>>(2)?,
                    }))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                entities.push(v);
            }
        }
        let mut edges = Vec::new();
        {
            let mut stmt = conn
                .prepare("SELECT id, from_entity_id, to_entity_id, relation_type, knowledge_id FROM relationships ORDER BY id")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "from_entity_id": r.get::<_, i64>(1)?,
                        "to_entity_id": r.get::<_, i64>(2)?,
                        "relation_type": r.get::<_, String>(3)?,
                        "knowledge_id": r.get::<_, Option<i64>>(4)?,
                    }))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                edges.push(v);
            }
        }
        let include_pii = q.include_pii_map && crate::gate::has_pii_read(&principal.0);
        // Only resolve the pii_map when both the flag AND the principal allow it.
        let pii_map = if include_pii {
            let mut map = std::collections::BTreeMap::new();
            let mut stmt = conn
                .prepare("SELECT placeholder, value FROM pii_map ORDER BY placeholder")
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            for v in rows.flatten() {
                map.insert(v.0, v.1);
            }
            serde_json::to_value(map).unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Null
        };
        Ok(serde_json::json!({
            "exported_at": chrono::Utc::now().to_rfc3339(),
            "knowledge": knowledge,
            "entities": entities,
            "relationships": edges,
            "proposals": proposals,
            "pii_map": pii_map,
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_to_owner_maps_sub_and_none() {
        assert_eq!(principal_to_owner(&None), None);
        let p = crate::auth::Principal {
            sub: "user-42".to_string(),
            tenant: "alpha".to_string(),
            scopes: vec![],
            jti: "token-1".to_string(),
        };
        assert_eq!(principal_to_owner(&Some(p)), Some("user-42".to_string()));
    }
}
