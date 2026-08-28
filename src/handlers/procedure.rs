//! handlers for procedural memory + deterministic
//! categorization + decision evaluation. See `src/procedural.rs` for the pure
//! logic. These handlers are thin: validate → DB → render, exactly like
//! `verify.rs` (v1.5) and `suggest.rs` (v1.9).

use axum::extract::{Path, State};
use axum::response::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;
use crate::handlers::auth::OptPrincipal;
use crate::handlers::{HandlerError, MAX_CONTENT, MAX_TITLE};
use crate::procedural::{self, DecisionOutcome, DecisionRule, MemoryKind};
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
    // Screen this
    // sibling write core exactly like `ingest_one`, `/add`, `/ingest/memory`,
    // `/ingest/markdown` — the full two-layer screen (blocklist +
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
    // write gate scoped to the actual target domain.
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

    let (root_id, step_ids) =
        tokio::task::spawn_blocking(move || -> Result<(i64, Vec<i64>), HandlerError> {
            let mut conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let tx = conn
                .transaction()
                .map_err(|e| HandlerError::internal(format!("tx begin failed: {e}")))?;
            let (root_id, step_ids) = crate::service::procedure::store_procedure(
                &tx,
                &title_for_task,
                &content_for_task,
                domain.as_deref(),
                &steps,
                root_quarantine,
                &step_quarantine,
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
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

/// Encode the root + each step, writing into vec_knowledge. Best-effort —
/// the FTS5 shadow row (the knowledge store trigger creates it) makes the
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
            if let Ok(c) = crate::service::procedure::chunk_content(&conn, *sid) {
                targets.push((*sid, c));
            }
        }
        for (chunk_id, text) in &targets {
            // Same shape as add_chunk: encode one sentence; empty ⇒ skip.
            let emb = model.encode_one(text);
            if emb.is_empty() {
                continue;
            }
            let _ = crate::service::procedure::store_embedding(&conn, *chunk_id, emb.as_bytes());
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
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<ProcedureStepsResponse>, HandlerError> {
    // AuthZ read gate, scoped to the header domain. `None` (no JWT) = superuser.
    // The label binds into the SQL below (same /get/{id}
    // idiom) so an id cannot cross domains in shim mode — previously this was
    // a global-read gate + bare-id read leaking any procedure's full chain.
    let domain = crate::handlers::domain_from_headers(&headers);
    super::authorize(
        &principal.0,
        crate::auth::Action::Read,
        "",
        domain.as_deref().unwrap_or("global"),
    )?;
    let label = domain
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("global")
        .to_string();
    let record_gate = crate::handlers::gate::record_read_gate(&principal.0, &state.pool);
    let gate_principal = principal.0.clone();
    let pool = state.pool.clone();
    let procedure_id = id;
    let view =
        tokio::task::spawn_blocking(move || -> Result<ProcedureStepsResponse, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            // Root must exist + be a procedure.
            let root: Option<(Option<String>, String)> =
                crate::service::procedure::procedure_root(&conn, procedure_id, &label)
                    .ok()
                    .flatten();
            let Some((title, content)) = root else {
                return Err(HandlerError::not_found(format!(
                    "no procedure with id {procedure_id}"
                )));
            };
            // belt-and-braces (the /get idiom): re-authorize on the row's own
            // domain + the record gate before any content leaves.
            let row_meta: Option<(String, Option<String>, Option<String>)> =
                crate::service::procedure::row_access_meta(&conn, procedure_id)
                    .ok()
                    .flatten();
            if let Some((row_domain, row_owner, row_scope)) = row_meta
                && (!crate::handlers::can_read_domain(&gate_principal, &row_domain)
                    || !record_gate.admits(&row_owner, &row_scope))
            {
                return Err(HandlerError::not_found(format!(
                    "no procedure with id {procedure_id}"
                )));
            }
            // Ordered steps via the next_step edges (same domain label —
            // steps live with their procedure).
            let rows = crate::service::procedure::step_chain(&conn, procedure_id, &label)
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let steps: Vec<StepView> = rows
                .into_iter()
                .map(|(id, title, content, node_kind, step_index)| StepView {
                    id,
                    title,
                    content,
                    memory_kind: MemoryKind::from_str(&node_kind).as_str().to_string(),
                    step_index: step_index.unwrap_or(0),
                })
                .collect();
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
    // read gate. Stateless pure function, but uniform gating
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
    // read gate (loads a chunk + runs the stored rule).
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let pool = state.pool.clone();
    let outcome = tokio::task::spawn_blocking(move || -> Result<DecisionOutcome, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let content: String =
            crate::service::procedure::decision_rule_content(&conn, id).map_err(|e| match e {
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
