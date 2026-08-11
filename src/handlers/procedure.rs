//! v1.10.0 "Procedural" — handlers for procedural memory + deterministic
//! categorization + decision evaluation. See `src/procedural.rs` for the pure
//! logic. These handlers are thin: validate → DB → render, exactly like
//! `verify.rs` (v1.5) and `suggest.rs` (v1.9).

use axum::extract::{Path, State};
use axum::response::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::handlers::auth::OptPrincipal;
use crate::handlers::{HandlerError, MAX_CONTENT, MAX_TITLE};
use crate::procedural::{self, DecisionOutcome, DecisionRule, MemoryKind};
use crate::AppState;
// zerocopy::IntoBytes provides Vec<f32>::as_bytes() — the same cast the /ingest
// path uses to hand f32 vectors to vec_quantize_int8's blob parameter.
use zerocopy::IntoBytes;

// ─────────────────────────────────────────────────────────────────────────
// POST /procedure — ingest a procedure with ordered steps (one tx)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ProcedureRequest {
    pub title: String,
    /// The procedure root's body (overview / when-to-use). Required.
    pub content: String,
    /// Ordered steps. Step text is stored as its own `step`-kind chunk, linked
    /// to the root via `next_step` edges with an explicit step_index.
    #[serde(default)]
    pub steps: Vec<StepInput>,
    /// Optional domain scoping (defaults to 'global').
    #[serde(default)]
    pub domain: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StepInput {
    pub title: String,
    pub content: String,
    /// Optional — mark this step as carrying a decision rule (JSON in content).
    #[serde(default)]
    pub is_decision: bool,
}

#[derive(Debug, Serialize)]
pub struct ProcedureResponse {
    /// The procedure root chunk id.
    pub id: i64,
    pub status: &'static str,
    pub step_ids: Vec<i64>,
}

