//! The StewardOS account surfaces: the account record, the per-account
//! request history, and the decision_ref-gated pipeline — the
//! deliberately-not-a-CRM (identifiers and typed records only, never a
//! contacts/quotes/forecast system).
//!
//! Every handler clones the `workflow_decisions` posture line for line:
//! the account's domain resolves first (an absent id AND a non-account id
//! answer the SAME probe-blind 404 — the run-existence oracle stays shut),
//! then the Write gate on that domain plus the `workflow` role gate, then
//! ONE `WorkflowTx` in `spawn_blocking` carries the write and its audit
//! row. The listing is the round's exfiltration surface and carries the DPO
//! dual gate (Admin scope AND the DPO role, the scoreboard/corpus
//! precedent) plus an audited read per call. Zero statements of storage
//! live here — every read is a core fn in `crate::workflow::accounts` /
//! `crate::workflow::pipeline` (the no-SQL-in-handlers law counts this
//! file live).

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

/// The decision reference is the audit-recovery handle for the operator's
/// call — screened, bounded, no free-text pass-through. The WIRE vocabulary
/// is the shipped `decision_ref_required` / `decision_ref_invalid` pair and
/// the same 1..=256 bound and screening as the decision surface; the fn
/// carries the account surface's own name (one definition per name — the
/// dup_guard's law).
const MAX_ACCOUNT_REF_LEN: usize = 256;

fn validate_account_decision_ref(raw: &str) -> Result<String, HandlerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HandlerError::bad_request(
            "decision_ref_required",
            "an operator decision reference is required — the machine never \
             advances a pipeline stage or archives an account on its own \
             authority",
        ));
    }
    if trimmed.len() > MAX_ACCOUNT_REF_LEN
        || trimmed
            .chars()
            .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
    {
        return Err(HandlerError::bad_request(
            "decision_ref_invalid",
            format!(
                "decision_ref must be 1..={MAX_ACCOUNT_REF_LEN} chars with no \
                 control or invisible characters"
            ),
        ));
    }
    Ok(trimmed.to_string())
}

/// The bounded page: `limit` must land inside 1..=500 (named 400 outside),
/// default 100 — the corpus-page posture.
fn validate_account_limit(limit: Option<usize>) -> Result<usize, HandlerError> {
    let limit = limit.unwrap_or(100);
    if (1..=500).contains(&limit) {
        return Ok(limit);
    }
    Err(HandlerError::internal_with(
        "account_limit_out_of_bounds",
        "limit must land inside 1..=500 — the accounts page is bounded",
        axum::http::StatusCode::BAD_REQUEST,
    ))
}

/// The machinery's refusals surface named — the pinned error vocabulary at
/// the wire:
/// absent ids answer 404 probe-blind, the closed-vocabulary and archive
/// refusals answer 400 with their details, the core's own decision_ref
/// defense answers `decision_ref_required`, and anything else is internal
/// (never a silent drop).
fn account_err(e: String) -> HandlerError {
    if let Some(rest) = e.strip_prefix("illegal_stage_transition: ") {
        let (from, to) = rest.split_once('→').map_or_else(
            || (String::new(), String::new()),
            |(f, t)| (f.to_string(), t.to_string()),
        );
        return HandlerError::bad_request_with(
            "illegal_stage_transition",
            "the transition is not an allowed edge of the closed stage \
             vocabulary (self-transitions refuse too)",
            serde_json::json!({ "from": from, "to": to }),
        );
    }
    if e.starts_with("pipeline: decision_ref required") {
        return HandlerError::bad_request(
            "decision_ref_required",
            "a stage change requires an operator decision reference — the \
             machine never advances a pipeline stage on its own authority",
        );
    }
    match e.as_str() {
        "account_not_found" | "link_account_absent" => HandlerError::not_found("account not found"),
        "link_run_absent" => HandlerError::not_found("request run not found"),
        "account_archived" => HandlerError::bad_request(
            "account_archived",
            "the account is archived — new links and stage changes refuse",
        ),
        "pipeline_stage_unknown" => HandlerError::bad_request(
            "pipeline_stage_unknown",
            "stage must be one of lead | qualified | proposal | closed_won | \
             closed_lost",
        ),
        other => HandlerError::internal(other.to_string()),
    }
}

