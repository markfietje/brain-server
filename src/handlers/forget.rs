//! `DELETE /memory/{id}` — forget a knowledge entry. Per `API_CONTRACT.md`
//! §4. Cascades to the embedding + owned relations (FK CASCADE / SET NULL).

use axum::extract::{Path, State};
use axum::response::Json;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::auth::OptPrincipal;
use crate::handlers::{ForgetResponse, HandlerError};

/// `DELETE /memory/{id}`
pub async fn forget(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<ForgetResponse>, HandlerError> {
    // AuthZ admin gate — DELETE is Admin (destructive operator action); `None` = superuser.
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;

    let pool = state.pool.clone();

    let deleted = tokio::task::spawn_blocking(move || -> Result<bool, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(format!("transaction failed: {e}")))?;

        // A held id is frozen against every erasure; refuse in-transaction so
        // `/purge`'s 409 is the single hold-fence envelope.
        crate::legal_hold::refuse_if_held(&tx, &[id])?;

        let deleted = crate::service::forget::forget_one(&tx, id)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(deleted)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    if !deleted {
        return Err(HandlerError::not_found(format!("no memory with id {id}")));
    }

    Ok(Json(ForgetResponse { deleted: true }))
}