/// `POST /procedure` — ingest a procedure + its ordered steps in one tx.
pub async fn create(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<ProcedureRequest>,
) -> Result<Json<ProcedureResponse>, HandlerError> {
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(HandlerError::bad_request(
            "title_empty",
            "title must be non-empty",
        ));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(HandlerError::bad_request(
            "title_too_long",
            format!("title exceeds {MAX_TITLE} chars"),
        ));
    }
    let content = req.content.trim().to_string();
    if content.is_empty() {
        return Err(HandlerError::bad_request(
            "content_empty",
            "content must be non-empty",
        ));
    }
    if content.len() > MAX_CONTENT {
        return Err(HandlerError::payload_too_large(format!(
            "content exceeds {MAX_CONTENT} bytes"
        )));
    }
    if req.steps.len() > 100 {
        return Err(HandlerError::bad_request(
            "too_many_steps",
            "a procedure may have at most 100 steps",
        ));
    }
    // Validate each step before any write (fail fast — never half-ingest).
    let steps: Vec<(String, String, MemoryKind)> = req
        .steps
        .iter()
        .map(|s| {
            let t = s.title.trim().to_string();
            let c = s.content.trim().to_string();
            if t.is_empty() {
                return Err(HandlerError::bad_request(
                    "step_title_empty",
                    "every step must have a non-empty title",
                ));
            }
            if c.is_empty() {
                return Err(HandlerError::bad_request(
                    "step_content_empty",
                    "every step must have non-empty content",
                ));
            }
            let kind = if s.is_decision {
                MemoryKind::Decision
            } else {
                MemoryKind::Step
            };
            Ok((t, c, kind))
        })
        .collect::<Result<_, _>>()?;
    // v1.20.1 "Shield" M1 (extended to /procedure in v1.20.2): screen this
    // sibling write core exactly like `ingest_one`, `/add`, `/ingest/memory`,
    // `/ingest/markdown`. v1.20.3 (G5): the full two-layer screen (blocklist +
    // optional classifier). `Reject` → 400; `Quarantine` (default) → flag each
    // inserted chunk + skip the `next_step` edges so a quarantined plant can't
    // pollute the graph. The screen runs against the root content + title AND
    // every step's content + title (a step can carry the payload just as easily
    // as the root).
    use crate::screen::ScreenResult;
    let root_verdict = crate::screen::screen(&content, &title);
    let step_verdicts: Vec<ScreenResult> = steps
        .iter()
        .map(|(t, c, _)| crate::screen::screen(c, t))
        .collect();
    if root_verdict == ScreenResult::Reject || step_verdicts.contains(&ScreenResult::Reject) {
        return Err(HandlerError::bad_request(
            "input_rejected",
            "input contains suspicious patterns",
        ));
    }
    let root_quarantine = root_verdict == ScreenResult::Quarantine;
    let step_quarantine: Vec<bool> = step_verdicts
        .iter()
        .map(|v| *v == ScreenResult::Quarantine)
        .collect();
    // Under Quarantine (default), `flag_if_quarantined` runs per-chunk inside
    // the tx below and skips `next_step` edges for a quarantined root.

    let domain = match &req.domain {
        Some(d) => Some(crate::handlers::normalize_domain(d)?),
        None => None,
    };
    // v1.2.0 M3 AuthZ: write gate scoped to the actual target domain.
    super::authorize(
        &principal.0,
        crate::auth::Action::Write,
        "",
        domain.as_deref().unwrap_or("global"),
    )?;
    let domain_for_embed = domain.clone();

    let pool = crate::handlers::resolve_domain_pool(&state.registry, domain.as_deref())
        .unwrap_or(state.pool.clone());
    let title_for_task = title.clone();
    let content_for_task = content.clone();

    let (root_id, step_ids) = tokio::task::spawn_blocking(move || -> Result<(i64, Vec<i64>), HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("tx begin failed: {e}")))?;
        // Root chunk: memory_kind = 'procedure'.
        let content_hash = crate::audit::hash(&format!("{title_for_task}|{content_for_task}"));
        tx.execute(
            "INSERT INTO knowledge (title, content, content_hash, source, domain, node_kind, origin)
             VALUES (?1, ?2, ?3, 'manual', ?4, 'procedure', 'human')",
            rusqlite::params![title_for_task, content_for_task, content_hash, domain.as_deref().unwrap_or("global")],
        )
        .map_err(|e| HandlerError::internal(format!("procedure insert failed: {e}")))?;
        let root_id = tx.last_insert_rowid();
        // v1.20.2 B1: flag the root if the screen quarantined. Excluded from
        // recall via `WHERE flagged = 0`, KG edges skipped below.
        let root_flagged = crate::flag_if_quarantined(&tx, root_id, root_quarantine);
        // Embedding for the root (so /recall finds it). Reuses the same vec0
        // path as /ingest. ponytail: the model is in AppState but spawn_blocking
        // closes over pool, not state; we encode after the tx via a second
        // connection to avoid holding a write tx across the model call.
        let mut step_ids: Vec<i64> = Vec::new();
        for (idx, (step_title, step_content, step_kind)) in steps.iter().enumerate() {
            let hash = crate::audit::hash(&format!("{root_id}|{idx}|{step_title}|{step_content}"));
            let kind_str = step_kind.as_str();
            tx.execute(
                "INSERT INTO knowledge (title, content, content_hash, source, domain, node_kind, parent_id, origin)
                 VALUES (?1, ?2, ?3, 'manual', ?4, ?5, ?6, 'human')",
                rusqlite::params![
                    step_title,
                    step_content,
                    hash,
                    domain.as_deref().unwrap_or("global"),
                    kind_str,
                    root_id
                ],
            )
            .map_err(|e| HandlerError::internal(format!("step insert failed: {e}")))?;
            let step_id = tx.last_insert_rowid();
            // v1.20.2 B1: flag this step if its screen quarantined. The verdict
            // was computed per-step before the tx, so only the steps that
            // actually quarantined are flagged (a benign step in a quarantined
            // procedure stays clean).
            let _ = crate::flag_if_quarantined(&tx, step_id, step_quarantine[idx]);
            // next_step edge with explicit ordering. Skipped for a quarantined
            // root so a flagged plant can't reach the graph even via a step.
            // The edge kind is 'next_step'; step_index carries the position.
            // UNIQUE(from_chunk, to_chunk, kind) means a re-ingest of the same
            // pair is idempotent.
            if !root_flagged {
                tx.execute(
                    "INSERT INTO evidence_links (from_chunk, to_chunk, kind, step_index)
                     VALUES (?1, ?2, 'next_step', ?3)
                     ON CONFLICT(from_chunk, to_chunk, kind) DO UPDATE SET step_index = ?3",
                    rusqlite::params![root_id, step_id, idx as i64],
                )
                .map_err(|e| HandlerError::internal(format!("next_step edge failed: {e}")))?;
            }
            step_ids.push(step_id);
        }
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("tx commit failed: {e}")))?;
        Ok((root_id, step_ids))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    // Embedding pass (outside the write tx). Best-effort: a failure here must
    // not undo the successful ingest — the chunks are queryable via FTS5 even
    // without a vector. Logged, not fatal.
    let _ = embed_procedure_chunks(
        &state,
        root_id,
        &step_ids,
        &content,
        domain_for_embed.as_deref(),
    )
    .await;

    Ok(Json(ProcedureResponse {
        id: root_id,
        status: "created",
        step_ids,
    }))
}

