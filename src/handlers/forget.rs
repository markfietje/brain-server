//! `DELETE /memory/{id}` — forget a knowledge entry. Per `API_CONTRACT.md`
//! §4. Cascades to the embedding + owned relations (FK CASCADE / SET NULL).

use axum::extract::{Path, Query, State};
use axum::response::Json;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

/// `DELETE /memory/{id}?scrub_proposals=1`.
#[derive(Debug, serde::Deserialize)]
pub struct ForgetQuery {
    #[serde(default)]
    pub scrub_proposals: bool,
}

/// The blocking closure's outcome: (row deleted, retained proposal copies
/// as (id, status), disclosure truncated at the cap, proposals actually
/// scrubbed). Named so the closure signature stays under the
/// `type_complexity` lint.
type ForgetOutcome = (bool, Vec<(i64, String)>, bool, usize);

/// `DELETE /memory/{id}`
///
/// The promoted chunk's content survives in its HITL
/// decision record (`proposals`, stored verbatim at approve time). The
/// decision record's retention is legitimate — the SILENCE was not. The
/// response now names every retained copy (`retained_proposal_copies`), and
/// `?scrub_proposals=1` replaces the retained content with a dated marker
/// (audit row per proposal, in-tx) when the operator wants erasure to reach
/// the copy too.
pub async fn forget(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<ForgetQuery>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    // AuthZ admin gate — DELETE is Admin (destructive operator action); `None` = superuser.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let actor = super::recall::principal_label(&principal.0);

    let pool = state.pool.clone();

    let outcome = tokio::task::spawn_blocking(
        move || -> Result<ForgetOutcome, HandlerError> {
            let mut conn = pool.get().map_err(HandlerError::db_down)?;
            let tx = conn
                .transaction()
                .map_err(|e| HandlerError::internal(format!("transaction failed: {e}")))?;

            // A held id is frozen against every erasure; refuse in-transaction so
            // `/purge`'s 409 is the single hold-fence envelope.
            crate::legal_hold::refuse_if_held(&tx, &[id])?;

            // Capture the stored content BEFORE the delete so retained
            // proposal copies can be correlated (exact bytes = the promoted-copy
            // link; there is no fk between them by design).
            let content = crate::service::forget::chunk_content(&tx, id);

            let deleted = crate::service::forget::forget_one(&tx, id, &actor)
                .map_err(|e| HandlerError::internal(e.to_string()))?;

            // Disclose (and optionally scrub) the retained decision-record copies
            // in the SAME tx — the disclosure can never lag the erasure.
            let mut retained: Vec<(i64, String)> = Vec::new();
            let mut truncated = false;
            let mut scrubbed_count = 0usize;
            if deleted && let Some(content) = content.as_deref() {
                (retained, truncated) =
                    crate::service::forget::retained_proposal_copies(&tx, content)
                        .map_err(|e| HandlerError::internal(e.to_string()))?;
                if q.scrub_proposals && !retained.is_empty() {
                    let now = chrono::Utc::now().timestamp();
                    let ids: Vec<i64> = retained.iter().map(|(id, _)| *id).collect();
                    scrubbed_count =
                        crate::service::forget::scrub_proposal_content(&tx, &ids, now, &actor)
                            .map_err(|e| HandlerError::internal(e.to_string()))?;
                }
            }
            tx.commit()
                .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
            Ok((deleted, retained, truncated, scrubbed_count))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    let (deleted, retained, truncated, scrubbed_count) = outcome;

    if !deleted {
        return Err(HandlerError::not_found(format!("no memory with id {id}")));
    }

    Ok(Json(serde_json::json!({
        "deleted": true,
        // The honest erasure census: decision records that still carry the
        // content verbatim (empty when none — e.g. direct /add chunks).
        // Correlation is exact bytes (see the core); edited variants are
        // NOT correlated — use `/dsar purge` (subject-wide sweep) when the
        // erasure request is an Art-17-grade demand, or `?scrub_proposals=1`
        // for the verbatim copies named here.
        "retained_proposal_copies": retained
            .into_iter()
            .map(|(id, status)| serde_json::json!({"id": id, "status": status}))
            .collect::<Vec<_>>(),
        "retained_truncated": truncated,
        "scrubbed": q.scrub_proposals,
        "scrubbed_count": scrubbed_count,
    })))
}