/// Resolve the account's domain, or 404: an absent id and a non-account id
/// are the same answer (the probe never learns whether the id exists as a
/// different kind of run).
async fn account_domain(state: &Arc<AppState>, id: i64) -> Result<String, HandlerError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        crate::workflow::accounts::account_domain_of(&conn, id).map_err(|e| format!("{e}"))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?
    .ok_or_else(|| HandlerError::not_found("account not found"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAccountBody {
    pub name: String,
    pub domain: String,
}

/// `POST /accounts` — create the account record. The client supplies the
/// name and the domain; id, owner label, status, and clock are
/// server-derived. Agents cannot mint account rows (the role gate; the
/// matrix pins the class denial).
pub async fn post_account(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<CreateAccountBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Write, "", &body.domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let actor = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let domain = body.domain.clone();
    let created = tokio::task::spawn_blocking(
        move || -> Result<crate::workflow::accounts::AccountCreated, HandlerError> {
            let mut conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let created = crate::workflow::accounts::create_account(
                &mut tx, &domain, &body.name, &actor, now,
            )
            .map_err(account_err)?;
            tx.commit()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            Ok(created)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    // The name is caller-supplied identity text: echoed sanitized (the
    // relay to_principal precedent), stored screened by the core.
    let echo = crate::gate::sanitize_read(&created.record.name, false, &principal);
    Ok(Json(serde_json::json!({
        "account_id": created.run_id,
        "name": echo,
        "domain": body.domain,
        "audited": true,
    })))
}

/// `GET /accounts/{id}` — the record + its derived stage + the pipeline
/// timeline. Per-account read = Write on the account's domain plus the
/// `workflow` role (the decision-surface posture).
pub async fn get_account(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = account_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let view = tokio::task::spawn_blocking(
        move || -> Result<
            (
                crate::workflow::accounts::AccountRecord,
                crate::workflow::pipeline::Stage,
                Vec<serde_json::Value>,
            ),
            HandlerError,
        > {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            let record = crate::workflow::accounts::load_account(&conn, id)
                .map_err(|e| HandlerError::internal(e.to_string()))?
                .ok_or_else(|| HandlerError::not_found("account not found"))?;
            let stage = crate::workflow::pipeline::current_stage(&conn, id)
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let timeline = crate::workflow::pipeline::pipeline_timeline(&conn, id)
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            Ok((record, stage, timeline))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    let (record, stage, timeline) = view;
    Ok(Json(serde_json::json!({
        "account": {
            "account_id": record.account_id,
            "name": record.name,
            "owner_principal": record.owner_principal,
            "status": record.status,
            "created_at": record.created_at,
            "updated_at": record.updated_at,
        },
        "stage": stage.as_str(),
        "timeline": timeline,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineBody {
    pub stage: String,
    #[serde(default)]
    pub decision_ref: Option<String>,
}

/// `POST /accounts/{id}/pipeline` — advance the stage. The machine-refusal
/// law holds at the surface AND in the core: no decision reference, no
/// movement (`400 decision_ref_required` here, the core refuses
/// independently).
pub async fn post_pipeline(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<PipelineBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = account_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let raw_ref = body.decision_ref.as_deref().unwrap_or("");
    let decision_ref = validate_account_decision_ref(raw_ref)?;
    let echo = crate::gate::sanitize_read(&decision_ref, false, &principal);
    let now = chrono::Utc::now().timestamp();
    let receipt = tokio::task::spawn_blocking(
        move || -> Result<crate::workflow::pipeline::PipelineReceipt, HandlerError> {
            let mut conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let receipt = crate::workflow::pipeline::advance_pipeline(
                &mut tx,
                id,
                &body.stage,
                &decision_ref,
                now,
            )
            .map_err(account_err)?;
            tx.commit()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            Ok(receipt)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    Ok(Json(serde_json::json!({
        "account_id": id,
        "stage": receipt.to.as_str(),
        "prev_stage": receipt.from.as_str(),
        "decision_ref": echo,
        "audited": true,
    })))
}

/// `POST /accounts/{id}/requests/{run_id}/link` — attach one request run to
/// the account. The link + its audit commit atomically; re-links append new
/// audited rows (never a mutation).
pub async fn post_link(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path((id, run_id)): Path<(i64, i64)>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = account_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let now = chrono::Utc::now().timestamp();
    let outcome = tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        crate::workflow::accounts::link_request_to_account(&mut tx, id, run_id, now)
            .map_err(account_err)?;
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(())
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    outcome?;
    Ok(Json(serde_json::json!({
        "account_id": id,
        "run_id": run_id,
        "status": "linked",
        "audited": true,
    })))
}

#[derive(Debug, Deserialize)]
pub struct AccountRequestsQuery {
    pub limit: Option<usize>,
}

/// `GET /accounts/{id}/requests` — the bounded per-account history: the
/// link rows joined to their request runs' headlines and recorded decision
/// rows. The pure decision join over existing rows; no new table shape.
pub async fn get_account_requests(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Query(q): Query<AccountRequestsQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = account_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let limit = validate_account_limit(q.limit)?;
    let entries = tokio::task::spawn_blocking(
        move || -> Result<Vec<crate::workflow::accounts::AccountRequestEntry>, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            crate::workflow::accounts::load_account(&conn, id)
                .map_err(|e| HandlerError::internal(e.to_string()))?
                .ok_or_else(|| HandlerError::not_found("account not found"))?;
            crate::workflow::accounts::account_requests(&conn, id, limit)
                .map_err(|e| HandlerError::internal(e.to_string()))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    let rows: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "run_id": e.run_id,
                "linked_at": e.linked_at,
                "link_seq": e.link_seq,
                "run": {
                    "kind": e.run_kind,
                    "status": e.run_status,
                    "updated_at": e.run_updated_at,
                },
                "decisions": e.decisions.iter().map(|d| serde_json::json!({
                    "seq": d.seq,
                    "payload": d.payload,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let count = rows.len();
    Ok(Json(serde_json::json!({ "rows": rows, "count": count })))
}

#[derive(Debug, Deserialize)]
pub struct AccountsQuery {
    pub limit: Option<usize>,
}

/// `GET /accounts` — the bounded account listing. THE exfiltration surface:
/// the DPO dual gate (Admin scope AND the DPO role) plus an audited read —
/// every call lands a global audit row naming the principal, the filter,
/// and the row count.
pub async fn get_accounts(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<AccountsQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    let limit = validate_account_limit(q.limit)?;
    let who = principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".into());
    let rows = tokio::task::spawn_blocking(
        move || -> Result<Vec<crate::workflow::accounts::ListedAccount>, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            let rows = crate::workflow::accounts::account_listing(&conn, limit)
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            // EVERY listing lands an audit row: who, the filter, the count.
            // The read and its audit share one connection so the count
            // describes exactly the emitted page.
            crate::audit::record_tenant(
                &conn,
                crate::audit::AuditKind::Workflow,
                crate::workflow::ACTOR,
                "accounts_listing",
                crate::audit::AuditStatus::Ok,
                &format!("principal={who} limit={limit} rows={}", rows.len()),
                "global",
            );
            Ok(rows)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    let rows: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|a| {
            serde_json::json!({
                "account_id": a.account_id,
                "name": a.name,
                "status": a.status,
                "domain": a.domain,
                "created_at": a.created_at,
                "updated_at": a.updated_at,
            })
        })
        .collect();
    let count = rows.len();
    Ok(Json(serde_json::json!({ "rows": rows, "count": count })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// decision_ref_bounds_and_screening — the shipped vocabulary verbatim.
    #[test]
    fn account_decision_ref_bounds_and_screening() {
        assert_eq!(
            validate_account_decision_ref("  op-2026-09-22#7 ").unwrap(),
            "op-2026-09-22#7"
        );
        let err = validate_account_decision_ref("").expect_err("empty refuses");
        assert_eq!(err.inner.code, "decision_ref_required");
        let err = validate_account_decision_ref(&"x".repeat(MAX_ACCOUNT_REF_LEN + 1))
            .expect_err("over bound");
        assert_eq!(err.inner.code, "decision_ref_invalid");
        let err = validate_account_decision_ref("op\u{200B}-7").expect_err("invisible refuses");
        assert_eq!(err.inner.code, "decision_ref_invalid");
    }

    /// The bounds and the refusal mapping — the pinned error vocabulary at the wire.
    #[test]
    fn account_limit_bounds_and_error_mapping_are_pinned() {
        assert!(validate_account_limit(None).is_ok());
        assert_eq!(validate_account_limit(None).unwrap(), 100);
        assert!(validate_account_limit(Some(1)).is_ok());
        assert!(validate_account_limit(Some(500)).is_ok());
        assert_eq!(
            validate_account_limit(Some(0)).unwrap_err().inner.code,
            "account_limit_out_of_bounds"
        );
        assert_eq!(
            validate_account_limit(Some(501)).unwrap_err().status,
            axum::http::StatusCode::BAD_REQUEST
        );
        // The named refusals surface with their statuses.
        let illegal = account_err("illegal_stage_transition: lead→closed_won".into());
        assert_eq!(illegal.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(illegal.inner.code, "illegal_stage_transition");
        assert_eq!(
            illegal.inner.details,
            Some(serde_json::json!({"from": "lead", "to": "closed_won"}))
        );
        for (core, want_code, want_status) in [
            (
                "account_not_found",
                "not_found",
                axum::http::StatusCode::NOT_FOUND,
            ),
            (
                "link_account_absent",
                "not_found",
                axum::http::StatusCode::NOT_FOUND,
            ),
            (
                "link_run_absent",
                "not_found",
                axum::http::StatusCode::NOT_FOUND,
            ),
            (
                "account_archived",
                "account_archived",
                axum::http::StatusCode::BAD_REQUEST,
            ),
            (
                "pipeline_stage_unknown",
                "pipeline_stage_unknown",
                axum::http::StatusCode::BAD_REQUEST,
            ),
        ] {
            let mapped = account_err(core.to_string());
            assert_eq!(mapped.status, want_status, "{core}");
            assert_eq!(mapped.inner.code, want_code, "{core}");
        }
        let defense = account_err(
            "pipeline: decision_ref required — the machine never advances a \
             stage on its own authority"
                .into(),
        );
        assert_eq!(defense.inner.code, "decision_ref_required");
        assert_eq!(defense.status, axum::http::StatusCode::BAD_REQUEST);
        // Anything else stays internal — never a silent drop.
        assert_eq!(
            account_err("account: something else".into()).status,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