/// Encode the root + each step, inserting into vec_knowledge. Best-effort —
/// the FTS5 shadow row (created by the knowledge insert trigger) makes the
/// chunks retrievable even if this fails. Mirrors the /ingest path's tolerance.
async fn embed_procedure_chunks(
    state: &Arc<AppState>,
    root_id: i64,
    step_ids: &[i64],
    root_content: &str,
    _domain: Option<&str>,
) -> Result<(), rusqlite::Error> {
    // ponytail: encode one-at-a-time reusing the exact /add path (model.encode
    // → .into_iter().next() → .as_bytes()). Batching would be faster but would
    // need to re-derive the f32→bytes cast the existing code gets for free from
    // the single-encode shape; not worth the divergence for a best-effort path.
    let pool = state.pool.clone();
    let model = Arc::clone(&state.model);
    let root_content = root_content.to_string();
    let step_ids = step_ids.to_vec();
    tokio::task::spawn_blocking(move || -> Result<(), rusqlite::Error> {
        let conn = pool.get().map_err(|e| {
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(e.to_string()))
        })?;
        // Collect (id, text) pairs to embed.
        let mut targets: Vec<(i64, String)> = vec![(root_id, root_content)];
        for sid in &step_ids {
            if let Ok(c) = conn.query_row(
                "SELECT content FROM knowledge WHERE id = ?1",
                rusqlite::params![sid],
                |r| r.get::<_, String>(0),
            ) {
                targets.push((*sid, c));
            }
        }
        for (chunk_id, text) in &targets {
            // Same shape as add_chunk: encode one sentence, take the first vec.
            let Some(emb) = model
                .encode(std::slice::from_ref(text))
                .into_iter()
                .next()
            else {
                continue;
            };
            let _ = conn.execute(
                "INSERT OR REPLACE INTO vec_knowledge (knowledge_id, embedding_int8, embedding_bit, source, created_at)
                 VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'manual', datetime('now'))",
                rusqlite::params![chunk_id, emb.as_bytes()],
            );
        }
        Ok(())
    })
    .await
    .map_err(|e| {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(e.to_string()))
    })??;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// GET /procedure/{id}/steps — ordered step list
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct StepView {
    pub step_index: i64,
    pub id: i64,
    pub title: Option<String>,
    pub content: String,
    pub memory_kind: String,
}

#[derive(Debug, Serialize)]
pub struct ProcedureStepsResponse {
    pub procedure_id: i64,
    pub title: Option<String>,
    pub content: Option<String>,
    pub steps: Vec<StepView>,
}

