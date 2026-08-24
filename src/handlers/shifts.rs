//! The shift-ring surfaces (Watchbill).
//!
//! - `GET /ops/shifts?domain=&now=` — the ring view: which site owns the
//!   queue at `now`, whether an overlap window is running, and when the next
//!   boundary lands. Pure read-time arithmetic over stored shift rows.
//! - `POST /ops/shifts` — declare a site's on-call window. Refuses bad
//!   windows and double booking (409) before anything is stored; audited in
//!   the same transaction.

use axum::{
    Json,
    extract::{Query, State},
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::shifts::{self, MAX_OVERLAP_MINUTES, RingView, Shift, ShiftError};

/// Read cap mirrors the storage-layer bound (`shifts::MAX_SHIFTS_RETURNED`);
/// re-declared here only in the doc contract.
fn shift_err(e: ShiftError) -> HandlerError {
    match e {
        ShiftError::BadWindow => HandlerError::bad_request(
            "shift_window_invalid",
            "end_epoch must be after start_epoch",
        ),
        ShiftError::BadOverlap => HandlerError::bad_request(
            "shift_overlap_invalid",
            format!("overlap_minutes must be 0..={MAX_OVERLAP_MINUTES}"),
        ),
        ShiftError::DoubleBooking { with_id } => HandlerError::conflict_with(
            "shift_double_booked",
            "the window overlaps an existing shift beyond its declared overlap budget",
            serde_json::json!([{ "conflicts_with_shift_id": with_id }]),
        ),
        ShiftError::InvalidRoster(m) => HandlerError::bad_request("roster_invalid", m),
        ShiftError::Database(m) => HandlerError::internal(m),
    }
}

fn shift_json(s: &Shift) -> serde_json::Value {
    serde_json::json!({
        "id": s.id,
        "domain": s.domain,
        "site": s.site,
        "tz": s.tz,
        "start_epoch": s.start_epoch,
        "end_epoch": s.end_epoch,
        "overlap_minutes": s.overlap_minutes,
        "roster": s.roster,
    })
}

fn view_json(v: &RingView) -> serde_json::Value {
    serde_json::json!({
        "now": v.now,
        "domain": v.domain,
        "queue_scope_site": v.queue_scope_site,
        "incoming_site": v.incoming_site,
        "in_overlap": v.in_overlap,
        "next_boundary_epoch": v.next_boundary_epoch,
    })
}

/// `GET /ops/shifts?domain=&now=` — the ring view plus every stored shift.
pub async fn get_ops_shifts(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = params
        .get("domain")
        .cloned()
        .unwrap_or_else(|| "global".into());
    let now = match params.get("now").map(|s| s.parse::<i64>()) {
        Some(Ok(t)) => t,
        Some(Err(_)) => {
            return Err(HandlerError::bad_request(
                "now_invalid",
                "now must be epoch seconds",
            ));
        }
        None => chrono::Utc::now().timestamp(),
    };
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let (rows, view) = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let all = shifts::list_shifts(&conn, &domain).map_err(shift_err)?;
        let view = shifts::ring_view(&all, &domain, now);
        Ok((all, view))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    let payload: Vec<serde_json::Value> = rows.iter().map(shift_json).collect();
    let mut out = view_json(&view);
    out["shifts"] = serde_json::Value::Array(payload);
    Ok(Json(out))
}

/// `POST /ops/shifts` — declare a shift window (Admin-conservative Write;
/// audited in the same tx as the insert).
pub async fn post_ops_shift(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = body
        .get("domain")
        .and_then(|v| v.as_str())
        .unwrap_or("global")
        .to_string();
    let site = body
        .get("site")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .ok_or_else(|| {
            HandlerError::bad_request("site_invalid", "site is required (1..=128 chars)")
        })?
        .to_string();
    let tz_raw = body
        .get("tz")
        .and_then(|v| v.as_str())
        .unwrap_or("UTC")
        .trim();
    if tz_raw.is_empty() || tz_raw.len() > 64 {
        return Err(HandlerError::bad_request(
            "tz_invalid",
            "tz must be 1..=64 chars",
        ));
    }
    let tz = tz_raw.to_string();
    // Row-size bounds: an unbounded roster is a storage-amplification lever
    // for any principal holding Write on the domain.
    const MAX_ROSTER: usize = 64;
    const MAX_ROSTER_ID: usize = 256;
    let roster: Vec<String> = match body.get("roster") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(v) => {
            let parsed: Vec<String> = serde_json::from_value(v.clone())
                .map_err(|e| HandlerError::bad_request("roster_invalid", e.to_string()))?;
            if parsed.len() > MAX_ROSTER || parsed.iter().any(|id| id.len() > MAX_ROSTER_ID) {
                return Err(HandlerError::bad_request(
                    "roster_invalid",
                    format!(
                        "roster must hold at most {MAX_ROSTER} ids of at most {MAX_ROSTER_ID} chars"
                    ),
                ));
            }
            parsed
        }
    };
    let start = body
        .get("start_epoch")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| {
            HandlerError::bad_request("shift_window_invalid", "start_epoch is required")
        })?;
    let end = body
        .get("end_epoch")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| {
            HandlerError::bad_request("shift_window_invalid", "end_epoch is required")
        })?;
    let overlap = body
        .get("overlap_minutes")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    // Declaring shifts is pure operator configuration — Admin, not Write
    // (fail-closed bias; the /clients register precedent). An agent-class
    // principal must not be able to re-anchor the follow-the-sun queue.
    super::authorize(&principal, crate::auth::Action::Admin, "", &domain)?;
    let actor = super::recall::principal_label(&principal);
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let domain_resp = domain.clone();
    let site_resp = site.clone();
    let id: Result<i64, HandlerError> = tokio::task::spawn_blocking(move || {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        // Validation + insert + audit ride one tx: a refused shift writes nothing.
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let outcome = (|| {
            let draft = shifts::ShiftDraft {
                domain: &domain,
                site: &site,
                tz: &tz,
                start_epoch: start,
                end_epoch: end,
                overlap_minutes: overlap,
                roster: &roster,
            };
            let id = shifts::insert_shift(&conn, &draft).map_err(shift_err)?;
            crate::audit::record_tenant(
                &conn,
                crate::audit::AuditKind::Workflow,
                actor.trim(),
                &format!("shift:{id}"),
                crate::audit::AuditStatus::Ok,
                "shifts/create",
                &domain,
            );
            Ok(id)
        })();
        match outcome {
            Ok(id) => {
                conn.execute_batch("COMMIT")
                    .map_err(|e| HandlerError::internal(format!("{e}")))?;
                Ok(id)
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let id = id?;
    Ok(Json(
        serde_json::json!({ "id": id, "domain": domain_resp, "site": site_resp }),
    ))
}
