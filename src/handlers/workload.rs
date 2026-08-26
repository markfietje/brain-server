//! Workload visibility surfaces — people made visible.
//!
//! - `GET /ops/workload?domain=&now=` — per-principal burden computed from
//!   lineage only, plus fatigue signals for the scheduling human. Pure
//!   read: nothing here ever reassigns work.
//! - `GET /ops/coverage?domain=` — competence coverage: skills tags vs
//!   worktype demand queue.

use axum::{
    Json,
    extract::{Query, State},
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::shifts;
use crate::workflow::workload::{self, workload_view};

/// `GET /ops/workload?domain=&now=` — the burden + fatigue view (Read on the
/// domain). Read-only by construction: the domain functions SELECT; this
/// handler never writes and never audits a read.
pub async fn get_ops_workload(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = params
        .get("domain")
        .cloned()
        .unwrap_or_else(|| "global".into());
    let now = super::crew::now_param(&params)?;
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let out = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let rows = workload_view(&conn, &domain).map_err(HandlerError::internal)?;
        let all_shifts = shifts::list_shifts(&conn, &domain)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let signals = workload::fatigue_signals(&all_shifts, &rows);
        Ok(serde_json::json!({
            "schema_version": crate::workflow::wfm::WFM_SCHEMA_VERSION,
            "now": now,
            "domain": domain,
            "workload": rows.iter().map(|w| serde_json::json!({
                "principal": w.principal,
                "open_envelopes": w.open_envelopes,
                "handover_burden_outbound": w.handover_burden_outbound,
                "transfers_in_open": w.transfers_in_open,
                "reask_load": w.reask_load,
                "gate_backlog": w.gate_backlog,
            })).collect::<Vec<_>>(),
            "fatigue_signals": signals.iter().map(|s| serde_json::json!({
                "principal": s.principal,
                "consecutive_shifts": s.consecutive_shifts,
                "open_envelopes": s.open_envelopes,
                "reason": s.reason,
            })).collect::<Vec<_>>(),
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    Ok(Json(out?))
}

/// `GET /ops/coverage?domain=` — competence vs demand (Read on the domain).
pub async fn get_ops_coverage(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = params
        .get("domain")
        .cloned()
        .unwrap_or_else(|| "global".into());
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let out = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let rows = workload::coverage_view(&conn, &domain).map_err(HandlerError::internal)?;
        Ok(serde_json::json!({
            "schema_version": crate::workflow::wfm::WFM_SCHEMA_VERSION,
            "domain": domain,
            "coverage": rows.iter().map(|c| serde_json::json!({
                "worktype": c.worktype,
                "required_tags": c.required_tags,
                "qualified_principals": c.qualified_principals,
                "open_demand": c.open_demand,
                "covered": c.covered,
            })).collect::<Vec<_>>(),
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    Ok(Json(out?))
}