/// `GET /procedure/{id}/steps` — the ordered step chain for a procedure.
pub async fn steps(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<ProcedureStepsResponse>, HandlerError> {
    // v1.12.1 "Harden": AuthZ read gate. `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let pool = state.pool.clone();
    let procedure_id = id;
    let view =
        tokio::task::spawn_blocking(move || -> Result<ProcedureStepsResponse, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            // Root must exist + be a procedure.
            let root: Option<(Option<String>, String)> = conn
            .query_row(
                "SELECT title, content FROM knowledge WHERE id = ?1 AND node_kind = 'procedure'",
                rusqlite::params![procedure_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
            let Some((title, content)) = root else {
                return Err(HandlerError::not_found(format!(
                    "no procedure with id {procedure_id}"
                )));
            };
            // Ordered steps via the next_step edges.
            let mut stmt = conn
                .prepare(
                    "SELECT k.id, k.title, k.content, k.node_kind, el.step_index
                 FROM evidence_links el
                 JOIN knowledge k ON k.id = el.to_chunk
                 WHERE el.from_chunk = ?1 AND el.kind = 'next_step'
                 ORDER BY el.step_index ASC",
                )
                .map_err(|e| HandlerError::internal(format!("prepare failed: {e}")))?;
            let rows = stmt
                .query_map(rusqlite::params![procedure_id], |r| {
                    Ok(StepView {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        content: r.get(2)?,
                        memory_kind: MemoryKind::from_str(&r.get::<_, String>(3)?)
                            .as_str()
                            .to_string(),
                        step_index: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    })
                })
                .map_err(|e| HandlerError::internal(format!("query failed: {e}")))?;
            let steps: Vec<StepView> = rows.filter_map(|r| r.ok()).collect();
            Ok(ProcedureStepsResponse {
                procedure_id,
                title,
                content: Some(content),
                steps,
            })
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(view))
}

// ─────────────────────────────────────────────────────────────────────────
// POST /classify — deterministic categorization (Mem0's premium, free)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ClassifyRequest {
    pub text: String,
}

/// `POST /classify` — categorize a text deterministically. No LLM, no cloud.
/// Returns the category + confidence + the matched keywords (auditable) + the
/// full taxonomy so a client knows the universe of labels.
pub async fn classify(
    State(_state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<ClassifyRequest>,
) -> Result<Json<ClassifyResponse>, HandlerError> {
    // v1.2.0 M3 AuthZ: read gate. Stateless pure function, but uniform gating
    // keeps the surface predictable. `None` (no JWT) = superuser.
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return Err(HandlerError::bad_request(
            "text_empty",
            "text must be non-empty",
        ));
    }
    if text.len() > MAX_CONTENT {
        return Err(HandlerError::payload_too_large(format!(
            "text exceeds {MAX_CONTENT} bytes"
        )));
    }
    Ok(Json(ClassifyResponse {
        result: procedural::classify(&text),
        categories: procedural::categories(),
    }))
}

#[derive(Debug, Serialize)]
pub struct ClassifyResponse {
    pub result: procedural::CategoryResult,
    pub categories: &'static [&'static str],
}

// ─────────────────────────────────────────────────────────────────────────
// POST /decision/{id}/evaluate — deterministic rule evaluation
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct EvaluateRequest {
    /// Numeric input variables, e.g. `{"employee_count": 25}`.
    #[serde(default)]
    pub variables: std::collections::HashMap<String, f64>,
}

/// `POST /decision/{id}/evaluate` — evaluate the decision rule stored on the
/// `decision`-kind chunk `id` against the supplied variables. Pure once the
/// rule is loaded; the handler just loads + delegates.
pub async fn evaluate(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(req): Json<EvaluateRequest>,
) -> Result<Json<DecisionOutcome>, HandlerError> {
    // v1.2.0 M3 AuthZ: read gate (loads a chunk + runs the stored rule).
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let pool = state.pool.clone();
    let outcome = tokio::task::spawn_blocking(move || -> Result<DecisionOutcome, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let content: String = conn
            .query_row(
                "SELECT content FROM knowledge WHERE id = ?1 AND node_kind = 'decision'",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    HandlerError::not_found(format!("no decision rule with id {id}"))
                }
                other => HandlerError::internal(format!("query failed: {other}")),
            })?;
        let rule: DecisionRule = serde_json::from_str(&content).map_err(|e| {
            HandlerError::bad_request(
                "decision_rule_malformed",
                format!("stored content is not valid decision JSON: {e}"),
            )
        })?;
        Ok(procedural::evaluate_decision(&rule, &req.variables))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(outcome))
}
